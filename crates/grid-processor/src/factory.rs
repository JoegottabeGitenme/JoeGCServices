//! Factory for creating GridProcessor instances with shared caching.
//!
//! The `GridProcessorFactory` manages:
//! - Shared `ChunkCache` across all processors
//! - Shared storage connection (MinIO/S3)
//! - Common configuration settings
//!
//! # Example
//!
//! ```rust,ignore
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

use crate::cache::ChunkCache;
use crate::config::GridProcessorConfig;
use crate::minio_storage::MinioConfig;
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
/// ```rust,ignore
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
    /// MinIO configuration for creating storage
    minio_config: MinioConfig,
}

impl GridProcessorFactory {
    /// Create a new factory with MinIO/S3 storage configuration.
    ///
    /// # Arguments
    /// * `minio_config` - MinIO/S3 connection configuration
    /// * `chunk_cache_size_mb` - Memory budget for the chunk cache in MB
    pub fn new(minio_config: MinioConfig, chunk_cache_size_mb: usize) -> Self {
        let chunk_cache = Arc::new(RwLock::new(ChunkCache::new(
            chunk_cache_size_mb * 1024 * 1024,
        )));

        let config = GridProcessorConfig::from_env();

        Self {
            config,
            chunk_cache,
            minio_config,
        }
    }

    /// Create a new factory with default MinIO configuration from environment.
    pub fn from_env(chunk_cache_size_mb: usize) -> Self {
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
    }
}
