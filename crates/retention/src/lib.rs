//! Data retention and storage-reconciliation background tasks.
//!
//! This crate handles automatic cleanup of expired datasets based on
//! retention settings in model configuration files, plus reconciliation
//! between the database catalog and object storage.
//!
//! # Why this is a standalone crate
//!
//! These tasks previously ran inside `wms-api`. In Aug 2026 a Redis outage
//! prevented `wms-api` from starting, which silently stopped retention for
//! ~3 weeks: object storage grew 165 GB -> 623 GB, the disk hit 100%, Postgres
//! died mid-WAL-redo and every API returned 502.
//!
//! Retention is now hosted by the `ingester`, which has a much smaller
//! dependency surface (no Redis) and is already essential to the ingest
//! pipeline. `wms-api` still links this crate to serve its admin endpoints,
//! but no longer runs the background loops.
//!
//! See `docs/incident-2026-08-disk-exhaustion.md`.
//!
//! # Retention Safeguards
//!
//! The cleanup system includes safeguards to prevent data loss during ingestion outages:
//!
//! ## Forecast Models (HRRR, GFS)
//! - Uses `keep_latest_runs` to protect the N most recent complete model runs
//! - A run is "complete" when all expected forecast hours are ingested
//! - Only deletes old runs when a newer complete run exists
//!
//! ## Observation Models (MRMS, GOES)
//! - Uses `keep_latest_observations` to always keep the N most recent observations
//! - Prevents deletion even if data exceeds retention hours
//!
//! # Deleting expired data
//!
//! Marking datasets `expired` is cheap; removing their files is not. The deletion
//! phase of [`CleanupTask::run_once`] therefore
//!
//! - deletes each expired `.zarr` directory with one bulk prefix delete
//!   (`ObjectStorage::delete_prefix`), keeping per-zarr prefixes because listing a
//!   whole run directory is far too slow on RustFS,
//! - runs up to `CLEANUP_DELETE_CONCURRENCY` (default 16) of those at once,
//! - removes a path's database rows only after its files are gone, in batches as it
//!   goes, so a restart mid-cycle keeps its progress and a path whose storage delete
//!   failed stays `expired` and is retried next cycle, and
//! - skips (and logs) a model whose marking fails instead of aborting the cycle.
//!
//! A serial per-object delete managed ~4 s per zarr in production; with 65k expired
//! paths queued that was days of work, slower than ingest added data, and the disk
//! filled.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use tokio::time::{interval_at, Duration as TokioDuration, Instant, MissedTickBehavior};
use tracing::{debug, error, info, warn};

use storage::{Catalog, ObjectStorage};

/// Everything the retention tasks need to do their job.
///
/// Deliberately minimal: just the database catalog and object storage. Keeping
/// this small is what allows retention to run in any service (it is hosted by
/// the ingester) rather than being tied to one service's full application
/// state.
///
/// Cloning is cheap: `Catalog` shares its connection pool and `ObjectStorage`
/// is behind an `Arc`.
#[derive(Clone)]
pub struct RetentionContext {
    /// Database catalog (dataset records, expiry marking, deletion).
    pub catalog: Catalog,
    /// Object storage holding the zarr grids.
    pub storage: Arc<ObjectStorage>,
}

impl RetentionContext {
    /// Create a context from a catalog and object storage handle.
    pub fn new(catalog: Catalog, storage: Arc<ObjectStorage>) -> Self {
        Self { catalog, storage }
    }
}

/// Model type for determining retention behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    /// Forecast model with runs and forecast hours (e.g., HRRR, GFS)
    Forecast,
    /// Observation model with time-based data (e.g., MRMS, GOES)
    Observation,
    /// Static dataset that never expires (e.g., VIIRS nighttime lights)
    Static,
}

impl Default for ModelType {
    fn default() -> Self {
        ModelType::Forecast
    }
}

/// Per-model retention settings.
#[derive(Debug, Clone)]
pub struct ModelRetentionConfig {
    /// Retention hours (time-based cutoff)
    pub hours: u32,
    /// Model type (forecast or observation)
    pub model_type: ModelType,
    /// For forecast models: number of complete runs to always keep
    pub keep_latest_runs: u32,
    /// For observation models: number of observations to always keep
    pub keep_latest_observations: u32,
    /// For forecast models: expected number of forecast hours per run
    pub expected_forecast_hours: Option<u32>,
}

impl Default for ModelRetentionConfig {
    fn default() -> Self {
        Self {
            hours: 24,
            model_type: ModelType::Forecast,
            keep_latest_runs: 1,
            keep_latest_observations: 1,
            expected_forecast_hours: None,
        }
    }
}

/// Configuration for the cleanup task.
#[derive(Debug, Clone)]
pub struct CleanupConfig {
    /// Whether cleanup is enabled
    pub enabled: bool,
    /// How often to run cleanup (in seconds)
    pub interval_secs: u64,
    /// Per-model retention settings (full config)
    pub model_configs: HashMap<String, ModelRetentionConfig>,
    /// Default retention if model not specified (in hours)
    pub default_retention_hours: u32,
    /// How many expired storage paths (zarr directories) are deleted from
    /// object storage at the same time (`CLEANUP_DELETE_CONCURRENCY`).
    pub delete_concurrency: usize,
}

/// Default for [`CleanupConfig::delete_concurrency`].
pub const DEFAULT_DELETE_CONCURRENCY: usize = 16;

/// Parse `CLEANUP_DELETE_CONCURRENCY`; unset, unparsable or zero falls back to
/// [`DEFAULT_DELETE_CONCURRENCY`] (zero would stall the delete stream).
fn parse_delete_concurrency(raw: Option<&str>) -> usize {
    raw.and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_DELETE_CONCURRENCY)
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 3600, // Run every hour
            model_configs: HashMap::new(),
            default_retention_hours: 24, // 24 hours default
            delete_concurrency: DEFAULT_DELETE_CONCURRENCY,
        }
    }
}

/// Structure matching retention section in model YAML files.
#[derive(Debug, Deserialize)]
struct ModelConfigRetention {
    hours: Option<u32>,
    keep_latest_runs: Option<u32>,
    keep_latest_observations: Option<u32>,
    /// If true, data never expires (for static datasets like VIIRS)
    #[serde(rename = "static")]
    is_static: Option<bool>,
}

/// Structure matching dimensions section in model YAML files.
#[derive(Debug, Deserialize)]
struct ModelConfigDimensions {
    #[serde(rename = "type")]
    dimension_type: Option<String>,
}

/// Structure matching schedule section in model YAML files.
#[derive(Debug, Deserialize)]
struct ModelConfigSchedule {
    forecast_hours: Option<ForecastHoursConfig>,
}

/// Structure matching forecast_hours in schedule section.
/// Supports both the legacy single-range form ({start, end, step}) and the
/// multi-range form ({ranges: [{start, end, step}, ...]}).
#[derive(Debug, Deserialize)]
struct ForecastHoursConfig {
    start: Option<u32>,
    end: Option<u32>,
    step: Option<u32>,
    ranges: Option<Vec<ForecastRangeConfig>>,
}

#[derive(Debug, Deserialize)]
struct ForecastRangeConfig {
    start: Option<u32>,
    end: Option<u32>,
    step: Option<u32>,
}

