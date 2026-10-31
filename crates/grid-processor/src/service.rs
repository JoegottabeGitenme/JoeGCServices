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
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use storage::Catalog;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

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
    /// ONE per `read_point_series` / `read_forecast_point_series` however many instants it
    /// covers (plus one per forecast instant the series query could not settle). Lets a test (or
    /// a metric) see that a series really is one lookup and not one per step.
    catalog_lookups: AtomicU64,
    /// Permits for the grid reads of forecast series (`read_forecast_point_series`), shared by
    /// every request: the bound on the reads a process runs for series at once.
    forecast_read_permits: Arc<Semaphore>,
}

/// How many grid reads (and fallback lookups) `read_forecast_point_series` has in flight at
/// once across the whole `GridDataService`. The grids of a 16 day GFS curve are 200+ distinct
/// S3 objects per parameter, so a cold series is bound by this number times the S3 round trip.
/// Each read blocks a runtime worker, hence a bound rather than "all of them", and it is per
/// service rather than per request so that parallel requests cannot multiply it.
pub const FORECAST_SERIES_READ_CONCURRENCY: usize = 32;

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
            forecast_read_permits: Arc::new(Semaphore::new(FORECAST_SERIES_READ_CONCURRENCY)),
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
                // Same as an empty list: no instant is settled, each is looked up on its own.
                tracing::warn!(
                    "Range query for {}/{} failed, using per-instant lookups: {}",
                    model,
                    parameter,
                    e
                );
                Vec::new()
            }
        };

        let valid_times: Vec<DateTime<Utc>> = candidates.iter().map(|(t, _)| *t).collect();
        let matched = match_series_times(&valid_times, times, MATCH_TOLERANCE);

        // Read each distinct grid once, concurrently, then fan the results back out.
        let store = self.factory.storage();
        let cache = self.factory.chunk_cache();
        let config = self.factory.config().clone();
        let permits = Arc::new(Semaphore::new(concurrency.max(1)));
        let mut tasks: HashMap<usize, PointRead> = HashMap::new();
        for idx in matched.iter().flatten() {
            tasks.entry(*idx).or_insert_with(|| {
                let entry = candidates[*idx].1.clone();
                let (store, cache, config) = (store.clone(), cache.clone(), config.clone());
                spawn_permitted(&permits, async move {
                    point_from_entry(&entry, lon, lat, store, cache, config).await
                })
            });
        }

        let mut by_entry: HashMap<usize, Outcome> = HashMap::new();
        for (idx, handle) in tasks {
            by_entry.insert(idx, join_outcome(handle).await);
        }

        times
            .iter()
            .zip(matched)
            .map(|(t, m)| match m {
                None => Err(not_found(t)),
                Some(idx) => outcome_to_result(&by_entry[&idx]),
            })
            .collect()
    }

    /// Point values for many instants of one FORECAST parameter, in one catalog query and with
    /// the grid reads running concurrently. Returns one result per element of `times`, in the
    /// same order.
    ///
    /// This is `read_point` for `DatasetQuery::forecast(..).at_valid_time(t)` repeated for every
    /// `t`, made fast, and it returns exactly what that repetition returns. (It is a sibling of
    /// `read_point_series` rather than a mode of it because that function deliberately answers
    /// differently for an instant with no dataset: see the paragraph on tolerance below.)
    ///
    /// The per-instant form asks the catalog for the dataset NEAREST to `t` (`find_by_time` /
    /// `find_by_time_and_level`: smallest `|valid_time - t|`, ties to the newest run, then the
    /// most recently ingested), once per step and one after another. A 16 day GFS curve is 209
    /// instants times 4 parameters: 836 sequential Postgres round trips and 836 sequential
    /// blocking grid reads.
    ///
    /// How the same answer is reached with one lookup:
    /// - ONE query lists the newest dataset of every distinct valid time in
    ///   `[first - NEAREST_MARGIN, last + NEAREST_MARGIN]`, under the same `level` restriction;
    /// - each instant takes the nearest listed valid time (`match_nearest_series_times`), which
    ///   is an exact match whenever the parameter has a dataset for that instant. A run with a
    ///   hole at that instant is covered by the previous run's dataset, as in the SQL;
    /// - an instant whose nearest dataset is not an exact match, and where the list cannot PROVE
    ///   it is the nearest the whole table holds (nothing listed, a distance tie, or a closer
    ///   dataset possible outside the queried window), is resolved with the per-instant catalog
    ///   lookup, i.e. by the original code path. So there is no case where this function and the
    ///   repeated `read_point` disagree on which dataset answers;
    /// - if the range query itself fails, nothing is listed and every instant takes that
    ///   per-instant lookup (a warning is logged once), so a failed query costs the speed-up,
    ///   not the series.
    ///
    /// Unlike `read_point_series` there is NO tolerance and no `NotFound` for a missing instant:
    /// a forecast instant with no dataset of its own is answered by the nearest one, exactly as
    /// `read_point` does (and the EDR handler leaves forecast values as read).
    ///
    /// Every grid read (and every fallback lookup) waits for a permit of the service's
    /// `forecast_read_permits` ([`FORECAST_SERIES_READ_CONCURRENCY`]). The pool is shared by all
    /// parameters of all requests, so it bounds the reads (and runtime workers) forecast series
    /// occupy in the whole process, however many requests are in flight. A dataset the list
    /// settles is read once however many instants it answers; a fallback instant does its own
    /// lookup and read, which may repeat a read of the list.
    pub async fn read_forecast_point_series(
        &self,
        model: &str,
        parameter: &str,
        level: Option<&str>,
        times: &[DateTime<Utc>],
        lon: f64,
        lat: f64,
    ) -> Vec<Result<PointValue>> {
        let (Some(first), Some(last)) = (times.iter().min(), times.iter().max()) else {
            return Vec::new();
        };
        let window = (*first - NEAREST_MARGIN, *last + NEAREST_MARGIN);

        self.catalog_lookups.fetch_add(1, Ordering::Relaxed);
        let candidates = match self
            .catalog
            .find_newest_datasets_in_valid_time_range(model, parameter, level, window.0, window.1)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                // Same as an empty list: no instant is settled, each is looked up on its own.
                tracing::warn!(
                    "Range query for {}/{} failed, using per-instant lookups: {}",
                    model,
                    parameter,
                    e
                );
                Vec::new()
            }
        };

        let valid_times: Vec<DateTime<Utc>> = candidates.iter().map(|(t, _)| *t).collect();
        let matched = match_nearest_series_times(&valid_times, times, window);

        let store = self.factory.storage();
        let cache = self.factory.chunk_cache();
        let config = self.factory.config().clone();

        // One read per distinct listed dataset ... (the entries, each with its zarr metadata
        // document, are shared by the tasks rather than copied into every one)
        let candidates = Arc::new(candidates);
        let mut reads: HashMap<usize, PointRead> = HashMap::new();
        for idx in matched.iter().flatten() {
            reads.entry(*idx).or_insert_with(|| {
                let (candidates, idx) = (Arc::clone(&candidates), *idx);
                let (store, cache, config) = (store.clone(), cache.clone(), config.clone());
                spawn_permitted(&self.forecast_read_permits, async move {
                    point_from_entry(&candidates[idx].1, lon, lat, store, cache, config).await
                })
            });
        }
        // ... and the original per-instant lookup + read for each instant the list cannot settle.
        let mut fallbacks: HashMap<DateTime<Utc>, PointRead> = HashMap::new();
        for (t, m) in times.iter().zip(&matched) {
            if m.is_some() || fallbacks.contains_key(t) {
                continue;
            }
            self.catalog_lookups.fetch_add(1, Ordering::Relaxed);
            let t = *t;
            let catalog = Arc::clone(&self.catalog);
            let (model, parameter) = (model.to_string(), parameter.to_string());
            let level = level.map(str::to_string);
            let (store, cache, config) = (store.clone(), cache.clone(), config.clone());
            fallbacks.insert(
                t,
                spawn_permitted(&self.forecast_read_permits, async move {
                    let entry =
                        find_nearest_dataset(&catalog, &model, &parameter, level.as_deref(), t)
                            .await?
                            .ok_or_else(|| {
                                GridProcessorError::NotFound(format!(
                                    "No dataset found for {}/{} with specified time/level",
                                    model, parameter
                                ))
                            })?;
                    point_from_entry(&entry, lon, lat, store, cache, config).await
                }),
            );
        }

        let mut by_entry: HashMap<usize, Outcome> = HashMap::new();
        for (idx, handle) in reads {
            by_entry.insert(idx, join_outcome(handle).await);
        }
        let mut by_instant: HashMap<DateTime<Utc>, Outcome> = HashMap::new();
        for (t, handle) in fallbacks {
            by_instant.insert(t, join_outcome(handle).await);
        }

        times
            .iter()
            .zip(matched)
            .map(|(t, m)| match m {
                Some(idx) => outcome_to_result(&by_entry[&idx]),
                None => outcome_to_result(&by_instant[t]),
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
                find_nearest_dataset(
                    &self.catalog,
                    &query.model,
                    &query.parameter,
                    level,
                    *valid_time,
                )
                .await
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

/// How far beyond a forecast series' first and last instant `read_forecast_point_series` lists
/// datasets. It only exists so that an instant with no dataset of its own (a parameter missing
/// at some hour, or at the edge of a series) can still be answered from the list: see
/// `match_nearest_series_times` for when that is allowed. Listing a day each side costs a few
/// rows per parameter (one per distinct valid time) and makes the per-instant lookup
/// fallback rare.
pub const NEAREST_MARGIN: Duration = Duration::hours(24);

/// For each of `requested`, the index into `valid_times` of the dataset the catalog's
/// "nearest valid time" lookup (`find_by_time[_and_level]`) would answer with, or `None` if the
/// list cannot prove which one that is. `covered` is the inclusive `(from, to)` window the list
/// was fetched for; `valid_times` is that window's datasets' valid times, ascending (several
/// entries may share one, the first is the preferred one: newest run, then most recently
/// ingested). Times are compared to the microsecond, as Postgres stores them.
///
/// The lookup is `ORDER BY ABS(valid_time - t), reference_time DESC, ingested_at DESC LIMIT 1`
/// over ALL of the parameter's datasets. `Some(i)` is returned only when `i` is the first entry
/// of the unique valid time nearest to `t` in the list AND no dataset outside `covered` could be
/// as near:
/// - the nearest listed valid time is unique. A tie between a time before and a time after `t`
///   is decided by run time and ingest time, which the list does not carry: `None`;
/// - its distance `d` is at most `t - from` and at most `to - t`. A dataset outside the window
///   is strictly farther than that from `t`, so it can neither beat nor tie it. An exact match
///   (`d = 0`) always qualifies;
/// - within that valid time the SQL ordering and the list's ordering are the same, so the
///   first entry is the row the SQL returns.
///
/// Everything else (nothing listed, a tie, an instant nearer an edge of the window than its
/// nearest dataset) is `None`, to be settled by the real lookup. `requested` may be in any
/// order and may repeat.
pub fn match_nearest_series_times(
    valid_times: &[DateTime<Utc>],
    requested: &[DateTime<Utc>],
    covered: (DateTime<Utc>, DateTime<Utc>),
) -> Vec<Option<usize>> {
    let us: Vec<i64> = valid_times.iter().map(|t| t.timestamp_micros()).collect();
    let (from, to) = (covered.0.timestamp_micros(), covered.1.timestamp_micros());
    requested
        .iter()
        .map(|t| {
            let t = t.timestamp_micros();
            let above = us.partition_point(|v| *v < t); // first v >= t
            let before = above.checked_sub(1).map(|i| (t - us[i], us[i]));
            let after = us.get(above).map(|v| (*v - t, *v));
            let (distance, nearest) = match (before, after) {
                (None, None) => return None,
                (Some(b), None) => b,
                (None, Some(a)) => a,
                (Some(b), Some(a)) if a.0 < b.0 => a,
                (Some(b), Some(a)) if b.0 < a.0 => b,
                // equally near on both sides: the winner depends on run and ingest time
                (Some(_), Some(_)) => return None,
            };
            if distance > t - from || distance > to - t {
                return None;
            }
            Some(us.partition_point(|v| *v < nearest))
        })
        .collect()
}

/// The dataset nearest in valid time to `valid_time`, restricted to `level` if given. This is
/// the lookup behind `DatasetQuery::at_valid_time`, shared by `find_dataset` and the fallback of
/// `read_forecast_point_series`.
async fn find_nearest_dataset(
    catalog: &Catalog,
    model: &str,
    parameter: &str,
    level: Option<&str>,
    valid_time: DateTime<Utc>,
) -> Result<Option<storage::CatalogEntry>> {
    match level {
        Some(lev) => catalog
            .find_by_time_and_level(model, parameter, valid_time, lev)
            .await
            .map_err(|e| GridProcessorError::Catalog(e.to_string())),
        None => catalog
            .find_by_time(model, parameter, valid_time)
            .await
            .map_err(|e| GridProcessorError::Catalog(e.to_string())),
    }
}

/// A finished point read, in a form that can be handed out once per requesting instant.
type Outcome = std::result::Result<PointValue, String>;

/// A grid read running on its own task. Dropping it (the request that wanted it went away)
/// cancels the read, or its wait for a permit, instead of leaving a whole series' worth of
/// reads running for nobody.
struct PointRead(JoinHandle<Result<PointValue>>);

impl Drop for PointRead {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Run `work` on its own task once a permit of `permits` is free, holding the permit until it
/// is done. Each read blocks a runtime worker, so the task (and the permit) is what bounds how
/// many workers a series can occupy.
fn spawn_permitted<F>(permits: &Arc<Semaphore>, work: F) -> PointRead
where
    F: Future<Output = Result<PointValue>> + Send + 'static,
{
    let permits = Arc::clone(permits);
    PointRead(tokio::spawn(async move {
        let _permit = permits
            .acquire_owned()
            .await
            .map_err(|e| GridProcessorError::read_failed(e.to_string()))?;
        work.await
    }))
}

/// Wait for a read from `spawn_permitted`.
async fn join_outcome(mut read: PointRead) -> Outcome {
    match (&mut read.0).await {
        Ok(Ok(p)) => Ok(p),
        Ok(Err(e)) => Err(e.to_string()),
        Err(join) => Err(format!("point read task failed: {join}")),
    }
}

/// One element of a series' result from a shared `Outcome`.
fn outcome_to_result(outcome: &Outcome) -> Result<PointValue> {
    match outcome {
        Ok(p) => Ok(p.clone()),
        Err(msg) => Err(GridProcessorError::read_failed(msg.clone())),
    }
}

/// Read one point from the grid behind `entry`. The shared body of `read_point`,
/// `read_point_series` and `read_forecast_point_series`; takes owned handles so it can run on
/// its own task.
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

    // ------------------------------------------------------------------
    // match_nearest_series_times
    // ------------------------------------------------------------------

    /// The window a series over `times` is listed for.
    fn window(times: &[DateTime<Utc>]) -> (DateTime<Utc>, DateTime<Utc>) {
        (
            *times.iter().min().unwrap() - NEAREST_MARGIN,
            *times.iter().max().unwrap() + NEAREST_MARGIN,
        )
    }

    #[test]
    fn nearest_an_exact_match_is_always_resolved() {
        let v = hourly();
        let req = [t(10, 0, 0), t(12, 0, 0), t(15, 0, 0)];
        // even with a window that is exactly the series
        assert_eq!(
            match_nearest_series_times(&v, &req, (t(10, 0, 0), t(15, 0, 0))),
            [Some(0), Some(2), Some(5)]
        );
        assert_eq!(
            match_nearest_series_times(&v, &req, window(&req)),
            [Some(0), Some(2), Some(5)]
        );
    }

    #[test]
    fn nearest_the_newest_run_wins_among_datasets_of_one_valid_time() {
        // the list holds the newest run first (the catalog's ordering): index 1 is the preferred
        // 11:00, 2 and 3 are older runs' 11:00
        let v = vec![
            t(10, 0, 0),
            t(11, 0, 0),
            t(11, 0, 0),
            t(11, 0, 0),
            t(12, 0, 0),
        ];
        let req = [t(11, 0, 0)];
        assert_eq!(
            match_nearest_series_times(&v, &req, window(&req)),
            [Some(1)]
        );
        // and a nearest-but-not-exact instant lands on that same first entry
        let req = [t(11, 0, 1)];
        assert_eq!(
            match_nearest_series_times(&v, &req, window(&req)),
            [Some(1)]
        );
    }

    #[test]
    fn nearest_a_hole_in_the_newest_run_is_filled_by_the_previous_run() {
        // newest run (ref 06z) has no 12:00 grid; the previous run (00z) does. The catalog
        // lists one dataset per valid time, so 12:00 is simply the 00z run's entry, and it is
        // an exact match: no neighbour is used.
        let v = vec![t(10, 0, 0), t(11, 0, 0), t(12, 0, 0), t(13, 0, 0)];
        let req = [t(11, 0, 0), t(12, 0, 0), t(13, 0, 0)];
        assert_eq!(
            match_nearest_series_times(&v, &req, window(&req)),
            [Some(1), Some(2), Some(3)]
        );
    }

    #[test]
    fn nearest_an_instant_without_a_dataset_takes_the_nearest_one_not_none() {
        // 12:20 has no dataset; the SQL would answer with 12:00's, 40 minutes nearer than 13:00
        let v = vec![t(10, 0, 0), t(12, 0, 0), t(13, 0, 0)];
        let req = [t(12, 20, 0), t(12, 40, 0)];
        assert_eq!(
            match_nearest_series_times(&v, &req, window(&req)),
            [Some(1), Some(2)]
        );
        // far beyond the last dataset: still that dataset, while it is provably the nearest
        let req = [t(20, 0, 0)];
        assert_eq!(
            match_nearest_series_times(&v, &req, window(&req)),
            [Some(2)]
        );
    }

    #[test]
    fn nearest_a_tie_between_two_valid_times_is_left_to_the_real_lookup() {
        // 12:00 is exactly between 11:00 and 13:00: the SQL breaks the tie on reference_time
        // and ingested_at, which the list does not carry
        let v = vec![t(10, 0, 0), t(11, 0, 0), t(13, 0, 0)];
        let req = [t(12, 0, 0), t(11, 30, 0)];
        assert_eq!(
            match_nearest_series_times(&v, &req, window(&req)),
            [None, Some(1)]
        );
    }

    #[test]
    fn nearest_nothing_listed_is_left_to_the_real_lookup() {
        let req = [t(12, 0, 0)];
        assert_eq!(match_nearest_series_times(&[], &req, window(&req)), [None]);
        assert!(match_nearest_series_times(&hourly(), &[], (t(0, 0, 0), t(23, 0, 0))).is_empty());
    }

    #[test]
    fn nearest_a_closer_dataset_could_hide_outside_the_window() {
        // The list was fetched for [09:00, 11:00] and holds one dataset, at 10:00. A dataset
        // just outside the window (not listed) could be nearer to an instant close to the edge.
        let v = vec![t(10, 0, 0)];
        let w = (t(9, 0, 0), t(11, 0, 0));
        // 09:00 is on the edge, 1 h from 10:00: a dataset at 08:30 would be nearer -> undecided
        assert_eq!(match_nearest_series_times(&v, &[t(9, 0, 0)], w), [None]);
        // 09:15 is 15 min from the edge but 45 min from 10:00 -> undecided
        assert_eq!(match_nearest_series_times(&v, &[t(9, 15, 0)], w), [None]);
        // 09:30 is 30 min from both: anything outside the window is farther than 30 min, so
        // 10:00 is the nearest whatever the rest of the table holds
        assert_eq!(match_nearest_series_times(&v, &[t(9, 30, 0)], w), [Some(0)]);
        // the far edge behaves the same way
        assert_eq!(match_nearest_series_times(&v, &[t(11, 0, 0)], w), [None]);
        assert_eq!(match_nearest_series_times(&v, &[t(10, 45, 0)], w), [None]);
        assert_eq!(
            match_nearest_series_times(&v, &[t(10, 30, 0)], w),
            [Some(0)]
        );
    }

    #[test]
    fn nearest_requests_may_be_unsorted_and_repeat() {
        let v = hourly();
        let req = [t(14, 0, 0), t(10, 0, 0), t(14, 0, 0), t(13, 0, 0)];
        assert_eq!(
            match_nearest_series_times(&v, &req, window(&req)),
            [Some(4), Some(0), Some(4), Some(3)]
        );
    }

    /// A row of the datasets table, reduced to what the nearest-valid-time lookup orders by.
    #[derive(Clone, Copy, Debug)]
    struct Row {
        id: usize,
        level: u8,
        valid: i64,
        reference: i64,
        ingested: i64,
    }

    /// `find_by_time_and_level`, in memory: smallest |valid - t|, then newest reference_time,
    /// then newest ingested_at, over ALL rows of `level`.
    fn sql_nearest(rows: &[Row], level: u8, t: i64) -> Option<usize> {
        rows.iter()
            .filter(|r| r.level == level)
            .min_by_key(|r| ((r.valid - t).abs(), -r.reference, -r.ingested))
            .map(|r| r.id)
    }

    /// The catalog range query with DISTINCT ON, in memory: rows of `level` with a valid time in
    /// `[from, to]`, one per valid time (the newest run, then the most recently ingested).
    fn listed(rows: &[Row], level: u8, from: i64, to: i64) -> Vec<Row> {
        let mut v: Vec<Row> = rows
            .iter()
            .copied()
            .filter(|r| r.level == level && r.valid >= from && r.valid <= to)
            .collect();
        v.sort_by_key(|r| (r.valid, -r.reference, -r.ingested));
        v.dedup_by_key(|r| r.valid);
        v
    }

    /// Whenever the matcher commits to a dataset, it is the dataset the real lookup returns.
    /// Random catalogs (several runs with holes, several levels, valid times on a 10 minute
    /// grid so ties and exact matches both happen), random series windows and instants.
    #[test]
    fn nearest_agrees_with_the_sql_lookup_whenever_it_commits() {
        let mut state = 0x9E3779B97F4A7C15u64;
        let mut next = move |n: u64| {
            // xorshift64*
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            state.wrapping_mul(0x2545F4914F6CDD1D) % n
        };
        let min = 60_000_000i64;
        let (mut committed, mut exact, mut declined) = (0, 0, 0);
        for _ in 0..300 {
            // a catalog: 1-5 runs, each with a random subset of steps, 2 levels
            let mut rows = Vec::new();
            let runs = 1 + next(5) as i64;
            for run in 0..runs {
                let reference = run * 360 * min; // 6 hourly runs
                let step = [10, 30, 60, 180][next(4) as usize] * min;
                for k in 0..(20 + next(60) as i64) {
                    for level in 0..2u8 {
                        if next(100) < 85 {
                            let id = rows.len();
                            rows.push(Row {
                                id,
                                level,
                                valid: reference + k * step,
                                reference,
                                // unique, unrelated to reference order on purpose
                                ingested: id as i64 * 7919 % 10_007,
                            });
                        }
                    }
                }
            }
            // a series: some instants, on and off the grid
            let n = 1 + next(40) as usize;
            let base = next(30 * 6) as i64 * 10 * min;
            let req: Vec<i64> = (0..n)
                .map(|_| {
                    // on the 10 minute grid, or up to 9 minutes off it
                    let off_grid = if next(2) == 0 {
                        0
                    } else {
                        next(10) as i64 * min
                    };
                    base + next(24 * 6) as i64 * 10 * min + off_grid
                })
                .collect();
            let (lo, hi) = (*req.iter().min().unwrap(), *req.iter().max().unwrap());
            let margin = [0, 10, 60, 24 * 60][next(4) as usize] * min;
            let (from, to) = (lo - margin, hi + margin);
            for level in 0..2u8 {
                let list = listed(&rows, level, from, to);
                let valid_times: Vec<DateTime<Utc>> = list
                    .iter()
                    .map(|r| DateTime::from_timestamp_micros(r.valid).unwrap())
                    .collect();
                let reqs: Vec<DateTime<Utc>> = req
                    .iter()
                    .map(|t| DateTime::from_timestamp_micros(*t).unwrap())
                    .collect();
                let got = match_nearest_series_times(
                    &valid_times,
                    &reqs,
                    (
                        DateTime::from_timestamp_micros(from).unwrap(),
                        DateTime::from_timestamp_micros(to).unwrap(),
                    ),
                );
                for (t, m) in req.iter().zip(got) {
                    match m {
                        Some(i) => {
                            committed += 1;
                            if list[i].valid == *t {
                                exact += 1;
                            }
                            assert_eq!(
                                Some(list[i].id),
                                sql_nearest(&rows, level, *t),
                                "t={t} level={level} window=({from},{to})"
                            );
                        }
                        None => declined += 1,
                    }
                }
            }
        }
        // the property must not hold vacuously: both kinds of answer occur a lot
        assert!(committed > 3000, "committed {committed}");
        assert!(exact > 300, "exact {exact}");
        assert!(declined > 100, "declined {declined}");
    }
}
