//! In-memory LRU cache for rendered tiles.
//!
//! This provides sub-millisecond access to recently rendered tiles,
//! complementing the Redis-based L2 cache.
//!
//! ## Memory-Based Eviction
//!
//! The cache uses memory-based eviction rather than entry count. When the
//! cache exceeds its configured memory limit, it evicts ~5% of entries
//! (by memory) in a batch to make room for new tiles.
//!
//! ## Metrics
//!
//! The cache tracks comprehensive metrics for Prometheus/Grafana:
//! - `size_bytes`: Current memory usage
//! - `entry_count`: Number of cached tiles
//! - `hits`/`misses`: Cache hit rate
//! - `evictions`: Total entries evicted
//! - `eviction_runs`: Number of batch eviction events
//! - `bytes_evicted_total`: Total bytes evicted (for rate tracking)

use bytes::Bytes;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::info;

/// In-memory LRU cache for rendered tiles.
///
/// Design considerations:
/// - RwLock for concurrent reads (common case: cache hits)
/// - Memory-based eviction when size exceeds limit
/// - Batch eviction (5% at a time) for efficiency during bursts
/// - TTL enforcement on read (lazy expiration)
/// - Comprehensive metrics for observability
pub struct TileMemoryCache {
    cache: Arc<RwLock<LruCache<String, CachedTile>>>,
    max_bytes: u64,
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

/// Statistics for the tile memory cache.
///
/// All fields are atomic for lock-free reads from metrics endpoints.
#[derive(Default)]
pub struct TileMemoryCacheStats {
    /// Total cache hits
    pub hits: AtomicU64,
    /// Total cache misses
    pub misses: AtomicU64,
    /// Total entries evicted (individual count)
    pub evictions: AtomicU64,
    /// Total entries expired via TTL
    pub expired: AtomicU64,
    /// Current cache size in bytes
    pub size_bytes: AtomicU64,
    /// Current number of entries in cache
    pub entry_count: AtomicU64,
    /// Number of batch eviction runs
    pub eviction_runs: AtomicU64,
    /// Total bytes evicted (for rate tracking)
    pub bytes_evicted_total: AtomicU64,
}

impl TileMemoryCacheStats {
    /// Calculate cache hit rate as a percentage (0-100).
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

    /// Get current entry count.
    pub fn entry_count(&self) -> u64 {
        self.entry_count.load(Ordering::Relaxed)
    }

    /// Get number of eviction runs.
    pub fn eviction_runs(&self) -> u64 {
        self.eviction_runs.load(Ordering::Relaxed)
    }

    /// Get total bytes evicted.
    pub fn bytes_evicted_total(&self) -> u64 {
        self.bytes_evicted_total.load(Ordering::Relaxed)
    }

    /// Get current size in bytes.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes.load(Ordering::Relaxed)
    }
}