impl ForecastHoursConfig {
    /// Total number of expected forecast hours (deduplicated across ranges).
    fn expected_count(&self) -> Option<u32> {
        if let Some(ref ranges) = self.ranges {
            let mut hours: Vec<u32> = Vec::new();
            for r in ranges {
                let start = r.start.unwrap_or(0);
                let end = r.end?;
                let step = r.step.unwrap_or(1).max(1);
                hours.extend((start..=end).step_by(step as usize));
            }
            hours.sort_unstable();
            hours.dedup();
            Some(hours.len() as u32)
        } else {
            let start = self.start.unwrap_or(0);
            let end = self.end?;
            let step = self.step.unwrap_or(1);
            if step > 0 {
                Some((end - start) / step + 1)
            } else {
                None
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ModelConfigFile {
    model: ModelConfigModel,
    dimensions: Option<ModelConfigDimensions>,
    schedule: Option<ModelConfigSchedule>,
    retention: Option<ModelConfigRetention>,
}

#[derive(Debug, Deserialize)]
struct ModelConfigModel {
    id: String,
    /// `model.enabled`; absent means enabled. A disabled model is no longer
    /// downloaded or ingested, so whatever is left in the catalog is purged
    /// (see [`purge_everything_if_disabled`]).
    enabled: Option<bool>,
}

/// Retention settings for a model that `model.enabled: false` switched off.
///
/// A disabled model is not downloaded or ingested any more, so nothing will
/// ever refresh or supersede what it left in the catalog. The usual
/// safeguards would then pin that data forever: `keep_latest_runs` /
/// `keep_latest_observations` always keep the newest N runs/observations, and
/// `hours` keeps the rest until it ages out. For a disabled model the whole
/// point is to reclaim the space, so everything is expired on the next cycle
/// (cutoff = now, nothing protected).
///
/// Static datasets (`retention.static: true` / `dimensions.type: static`, e.g.
/// VIIRS, which is `enabled: false` only because it is ingested by hand) are
/// left alone: they never expire by design.
fn purge_everything_if_disabled(
    enabled: bool,
    config: ModelRetentionConfig,
) -> ModelRetentionConfig {
    if enabled || config.model_type == ModelType::Static {
        return config;
    }
    ModelRetentionConfig {
        hours: 0,
        keep_latest_runs: 0,
        keep_latest_observations: 0,
        ..config
    }
}

impl CleanupConfig {
    /// Load cleanup configuration from environment and model config files.
    pub fn from_env_and_configs(config_dir: &str) -> Self {
        let enabled = std::env::var("ENABLE_CLEANUP")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(true); // Enabled by default

        let interval_secs = std::env::var("CLEANUP_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600); // 1 hour default

        let default_retention_hours = std::env::var("DEFAULT_RETENTION_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24); // 24 hours default

        let delete_concurrency =
            parse_delete_concurrency(std::env::var("CLEANUP_DELETE_CONCURRENCY").ok().as_deref());

        // Load model retention settings from config files
        let model_configs = Self::load_model_configs(config_dir);

        Self {
            enabled,
            interval_secs,
            model_configs,
            default_retention_hours,
            delete_concurrency,
        }
    }

    /// Load retention settings from model YAML files.
    fn load_model_configs(config_dir: &str) -> HashMap<String, ModelRetentionConfig> {
        let mut configs = HashMap::new();
        let models_dir = Path::new(config_dir).join("models");

        if !models_dir.exists() {
            warn!("Model config directory not found: {:?}", models_dir);
            return configs;
        }

        // Read all YAML files in the models directory
        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
                    if let Ok(contents) = std::fs::read_to_string(&path) {
                        match serde_yaml::from_str::<ModelConfigFile>(&contents) {
                            Ok(config) => {
                                let model_id = config.model.id.clone();

                                // Get retention settings first to check for static flag
                                let retention = config.retention.as_ref();
                                let is_static_retention =
                                    retention.and_then(|r| r.is_static).unwrap_or(false);

                                // Determine model type from dimensions.type or retention.static
                                let model_type = if is_static_retention {
                                    // retention.static: true takes precedence
                                    ModelType::Static
                                } else {
                                    config
                                        .dimensions
                                        .as_ref()
                                        .and_then(|d| d.dimension_type.as_ref())
                                        .map(|t| match t.as_str() {
                                            "observation" => ModelType::Observation,
                                            "static" => ModelType::Static,
                                            _ => ModelType::Forecast,
                                        })
                                        .unwrap_or(ModelType::Forecast)
                                };

                                // Calculate expected forecast hours from schedule
                                // (handles both single-range and multi-range forms)
                                let expected_forecast_hours = config
                                    .schedule
                                    .as_ref()
                                    .and_then(|s| s.forecast_hours.as_ref())
                                    .and_then(|fh| fh.expected_count());

                                // Get hours setting (not used for static models but kept for config consistency)
                                let hours = retention.and_then(|r| r.hours).unwrap_or(24);

                                // Get safeguard settings with defaults
                                let keep_latest_runs =
                                    retention.and_then(|r| r.keep_latest_runs).unwrap_or(1); // Default: always keep at least 1 run

                                let keep_latest_observations = retention
                                    .and_then(|r| r.keep_latest_observations)
                                    .unwrap_or(1); // Default: always keep at least 1 observation

                                let model_config = purge_everything_if_disabled(
                                    config.model.enabled.unwrap_or(true),
                                    ModelRetentionConfig {
                                        hours,
                                        model_type,
                                        keep_latest_runs,
                                        keep_latest_observations,
                                        expected_forecast_hours,
                                    },
                                );

                                info!(
                                    model = %model_id,
                                    enabled = config.model.enabled.unwrap_or(true),
                                    retention_hours = model_config.hours,
                                    model_type = ?model_config.model_type,
                                    keep_latest_runs = model_config.keep_latest_runs,
                                    keep_latest_observations = model_config.keep_latest_observations,
                                    expected_forecast_hours = ?model_config.expected_forecast_hours,
                                    "Loaded retention config"
                                );

                                configs.insert(model_id, model_config);
                            }
                            Err(e) => {
                                warn!(
                                    path = ?path,
                                    error = %e,
                                    "Failed to parse model config"
                                );
                            }
                        }
                    }
                }
            }
        }

        configs
    }

    /// Get retention config for a model.
    pub fn get_model_config(&self, model: &str) -> ModelRetentionConfig {
        self.model_configs
            .get(model)
            .cloned()
            .unwrap_or(ModelRetentionConfig {
                hours: self.default_retention_hours,
                ..Default::default()
            })
    }

    /// Get retention hours for a model (for backward compatibility).
    pub fn get_retention_hours(&self, model: &str) -> u32 {
        self.get_model_config(model).hours
    }

    /// For backward compatibility: get model_retentions as HashMap<String, u32>
    pub fn model_retentions(&self) -> HashMap<String, u32> {
        self.model_configs
            .iter()
            .map(|(k, v)| (k.clone(), v.hours))
            .collect()
    }
}

/// Configuration for the sync task.
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Whether sync is enabled
    pub enabled: bool,
    /// How often to run sync (in seconds)
    pub interval_secs: u64,
    /// Grace period (in seconds): objects written / rows ingested within this
    /// window are never treated as orphans. This prevents a race where data
    /// ingested while a sync cycle is in flight (bucket listings can take
    /// tens of seconds on large buckets) is wrongly deleted.
    pub grace_secs: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 3600, // Reconciliation job - hourly is plenty
            grace_secs: 900,     // 15 minutes
        }
    }
}

