//! In-memory LRU cache for parsed grid data.
//!
//! This cache stores parsed grid arrays (Vec<f32>) to avoid repeated
//! parsing of GRIB2/NetCDF files. Particularly beneficial for GOES
//! satellite data which requires expensive ncdump parsing.

use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
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
    pub entries: usize,
    pub total_values_cached: u64,
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
    
    /// Estimated memory usage in MB
    pub fn estimated_memory_mb(&self) -> f64 {
        // Each f32 is 4 bytes
        (self.total_values_cached as f64 * 4.0) / (1024.0 * 1024.0)
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
    /// Cache statistics
    stats: Arc<RwLock<GridCacheStats>>,
    /// Cache capacity
    capacity: usize,
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
            stats: Arc::new(RwLock::new(GridCacheStats::default())),
            capacity,
        }
    }

    /// Get parsed grid data from cache.
    ///
    /// Returns None on cache miss - caller should parse and insert.
    pub async fn get(&self, key: &str) -> Option<CachedGridData> {
        let mut cache = self.cache.write().await;
        
        if let Some(data) = cache.get(key) {
            let mut stats = self.stats.write().await;
            stats.hits += 1;
            Some(data.clone())
        } else {
            let mut stats = self.stats.write().await;
            stats.misses += 1;
            None
        }
    }

    /// Insert parsed grid data into cache.
    pub async fn insert(&self, key: String, data: CachedGridData) {
        let mut cache = self.cache.write().await;
        let values_count = data.data.len() as u64;
        
        cache.put(key, data);
        
        // Update stats
        let mut stats = self.stats.write().await;
        stats.entries = cache.len();
        stats.total_values_cached += values_count;
    }

    /// Get current cache statistics.
    pub async fn stats(&self) -> GridCacheStats {
        let cache = self.cache.read().await;
        let mut stats = self.stats.write().await;
        stats.entries = cache.len();
        stats.clone()
    }

    /// Clear the cache.
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        
        let mut stats = self.stats.write().await;
        *stats = GridCacheStats::default();
    }

    /// Get cache capacity.
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
    }
}
