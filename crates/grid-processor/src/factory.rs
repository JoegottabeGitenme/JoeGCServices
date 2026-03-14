//! Factory for creating GridProcessor instances with shared caching.
//!
//! The `GridProcessorFactory` manages:
//! - Shared `ChunkCache` across all processors
//! - Shared storage connection (MinIO/S3)
//! - Common configuration settings
//!
//! # Example
//!
//! ```text
//! use grid_processor::{GridProcessorFactory, MinioConfig};
//!
//! let minio_config = MinioConfig::from_env();
//! let factory = GridProcessorFactory::new(minio_config, 1024)?;
//!
//! // Create processors that share the same cache
//! let processor = factory.create_processor("/grids/gfs/.../TMP.zarr", &metadata)?;
//! let region = processor.read_region(&bbox).await?;
//! ```

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::cache::ChunkCache;
use crate::config::GridProcessorConfig;
use crate::error::Result;
use crate::minio_storage::{create_minio_storage, MinioConfig, MinioStorage};
use crate::types::{CacheStats, GridMetadata};
use crate::writer::ZarrMetadata;

/// Factory for creating GridProcessor instances that share a common cache.
///
/// The factory manages:
/// - Shared `ChunkCache` across all processors (for memory efficiency)
/// - MinIO configuration for creating storage on demand
/// - Common configuration settings
///
/// # Example
///
/// ```text
/// let factory = GridProcessorFactory::new(minio_config, 1024);
///
/// // All processors share the same cache
/// let stats = factory.cache_stats().await;
/// println!("Cache hit rate: {:.1}%", stats.hit_rate() * 100.0);
/// ```
pub struct GridProcessorFactory {
    /// Grid processor configuration
    config: GridProcessorConfig,
    /// Shared chunk cache for decompressed Zarr chunks
    chunk_cache: Arc<RwLock<ChunkCache>>,
    /// MinIO configuration (kept for diagnostics/logging)
    minio_config: MinioConfig,
    /// Shared MinIO storage — single S3 client reused across all tile renders
    shared_storage: Arc<MinioStorage>,
}

impl GridProcessorFactory {
    /// Create a new factory with MinIO/S3 storage configuration.
    ///
    /// Creates a single shared S3 client that is reused across all tile renders,
    /// preventing connection pool exhaustion under concurrent load.
    ///
    /// # Arguments
    /// * `minio_config` - MinIO/S3 connection configuration
    /// * `chunk_cache_size_mb` - Memory budget for the chunk cache in MB
    pub fn new(minio_config: MinioConfig, chunk_cache_size_mb: usize) -> Result<Self> {
        let chunk_cache = Arc::new(RwLock::new(ChunkCache::new(
            chunk_cache_size_mb * 1024 * 1024,
        )));

        let config = GridProcessorConfig::from_env();

        // Create a single shared S3 client with connection pooling.
        // This client is reused across all concurrent tile renders instead of
        // creating a new client (and connection pool) per request.
        let shared_storage = create_minio_storage(&minio_config)?;

        info!(
            endpoint = %minio_config.endpoint,
            bucket = %minio_config.bucket,
            "Created shared MinIO storage client for GridProcessorFactory"
        );

        Ok(Self {
            config,
            chunk_cache,
            minio_config,
            shared_storage,
        })
    }

    /// Create a new factory with default MinIO configuration from environment.
    pub fn from_env(chunk_cache_size_mb: usize) -> Result<Self> {
        Self::new(MinioConfig::from_env(), chunk_cache_size_mb)
    }

    /// Get cache statistics for monitoring.
    pub async fn cache_stats(&self) -> CacheStats {
        self.chunk_cache.read().await.stats()
    }

    /// Get the shared chunk cache reference.
    ///
    /// Useful for passing to processors that need direct cache access.
    pub fn chunk_cache(&self) -> Arc<RwLock<ChunkCache>> {
        self.chunk_cache.clone()
    }

    /// Get the processor configuration.
    pub fn config(&self) -> &GridProcessorConfig {
        &self.config
    }

    /// Get the MinIO configuration.
    pub fn minio_config(&self) -> &MinioConfig {
        &self.minio_config
    }

    /// Get the shared MinIO storage client.
    ///
    /// Returns a reference-counted handle to the single shared S3 client.
    /// All tile renders should use this instead of creating new clients,
    /// as it shares the underlying HTTP connection pool.
    pub fn storage(&self) -> Arc<MinioStorage> {
        self.shared_storage.clone()
    }

    /// Clear the chunk cache (for hot reload / cache invalidation).
    ///
    /// # Returns
    /// Tuple of (entries cleared, bytes freed)
    pub async fn clear_chunk_cache(&self) -> (usize, u64) {
        let mut cache = self.chunk_cache.write().await;
        let stats = cache.stats();
        let entries = stats.entries;
        let bytes = stats.memory_bytes;
        cache.clear();
        (entries, bytes)
    }
}