impl SyncConfig {
    /// Load sync configuration from environment.
    pub fn from_env() -> Self {
        let enabled = std::env::var("ENABLE_SYNC")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(true); // Enabled by default

        let interval_secs = std::env::var("SYNC_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600); // 1 hour default

        let grace_secs = std::env::var("SYNC_GRACE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(900); // 15 minute default

        Self {
            enabled,
            interval_secs,
            grace_secs,
        }
    }
}

/// Information about a protected run (for admin visibility).
#[derive(Debug, Clone, Serialize)]
pub struct ProtectedRunInfo {
    pub reference_time: DateTime<Utc>,
    pub dataset_count: i64,
    pub is_complete: bool,
    pub reason: String,
}

/// Background cleanup task.
pub struct CleanupTask {
    ctx: RetentionContext,
    config: CleanupConfig,
}

impl CleanupTask {
    /// Create a new cleanup task.
    pub fn new(ctx: RetentionContext, config: CleanupConfig) -> Self {
        Self { ctx, config }
    }

    /// Get the cleanup configuration.
    pub fn config(&self) -> &CleanupConfig {
        &self.config
    }

    /// Run the cleanup task once.
    ///
    /// Phase 1 marks datasets past retention as `expired`, model by model; a
    /// model whose marking fails is logged and skipped so it cannot block the
    /// others. Phase 2 deletes the files of every expired dataset (including
    /// ones marked by earlier cycles) concurrently and removes each database
    /// row only once its files are gone, so an interrupted or partially failed
    /// cycle keeps its progress and the leftovers are retried next time.
    pub async fn run_once(&self) -> Result<CleanupStats> {
        let mut stats = CleanupStats::default();

        info!("Starting cleanup cycle");

        // Get list of models from the database
        let models = self.ctx.catalog.list_models().await?;

        for model in &models {
            let model_config = self.config.get_model_config(model);

            // Skip static models - they never expire
            if model_config.model_type == ModelType::Static {
                debug!(
                    model = %model,
                    "Skipping cleanup for static dataset (never expires)"
                );
                continue;
            }

            let cutoff = Utc::now() - Duration::hours(model_config.hours as i64);

            info!(
                model = %model,
                retention_hours = model_config.hours,
                model_type = ?model_config.model_type,
                cutoff = %cutoff,
                "Checking for expired datasets"
            );

            let marked = match model_config.model_type {
                ModelType::Forecast => {
                    self.cleanup_forecast_model(model, &model_config, cutoff)
                        .await
                }
                ModelType::Observation => {
                    self.cleanup_observation_model(model, &model_config, cutoff)
                        .await
                }
                ModelType::Static => {
                    // Already handled above with continue, but need for exhaustive match
                    Ok(0)
                }
            };

            match marked {
                Ok(0) => {}
                Ok(count) => {
                    info!(model = %model, count, "Marked datasets as expired");
                    stats.marked_expired += count;
                }
                Err(e) => {
                    // One model's failure must not stop the others being
                    // marked, nor the deletion phase below.
                    warn!(
                        model = %model,
                        error = %e,
                        "Failed to mark expired datasets for model; skipping it this cycle"
                    );
                    stats.mark_errors += 1;
                }
            }
        }

        // Get storage paths of expired datasets
        let expired_paths = self.ctx.catalog.get_expired_storage_paths().await?;

        if !expired_paths.is_empty() {
            info!(
                count = expired_paths.len(),
                concurrency = self.config.delete_concurrency,
                "Deleting expired files from storage"
            );

            let storage = &self.ctx.storage;
            let catalog = &self.ctx.catalog;
            let outcome = purge_expired_paths(
                expired_paths,
                self.config.delete_concurrency,
                DB_DELETE_BATCH_SIZE,
                |path| async move { delete_storage_path(storage, &path).await },
                |paths| async move { Ok(catalog.delete_expired_by_paths(&paths).await?) },
            )
            .await?;

            stats.files_deleted = outcome.paths_deleted;
            stats.objects_deleted = outcome.objects_deleted;
            stats.delete_errors = outcome.delete_errors;
            stats.records_deleted = outcome.records_deleted;

            // NOTE: this previously invalidated wms-api's in-process
            // GetCapabilities cache. Retention now runs outside wms-api, so
            // that is no longer reachable - and it is not needed: the
            // capabilities cache has a short TTL (CAPABILITIES_CACHE_TTL_SECS,
            // default 120s), so documents refresh on their own shortly after
            // data is removed.
        }

        info!(
            marked = stats.marked_expired,
            mark_errors = stats.mark_errors,
            files = stats.files_deleted,
            objects = stats.objects_deleted,
            records = stats.records_deleted,
            errors = stats.delete_errors,
            "Cleanup cycle complete"
        );

        Ok(stats)
    }

    /// Cleanup logic for forecast models (HRRR, GFS).
    /// Protects the latest N runs (by reference_time) before deleting old data.
    async fn cleanup_forecast_model(
        &self,
        model: &str,
        config: &ModelRetentionConfig,
        cutoff: DateTime<Utc>,
    ) -> Result<u64> {
        // Get all runs for this model, sorted by reference_time DESC
        let runs = self.ctx.catalog.get_model_runs_with_counts(model).await?;

        if runs.is_empty() {
            debug!(model = %model, "No runs found, skipping cleanup");
            return Ok(0);
        }

        // Protect the N most recent runs regardless of completeness
        // The query already returns runs sorted by reference_time DESC
        let runs_to_protect: Vec<DateTime<Utc>> = runs
            .iter()
            .take(config.keep_latest_runs as usize)
            .map(|(ref_time, _)| *ref_time)
            .collect();

        info!(
            model = %model,
            protected_runs = ?runs_to_protect.iter().map(|r| r.to_rfc3339()).collect::<Vec<_>>(),
            total_runs = runs.len(),
            keep_latest_runs = config.keep_latest_runs,
            "Protecting most recent runs from cleanup"
        );

        // Mark expired datasets where reference_time < cutoff, except protected runs
        self.ctx
            .catalog
            .mark_model_expired_except_runs(model, cutoff, &runs_to_protect)
            .await
            .map_err(Into::into)
    }

    /// Cleanup logic for observation models (MRMS, GOES).
    /// Always keeps the latest N observations regardless of age.
    async fn cleanup_observation_model(
        &self,
        model: &str,
        config: &ModelRetentionConfig,
        cutoff: DateTime<Utc>,
    ) -> Result<u64> {
        // For observation models, each reference_time is essentially one observation
        let observations = self.ctx.catalog.get_model_runs_with_counts(model).await?;

        if observations.is_empty() {
            debug!(model = %model, "No observations found, skipping cleanup");
            return Ok(0);
        }

        // Sort by time descending (newest first)
        let mut sorted_obs: Vec<DateTime<Utc>> = observations.iter().map(|(t, _)| *t).collect();
        sorted_obs.sort_by(|a, b| b.cmp(a));

        // Protect the latest N observations
        let protected_obs: Vec<DateTime<Utc>> = sorted_obs
            .iter()
            .take(config.keep_latest_observations as usize)
            .cloned()
            .collect();

        info!(
            model = %model,
            total_observations = observations.len(),
            protected_count = protected_obs.len(),
            keep_latest = config.keep_latest_observations,
            "Protecting latest observations from cleanup"
        );

        // Mark expired datasets, but exclude protected observations
        self.ctx
            .catalog
            .mark_model_expired_except_runs(model, cutoff, &protected_obs)
            .await
            .map_err(Into::into)
    }

    /// Get information about protected runs for a model (for admin API).
    pub async fn get_protected_runs(&self, model: &str) -> Result<Vec<ProtectedRunInfo>> {
        let model_config = self.config.get_model_config(model);
        // Runs are already sorted by reference_time DESC from the query
        let runs = self.ctx.catalog.get_model_runs_with_counts(model).await?;

        if runs.is_empty() {
            return Ok(vec![]);
        }

        match model_config.model_type {
            ModelType::Forecast => {
                // Protect the N most recent runs by reference_time
                let protected: Vec<ProtectedRunInfo> = runs
                    .iter()
                    .take(model_config.keep_latest_runs as usize)
                    .enumerate()
                    .map(|(idx, (ref_time, count))| ProtectedRunInfo {
                        reference_time: *ref_time,
                        dataset_count: *count,
                        is_complete: true,
                        reason: format!(
                            "Recent run #{}, protected by keep_latest_runs={}",
                            idx + 1,
                            model_config.keep_latest_runs
                        ),
                    })
                    .collect();

                Ok(protected)
            }
            ModelType::Observation => {
                let protected: Vec<ProtectedRunInfo> = runs
                    .iter()
                    .take(model_config.keep_latest_observations as usize)
                    .map(|(ref_time, count)| ProtectedRunInfo {
                        reference_time: *ref_time,
                        dataset_count: *count,
                        is_complete: true,
                        reason: format!(
                            "Latest observation, protected by keep_latest_observations={}",
                            model_config.keep_latest_observations
                        ),
                    })
                    .collect();

                Ok(protected)
            }
            ModelType::Static => {
                // All data in a static dataset is protected (never expires)
                let protected: Vec<ProtectedRunInfo> = runs
                    .iter()
                    .map(|(ref_time, count)| ProtectedRunInfo {
                        reference_time: *ref_time,
                        dataset_count: *count,
                        is_complete: true,
                        reason: "Static dataset - never expires".to_string(),
                    })
                    .collect();

                Ok(protected)
            }
        }
    }

    /// Run the cleanup task in a loop.
    pub async fn run_forever(self) {
        if !self.config.enabled {
            info!("Cleanup task disabled");
            return;
        }

        info!(
            interval_secs = self.config.interval_secs,
            "Starting cleanup background task"
        );

        run_now_then_every(
            TokioDuration::from_secs(self.config.interval_secs),
            || async {
                if let Err(e) = self.run_once().await {
                    error!(error = %e, "Cleanup cycle failed");
                }
            },
        )
        .await;
    }
}

/// Statistics from a cleanup run.
#[derive(Debug, Default, Clone, Serialize)]
pub struct CleanupStats {
    /// Number of datasets marked as expired
    pub marked_expired: u64,
    /// Number of models whose expiry marking failed (skipped for this cycle)
    pub mark_errors: u64,
    /// Number of storage paths (zarr directories / single files) deleted from storage
    pub files_deleted: u64,
    /// Number of individual objects removed from storage
    pub objects_deleted: u64,
    /// Number of database records deleted
    pub records_deleted: u64,
    /// Number of storage paths whose delete failed (their rows are kept for retry)
    pub delete_errors: u64,
}

/// Expired rows are removed from the database in batches of this many
/// successfully deleted paths, so a restart mid-cycle loses little progress.
const DB_DELETE_BATCH_SIZE: usize = 500;

/// Log a progress line every this many processed paths.
const PROGRESS_LOG_EVERY: usize = 1000;

/// Delete everything stored for one catalog `storage_path`; returns how many
/// objects were removed.
///
/// A `.zarr` path is a directory of objects (7-11 for HRRR/GOES) removed with
/// one prefix delete. The trailing slash keeps it inside that directory. Only
/// per-zarr prefixes are used: listing a whole run directory is far too slow
/// on RustFS.
async fn delete_storage_path(storage: &ObjectStorage, path: &str) -> Result<u64> {
    if path.ends_with(".zarr") {
        Ok(storage.delete_prefix(&format!("{}/", path)).await?)
    } else {
        storage.delete(path).await?;
        Ok(1)
    }
}

/// Result of [`purge_expired_paths`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PurgeOutcome {
    /// Paths whose storage delete succeeded.
    paths_deleted: u64,
    /// Objects removed from storage across those paths.
    objects_deleted: u64,
    /// Paths whose storage delete failed; their rows were NOT deleted.
    delete_errors: u64,
    /// Database rows deleted.
    records_deleted: u64,
}

/// Delete expired storage paths concurrently and their database rows
/// incrementally.
///
/// - `delete_path(path)` removes a path's files from storage and returns how
///   many objects it removed. Up to `concurrency` calls run at once.
/// - `delete_rows(paths)` removes the database rows of paths whose files are
///   gone and returns how many rows it deleted. It is called with at most
///   `batch_size` paths, whenever that many have accumulated and once more at
///   the end for the remainder.
///
/// A path whose `delete_path` failed is logged and counted, and is never
/// passed to `delete_rows`, so its row stays `expired` and is retried by the
/// next cycle. A `delete_rows` failure aborts the run (the database is
/// unusable); the files of the unflushed paths are already gone, which makes
/// their retry next cycle a cheap no-op.
async fn purge_expired_paths<D, DF, DE, R, RF>(
    paths: Vec<String>,
    concurrency: usize,
    batch_size: usize,
    delete_path: D,
    delete_rows: R,
) -> Result<PurgeOutcome>
where
    D: Fn(String) -> DF,
    DF: Future<Output = std::result::Result<u64, DE>>,
    DE: Display,
    R: Fn(Vec<String>) -> RF,
    RF: Future<Output = Result<u64>>,
{
    let total = paths.len();
    let batch_size = batch_size.max(1);
    let started = Instant::now();
    let mut outcome = PurgeOutcome::default();
    let mut done_paths: Vec<String> = Vec::with_capacity(batch_size);
    let mut processed = 0usize;

    let mut results = stream::iter(paths)
        .map(|path| {
            let deleting = delete_path(path.clone());
            async move { (path, deleting.await) }
        })
        .buffer_unordered(concurrency.max(1));

    while let Some((path, result)) = results.next().await {
        processed += 1;
        match result {
            Ok(objects) => {
                outcome.paths_deleted += 1;
                outcome.objects_deleted += objects;
                done_paths.push(path);
                if done_paths.len() >= batch_size {
                    outcome.records_deleted += flush_rows(&delete_rows, &mut done_paths).await?;
                }
            }
            Err(e) => {
                // Keep the row: the next cycle retries this path.
                warn!(path = %path, error = %e, "Failed to delete file");
                outcome.delete_errors += 1;
            }
        }

        if processed % PROGRESS_LOG_EVERY == 0 && processed < total {
            let elapsed = started.elapsed().as_secs_f64();
            info!(
                done = processed,
                total,
                deleted = outcome.paths_deleted,
                errors = outcome.delete_errors,
                rate_per_sec = format!("{:.1}", processed as f64 / elapsed.max(f64::EPSILON)),
                "Expired file deletion progress"
            );
        }
    }

    outcome.records_deleted += flush_rows(&delete_rows, &mut done_paths).await?;

    let elapsed = started.elapsed();
    info!(
        total,
        files_deleted = outcome.paths_deleted,
        objects_deleted = outcome.objects_deleted,
        delete_errors = outcome.delete_errors,
        records_deleted = outcome.records_deleted,
        elapsed_secs = format!("{:.1}", elapsed.as_secs_f64()),
        rate_per_sec = format!(
            "{:.1}",
            total as f64 / elapsed.as_secs_f64().max(f64::EPSILON)
        ),
        "Finished deleting expired files"
    );

    Ok(outcome)
}

/// Delete the database rows of the accumulated, already-deleted paths.
async fn flush_rows<R, RF>(delete_rows: &R, done_paths: &mut Vec<String>) -> Result<u64>
where
    R: Fn(Vec<String>) -> RF,
    RF: Future<Output = Result<u64>>,
{
    if done_paths.is_empty() {
        return Ok(0);
    }
    let batch = std::mem::take(done_paths);
    let batch_len = batch.len();
    match delete_rows(batch).await {
        Ok(rows) => {
            debug!(
                paths = batch_len,
                rows, "Deleted expired records from database"
            );
            Ok(rows)
        }
        Err(e) => {
            error!(paths = batch_len, error = %e, "Failed to delete expired records from database");
            Err(e)
        }
    }
}

/// Statistics from a sync operation.
#[derive(Debug, Default, Clone, Serialize)]
pub struct SyncStats {
    /// Number of database records checked
    pub db_records_checked: u64,
    /// Number of MinIO objects checked
    pub minio_objects_checked: u64,
    /// Number of orphan DB records found (no file in MinIO)
    pub orphan_db_records: u64,
    /// Number of orphan MinIO objects found (no DB record)
    pub orphan_minio_objects: u64,
    /// Number of orphan DB records deleted
    pub orphan_db_deleted: u64,
    /// Number of orphan MinIO objects deleted
    pub orphan_minio_deleted: u64,
    /// Errors encountered during sync
    pub errors: Vec<String>,
}

/// Detailed preview of what will be synced.
#[derive(Debug, Default, Clone, Serialize)]
pub struct SyncPreview {
    /// Paths in database but not in MinIO storage
    pub orphan_db_paths: Vec<String>,
    /// Paths in MinIO storage but not in database
    pub orphan_minio_paths: Vec<String>,
    /// Total database records checked
    pub db_records_checked: u64,
    /// Total MinIO objects checked
    pub minio_objects_checked: u64,
}

/// Result of comparing DB and MinIO state (orphan candidates after
/// grace-period filtering).
#[derive(Debug, Default, Clone)]
struct SyncAnalysis {
    /// Paths in database but not in MinIO storage
    orphan_db_paths: Vec<String>,
    /// Paths in MinIO storage but not in database
    orphan_minio_paths: Vec<String>,
    /// Total database records checked
    db_records_checked: u64,
    /// Total MinIO objects checked
    minio_objects_checked: u64,
}

/// Run `task` now, then once every `period`, forever.
///
/// A tokio interval's first tick completes immediately, so "run once, then loop on
/// `interval(period)`" ran the task twice at startup: after every ingester restart the sync
/// listed the whole bucket (~20 minutes) twice back to back. The ticker starts one period out
/// instead, and a run that overruns its period skips the missed ticks rather than firing them
/// in a burst.
async fn run_now_then_every<F, Fut>(period: TokioDuration, mut task: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut ticker = interval_at(Instant::now() + period, period);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    task().await;
    loop {
        ticker.tick().await;
        task().await;
    }
}

/// Decide which DB rows and MinIO objects are orphans.
///
/// - `minio`: logical path -> newest last-modified, from a listing that began at `listing_started`.
/// - `db_rows`: (storage path, newest ingested_at), snapshotted after the listing finished.
///
/// A DB row is an orphan only if its path is missing from the listing AND it was ingested more
/// than `grace` before the listing *started*. Anything written after that may have landed under
/// a prefix the lister had already passed, so its absence from the listing proves nothing. (The
/// grace used to be measured back from the snapshot, but the listing takes longer than the
/// grace: rows ingested in the listing's first minutes were deleted with their objects intact.)
///
/// A MinIO object is an orphan if no row references it at snapshot time and it was last
/// modified more than `grace` before `now`. Only `shredded/` and `grids/` count; `raw/` files
/// may be awaiting processing.
fn find_orphans(
    minio: &HashMap<String, DateTime<Utc>>,
    db_rows: &[(String, DateTime<Utc>)],
    listing_started: DateTime<Utc>,
    now: DateTime<Utc>,
    grace: Duration,
) -> (Vec<String>, Vec<String>) {
    let row_cutoff = listing_started - grace;
    let orphan_db: Vec<String> = db_rows
        .iter()
        .filter(|(path, ingested_at)| *ingested_at < row_cutoff && !minio.contains_key(path))
        .map(|(path, _)| path.clone())
        .collect();

    let db_paths: std::collections::HashSet<&str> =
        db_rows.iter().map(|(path, _)| path.as_str()).collect();
    let orphan_minio: Vec<String> = minio
        .iter()
        .filter(|(path, newest_modified)| {
            (path.starts_with("shredded/") || path.starts_with("grids/"))
                && !db_paths.contains(path.as_str())
                && now.signed_duration_since(**newest_modified) > grace
        })
        .map(|(path, _)| path.clone())
        .collect();

    (orphan_db, orphan_minio)
}

/// Sync task for reconciling database and MinIO storage.
pub struct SyncTask {
    ctx: RetentionContext,
    config: SyncConfig,
}

impl SyncTask {
    /// Create a new sync task.
    pub fn new(ctx: RetentionContext, config: SyncConfig) -> Self {
        Self { ctx, config }
    }

