//! In-memory LRU cache for parsed grid data.
//!
//! This cache stores parsed grid arrays (Vec<f32>) to avoid repeated
//! parsing of GRIB2/NetCDF files. Particularly beneficial for GOES
//! satellite data which requires expensive ncdump parsing.

use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

/// Cached grid data with projection parameters
#[derive(Clone, Debug)]
pub struct CachedGridData {
    /// Grid values as f32
    pub data: Arc<Vec<f32>>,
    /// Grid width
    pub width: usize,
    /// Grid height  
    pub height: usize,
    /// GOES projection parameters (if applicable)
    pub goes_projection: Option<GoesProjectionParams>,
}

impl CachedGridData {
    /// Calculate memory usage in bytes for this cached entry
    pub fn memory_bytes(&self) -> u64 {
        // Vec<f32>: 4 bytes per element
        // Plus Arc overhead (~16 bytes) and struct overhead
        (self.data.len() * 4 + 64) as u64
    }
}

/// GOES projection parameters for coordinate transforms
#[derive(Clone, Debug)]
pub struct GoesProjectionParams {
    pub x_origin: f64,
    pub y_origin: f64,
    pub dx: f64,
    pub dy: f64,
    pub perspective_point_height: f64,
    pub semi_major_axis: f64,
    pub semi_minor_axis: f64,
    pub longitude_origin: f64,
}

/// Statistics for the grid data cache
#[derive(Debug, Default, Clone)]
pub struct GridCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
    pub capacity: usize,
    pub memory_bytes: u64,
}

impl GridCacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
    
    /// Memory usage in MB
    pub fn memory_mb(&self) -> f64 {
        self.memory_bytes as f64 / (1024.0 * 1024.0)
    }
    
    /// Cache utilization percentage (entries / capacity)
    pub fn utilization(&self) -> f64 {
        if self.capacity == 0 {
            0.0
        } else {
            (self.entries as f64 / self.capacity as f64) * 100.0
        }
    }
}

/// In-memory LRU cache for parsed grid data.
///
/// Caches parsed grid arrays to avoid repeated parsing of source files.
/// Especially important for NetCDF (GOES) which uses slow ncdump parsing.
///
/// # Cache Key Format
/// Keys should be unique per dataset+time, e.g.:
/// - "goes18/20251202_03z/cmi_c13.nc"
/// - "gfs/20251202_00z/TMP_2m"
pub struct GridDataCache {
    /// LRU cache: path -> parsed grid data
    cache: Arc<RwLock<LruCache<String, CachedGridData>>>,
    /// Cache capacity (entry count)
    capacity: usize,
    /// Atomic counters for lock-free stats access
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    memory_bytes: AtomicU64,
}

impl GridDataCache {
    /// Create a new grid data cache.
    ///
    /// # Arguments
    /// * `capacity` - Maximum number of grids to cache
    ///
    /// # Memory Guidelines
    /// GOES CONUS: 2500 x 1500 = 3.75M values = 15 MB per grid
    /// - capacity=50: ~750 MB
    /// - capacity=100: ~1.5 GB
    /// - capacity=200: ~3 GB
    pub fn new(capacity: usize) -> Self {
        let cache_size = NonZeroUsize::new(capacity.max(1)).expect("Capacity must be > 0");
        
        Self {
            cache: Arc::new(RwLock::new(LruCache::new(cache_size))),
            capacity,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            memory_bytes: AtomicU64::new(0),
        }
    }