// Implement From<&ZarrMetadata> for GridMetadata to simplify conversions
impl From<&ZarrMetadata> for GridMetadata {
    fn from(zarr: &ZarrMetadata) -> Self {
        GridMetadata {
            model: zarr.model.clone(),
            parameter: zarr.parameter.clone(),
            level: zarr.level.clone(),
            units: zarr.units.clone(),
            reference_time: zarr.reference_time,
            forecast_hour: zarr.forecast_hour,
            bbox: zarr.bbox,
            shape: zarr.shape,
            chunk_shape: zarr.chunk_shape,
            num_chunks: zarr.num_chunks,
            fill_value: zarr.fill_value,
            row_origin: zarr.row_origin,
            projection: zarr.projection,
        }
    }
}

impl From<ZarrMetadata> for GridMetadata {
    fn from(zarr: ZarrMetadata) -> Self {
        GridMetadata::from(&zarr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BoundingBox;
    use chrono::{TimeZone, Utc};

    fn create_test_zarr_metadata() -> ZarrMetadata {
        use crate::types::{ProjectionType, RowOrigin};
        ZarrMetadata {
            model: "gfs".to_string(),
            parameter: "TMP".to_string(),
            level: "2 m above ground".to_string(),
            units: "K".to_string(),
            reference_time: Utc.with_ymd_and_hms(2024, 12, 22, 0, 0, 0).unwrap(),
            forecast_hour: 6,
            bbox: BoundingBox::new(0.0, -90.0, 360.0, 90.0),
            shape: (1440, 721),
            chunk_shape: (512, 512),
            num_chunks: (3, 2),
            fill_value: f32::NAN,
            dtype: "float32".to_string(),
            compression: "blosc".to_string(),
            row_origin: RowOrigin::North,
            projection: ProjectionType::Geographic,
        }
    }

    #[test]
    fn test_zarr_metadata_to_grid_metadata() {
        let zarr = create_test_zarr_metadata();
        let grid: GridMetadata = zarr.clone().into();

        assert_eq!(grid.model, zarr.model);
        assert_eq!(grid.parameter, zarr.parameter);
        assert_eq!(grid.level, zarr.level);
        assert_eq!(grid.units, zarr.units);
        assert_eq!(grid.shape, zarr.shape);
        assert_eq!(grid.chunk_shape, zarr.chunk_shape);
        assert_eq!(grid.projection, zarr.projection);
    }

    #[test]
    fn test_factory_new_returns_result() {
        // Factory::new now returns Result since it creates the shared S3 client
        let config = MinioConfig::default();
        let result = GridProcessorFactory::new(config, 1024);
        assert!(
            result.is_ok(),
            "Factory::new should succeed with default MinIO config"
        );
    }

    #[test]
    fn test_factory_from_env_returns_result() {
        let result = GridProcessorFactory::from_env(512);
        assert!(result.is_ok(), "Factory::from_env should succeed");
    }

    #[test]
    fn test_factory_shared_storage_returns_arc() {
        let config = MinioConfig::default();
        let factory = GridProcessorFactory::new(config, 256).unwrap();

        // storage() should return cloned Arcs pointing to the same underlying client
        let store1 = factory.storage();
        let store2 = factory.storage();

        // The factory holds one reference, plus our two clones = 3
        assert_eq!(Arc::strong_count(&store1), 3);
        drop(store2);
        assert_eq!(Arc::strong_count(&store1), 2);
    }

    #[test]
    fn test_factory_storage_identity() {
        // Multiple calls to storage() should return handles to the SAME client,
        // not create new ones. This is the key property that fixes connection exhaustion.
        let config = MinioConfig::default();
        let factory = GridProcessorFactory::new(config, 256).unwrap();

        let store1 = factory.storage();
        let store2 = factory.storage();

        // Both Arcs point to the same allocation
        assert!(Arc::ptr_eq(&store1, &store2));
    }

    #[tokio::test]
    async fn test_factory_chunk_cache_shared() {
        let config = MinioConfig::default();
        let factory = GridProcessorFactory::new(config, 128).unwrap();

        // chunk_cache() should return clones of the same Arc
        let cache1 = factory.chunk_cache();
        let cache2 = factory.chunk_cache();
        assert!(Arc::ptr_eq(&cache1, &cache2));
    }

    #[tokio::test]
    async fn test_factory_clear_chunk_cache() {
        let config = MinioConfig::default();
        let factory = GridProcessorFactory::new(config, 128).unwrap();

        let (entries, bytes) = factory.clear_chunk_cache().await;
        // Fresh cache should have nothing to clear
        assert_eq!(entries, 0);
        assert_eq!(bytes, 0);
    }

    #[tokio::test]
    async fn test_factory_cache_stats_initial() {
        let config = MinioConfig::default();
        let factory = GridProcessorFactory::new(config, 256).unwrap();

        let stats = factory.cache_stats().await;
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.memory_bytes, 0);
        assert_eq!(stats.evictions, 0);
    }

    #[test]
    fn test_factory_zero_cache_size() {
        // 0 MB cache should still work (minimal/disabled cache mode)
        let config = MinioConfig::default();
        let result = GridProcessorFactory::new(config, 0);
        assert!(result.is_ok(), "Factory with 0 MB cache should succeed");
    }
}