    /// Create a new sync task with default config (for admin API use).
    pub fn new_default(ctx: RetentionContext) -> Self {
        Self {
            ctx,
            config: SyncConfig::default(),
        }
    }

    /// Run the sync task in a loop.
    pub async fn run_forever(self) {
        if !self.config.enabled {
            info!("Sync task disabled");
            return;
        }

        info!(
            interval_secs = self.config.interval_secs,
            "Starting sync background task"
        );

        run_now_then_every(
            TokioDuration::from_secs(self.config.interval_secs),
            || async {
                if let Err(e) = self.run().await {
                    error!(error = %e, "Sync cycle failed");
                }
            },
        )
        .await;
    }

    /// Perform a dry-run sync to identify orphans without deleting.
    pub async fn dry_run(&self) -> Result<SyncStats> {
        self.run_sync(false).await
    }

    /// Perform a sync and delete orphans.
    pub async fn run(&self) -> Result<SyncStats> {
        self.run_sync(true).await
    }

    /// Get detailed preview of orphan paths without deleting anything.
    pub async fn preview(&self) -> Result<SyncPreview> {
        info!("Generating sync preview");

        let analysis = self.analyze().await?;

        Ok(SyncPreview {
            orphan_db_paths: analysis.orphan_db_paths,
            orphan_minio_paths: analysis.orphan_minio_paths,
            db_records_checked: analysis.db_records_checked,
            minio_objects_checked: analysis.minio_objects_checked,
        })
    }

