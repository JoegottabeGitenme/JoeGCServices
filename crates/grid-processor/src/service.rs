//! High-level grid data service.
//!
//! The `GridDataService` provides a unified interface for accessing grid data
//! that handles catalog queries, storage access, and model-specific quirks.
//!
//! This is the recommended interface for OGC services (WMS, WMTS, EDR, WCS).
//!
//! # Example
//!
//! ```text
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

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use storage::Catalog;
use tokio::sync::Semaphore;

use crate::cache::ChunkCache;
use crate::config::GridProcessorConfig;
use crate::error::{GridProcessorError, Result};
use crate::factory::GridProcessorFactory;
use crate::minio_storage::{MinioConfig, MinioStorage};
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
/// ```text
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
    /// Catalog lookups issued so far: one per `read_point` / `read_region` / `get_metadata`, and
    /// ONE per `read_point_series` however many instants it covers. Lets a test (or a metric)
    /// see that a series really is one lookup and not one per step.
    catalog_lookups: AtomicU64,
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
    ) -> Result<Self> {
        let factory = GridProcessorFactory::new(minio_config, chunk_cache_size_mb)?;
        Ok(Self::with_factory(catalog, factory))
    }

    /// Create a new GridDataService with a pre-configured factory.
    ///
    /// Useful when you want to share a factory across multiple services.
    pub fn with_factory(catalog: Arc<Catalog>, factory: GridProcessorFactory) -> Self {
        Self {
            catalog,
            factory,
            catalog_lookups: AtomicU64::new(0),
        }
    }

    /// Number of catalog lookups issued so far (see the field).
    pub fn catalog_lookups(&self) -> u64 {
        self.catalog_lookups.load(Ordering::Relaxed)
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
    /// ```text
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

        // Use the shared MinIO storage client (avoids creating a new S3 client per request)
        let store = self.factory.storage();

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

        point_from_entry(
            &entry,
            lon,
            lat,
            self.factory.storage(),
            self.factory.chunk_cache(),
            self.factory.config().clone(),
        )
        .await
    }

    /// Point values for many instants of one observation parameter, in one catalog query and
    /// with the grid reads running concurrently. Returns one result per element of `times`, in
    /// the same order.
    ///
    /// This is `read_point` for `DatasetQuery::observation(..).at_valid_time(t)` repeated for
    /// every `t`, made fast. That form asks the catalog for the dataset NEAREST to `t`, once per
    /// step and one after another, so a 72-hour series paid 72 sequential Postgres round trips
    /// and 72 sequential blocking grid reads (measured on production: 0.4-0.6 s warm, 1-10 s
    /// cold, 19.8 s after a restart).
    ///
    /// Differences from calling `read_point` per instant:
    /// - one range query lists the candidate datasets; each instant is matched to the dataset
    ///   whose valid time is within `MATCH_TOLERANCE` (1 s) of it, otherwise that element is
    ///   `Err(NotFound)`. The per-instant form would have returned the nearest dataset, a
    ///   different instant's grid, which a series must not report as this instant's;
    /// - `level` restricts the candidates exactly as `find_by_time_and_level` does;
    /// - distinct grids are read at most `concurrency` at a time, each on its own task.
    pub async fn read_point_series(
        &self,
        model: &str,
        parameter: &str,
        level: Option<&str>,
        times: &[DateTime<Utc>],
        lon: f64,
        lat: f64,
        concurrency: usize,
    ) -> Vec<Result<PointValue>> {
        let not_found = |t: &DateTime<Utc>| {
            GridProcessorError::NotFound(format!(
                "No dataset found for {}/{} valid at {}",
                model, parameter, t
            ))
        };
        let (Some(first), Some(last)) = (times.iter().min(), times.iter().max()) else {
            return Vec::new();
        };

        self.catalog_lookups.fetch_add(1, Ordering::Relaxed);
        let candidates = match self
            .catalog
            .find_datasets_in_valid_time_range(
                model,
                parameter,
                level,
                *first - MATCH_TOLERANCE,
                *last + MATCH_TOLERANCE,
            )
            .await
        {
            Ok(c) => c,
            Err(e) => {
                let msg = e.to_string();
                return times
                    .iter()
                    .map(|_| Err(GridProcessorError::Catalog(msg.clone())))
                    .collect();
            }
        };

        let valid_times: Vec<DateTime<Utc>> = candidates.iter().map(|(t, _)| *t).collect();
        let matched = match_series_times(&valid_times, times, MATCH_TOLERANCE);

        // Read each distinct grid once, concurrently, then fan the results back out.
        let store = self.factory.storage();
        let cache = self.factory.chunk_cache();
        let config = self.factory.config().clone();
        let permits = Arc::new(Semaphore::new(concurrency.max(1)));
        let mut tasks: HashMap<usize, tokio::task::JoinHandle<Result<PointValue>>> = HashMap::new();
        for idx in matched.iter().flatten() {
            tasks.entry(*idx).or_insert_with(|| {
                let entry = candidates[*idx].1.clone();
                let (store, cache, config) = (store.clone(), cache.clone(), config.clone());
                let permits = Arc::clone(&permits);
                tokio::spawn(async move {
                    let _permit = permits
                        .acquire_owned()
                        .await
                        .map_err(|e| GridProcessorError::read_failed(e.to_string()))?;
                    point_from_entry(&entry, lon, lat, store, cache, config).await
                })
            });
        }

        let mut by_entry: HashMap<usize, std::result::Result<PointValue, String>> = HashMap::new();
        for (idx, handle) in tasks {
            let outcome = match handle.await {
                Ok(Ok(p)) => Ok(p),
                Ok(Err(e)) => Err(e.to_string()),
                Err(join) => Err(format!("point read task failed: {join}")),
            };
            by_entry.insert(idx, outcome);
        }

        times
            .iter()
            .zip(matched)
            .map(|(t, m)| match m {
                None => Err(not_found(t)),
                Some(idx) => match &by_entry[&idx] {
                    Ok(p) => Ok(p.clone()),
                    Err(msg) => Err(GridProcessorError::read_failed(msg.clone())),
                },
            })
            .collect()
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
        self.catalog_lookups.fetch_add(1, Ordering::Relaxed);
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
                // Get latest available data.
                // For observation data, use valid_time ordering (most recent observation).
                // For forecast data, use reference_time + forecast_hour ordering (latest run, earliest hour).
                if query.observation_data {
                    match level {
                        Some(lev) => self
                            .catalog
                            .get_latest_at_level(&query.model, &query.parameter, lev)
                            .await
                            .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                        None => self
                            .catalog
                            .get_latest(&query.model, &query.parameter)
                            .await
                            .map_err(|e| GridProcessorError::Catalog(e.to_string())),
                    }
                } else {
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
}

/// How far a dataset's valid time may be from a requested instant and still count as that
/// instant's. Matches the guard the EDR position handler already applied to nearest-dataset
/// answers (sub-second representation differences only).
pub const MATCH_TOLERANCE: Duration = Duration::seconds(1);

/// For each of `requested`, the index into `valid_times` of the dataset valid at that instant
/// (within `tolerance`), or `None`. `valid_times` must be sorted ascending; when several
/// entries are equally close the first (lowest index) wins, which for the catalog's ordering is
/// the most recently ingested. `requested` may be in any order and may repeat.
pub fn match_series_times(
    valid_times: &[DateTime<Utc>],
    requested: &[DateTime<Utc>],
    tolerance: Duration,
) -> Vec<Option<usize>> {
    requested
        .iter()
        .map(|t| {
            let above = valid_times.partition_point(|v| v < t); // first v >= t
            let candidates = [
                above.checked_sub(1),
                (above < valid_times.len()).then_some(above),
            ];
            candidates
                .into_iter()
                .flatten()
                .map(|i| (i, (valid_times[i] - *t).abs()))
                // `min_by_key` keeps the first of equals, i.e. the lower index
                .min_by_key(|(_, d)| *d)
                .filter(|(_, d)| *d <= tolerance)
                .map(|(i, _)| {
                    // several datasets can share one valid time: take the first of that run
                    let v = valid_times[i];
                    valid_times.partition_point(|x| *x < v)
                })
        })
        .collect()
}

/// Read one point from the grid behind `entry`. The shared body of `read_point` and
/// `read_point_series`; takes owned handles so it can run on its own task.
async fn point_from_entry(
    entry: &storage::CatalogEntry,
    lon: f64,
    lat: f64,
    store: Arc<MinioStorage>,
    chunk_cache: Arc<tokio::sync::RwLock<ChunkCache>>,
    config: GridProcessorConfig,
) -> Result<PointValue> {
    // Parse metadata
    let zarr_json = entry.zarr_metadata.as_ref().ok_or_else(|| {
        GridProcessorError::Metadata("Catalog entry missing zarr_metadata".to_string())
    })?;

    let zarr_meta = ZarrMetadata::from_json(zarr_json)
        .map_err(|e| GridProcessorError::Metadata(e.to_string()))?;

    // Build path and create processor
    let zarr_path = normalize_path(&entry.storage_path);
    let level_path = append_level_path(&zarr_path, 0);

    let grid_metadata = GridMetadata::from(&zarr_meta);
    let processor =
        ZarrGridProcessor::with_metadata(store, &level_path, grid_metadata, chunk_cache, config)?;

    // Query the point. Category codes are read from the nearest cell: interpolating between two
    // codes produces a code that does not exist.
    let value = if crate::downsample::is_categorical_parameter(&zarr_meta.parameter) {
        processor.read_point_nearest(lon, lat).await?
    } else {
        processor.read_point(lon, lat).await?
    };

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
    // ------------------------------------------------------------------
    // match_series_times
    // ------------------------------------------------------------------

    fn t(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 10, 9, h, m, s).unwrap()
    }

    fn hourly() -> Vec<DateTime<Utc>> {
        (10..=15).map(|h| t(h, 0, 0)).collect()
    }

    #[test]
    fn instants_match_the_dataset_valid_at_that_instant() {
        let m = match_series_times(
            &hourly(),
            &[t(10, 0, 0), t(12, 0, 0), t(15, 0, 0)],
            MATCH_TOLERANCE,
        );
        assert_eq!(m, [Some(0), Some(2), Some(5)]);
    }

    #[test]
    fn an_instant_with_no_dataset_is_none_not_the_nearest_one() {
        // 16:00 is an hour past the last dataset; 12:30 is between two.
        let v = vec![t(10, 0, 0), t(11, 0, 0), t(13, 0, 0)];
        let m = match_series_times(
            &v,
            &[t(12, 0, 0), t(12, 30, 0), t(16, 0, 0), t(9, 0, 0)],
            MATCH_TOLERANCE,
        );
        assert_eq!(m, [None, None, None, None]);
    }

    #[test]
    fn the_tolerance_is_inclusive_one_second_and_works_on_either_side() {
        let v = hourly();
        assert_eq!(
            match_series_times(&v, &[t(12, 0, 1)], MATCH_TOLERANCE),
            [Some(2)]
        );
        assert_eq!(
            match_series_times(&v, &[t(11, 59, 59)], MATCH_TOLERANCE),
            [Some(2)]
        );
        assert_eq!(
            match_series_times(&v, &[t(12, 0, 2)], MATCH_TOLERANCE),
            [None]
        );
        assert_eq!(
            match_series_times(&v, &[t(11, 59, 58)], MATCH_TOLERANCE),
            [None]
        );
    }

    #[test]
    fn requests_may_be_unsorted_and_repeat() {
        let m = match_series_times(
            &hourly(),
            &[t(14, 0, 0), t(10, 0, 0), t(14, 0, 0), t(13, 0, 0)],
            MATCH_TOLERANCE,
        );
        assert_eq!(m, [Some(4), Some(0), Some(4), Some(3)]);
    }

    #[test]
    fn datasets_sharing_a_valid_time_resolve_to_the_first_of_them() {
        // (the catalog lists the most recently ingested first)
        let v = vec![
            t(10, 0, 0),
            t(11, 0, 0),
            t(11, 0, 0),
            t(11, 0, 0),
            t(12, 0, 0),
        ];
        assert_eq!(
            match_series_times(&v, &[t(11, 0, 0)], MATCH_TOLERANCE),
            [Some(1)]
        );
        assert_eq!(
            match_series_times(&v, &[t(11, 0, 1)], MATCH_TOLERANCE),
            [Some(1)]
        );
        assert_eq!(
            match_series_times(&v, &[t(12, 0, 0)], MATCH_TOLERANCE),
            [Some(4)]
        );
    }

    #[test]
    fn nothing_available_or_nothing_requested() {
        assert_eq!(
            match_series_times(&[], &[t(10, 0, 0)], MATCH_TOLERANCE),
            [None]
        );
        assert!(match_series_times(&hourly(), &[], MATCH_TOLERANCE).is_empty());
    }

    #[test]
    fn a_wider_tolerance_widens_the_match() {
        let v = hourly();
        assert_eq!(
            match_series_times(&v, &[t(12, 20, 0)], Duration::minutes(30)),
            [Some(2)]
        );
        assert_eq!(
            match_series_times(&v, &[t(12, 40, 0)], Duration::minutes(30)),
            [Some(3)]
        );
    }
}
