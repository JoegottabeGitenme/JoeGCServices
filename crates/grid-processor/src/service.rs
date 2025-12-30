//! High-level grid data service.
//!
//! The `GridDataService` provides a unified interface for accessing grid data
//! that handles catalog queries, storage access, and model-specific quirks.
//!
//! This is the recommended interface for OGC services (WMS, WMTS, EDR, WCS).
//!
//! # Example
//!
//! ```rust,ignore
//! use grid_processor::{GridDataService, DatasetQuery, BoundingBox};
//!
//! // Create service (typically at application startup)
//! let service = GridDataService::new(catalog, minio_config, 1024);
//!
//! // Query for a forecast dataset
//! let query = DatasetQuery::forecast("gfs", "TMP")
//!     .at_level("2 m above ground")
//!     .at_forecast_hour(6);
//!
//! // Read a region (for tile rendering)
//! let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
//! let region = service.read_region(&query, &bbox, Some((256, 256))).await?;
//! ```

use std::sync::Arc;

use storage::Catalog;

use crate::error::{GridProcessorError, Result};
use crate::factory::GridProcessorFactory;
use crate::minio_storage::{create_minio_storage, MinioConfig};
use crate::processor::{
    parse_multiscale_metadata, GridProcessor, MultiscaleGridProcessorFactory, ZarrGridProcessor,
};
use crate::query::{DatasetQuery, PointValue, TimeSpecification};
use crate::types::{BoundingBox, CacheStats, GridMetadata, GridRegion};
use crate::writer::ZarrMetadata;

/// High-level service for accessing grid data.
///
/// This is the primary interface for services (WMS, EDR, WCS) to access
/// weather data. It handles:
/// - Catalog queries (finding the right dataset by model/param/time/level)
/// - Storage access (fetching from MinIO/S3)
/// - Chunk caching (shared across requests)
/// - Model-specific handling (0-360 longitude, projection quirks)
///
/// # Example
///
/// ```rust,ignore
/// let service = GridDataService::new(catalog, minio_config, 1024);
///
/// let query = DatasetQuery::forecast("gfs", "TMP")
///     .at_level("2 m above ground")
///     .at_forecast_hour(6);
///
/// let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
/// let region = service.read_region(&query, &bbox, Some((256, 256))).await?;
/// ```
pub struct GridDataService {
    /// Catalog for dataset lookups
    catalog: Arc<Catalog>,
    /// Factory for creating processors (manages chunk cache)
    factory: GridProcessorFactory,
}

impl GridDataService {
    /// Create a new GridDataService.
    ///
    /// # Arguments
    /// * `catalog` - Catalog for dataset lookups
    /// * `minio_config` - MinIO/S3 connection configuration
    /// * `chunk_cache_size_mb` - Memory budget for chunk cache in MB
    pub fn new(
        catalog: Arc<Catalog>,
        minio_config: MinioConfig,
        chunk_cache_size_mb: usize,
    ) -> Self {
        let factory = GridProcessorFactory::new(minio_config, chunk_cache_size_mb);
        Self { catalog, factory }
    }

    /// Create a new GridDataService with a pre-configured factory.
    ///
    /// Useful when you want to share a factory across multiple services.
    pub fn with_factory(catalog: Arc<Catalog>, factory: GridProcessorFactory) -> Self {
        Self { catalog, factory }
    }