    /// Analyze DB and MinIO state and compute orphan candidates.
    ///
    /// # Race-safety
    ///
    /// The ingester uploads zarr objects to MinIO *before* registering the
    /// catalog row, and listing a large bucket takes a long time (18 minutes
    /// for 950k objects in production). A naive snapshot comparison therefore
    /// deletes freshly ingested data (observed in production: MRMS, ingesting
    /// every 2 min, was wiped continuously once the bucket grew large enough).
    ///
    /// The MinIO listing is taken first and the DB snapshot after it, and
    /// [`find_orphans`] applies the grace period; see there for the rules.
    async fn analyze(&self) -> Result<SyncAnalysis> {
        let grace = Duration::seconds(self.config.grace_secs as i64);
        let listing_started = Utc::now();

        // Step 1: Get all objects from MinIO (with last-modified timestamps).
        // Need to check shredded/ (legacy GRIB2 data), raw/ (GOES NetCDF data),
        // and grids/ (Zarr data). This listing is taken BEFORE the DB snapshot
        // (see race-safety notes above).
        let mut minio_paths = self.ctx.storage.list_with_modified("shredded/").await?;
        let raw_paths = self.ctx.storage.list_with_modified("raw/").await?;
        minio_paths.extend(raw_paths);
        let grid_paths = self.ctx.storage.list_with_modified("grids/").await?;
        minio_paths.extend(grid_paths);

        // For Zarr directories, we need to extract the parent .zarr path from
        // individual files, e.g. "grids/mrms/123/foo.zarr/c/0/0" -> "grids/mrms/123/foo.zarr".
        // Track the NEWEST last_modified per logical path so that a zarr
        // directory still being uploaded is protected by the grace period.
        let mut minio_path_ages: HashMap<String, DateTime<Utc>> = HashMap::new();
        for (path, modified) in &minio_paths {
            let logical_path = if let Some(zarr_idx) = path.find(".zarr/") {
                path[..zarr_idx + 5].to_string() // +5 for ".zarr"
            } else {
                path.clone()
            };
            minio_path_ages
                .entry(logical_path)
                .and_modify(|newest| {
                    if modified > newest {
                        *newest = *modified;
                    }
                })
                .or_insert(*modified);
        }
        let minio_objects_checked = minio_paths.len() as u64;

        info!(
            count = minio_paths.len(),
            unique_paths = minio_path_ages.len(),
            listing_secs = (Utc::now() - listing_started).num_seconds(),
            "Retrieved MinIO objects (shredded + raw + grids)"
        );

        // Step 2: DB snapshot, taken AFTER the MinIO listing completed.
        let db_rows = self
            .ctx
            .catalog
            .get_storage_paths_with_ingested_at()
            .await?;
        let db_records_checked = db_rows.len() as u64;

        info!(count = db_rows.len(), "Retrieved database storage paths");

        let (orphan_db_paths, orphan_minio_paths) = find_orphans(
            &minio_path_ages,
            &db_rows,
            listing_started,
            Utc::now(),
            grace,
        );

        Ok(SyncAnalysis {
            orphan_db_paths,
            orphan_minio_paths,
            db_records_checked,
            minio_objects_checked,
        })
    }

