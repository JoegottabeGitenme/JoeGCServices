//! In-memory LRU cache for GRIB data to reduce MinIO reads.

use bytes::Bytes;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::Mutex;
use wms_common::WmsResult;

use crate::ObjectStorage;

/// In-memory LRU cache for GRIB file data.
/// 
/// This cache stores raw GRIB file bytes to avoid repeated MinIO fetches
/// for frequently accessed datasets. Particularly beneficial for:
/// - Real-time/recent data (e.g., latest GFS run)
/// - Popular layers (temperature, wind, radar)
/// - Tiles at common zoom levels
pub struct GribCache {
    /// LRU cache storing path -> GRIB bytes
    cache: Arc<Mutex<LruCache<String, Bytes>>>,
    /// Reference to MinIO storage for cache misses
    storage: Arc<ObjectStorage>,
    /// Cache statistics
    stats: Arc<Mutex<CacheStats>>,
    /// Cache capacity (stored separately for easy access)
    capacity: usize,
}

#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub total_bytes_cached: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

impl GribCache {
    /// Create a new GRIB cache with specified capacity.
    /// 
    /// # Arguments
    /// * `capacity` - Maximum number of GRIB files to cache
    /// * `storage` - ObjectStorage instance for cache misses
    /// 
    /// # Capacity Guidelines
    /// - Small (100 entries): ~500MB RAM (assuming ~5MB per GRIB)
    /// - Medium (500 entries): ~2.5GB RAM
    /// - Large (1000 entries): ~5GB RAM
    pub fn new(capacity: usize, storage: Arc<ObjectStorage>) -> Self {
        let cache_size = NonZeroUsize::new(capacity).expect("Capacity must be > 0");
        
        Self {
            cache: Arc::new(Mutex::new(LruCache::new(cache_size))),
            storage,
            stats: Arc::new(Mutex::new(CacheStats::default())),
            capacity,
        }
    }

    /// Get GRIB data from cache or fetch from storage.
    /// 
    /// This method:
    /// 1. Checks the in-memory cache first (fast path)
    /// 2. On cache miss, fetches from MinIO
    /// 3. Stores fetched data in cache for future requests
    /// 4. Updates cache statistics
    pub async fn get(&self, path: &str) -> WmsResult<Bytes> {
        // Try cache first
        {
            let mut cache = self.cache.lock().await;
            if let Some(data) = cache.get(path) {
                // Cache hit - update stats and return
                let mut stats = self.stats.lock().await;
                stats.hits += 1;
                return Ok(data.clone());
            }
        }

        // Cache miss - fetch from storage
        let data = self.storage.get(path).await?;
        
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.misses += 1;
        }

        // Store in cache
        {
            let mut cache = self.cache.lock().await;
            let data_size = data.len() as u64;
            
            // If eviction occurs, track it
            if cache.len() >= cache.cap().get() {
                let mut stats = self.stats.lock().await;
                stats.evictions += 1;
            }
            
            cache.put(path.to_string(), data.clone());
            
            // Update total cached bytes
            let mut stats = self.stats.lock().await;
            stats.total_bytes_cached += data_size;
        }

        Ok(data)
    }

    /// Get current cache statistics.
    pub async fn stats(&self) -> CacheStats {
        self.stats.lock().await.clone()
    }

    /// Get current cache size (number of entries).
    pub async fn len(&self) -> usize {
        self.cache.lock().await.len()
    }

    /// Check if cache is empty.
    pub async fn is_empty(&self) -> bool {
        self.cache.lock().await.is_empty()
    }

    /// Clear all cached entries.
    pub async fn clear(&self) {
        let mut cache = self.cache.lock().await;
        cache.clear();
        
        let mut stats = self.stats.lock().await;
        *stats = CacheStats::default();
    }

    /// Get cache capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Tests would require mocking ObjectStorage
    // For now, we'll skip unit tests and rely on integration tests
}