    /// Get parsed grid data from cache.
    ///
    /// Returns None on cache miss - caller should parse and insert.
    pub async fn get(&self, key: &str) -> Option<CachedGridData> {
        let mut cache = self.cache.write().await;
        
        if let Some(data) = cache.get(key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(data.clone())
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Insert parsed grid data into cache.
    pub async fn insert(&self, key: String, data: CachedGridData) {
        let mut cache = self.cache.write().await;
        let new_entry_bytes = data.memory_bytes();
        
        // Check if we'll evict an entry
        if cache.len() >= cache.cap().get() {
            // Get the LRU entry that will be evicted
            if let Some((_, evicted)) = cache.peek_lru() {
                let evicted_bytes = evicted.memory_bytes();
                self.memory_bytes.fetch_sub(evicted_bytes, Ordering::Relaxed);
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
        
        cache.put(key, data);
        self.memory_bytes.fetch_add(new_entry_bytes, Ordering::Relaxed);
    }

    /// Get current cache statistics.
    pub async fn stats(&self) -> GridCacheStats {
        let cache = self.cache.read().await;
        GridCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            entries: cache.len(),
            capacity: self.capacity,
            memory_bytes: self.memory_bytes.load(Ordering::Relaxed),
        }
    }

    /// Clear the cache.
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        self.memory_bytes.store(0, Ordering::Relaxed);
        // Note: we don't reset hits/misses/evictions - they're cumulative
    }

    /// Get cache capacity (entry count limit).
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get current number of entries in cache.
    pub async fn len(&self) -> usize {
        self.cache.read().await.len()
    }

    /// Check if cache is empty.
    pub async fn is_empty(&self) -> bool {
        self.cache.read().await.is_empty()
    }
    
    /// Get current memory usage in bytes.
    pub fn memory_bytes(&self) -> u64 {
        self.memory_bytes.load(Ordering::Relaxed)
    }
    
    /// Evict entries until memory usage is below the target bytes.
    /// Returns the number of entries evicted.
    pub async fn evict_to_memory_target(&self, target_bytes: u64) -> usize {
        let mut cache = self.cache.write().await;
        let mut evicted_count = 0;
        
        while self.memory_bytes.load(Ordering::Relaxed) > target_bytes && !cache.is_empty() {
            if let Some((_, evicted)) = cache.pop_lru() {
                let evicted_bytes = evicted.memory_bytes();
                self.memory_bytes.fetch_sub(evicted_bytes, Ordering::Relaxed);
                self.evictions.fetch_add(1, Ordering::Relaxed);
                evicted_count += 1;
            } else {
                break;
            }
        }
        
        evicted_count
    }
    
    /// Evict a percentage of entries (0.0 to 1.0).
    /// Returns the number of entries evicted.
    pub async fn evict_percentage(&self, percentage: f64) -> usize {
        let mut cache = self.cache.write().await;
        let entries_to_evict = (cache.len() as f64 * percentage.clamp(0.0, 1.0)) as usize;
        let mut evicted_count = 0;
        
        for _ in 0..entries_to_evict {
            if let Some((_, evicted)) = cache.pop_lru() {
                let evicted_bytes = evicted.memory_bytes();
                self.memory_bytes.fetch_sub(evicted_bytes, Ordering::Relaxed);
                self.evictions.fetch_add(1, Ordering::Relaxed);
                evicted_count += 1;
            } else {
                break;
            }
        }
        
        evicted_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_hit_miss() {
        let cache = GridDataCache::new(10);
        
        // Miss on empty cache
        assert!(cache.get("test_key").await.is_none());
        
        let stats = cache.stats().await;
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);
        
        // Insert data
        let data = CachedGridData {
            data: Arc::new(vec![1.0, 2.0, 3.0]),
            width: 3,
            height: 1,
            goes_projection: None,
        };
        cache.insert("test_key".to_string(), data).await;
        
        // Hit on populated cache
        let result = cache.get("test_key").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().data.len(), 3);
        
        let stats = cache.stats().await;
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 1);
        assert!(stats.memory_bytes > 0);
    }

    #[tokio::test]
    async fn test_lru_eviction() {
        let cache = GridDataCache::new(2);
        
        // Insert 3 items into capacity-2 cache
        for i in 0..3 {
            let data = CachedGridData {
                data: Arc::new(vec![i as f32]),
                width: 1,
                height: 1,
                goes_projection: None,
            };
            cache.insert(format!("key_{}", i), data).await;
        }
        
        // First key should be evicted
        assert!(cache.get("key_0").await.is_none());
        // Last two should still be present
        assert!(cache.get("key_1").await.is_some());
        assert!(cache.get("key_2").await.is_some());
        
        // Check eviction was tracked
        let stats = cache.stats().await;
        assert_eq!(stats.evictions, 1);
    }
    
    #[tokio::test]
    async fn test_memory_tracking() {
        let cache = GridDataCache::new(10);
        
        // Insert a known-size grid
        let data = CachedGridData {
            data: Arc::new(vec![0.0f32; 1000]), // 1000 floats = 4000 bytes + overhead
            width: 100,
            height: 10,
            goes_projection: None,
        };
        cache.insert("test".to_string(), data).await;
        
        let stats = cache.stats().await;
        assert!(stats.memory_bytes >= 4000); // At least 4KB for the data
        assert!(stats.memory_bytes < 8000); // But not too much overhead
    }
    
    #[tokio::test]
    async fn test_evict_percentage() {
        let cache = GridDataCache::new(10);
        
        // Insert 10 items
        for i in 0..10 {
            let data = CachedGridData {
                data: Arc::new(vec![i as f32; 100]),
                width: 10,
                height: 10,
                goes_projection: None,
            };
            cache.insert(format!("key_{}", i), data).await;
        }
        
        assert_eq!(cache.len().await, 10);
        
        // Evict 50%
        let evicted = cache.evict_percentage(0.5).await;
        assert_eq!(evicted, 5);
        assert_eq!(cache.len().await, 5);
    }
}