    /// Core sync logic.
    async fn run_sync(&self, delete: bool) -> Result<SyncStats> {
        let mut stats = SyncStats::default();

        info!(
            delete = delete,
            grace_secs = self.config.grace_secs,
            "Starting database/storage sync"
        );

        let analysis = self.analyze().await?;
        stats.db_records_checked = analysis.db_records_checked;
        stats.minio_objects_checked = analysis.minio_objects_checked;

        let orphan_db_paths = analysis.orphan_db_paths;
        stats.orphan_db_records = orphan_db_paths.len() as u64;

        if !orphan_db_paths.is_empty() {
            info!(
                count = orphan_db_paths.len(),
                "Found orphan database records (missing from MinIO)"
            );

            if orphan_db_paths.len() <= 20 {
                for path in &orphan_db_paths {
                    info!(path = %path, "Orphan DB record");
                }
            }

            if delete {
                match self
                    .ctx
                    .catalog
                    .delete_orphan_records(&orphan_db_paths)
                    .await
                {
                    Ok(deleted) => {
                        stats.orphan_db_deleted = deleted;
                        info!(count = deleted, "Deleted orphan database records");
                    }
                    Err(e) => {
                        let msg = format!("Failed to delete orphan DB records: {}", e);
                        error!("{}", msg);
                        stats.errors.push(msg);
                    }
                }
            }
        }

        let orphan_minio_paths = analysis.orphan_minio_paths;
        stats.orphan_minio_objects = orphan_minio_paths.len() as u64;

        if !orphan_minio_paths.is_empty() {
            info!(
                count = orphan_minio_paths.len(),
                "Found orphan MinIO objects (missing from database)"
            );

            if orphan_minio_paths.len() <= 20 {
                for path in &orphan_minio_paths {
                    info!(path = %path, "Orphan MinIO object");
                }
            }

            if delete {
                for path in &orphan_minio_paths {
                    // Use delete_prefix for Zarr directories to delete all files
                    let result = if path.ends_with(".zarr") {
                        // Add trailing slash to ensure we only delete within the zarr directory
                        let prefix = format!("{}/", path);
                        self.ctx.storage.delete_prefix(&prefix).await.map(|_| ())
                    } else {
                        self.ctx.storage.delete(path).await
                    };

                    match result {
                        Ok(()) => {
                            stats.orphan_minio_deleted += 1;
                        }
                        Err(e) => {
                            let msg = format!("Failed to delete MinIO object {}: {}", path, e);
                            warn!("{}", msg);
                            stats.errors.push(msg);
                        }
                    }
                }
                info!(
                    count = stats.orphan_minio_deleted,
                    "Deleted orphan MinIO objects"
                );
            }
        }

        info!(
            db_checked = stats.db_records_checked,
            minio_checked = stats.minio_objects_checked,
            orphan_db = stats.orphan_db_records,
            orphan_minio = stats.orphan_minio_objects,
            deleted_db = stats.orphan_db_deleted,
            deleted_minio = stats.orphan_minio_deleted,
            errors = stats.errors.len(),
            "Sync complete"
        );

        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Once at startup, then once per period: not twice at startup (the immediate first tick of
    /// a plain `interval`), and after a run that overran its period one catch-up run rather than
    /// one per missed tick.
    #[tokio::test(start_paused = true)]
    async fn a_periodic_task_runs_once_at_startup_then_once_per_period() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let period = TokioDuration::from_secs(3600);
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let task = tokio::spawn(run_now_then_every(period, move || {
            let counter = Arc::clone(&counter);
            async move {
                // The third run overruns by 2.5 periods, like a sync stuck behind a slow listing.
                if counter.fetch_add(1, SeqCst) == 2 {
                    tokio::time::sleep(period * 5 / 2).await;
                }
            }
        }));
        let at =
            |secs: u64| tokio::time::sleep_until(Instant::now() + TokioDuration::from_secs(secs));

        at(1).await;
        assert_eq!(runs.load(SeqCst), 1, "exactly one run at startup");
        at(3598).await; // t = 3599 s
        assert_eq!(runs.load(SeqCst), 1, "nothing more before a full period");
        at(2).await; // t = 3601 s
        assert_eq!(runs.load(SeqCst), 2);
        at(3600).await; // t = 7201 s: third run starts and overruns until t = 16200 s
        assert_eq!(runs.load(SeqCst), 3);
        at(9000).await; // t = 16201 s: the overrun just ended
                        // One catch-up run for the overdue tick; Burst would fire both missed ticks (10800 and
                        // 14400 s) back to back.
        assert_eq!(runs.load(SeqCst), 4, "one catch-up run, not a burst");
        at(1798).await; // t = 17999 s
        assert_eq!(runs.load(SeqCst), 4, "then the original schedule");
        at(2).await; // t = 18001 s: the next scheduled tick (18000 s)
        assert_eq!(runs.load(SeqCst), 5);
        task.abort();
    }

    /// The 2026-10-25 production sync: the listing ran 21:36:34 -> 21:54:53, the snapshot was at
    /// 21:54:56, and 10 gfswave rows ingested at 21:37:33 (after the lister had passed
    /// `grids/gfswave/`) were deleted as orphans although their objects were in MinIO.
    #[test]
    fn find_orphans_never_deletes_a_row_written_after_the_listing_started() {
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let listing_started = t("2026-10-25T21:36:34Z");
        let now = t("2026-10-25T21:54:56Z");
        let grace = Duration::minutes(15);

        let minio: HashMap<String, DateTime<Utc>> = [
            ("grids/gfs/kept.zarr", "2026-10-25T19:00:00Z"),
            ("grids/old/orphan.zarr", "2026-10-25T20:00:00Z"),
            ("grids/new/uploading.zarr", "2026-10-25T21:50:00Z"),
            ("raw/goes/pending.nc", "2026-10-25T18:00:00Z"),
        ]
        .into_iter()
        .map(|(p, m)| (p.to_string(), t(m)))
        .collect();
        let db_rows: Vec<(String, DateTime<Utc>)> = [
            // written mid-listing, missed by it: the production case
            (
                "grids/gfswave/20261025_12z/htsgw_surface_f006.zarr",
                "2026-10-25T21:37:33Z",
            ),
            // written shortly before the listing began (inside the grace): still protected
            ("grids/hrrr/recent.zarr", "2026-10-25T21:30:00Z"),
            // long-standing row whose objects really are gone
            ("grids/hrrr/gone.zarr", "2026-10-25T19:00:00Z"),
            // long-standing row whose objects are listed
            ("grids/gfs/kept.zarr", "2026-10-25T19:00:00Z"),
        ]
        .into_iter()
        .map(|(p, i)| (p.to_string(), t(i)))
        .collect();

        let (mut orphan_db, mut orphan_minio) =
            find_orphans(&minio, &db_rows, listing_started, now, grace);
        orphan_db.sort();
        orphan_minio.sort();

        assert_eq!(orphan_db, ["grids/hrrr/gone.zarr"]);
        // Old and unreferenced: orphan. Recently modified, raw/, or referenced: not.
        assert_eq!(orphan_minio, ["grids/old/orphan.zarr"]);
    }

    #[test]
    fn test_cleanup_config_default() {
        let config = CleanupConfig::default();
        assert!(config.enabled);
        assert_eq!(config.interval_secs, 3600);
        assert_eq!(config.default_retention_hours, 24);
    }

    #[test]
    fn test_sync_config_default() {
        let config = SyncConfig::default();
        assert!(config.enabled);
        assert_eq!(config.interval_secs, 3600);
        assert_eq!(config.grace_secs, 900);
    }

    #[test]
    fn test_model_type_static() {
        // Test that static model type is properly recognized
        let config = ModelRetentionConfig {
            hours: 24,
            model_type: ModelType::Static,
            keep_latest_runs: 1,
            keep_latest_observations: 1,
            expected_forecast_hours: None,
        };
        assert_eq!(config.model_type, ModelType::Static);
    }

    #[test]
    fn test_static_retention_config_parsing() {
        // Test that retention.static: true is parsed correctly
        let yaml = r#"
model:
  id: test-static
dimensions:
  type: static
retention:
  static: true
"#;
        let config: ModelConfigFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model.id, "test-static");
        assert!(config.retention.as_ref().unwrap().is_static.unwrap());
        assert_eq!(
            config.dimensions.as_ref().unwrap().dimension_type,
            Some("static".to_string())
        );
    }

    #[test]
    fn test_static_from_retention_flag() {
        // Test that retention.static: true overrides dimensions.type
        let yaml = r#"
model:
  id: test-override
dimensions:
  type: forecast
retention:
  static: true
  hours: 48
"#;
        let config: ModelConfigFile = serde_yaml::from_str(yaml).unwrap();
        let retention = config.retention.as_ref();
        let is_static = retention.and_then(|r| r.is_static).unwrap_or(false);

        // retention.static: true should make this a Static model
        let model_type = if is_static {
            ModelType::Static
        } else {
            config
                .dimensions
                .as_ref()
                .and_then(|d| d.dimension_type.as_ref())
                .map(|t| match t.as_str() {
                    "observation" => ModelType::Observation,
                    "static" => ModelType::Static,
                    _ => ModelType::Forecast,
                })
                .unwrap_or(ModelType::Forecast)
        };

        assert_eq!(model_type, ModelType::Static);
    }

    #[test]
    fn test_static_from_dimensions_type() {
        // Test that dimensions.type: static works without retention.static
        let yaml = r#"
model:
  id: test-dims-static
dimensions:
  type: static
retention:
  hours: 24
"#;
        let config: ModelConfigFile = serde_yaml::from_str(yaml).unwrap();
        let retention = config.retention.as_ref();
        let is_static = retention.and_then(|r| r.is_static).unwrap_or(false);

        let model_type = if is_static {
            ModelType::Static
        } else {
            config
                .dimensions
                .as_ref()
                .and_then(|d| d.dimension_type.as_ref())
                .map(|t| match t.as_str() {
                    "observation" => ModelType::Observation,
                    "static" => ModelType::Static,
                    _ => ModelType::Forecast,
                })
                .unwrap_or(ModelType::Forecast)
        };

        assert_eq!(model_type, ModelType::Static);
    }

    #[test]
    fn test_cleanup_config_get_model_config_returns_static() {
        // Test that CleanupConfig properly returns Static model type
        let mut model_configs = HashMap::new();
        model_configs.insert(
            "viirs".to_string(),
            ModelRetentionConfig {
                hours: 24, // Doesn't matter for static
                model_type: ModelType::Static,
                keep_latest_runs: 1,
                keep_latest_observations: 1,
                expected_forecast_hours: None,
            },
        );

        let config = CleanupConfig {
            enabled: true,
            interval_secs: 3600,
            model_configs,
            default_retention_hours: 24,
            delete_concurrency: DEFAULT_DELETE_CONCURRENCY,
        };

        let viirs_config = config.get_model_config("viirs");
        assert_eq!(viirs_config.model_type, ModelType::Static);

        // Unknown model should default to Forecast, not Static
        let unknown_config = config.get_model_config("unknown-model");
        assert_eq!(unknown_config.model_type, ModelType::Forecast);
    }

