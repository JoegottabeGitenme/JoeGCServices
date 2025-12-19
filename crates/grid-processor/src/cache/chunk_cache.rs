//! LRU cache for decompressed grid chunks.

use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::types::CacheStats;

/// Cache key for chunks: (zarr_path_hash, chunk_x, chunk_y).
pub type ChunkKey = (u64, usize, usize);

/// LRU cache for decompressed chunks with memory-bounded eviction.
pub struct ChunkCache {
    cache: LruCache<ChunkKey, Vec<f32>>,
    memory_limit: usize,
    current_memory: usize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl ChunkCache {
    /// Create a new chunk cache with the given memory limit in bytes.
    pub fn new(memory_limit: usize) -> Self {
        // Estimate max entries assuming ~1MB per chunk (512×512×4 bytes)
        let chunk_size_estimate = 512 * 512 * 4;
        let max_entries = (memory_limit / chunk_size_estimate).max(16);

        Self {
            cache: LruCache::new(NonZeroUsize::new(max_entries).unwrap()),
            memory_limit,
            current_memory: 0,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// Try to get a chunk from the cache.
    ///
    /// Returns `Some(data)` if found (cache hit), `None` if not found (cache miss).
    pub fn get(&mut self, key: &ChunkKey) -> Option<&Vec<f32>> {
        if let Some(data) = self.cache.get(key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(data)
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Check if a key exists in the cache without updating LRU order.
    pub fn contains(&self, key: &ChunkKey) -> bool {
        self.cache.contains(key)
    }

    /// Insert a chunk into the cache.
    ///
    /// If the cache is at capacity, the least recently used entries
    /// will be evicted to make room.
    pub fn insert(&mut self, key: ChunkKey, data: Vec<f32>) {
        let data_size = data.len() * std::mem::size_of::<f32>();

        // Evict if necessary to make room
        while self.current_memory + data_size > self.memory_limit && !self.cache.is_empty() {
            if let Some((_, evicted)) = self.cache.pop_lru() {
                let evicted_size = evicted.len() * std::mem::size_of::<f32>();
                self.current_memory = self.current_memory.saturating_sub(evicted_size);
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Only insert if the data fits (or cache was empty)
        if data_size <= self.memory_limit {
            self.cache.put(key, data);
            self.current_memory += data_size;
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries: self.cache.len(),
            memory_bytes: self.current_memory as u64,
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }

    /// Clear all entries from the cache.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.current_memory = 0;
    }

    /// Evict entries to reach target memory usage.
    ///
    /// Returns the number of entries evicted.
    pub fn evict_to_target(&mut self, target_bytes: usize) -> usize {
        let mut evicted = 0;
        while self.current_memory > target_bytes && !self.cache.is_empty() {
            if let Some((_, data)) = self.cache.pop_lru() {
                let data_size = data.len() * std::mem::size_of::<f32>();
                self.current_memory = self.current_memory.saturating_sub(data_size);
                self.evictions.fetch_add(1, Ordering::Relaxed);
                evicted += 1;
            }
        }
        evicted
    }

    /// Get the current memory usage in bytes.
    pub fn memory_usage(&self) -> usize {
        self.current_memory
    }

    /// Get the memory limit in bytes.
    pub fn memory_limit(&self) -> usize {
        self.memory_limit
    }

    /// Get the number of entries in the cache.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

/// Helper function to compute a hash for the zarr path.
/// Used as part of the cache key to distinguish chunks from different grids.
pub fn hash_path(path: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = ChunkCache::new(1024 * 1024); // 1MB

        let key = (123, 0, 0);
        let data: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];

        assert!(cache.get(&key).is_none());
        cache.insert(key, data.clone());
        assert_eq!(cache.get(&key), Some(&data));
    }

    #[test]
    fn test_cache_lru_eviction() {
        // Small cache that can only hold a few entries
        let mut cache = ChunkCache::new(64); // 64 bytes = 16 f32s max

        // Insert entries that will fill the cache
        for i in 0..10 {
            let key = (0, i, 0);
            let data: Vec<f32> = vec![i as f32; 4]; // 16 bytes each
            cache.insert(key, data);
        }

        // Earlier entries should have been evicted
        assert!(cache.get(&(0, 0, 0)).is_none());

        // Later entries should still be present
        assert!(cache.get(&(0, 9, 0)).is_some());

        // Check eviction count
        let stats = cache.stats();
        assert!(stats.evictions > 0);
    }

    #[test]
    fn test_cache_stats() {
        let mut cache = ChunkCache::new(1024 * 1024);

        // Generate some hits and misses
        let key1 = (0, 0, 0);
        let key2 = (0, 1, 0);
        let data: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];

        cache.insert(key1, data.clone());

        // Hit
        cache.get(&key1);
        // Miss
        cache.get(&key2);

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.entries, 1);
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = ChunkCache::new(1024 * 1024);

        let key = (0, 0, 0);
        let data: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
        cache.insert(key, data);

        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.memory_usage(), 0);
    }

    #[test]
    fn test_evict_to_target() {
        let mut cache = ChunkCache::new(1024);

        // Fill cache with several entries
        for i in 0..10 {
            let key = (0, i, 0);
            let data: Vec<f32> = vec![i as f32; 16]; // 64 bytes each
            cache.insert(key, data);
        }

        let before = cache.memory_usage();
        let evicted = cache.evict_to_target(before / 2);
        assert!(evicted > 0);
        assert!(cache.memory_usage() <= before / 2);
    }

    #[test]
    fn test_hash_path() {
        let hash1 = hash_path("grids/gfs/20241212/TMP.zarr");
        let hash2 = hash_path("grids/gfs/20241212/TMP.zarr");
        let hash3 = hash_path("grids/hrrr/20241212/TMP.zarr");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}