    /// Read a geographic region for a dataset.
    ///
    /// This is the primary method for tile rendering and area queries.
    ///
    /// # Arguments
    /// * `query` - Dataset query specifying model, parameter, time, level
    /// * `bbox` - Geographic bounding box to read
    /// * `output_size` - Optional output dimensions for pyramid level selection
    ///
    /// # Returns
    /// `GridRegion` containing the data and metadata
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let query = DatasetQuery::forecast("gfs", "TMP")
    ///     .at_level("2 m above ground")
    ///     .at_forecast_hour(6);
    ///
    /// let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
    /// let region = service.read_region(&query, &bbox, Some((256, 256))).await?;
    /// ```
    pub async fn read_region(
        &self,
        query: &DatasetQuery,
        bbox: &BoundingBox,
        output_size: Option<(usize, usize)>,
    ) -> Result<GridRegion> {
        // Find the dataset in the catalog
        let entry = self.find_dataset(query).await?.ok_or_else(|| {
            GridProcessorError::NotFound(format!(
                "No dataset found for {}/{} with specified time/level",
                query.model, query.parameter
            ))
        })?;

        // Parse Zarr metadata
        let zarr_json = entry.zarr_metadata.as_ref().ok_or_else(|| {
            GridProcessorError::Metadata("Catalog entry missing zarr_metadata".to_string())
        })?;

        let zarr_meta = ZarrMetadata::from_json(zarr_json)
            .map_err(|e| GridProcessorError::Metadata(e.to_string()))?;

        // Build storage path
        let zarr_path = normalize_path(&entry.storage_path);

        // Create storage
        let store = create_minio_storage(self.factory.minio_config())
            .map_err(|e| GridProcessorError::Storage(e.to_string()))?;

        // Check for multiscale support
        let multiscale_meta = parse_multiscale_metadata(zarr_json);

        // Read the region
        if let (Some(ms_meta), Some(out_size)) = (multiscale_meta, output_size) {
            if ms_meta.num_levels() > 1 {
                // Use pyramid-aware loading
                let ms_factory = MultiscaleGridProcessorFactory::new(
                    store,
                    &zarr_path,
                    ms_meta,
                    self.factory.chunk_cache(),
                    self.factory.config().clone(),
                );

                let (region, _level) = ms_factory.read_region_for_output(bbox, out_size).await?;
                return Ok(region);
            }
        }

        // Standard loading (native resolution)
        let level_path = append_level_path(&zarr_path, 0);
        let grid_metadata = GridMetadata::from(&zarr_meta);
        let processor = ZarrGridProcessor::with_metadata(
            store,
            &level_path,
            grid_metadata,
            self.factory.chunk_cache(),
            self.factory.config().clone(),
        )?;
        processor.read_region(bbox).await
    }

    /// Query a single point value.
    ///
    /// This is used for GetFeatureInfo and EDR Position queries.
    ///
    /// # Arguments
    /// * `query` - Dataset query specifying model, parameter, time, level
    /// * `lon` - Longitude in degrees (-180 to 180 or 0 to 360)
    /// * `lat` - Latitude in degrees (-90 to 90)
    ///
    /// # Returns
    /// `PointValue` containing the value and metadata
    pub async fn read_point(&self, query: &DatasetQuery, lon: f64, lat: f64) -> Result<PointValue> {
        // Find the dataset
        let entry = self.find_dataset(query).await?.ok_or_else(|| {
            GridProcessorError::NotFound(format!(
                "No dataset found for {}/{} with specified time/level",
                query.model, query.parameter
            ))
        })?;

        // Parse metadata
        let zarr_json = entry.zarr_metadata.as_ref().ok_or_else(|| {
            GridProcessorError::Metadata("Catalog entry missing zarr_metadata".to_string())
        })?;

        let zarr_meta = ZarrMetadata::from_json(zarr_json)
            .map_err(|e| GridProcessorError::Metadata(e.to_string()))?;

        // Build path and create processor
        let zarr_path = normalize_path(&entry.storage_path);
        let level_path = append_level_path(&zarr_path, 0);

        // Create storage
        let store = create_minio_storage(self.factory.minio_config())
            .map_err(|e| GridProcessorError::Storage(e.to_string()))?;

        let grid_metadata = GridMetadata::from(&zarr_meta);
        let processor = ZarrGridProcessor::with_metadata(
            store,
            &level_path,
            grid_metadata,
            self.factory.chunk_cache(),
            self.factory.config().clone(),
        )?;

        // Query the point
        let value = processor.read_point(lon, lat).await?;

        Ok(PointValue {
            value,
            units: zarr_meta.units.clone(),
            model: zarr_meta.model.clone(),
            parameter: zarr_meta.parameter.clone(),
            level: zarr_meta.level.clone(),
            time: zarr_meta.reference_time,
            forecast_hour: Some(zarr_meta.forecast_hour),
        })
    }

    /// Get metadata for a dataset without loading data.
    ///
    /// Useful for checking dataset availability or getting bounds.
    pub async fn get_metadata(&self, query: &DatasetQuery) -> Result<GridMetadata> {
        let entry = self.find_dataset(query).await?.ok_or_else(|| {
            GridProcessorError::NotFound(format!(
                "No dataset found for {}/{} with specified time/level",
                query.model, query.parameter
            ))
        })?;

        let zarr_json = entry.zarr_metadata.as_ref().ok_or_else(|| {
            GridProcessorError::Metadata("Catalog entry missing zarr_metadata".to_string())
        })?;

        let zarr_meta = ZarrMetadata::from_json(zarr_json)
            .map_err(|e| GridProcessorError::Metadata(e.to_string()))?;

        Ok(GridMetadata::from(&zarr_meta))
    }