    #[test]
    fn test_viirs_like_config_parsing() {
        // Test parsing a config that matches the actual VIIRS config structure
        let yaml = r#"
model:
  id: viirs
  name: "VIIRS Nighttime Lights"
dimensions:
  type: static
  time: false
  elevation: false
schedule:
  type: static
retention:
  static: true
"#;
        let config: ModelConfigFile = serde_yaml::from_str(yaml).unwrap();

        // Verify the config was parsed correctly
        assert_eq!(config.model.id, "viirs");
        assert_eq!(
            config.dimensions.as_ref().unwrap().dimension_type,
            Some("static".to_string())
        );
        assert!(config.retention.as_ref().unwrap().is_static.unwrap());

        // Simulate the logic from load_model_configs
        let retention = config.retention.as_ref();
        let is_static_retention = retention.and_then(|r| r.is_static).unwrap_or(false);

        let model_type = if is_static_retention {
            ModelType::Static
        } else {
            config
                .dimensions
                .as_ref()
                .and_then(|d| d.dimension_type.as_ref())
                .map(|t| match t.as_str() {
                    "observation" => ModelType::Observation,
                    "static" => ModelType::Static,
                    _ => ModelType::Forecast,
                })
                .unwrap_or(ModelType::Forecast)
        };

        assert_eq!(
            model_type,
            ModelType::Static,
            "VIIRS-like config should be recognized as Static"
        );
    }

    #[test]
    fn test_static_model_skipped_in_cleanup_logic() {
        // Test the skip logic that would be used in run_once()
        // This tests the condition without needing a full database mock

        let static_config = ModelRetentionConfig {
            hours: 24,
            model_type: ModelType::Static,
            keep_latest_runs: 1,
            keep_latest_observations: 1,
            expected_forecast_hours: None,
        };

        let forecast_config = ModelRetentionConfig {
            hours: 24,
            model_type: ModelType::Forecast,
            keep_latest_runs: 1,
            keep_latest_observations: 1,
            expected_forecast_hours: None,
        };

        // Static models should be skipped (continue in the loop)
        let should_skip_static = static_config.model_type == ModelType::Static;
        assert!(
            should_skip_static,
            "Static models should be skipped in cleanup"
        );

        // Forecast models should NOT be skipped
        let should_skip_forecast = forecast_config.model_type == ModelType::Static;
        assert!(
            !should_skip_forecast,
            "Forecast models should NOT be skipped in cleanup"
        );
    }

    #[test]
    fn test_load_viirs_config_from_disk() {
        // Integration test: load the actual VIIRS config from disk
        // This ensures the real config file is parsed correctly

        let config_dir = std::env::var("CARGO_MANIFEST_DIR")
            .map(|dir| format!("{}/../..", dir))
            .unwrap_or_else(|_| ".".to_string());
        let config_path = format!("{}/config", config_dir);

        // Skip test if config directory doesn't exist (CI environment)
        if !Path::new(&config_path).join("models/viirs.yaml").exists() {
            eprintln!(
                "Skipping test: viirs.yaml not found at {}/models/viirs.yaml",
                config_path
            );
            return;
        }

        let configs = CleanupConfig::load_model_configs(&config_path);

        // Verify VIIRS was loaded and is Static
        let viirs_config = configs.get("viirs");
        assert!(
            viirs_config.is_some(),
            "VIIRS config should be loaded from disk"
        );

        let viirs = viirs_config.unwrap();
        assert_eq!(
            viirs.model_type,
            ModelType::Static,
            "VIIRS should be recognized as Static model type"
        );
    }

    #[test]
    fn test_load_model_configs_sets_static_for_retention_flag() {
        // Test that load_model_configs correctly sets Static when retention.static: true
        use std::io::Write;

        // Create a temp directory with a test config
        let temp_dir = std::env::temp_dir().join("cleanup_test_configs");
        let models_dir = temp_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        let test_yaml = r#"
model:
  id: test-static-model
dimensions:
  type: forecast
retention:
  static: true
  hours: 48
"#;
        let config_path = models_dir.join("test-static.yaml");
        let mut file = std::fs::File::create(&config_path).unwrap();
        file.write_all(test_yaml.as_bytes()).unwrap();

        // Load configs from temp directory
        let configs = CleanupConfig::load_model_configs(temp_dir.to_str().unwrap());

        // Verify the static model was loaded correctly
        let test_config = configs.get("test-static-model");
        assert!(test_config.is_some(), "Test config should be loaded");

        let config = test_config.unwrap();
        assert_eq!(
            config.model_type,
            ModelType::Static,
            "retention.static: true should result in ModelType::Static"
        );

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    // ------------------------------------------------------------------
    // Disabled models: leftovers are purged, not pinned by the safeguards
    // ------------------------------------------------------------------

    fn protected(model_type: ModelType) -> ModelRetentionConfig {
        ModelRetentionConfig {
            hours: 48,
            model_type,
            keep_latest_runs: 2,
            keep_latest_observations: 18,
            expected_forecast_hours: Some(65),
        }
    }

    #[test]
    fn enabled_model_keeps_its_retention_settings() {
        let c = purge_everything_if_disabled(true, protected(ModelType::Forecast));
        assert_eq!(
            (c.hours, c.keep_latest_runs, c.keep_latest_observations),
            (48, 2, 18)
        );
        assert_eq!(c.expected_forecast_hours, Some(65));
    }

    #[test]
    fn disabled_forecast_and_observation_models_expire_everything() {
        for t in [ModelType::Forecast, ModelType::Observation] {
            let c = purge_everything_if_disabled(false, protected(t));
            assert_eq!(c.hours, 0, "{t:?}: cutoff must be now");
            assert_eq!(c.keep_latest_runs, 0, "{t:?}: no run may be protected");
            assert_eq!(
                c.keep_latest_observations, 0,
                "{t:?}: no observation may be protected"
            );
            assert_eq!(c.model_type, t, "{t:?}: model type is unchanged");
        }
    }

    #[test]
    fn disabled_static_model_is_left_alone() {
        // VIIRS is `enabled: false` because it is ingested by hand, not because it is unwanted.
        let c = purge_everything_if_disabled(false, protected(ModelType::Static));
        assert_eq!(c.model_type, ModelType::Static);
        assert_eq!(
            (c.hours, c.keep_latest_runs, c.keep_latest_observations),
            (48, 2, 18)
        );
    }

    #[test]
    fn load_model_configs_still_loads_disabled_models_and_purges_them() {
        use std::io::Write;
        let temp_dir =
            std::env::temp_dir().join(format!("cleanup_disabled_{}", std::process::id()));
        let models_dir = temp_dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let write = |name: &str, yaml: &str| {
            std::fs::File::create(models_dir.join(name))
                .unwrap()
                .write_all(yaml.as_bytes())
                .unwrap();
        };
        write(
            "off.yaml",
            "model: {id: off-model, enabled: false}\ndimensions: {type: forecast}\nretention: {hours: 48, keep_latest_runs: 2}\n",
        );
        write(
            "on.yaml",
            "model: {id: on-model, enabled: true}\ndimensions: {type: forecast}\nretention: {hours: 48, keep_latest_runs: 2}\n",
        );
        write(
            "no-flag.yaml",
            "model: {id: no-flag}\ndimensions: {type: observation}\nretention: {hours: 3, keep_latest_observations: 36}\n",
        );
        write(
            "off-static.yaml",
            "model: {id: off-static, enabled: false}\ndimensions: {type: static}\nretention: {static: true}\n",
        );

        let configs = CleanupConfig::load_model_configs(temp_dir.to_str().unwrap());
        std::fs::remove_dir_all(&temp_dir).ok();

        let off = &configs["off-model"];
        assert_eq!((off.hours, off.keep_latest_runs), (0, 0));
        let on = &configs["on-model"];
        assert_eq!((on.hours, on.keep_latest_runs), (48, 2));
        let no_flag = &configs["no-flag"];
        assert_eq!((no_flag.hours, no_flag.keep_latest_observations), (3, 36));
        assert_eq!(configs["off-static"].model_type, ModelType::Static);
    }

    #[test]
    fn real_configs_purge_the_data_diet_models_and_keep_goes_conus() {
        let config_path = format!("{}/../../config", env!("CARGO_MANIFEST_DIR"));
        if !Path::new(&config_path).join("models/aigfs.yaml").exists() {
            eprintln!("Skipping test: config/models not found at {config_path}");
            return;
        }
        let configs = CleanupConfig::load_model_configs(&config_path);
        for id in ["aigfs", "aigefs", "goes18-fulldisk", "goes19-fulldisk"] {
            let c = configs
                .get(id)
                .unwrap_or_else(|| panic!("{id} must still have a retention config"));
            assert_eq!(
                (c.hours, c.keep_latest_runs, c.keep_latest_observations),
                (0, 0, 0),
                "{id} is disabled: its leftovers must be purged, not protected"
            );
        }
        for id in ["goes18", "goes19"] {
            let c = &configs[id];
            assert_eq!(
                (c.hours, c.keep_latest_observations),
                (3, 36),
                "{id} is still ingested"
            );
        }
        assert_eq!(
            configs["viirs"].model_type,
            ModelType::Static,
            "VIIRS must stay static"
        );
    }

    // ------------------------------------------------------------------
    // Bulk, concurrent storage deletion (`purge_expired_paths`)
    // ------------------------------------------------------------------

    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::Mutex;

    fn paths(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("grids/hrrr/20261029_12z/p{:03}.zarr", i))
            .collect()
    }

