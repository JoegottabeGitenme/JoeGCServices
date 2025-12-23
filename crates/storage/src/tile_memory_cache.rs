//! In-memory LRU cache for rendered tiles.
//!
//! This provides sub-millisecond access to recently rendered tiles,
//! complementing the Redis-based L2 cache.

use bytes::Bytes;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// In-memory LRU cache for rendered tiles.
///
/// Design considerations:
/// - RwLock for concurrent reads (common case: cache hits)
/// - LRU eviction when capacity is reached
/// - TTL enforcement on read (lazy expiration)
/// - Metrics for hit/miss/eviction tracking
pub struct TileMemoryCache {
    cache: Arc<RwLock<LruCache<String, CachedTile>>>,
    capacity: usize,
    default_ttl: Duration,
    stats: Arc<TileMemoryCacheStats>,
}

struct CachedTile {
    data: Bytes,
    inserted_at: Instant,
    ttl: Duration,
}

impl CachedTile {
    fn is_expired(&self) -> bool {
        self.inserted_at.elapsed() > self.ttl
    }
}

#[derive(Default)]
pub struct TileMemoryCacheStats {
    pub hits: AtomicU64,
    pub misses: AtomicU64,
    pub evictions: AtomicU64,
    pub expired: AtomicU64,
    pub size_bytes: AtomicU64,
}

impl TileMemoryCacheStats {
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            (hits as f64 / total as f64) * 100.0
        }
    }
}

impl TileMemoryCache {
    /// Create new cache with specified capacity.
    ///
    /// Memory estimation:
    /// - 1,000 tiles × 30KB avg = ~30MB
    /// - 5,000 tiles × 30KB avg = ~150MB
    /// - 10,000 tiles × 30KB avg = ~300MB
    /// - 25,000 tiles × 30KB avg = ~750MB
    pub fn new(capacity: usize, default_ttl_secs: u64) -> Self {
        let cache_size = NonZeroUsize::new(capacity).expect("Capacity must be > 0");

        Self {
            cache: Arc::new(RwLock::new(LruCache::new(cache_size))),
            capacity,
            default_ttl: Duration::from_secs(default_ttl_secs),
            stats: Arc::new(TileMemoryCacheStats::default()),
        }
    }