impl TileMemoryCache {
    /// Create new cache with specified memory limit.
    ///
    /// The cache will evict LRU entries when memory usage exceeds the limit.
    /// Eviction happens in batches of ~5% of the max size for efficiency.
    ///
    /// # Arguments
    /// * `max_size_mb` - Maximum cache size in megabytes (e.g., 1024 for 1GB)
    /// * `default_ttl_secs` - Default TTL for entries in seconds
    ///
    /// # Example
    /// ```
    /// use storage::TileMemoryCache;
    ///
    /// // Create a 1GB cache with 5 minute TTL
    /// let cache = TileMemoryCache::new(1024, 300);
    /// ```
    pub fn new(max_size_mb: usize, default_ttl_secs: u64) -> Self {
        // Use effectively unbounded LruCache since we manage eviction based on memory ourselves.
        // 10 million entry capacity is chosen because:
        // - LruCache requires an entry limit, but we don't want it to evict (we do memory-based eviction)
        // - At ~30KB average tile size, a 10GB cache would hold ~350K entries
        // - 10M provides 30x headroom for smaller tiles or larger caches
        // - Memory overhead is minimal (just the LruCache internal bookkeeping)
        const LRU_CAPACITY: usize = 10_000_000;
        let cache_size = NonZeroUsize::new(LRU_CAPACITY).expect("Capacity must be > 0");

        Self {
            cache: Arc::new(RwLock::new(LruCache::new(cache_size))),
            max_bytes: (max_size_mb as u64) * 1024 * 1024,
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
                self.stats.entry_count.fetch_sub(1, Ordering::Relaxed);
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
    /// If the cache would exceed its memory limit, a batch eviction is triggered
    /// first, removing ~5% of entries (by memory) to make room.
    pub async fn set(&self, key: &str, data: Bytes, ttl: Option<Duration>) {
        let tile_size = data.len() as u64;

        let mut cache = self.cache.write().await;

        // Check if we need to evict before inserting (done inside lock to avoid race)
        let current_bytes = self.stats.size_bytes.load(Ordering::Relaxed);
        if current_bytes + tile_size > self.max_bytes {
            self.evict_batch_locked(&mut cache);
        }

        // Check if we're replacing an existing entry
        if let Some(existing) = cache.peek(key) {
            let existing_size = existing.data.len() as u64;
            self.stats
                .size_bytes
                .fetch_sub(existing_size, Ordering::Relaxed);
            // entry_count stays the same since we're replacing
        } else {
            // New entry
            self.stats.entry_count.fetch_add(1, Ordering::Relaxed);
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

    /// Evict ~5% of cache capacity (by memory) using LRU order.
    ///
    /// This version takes a mutable reference to an already-locked cache,
    /// avoiding a potential race condition between checking size and evicting.
    ///
    /// Returns (entries_evicted, bytes_freed).
    fn evict_batch_locked(
        &self,
        cache: &mut LruCache<String, CachedTile>,
    ) -> (usize, u64) {
        let current_bytes = self.stats.size_bytes.load(Ordering::Relaxed);

        // Target: free 5% of max capacity
        let target_free = self.max_bytes / 20;
        let mut bytes_freed = 0u64;
        let mut entries_evicted = 0usize;

        while bytes_freed < target_free {
            if let Some((_, evicted)) = cache.pop_lru() {
                let entry_size = evicted.data.len() as u64;
                bytes_freed += entry_size;
                entries_evicted += 1;
            } else {
                break; // Cache is empty
            }
        }

        // Update stats
        self.stats
            .size_bytes
            .fetch_sub(bytes_freed, Ordering::Relaxed);
        self.stats
            .entry_count
            .fetch_sub(entries_evicted as u64, Ordering::Relaxed);
        self.stats
            .evictions
            .fetch_add(entries_evicted as u64, Ordering::Relaxed);
        self.stats.eviction_runs.fetch_add(1, Ordering::Relaxed);
        self.stats
            .bytes_evicted_total
            .fetch_add(bytes_freed, Ordering::Relaxed);

        // Log the eviction
        info!(
            entries_evicted = entries_evicted,
            bytes_freed_mb = format!("{:.2}", bytes_freed as f64 / (1024.0 * 1024.0)),
            cache_size_mb =
                format!("{:.2}", (current_bytes - bytes_freed) as f64 / (1024.0 * 1024.0)),
            max_size_mb = format!("{:.2}", self.max_bytes as f64 / (1024.0 * 1024.0)),
            "L1 cache batch eviction completed"
        );

        (entries_evicted, bytes_freed)
    }

    /// Get current statistics.
    pub fn stats(&self) -> TileMemoryCacheStats {
        TileMemoryCacheStats {
            hits: AtomicU64::new(self.stats.hits.load(Ordering::Relaxed)),
            misses: AtomicU64::new(self.stats.misses.load(Ordering::Relaxed)),
            evictions: AtomicU64::new(self.stats.evictions.load(Ordering::Relaxed)),
            expired: AtomicU64::new(self.stats.expired.load(Ordering::Relaxed)),
            size_bytes: AtomicU64::new(self.stats.size_bytes.load(Ordering::Relaxed)),
            entry_count: AtomicU64::new(self.stats.entry_count.load(Ordering::Relaxed)),
            eviction_runs: AtomicU64::new(self.stats.eviction_runs.load(Ordering::Relaxed)),
            bytes_evicted_total: AtomicU64::new(
                self.stats.bytes_evicted_total.load(Ordering::Relaxed),
            ),
        }
    }

    /// Current number of entries in cache.
    pub async fn len(&self) -> usize {
        self.stats.entry_count.load(Ordering::Relaxed) as usize
    }

    /// Check if cache is empty.
    pub async fn is_empty(&self) -> bool {
        self.stats.entry_count.load(Ordering::Relaxed) == 0
    }

    /// Get maximum cache size in bytes.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Get current cache utilization ratio (0.0 - 1.0).
    pub fn utilization(&self) -> f64 {
        let size = self.stats.size_bytes.load(Ordering::Relaxed);
        if self.max_bytes == 0 {
            0.0
        } else {
            size as f64 / self.max_bytes as f64
        }
    }

    /// Get current cache size in bytes.
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
        self.stats.entry_count.store(0, Ordering::Relaxed);
        self.stats.eviction_runs.store(0, Ordering::Relaxed);
        self.stats.bytes_evicted_total.store(0, Ordering::Relaxed);
    }

    /// Evict a percentage of entries (0.0 to 1.0) using LRU order.
    /// Returns the number of entries evicted.
    ///
    /// This is used by the memory pressure monitor to free memory.
    pub async fn evict_percentage(&self, percentage: f64) -> usize {
        let mut cache = self.cache.write().await;
        let entry_count = self.stats.entry_count.load(Ordering::Relaxed);
        let entries_to_evict = (entry_count as f64 * percentage.clamp(0.0, 1.0)) as usize;
        let mut evicted_count = 0;
        let mut bytes_freed = 0u64;

        for _ in 0..entries_to_evict {
            if let Some((_, evicted)) = cache.pop_lru() {
                bytes_freed += evicted.data.len() as u64;
                evicted_count += 1;
            } else {
                break;
            }
        }

        // Update stats
        if bytes_freed > 0 {
            self.stats
                .size_bytes
                .fetch_sub(bytes_freed, Ordering::Relaxed);
        }
        self.stats
            .entry_count
            .fetch_sub(evicted_count as u64, Ordering::Relaxed);
        self.stats
            .evictions
            .fetch_add(evicted_count as u64, Ordering::Relaxed);
        self.stats.eviction_runs.fetch_add(1, Ordering::Relaxed);
        self.stats
            .bytes_evicted_total
            .fetch_add(bytes_freed, Ordering::Relaxed);

        info!(
            evicted_entries = evicted_count,
            bytes_freed_mb = format!("{:.2}", bytes_freed as f64 / (1024.0 * 1024.0)),
            trigger = "memory_pressure",
            "L1 cache eviction completed"
        );

        evicted_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_basic_operations() {
        let cache = TileMemoryCache::new(100, 60); // 100MB, 60s TTL

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
        assert_eq!(stats.entry_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_cache_ttl_expiration() {
        let cache = TileMemoryCache::new(100, 1); // 100MB, 1s TTL

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
        assert_eq!(stats.entry_count.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_cache_memory_based_eviction() {
        // Create 1MB cache
        let cache = TileMemoryCache::new(1, 60);

        // Add tiles until we exceed limit (each tile is 100KB)
        let tile_100kb = Bytes::from(vec![0u8; 100 * 1024]);
        for i in 0..15 {
            // 15 × 100KB = 1.5MB > 1MB limit
            cache
                .set(&format!("tile{}", i), tile_100kb.clone(), None)
                .await;
        }

        // Should have evicted some entries
        let stats = cache.stats();
        assert!(stats.evictions.load(Ordering::Relaxed) > 0);
        assert!(stats.eviction_runs.load(Ordering::Relaxed) > 0);
        assert!(stats.size_bytes.load(Ordering::Relaxed) <= 1024 * 1024);
    }

    #[tokio::test]
    async fn test_cache_entry_count_tracking() {
        let cache = TileMemoryCache::new(100, 60);

        cache.set("tile1", Bytes::from("data1"), None).await;
        cache.set("tile2", Bytes::from("data2"), None).await;

        let stats = cache.stats();
        assert_eq!(stats.entry_count.load(Ordering::Relaxed), 2);

        // Replace an entry - count should stay the same
        cache.set("tile1", Bytes::from("new data1"), None).await;
        let stats = cache.stats();
        assert_eq!(stats.entry_count.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn test_cache_size_tracking() {
        let cache = TileMemoryCache::new(100, 60);

        let data1 = Bytes::from("hello"); // 5 bytes
        let data2 = Bytes::from("world!"); // 6 bytes

        cache.set("tile1", data1, None).await;
        assert_eq!(cache.size_bytes(), 5);

        cache.set("tile2", data2, None).await;
        assert_eq!(cache.size_bytes(), 11);

        // Replace tile1 with larger data
        let data3 = Bytes::from("hello world"); // 11 bytes
        cache.set("tile1", data3, None).await;
        assert_eq!(cache.size_bytes(), 17); // 11 + 6
    }

    #[tokio::test]
    async fn test_cache_utilization() {
        // 1MB cache
        let cache = TileMemoryCache::new(1, 60);
        assert_eq!(cache.utilization(), 0.0);

        // Add 512KB of data
        let data = Bytes::from(vec![0u8; 512 * 1024]);
        cache.set("tile1", data, None).await;

        let util = cache.utilization();
        assert!((util - 0.5).abs() < 0.01); // Should be ~50%
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let cache = TileMemoryCache::new(100, 60);

        cache.set("tile1", Bytes::from("data1"), None).await;
        cache.set("tile2", Bytes::from("data2"), None).await;
        assert_eq!(cache.len().await, 2);

        cache.clear().await;
        assert!(cache.is_empty().await);
        assert_eq!(cache.size_bytes(), 0);

        // Stats should be reset
        let stats = cache.stats();
        assert_eq!(stats.hits.load(Ordering::Relaxed), 0);
        assert_eq!(stats.misses.load(Ordering::Relaxed), 0);
        assert_eq!(stats.entry_count.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_evict_percentage() {
        let cache = TileMemoryCache::new(100, 60);

        // Add 10 tiles
        for i in 0..10 {
            cache
                .set(&format!("tile{}", i), Bytes::from("data"), None)
                .await;
        }
        assert_eq!(cache.len().await, 10);

        // Evict 50%
        let evicted = cache.evict_percentage(0.5).await;
        assert_eq!(evicted, 5);
        assert_eq!(cache.len().await, 5);

        let stats = cache.stats();
        assert_eq!(stats.eviction_runs.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_max_bytes() {
        let cache = TileMemoryCache::new(512, 60); // 512MB
        assert_eq!(cache.max_bytes(), 512 * 1024 * 1024);
    }
}