    /// `purge_expired_paths` with a storage that fails every path containing "bad", and a row
    /// store that records each batch it is asked to delete. Returns the outcome and the batches.
    async fn purge_with_failures(
        all: Vec<String>,
        concurrency: usize,
        batch_size: usize,
    ) -> (PurgeOutcome, Vec<Vec<String>>) {
        let batches: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());
        let outcome = purge_expired_paths(
            all,
            concurrency,
            batch_size,
            |path: String| async move {
                if path.contains("bad") {
                    Err(format!("storage unavailable for {}", path))
                } else {
                    Ok(10u64)
                }
            },
            |batch: Vec<String>| {
                let batches = &batches;
                async move {
                    let n = batch.len() as u64;
                    batches.lock().unwrap().push(batch);
                    Ok(n)
                }
            },
        )
        .await
        .expect("purge should succeed");
        (outcome, batches.into_inner().unwrap())
    }

    #[tokio::test]
    async fn rows_are_deleted_only_for_paths_whose_storage_delete_succeeded() {
        let mut all = paths(40);
        let bad: Vec<String> = (0..7).map(|i| format!("grids/gfs/bad{}.zarr", i)).collect();
        all.extend(bad.clone());

        let (outcome, batches) = purge_with_failures(all, 8, 16).await;

        assert_eq!(outcome.paths_deleted, 40);
        assert_eq!(outcome.objects_deleted, 400);
        assert_eq!(outcome.delete_errors, 7);
        assert_eq!(outcome.records_deleted, 40);

        let mut deleted_rows: Vec<String> = batches.into_iter().flatten().collect();
        deleted_rows.sort();
        assert_eq!(deleted_rows, paths(40), "exactly the successful paths");
        assert!(
            deleted_rows.iter().all(|p| !bad.contains(p)),
            "a failed path's row must be kept so the next cycle retries it"
        );
    }

    #[tokio::test]
    async fn the_database_is_not_touched_when_every_storage_delete_fails() {
        let all: Vec<String> = (0..5).map(|i| format!("grids/bad{}.zarr", i)).collect();

        let (outcome, batches) = purge_with_failures(all, 4, 2).await;

        assert_eq!(outcome.delete_errors, 5);
        assert_eq!(outcome.paths_deleted, 0);
        assert_eq!(outcome.records_deleted, 0);
        assert!(batches.is_empty(), "no row deletes: {:?}", batches);
    }

    #[tokio::test]
    async fn nothing_to_purge_is_a_no_op() {
        let (outcome, batches) = purge_with_failures(Vec::new(), 4, 2).await;
        assert_eq!(outcome, PurgeOutcome::default());
        assert!(batches.is_empty());
    }

    /// Rows go out in batches as the work progresses, not in one go at the end: that is what
    /// preserves progress when the ingester restarts mid-cycle.
    #[tokio::test]
    async fn rows_are_deleted_incrementally_in_batches() {
        let events: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let outcome = purge_expired_paths(
            paths(25),
            1, // sequential, so the interleaving is deterministic
            10,
            |path: String| {
                let events = &events;
                async move {
                    events.lock().unwrap().push(format!("del {}", path));
                    Ok::<u64, String>(1)
                }
            },
            |batch: Vec<String>| {
                let events = &events;
                async move {
                    events.lock().unwrap().push(format!("rows {}", batch.len()));
                    Ok(batch.len() as u64)
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(outcome.records_deleted, 25);

        let events = events.into_inner().unwrap();
        let summary: Vec<&str> = events
            .iter()
            .map(|e| {
                if e.starts_with("del") {
                    "del"
                } else {
                    e.as_str()
                }
            })
            .collect();
        let mut expected = Vec::new();
        expected.extend(vec!["del"; 10]);
        expected.push("rows 10");
        expected.extend(vec!["del"; 10]);
        expected.push("rows 10");
        expected.extend(vec!["del"; 5]);
        expected.push("rows 5");
        assert_eq!(summary, expected);
    }

    /// Deletes really overlap (virtual time: 20 paths x 100 ms at 4 at a time = 500 ms, versus
    /// 2 s sequentially) and never exceed the configured bound.
    #[tokio::test(start_paused = true)]
    async fn storage_deletes_run_concurrently_up_to_the_configured_bound() {
        let in_flight = AtomicUsize::new(0);
        let max_in_flight = AtomicUsize::new(0);
        let started = Instant::now();

        let outcome = purge_expired_paths(
            paths(20),
            4,
            500,
            |_path: String| {
                let (in_flight, max_in_flight) = (&in_flight, &max_in_flight);
                async move {
                    let now = in_flight.fetch_add(1, SeqCst) + 1;
                    max_in_flight.fetch_max(now, SeqCst);
                    tokio::time::sleep(TokioDuration::from_millis(100)).await;
                    in_flight.fetch_sub(1, SeqCst);
                    Ok::<u64, String>(1)
                }
            },
            |batch: Vec<String>| async move { Ok(batch.len() as u64) },
        )
        .await
        .unwrap();

        assert_eq!(outcome.paths_deleted, 20);
        assert_eq!(max_in_flight.load(SeqCst), 4);
        assert_eq!(started.elapsed(), TokioDuration::from_millis(500));
    }

    #[tokio::test]
    async fn zero_concurrency_is_treated_as_one_instead_of_hanging() {
        let result = tokio::time::timeout(
            TokioDuration::from_secs(5),
            purge_with_failures(paths(3), 0, 0),
        )
        .await
        .expect("must not hang");
        assert_eq!(result.0.paths_deleted, 3);
    }

    /// A database failure aborts the run, but batches already committed stay committed.
    #[tokio::test]
    async fn a_database_failure_aborts_the_purge_and_keeps_earlier_batches() {
        let committed: Mutex<Vec<usize>> = Mutex::new(Vec::new());
        let calls = AtomicUsize::new(0);

        let result = purge_expired_paths(
            paths(30),
            1,
            10,
            |_path: String| async move { Ok::<u64, String>(1) },
            |batch: Vec<String>| {
                let (committed, calls) = (&committed, &calls);
                async move {
                    if calls.fetch_add(1, SeqCst) == 1 {
                        anyhow::bail!("connection reset");
                    }
                    committed.lock().unwrap().push(batch.len());
                    Ok(batch.len() as u64)
                }
            },
        )
        .await;

        let err = result.expect_err("a failed row delete must surface");
        assert!(err.to_string().contains("connection reset"), "{err}");
        assert_eq!(
            *committed.lock().unwrap(),
            vec![10],
            "first batch committed"
        );
    }

    #[test]
    fn delete_concurrency_comes_from_the_environment_with_a_safe_default() {
        assert_eq!(parse_delete_concurrency(None), 16);
        assert_eq!(parse_delete_concurrency(Some("32")), 32);
        assert_eq!(parse_delete_concurrency(Some(" 4 ")), 4);
        // zero would stall the delete stream; junk falls back to the default
        assert_eq!(parse_delete_concurrency(Some("0")), 16);
        assert_eq!(parse_delete_concurrency(Some("many")), 16);
        assert_eq!(parse_delete_concurrency(Some("-3")), 16);
        assert_eq!(CleanupConfig::default().delete_concurrency, 16);
    }
}