    /// Get tile from cache (returns None if expired or missing).
    ///
    /// This method:
    /// 1. Checks if tile exists in cache
    /// 2. Validates TTL (lazy expiration)
    /// 3. Updates hit/miss statistics
    pub async fn get(&self, key: &str) -> Option<Bytes> {
        // Try to read from cache
        let mut cache = self.cache.write().await;

        if let Some(cached_tile) = cache.get(key) {
            // Check if expired
            if cached_tile.is_expired() {
                // Get size before removing
                let tile_size = cached_tile.data.len() as u64;
                // Remove expired tile
                cache.pop(key);
                self.stats.expired.fetch_add(1, Ordering::Relaxed);
                self.stats.misses.fetch_add(1, Ordering::Relaxed);
                self.stats
                    .size_bytes
                    .fetch_sub(tile_size, Ordering::Relaxed);
                None
            } else {
                // Cache hit
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                Some(cached_tile.data.clone())
            }
        } else {
            // Cache miss
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Store tile in cache.
    ///
    /// If cache is at capacity, the least recently used tile will be evicted.
    pub async fn set(&self, key: &str, data: Bytes, ttl: Option<Duration>) {
        let mut cache = self.cache.write().await;
        let tile_size = data.len() as u64;

        // Check if we'll evict an entry and track its size
        if cache.len() >= self.capacity {
            if let Some((_, lru_entry)) = cache.peek_lru() {
                let evicted_size = lru_entry.data.len() as u64;
                self.stats
                    .size_bytes
                    .fetch_sub(evicted_size, Ordering::Relaxed);
            }
            self.stats.evictions.fetch_add(1, Ordering::Relaxed);
        }

        // Insert new tile
        let cached_tile = CachedTile {
            data,
            inserted_at: Instant::now(),
            ttl: ttl.unwrap_or(self.default_ttl),
        };

        cache.put(key.to_string(), cached_tile);

        // Update size metric
        self.stats
            .size_bytes
            .fetch_add(tile_size, Ordering::Relaxed);
    }

    /// Get current statistics.
    pub fn stats(&self) -> TileMemoryCacheStats {
        TileMemoryCacheStats {
            hits: AtomicU64::new(self.stats.hits.load(Ordering::Relaxed)),
            misses: AtomicU64::new(self.stats.misses.load(Ordering::Relaxed)),
            evictions: AtomicU64::new(self.stats.evictions.load(Ordering::Relaxed)),
            expired: AtomicU64::new(self.stats.expired.load(Ordering::Relaxed)),
            size_bytes: AtomicU64::new(self.stats.size_bytes.load(Ordering::Relaxed)),
        }
    }

    /// Current number of entries in cache.
    pub async fn len(&self) -> usize {
        self.cache.read().await.len()
    }

    /// Check if cache is empty.
    pub async fn is_empty(&self) -> bool {
        self.cache.read().await.is_empty()
    }

    /// Get cache capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Estimated memory usage in bytes.
    ///
    /// Note: This is an approximation based on inserted tiles.
    /// It doesn't account for evictions, so may be higher than actual usage.
    pub fn size_bytes(&self) -> u64 {
        self.stats.size_bytes.load(Ordering::Relaxed)
    }

    /// Clear all cached entries and reset statistics.
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();

        // Reset stats
        self.stats.hits.store(0, Ordering::Relaxed);
        self.stats.misses.store(0, Ordering::Relaxed);
        self.stats.evictions.store(0, Ordering::Relaxed);
        self.stats.expired.store(0, Ordering::Relaxed);
        self.stats.size_bytes.store(0, Ordering::Relaxed);
    }

    /// Evict a percentage of entries (0.0 to 1.0) using LRU order.
    /// Returns the number of entries evicted.
    ///
    /// This is used by the memory pressure monitor to free memory.
    pub async fn evict_percentage(&self, percentage: f64) -> usize {
        let mut cache = self.cache.write().await;
        let entries_to_evict = (cache.len() as f64 * percentage.clamp(0.0, 1.0)) as usize;
        let mut evicted_count = 0;
        let mut bytes_freed = 0u64;

        for _ in 0..entries_to_evict {
            if let Some((_, evicted)) = cache.pop_lru() {
                bytes_freed += evicted.data.len() as u64;
                self.stats.evictions.fetch_add(1, Ordering::Relaxed);
                evicted_count += 1;
            } else {
                break;
            }
        }

        // Subtract freed bytes from size tracking
        if bytes_freed > 0 {
            let current = self.stats.size_bytes.load(Ordering::Relaxed);
            self.stats
                .size_bytes
                .store(current.saturating_sub(bytes_freed), Ordering::Relaxed);
        }

        evicted_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_basic_operations() {
        let cache = TileMemoryCache::new(10, 60);

        // Test empty cache
        assert!(cache.is_empty().await);
        assert_eq!(cache.len().await, 0);

        // Test cache miss
        assert!(cache.get("tile1").await.is_none());

        // Test cache set and hit
        let data = Bytes::from("test data");
        cache.set("tile1", data.clone(), None).await;
        assert_eq!(cache.len().await, 1);

        let retrieved = cache.get("tile1").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), data);

        // Check stats
        let stats = cache.stats();
        assert_eq!(stats.hits.load(Ordering::Relaxed), 1);
        assert_eq!(stats.misses.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_cache_ttl_expiration() {
        let cache = TileMemoryCache::new(10, 1); // 1 second TTL

        let data = Bytes::from("test data");
        cache
            .set("tile1", data.clone(), Some(Duration::from_millis(100)))
            .await;

        // Should hit immediately
        assert!(cache.get("tile1").await.is_some());

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should be expired now
        assert!(cache.get("tile1").await.is_none());

        // Check expired stat
        let stats = cache.stats();
        assert_eq!(stats.expired.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_cache_lru_eviction() {
        let cache = TileMemoryCache::new(2, 60); // Capacity of 2

        // Fill cache
        cache.set("tile1", Bytes::from("data1"), None).await;
        cache.set("tile2", Bytes::from("data2"), None).await;
        assert_eq!(cache.len().await, 2);

        // Add third tile, should evict tile1 (least recently used)
        cache.set("tile3", Bytes::from("data3"), None).await;
        assert_eq!(cache.len().await, 2);

        // tile1 should be gone
        assert!(cache.get("tile1").await.is_none());

        // tile2 and tile3 should still be there
        assert!(cache.get("tile2").await.is_some());
        assert!(cache.get("tile3").await.is_some());

        // Check eviction stat
        let stats = cache.stats();
        assert_eq!(stats.evictions.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let cache = TileMemoryCache::new(10, 60);

        cache.set("tile1", Bytes::from("data1"), None).await;
        cache.set("tile2", Bytes::from("data2"), None).await;
        assert_eq!(cache.len().await, 2);

        cache.clear().await;
        assert!(cache.is_empty().await);

        // Stats should be reset
        let stats = cache.stats();
        assert_eq!(stats.hits.load(Ordering::Relaxed), 0);
        assert_eq!(stats.misses.load(Ordering::Relaxed), 0);
    }
}