    /// Get cache statistics for monitoring.
    pub async fn cache_stats(&self) -> CacheStats {
        self.factory.cache_stats().await
    }

    /// Clear the chunk cache.
    ///
    /// Returns (entries cleared, bytes freed).
    pub async fn clear_cache(&self) -> (usize, u64) {
        self.factory.clear_chunk_cache().await
    }

    /// Get access to the underlying factory.
    ///
    /// Useful for advanced use cases that need direct processor access.
    pub fn factory(&self) -> &GridProcessorFactory {
        &self.factory
    }

    // ========================================================================
    // Private helpers
    // ========================================================================

    /// Find a dataset in the catalog based on the query.
    async fn find_dataset(&self, query: &DatasetQuery) -> Result<Option<storage::CatalogEntry>> {
        let level = query.level.as_deref();

        match &query.time_spec {
            TimeSpecification::Observation { time } => {
                // For observations, find by time (level not used in find_by_time)
                self.catalog
                    .find_by_time(&query.model, &query.parameter, *time)
                    .await
                    .map_err(|e| GridProcessorError::Catalog(e.to_string()))
            }

            TimeSpecification::Forecast {
                reference_time: _,
                forecast_hour,
            } => {
                // Note: Current catalog doesn't support filtering by reference_time directly,
                // so we use the best matching query based on what's available.
                match (forecast_hour, level) {
                    (Some(hour), Some(lev)) => self
                        .catalog
                        .find_by_forecast_hour_and_level(&query.model, &query.parameter, *hour, lev)
                        .await
                        .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                    (Some(hour), None) => self
                        .catalog
                        .find_by_forecast_hour(&query.model, &query.parameter, *hour)
                        .await
                        .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                    (None, Some(lev)) => self
                        .catalog
                        .get_latest_run_earliest_forecast_at_level(
                            &query.model,
                            &query.parameter,
                            lev,
                        )
                        .await
                        .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                    (None, None) => self
                        .catalog
                        .get_latest_run_earliest_forecast(&query.model, &query.parameter)
                        .await
                        .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                }
            }

            TimeSpecification::ValidTime { valid_time } => {
                // Find forecast closest to the requested valid time
                match level {
                    Some(lev) => self
                        .catalog
                        .find_by_time_and_level(&query.model, &query.parameter, *valid_time, lev)
                        .await
                        .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                    None => self
                        .catalog
                        .find_by_time(&query.model, &query.parameter, *valid_time)
                        .await
                        .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                }
            }

            TimeSpecification::Latest => {
                // Get latest available data
                match level {
                    Some(lev) => self
                        .catalog
                        .get_latest_run_earliest_forecast_at_level(
                            &query.model,
                            &query.parameter,
                            lev,
                        )
                        .await
                        .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                    None => self
                        .catalog
                        .get_latest_run_earliest_forecast(&query.model, &query.parameter)
                        .await
                        .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                }
            }
        }
    }
}

/// Normalize a storage path to have a leading slash.
fn normalize_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

/// Append a pyramid level to a Zarr path.
fn append_level_path(zarr_path: &str, level: u32) -> String {
    let base = zarr_path.trim_end_matches('/');
    if base.ends_with(".zarr") {
        format!("{}/{}", base, level)
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path("grids/gfs/test.zarr"),
            "/grids/gfs/test.zarr"
        );
        assert_eq!(
            normalize_path("/grids/gfs/test.zarr"),
            "/grids/gfs/test.zarr"
        );
    }

    #[test]
    fn test_append_level_path() {
        assert_eq!(
            append_level_path("/grids/gfs/test.zarr", 0),
            "/grids/gfs/test.zarr/0"
        );
        assert_eq!(
            append_level_path("/grids/gfs/test.zarr/", 2),
            "/grids/gfs/test.zarr/2"
        );
        assert_eq!(
            append_level_path("/grids/gfs/test.zarr/0", 0),
            "/grids/gfs/test.zarr/0"
        );
    }
}
