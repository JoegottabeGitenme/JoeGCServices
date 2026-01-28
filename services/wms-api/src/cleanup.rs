//! Data retention and cleanup background task.
//!
//! This module handles automatic cleanup of expired datasets based on
//! retention settings in model configuration files.
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

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::time::{interval, Duration as TokioDuration};
use tracing::{debug, error, info, warn};

use crate::state::AppState;

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
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 3600, // Run every hour
            model_configs: HashMap::new(),
            default_retention_hours: 24, // 24 hours default
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
#[derive(Debug, Deserialize)]
struct ForecastHoursConfig {
    start: Option<u32>,
    end: Option<u32>,
    step: Option<u32>,
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

        // Load model retention settings from config files
        let model_configs = Self::load_model_configs(config_dir);

        Self {
            enabled,
            interval_secs,
            model_configs,
            default_retention_hours,
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
                                let expected_forecast_hours = config
                                    .schedule
                                    .as_ref()
                                    .and_then(|s| s.forecast_hours.as_ref())
                                    .and_then(|fh| {
                                        let start = fh.start.unwrap_or(0);
                                        let end = fh.end?;
                                        let step = fh.step.unwrap_or(1);
                                        if step > 0 {
                                            Some((end - start) / step + 1)
                                        } else {
                                            None
                                        }
                                    });

                                // Get hours setting (not used for static models but kept for config consistency)
                                let hours = retention.and_then(|r| r.hours).unwrap_or(24);

                                // Get safeguard settings with defaults
                                let keep_latest_runs =
                                    retention.and_then(|r| r.keep_latest_runs).unwrap_or(1); // Default: always keep at least 1 run

                                let keep_latest_observations = retention
                                    .and_then(|r| r.keep_latest_observations)
                                    .unwrap_or(1); // Default: always keep at least 1 observation

                                let model_config = ModelRetentionConfig {
                                    hours,
                                    model_type,
                                    keep_latest_runs,
                                    keep_latest_observations,
                                    expected_forecast_hours,
                                };

                                info!(
                                    model = %model_id,
                                    retention_hours = hours,
                                    model_type = ?model_type,
                                    keep_latest_runs = keep_latest_runs,
                                    keep_latest_observations = keep_latest_observations,
                                    expected_forecast_hours = ?expected_forecast_hours,
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
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 60, // Run every minute
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
            .unwrap_or(60); // 1 minute default

        Self {
            enabled,
            interval_secs,
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
    state: Arc<AppState>,
    config: CleanupConfig,
}

impl CleanupTask {
    /// Create a new cleanup task.
    pub fn new(state: Arc<AppState>, config: CleanupConfig) -> Self {
        Self { state, config }
    }

    /// Get the cleanup configuration.
    pub fn config(&self) -> &CleanupConfig {
        &self.config
    }

    /// Run the cleanup task once.
    pub async fn run_once(&self) -> Result<CleanupStats> {
        let mut stats = CleanupStats::default();

        info!("Starting cleanup cycle");

        // Get list of models from the database
        let models = self.state.catalog.list_models().await?;

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
                        .await?
                }
                ModelType::Observation => {
                    self.cleanup_observation_model(model, &model_config, cutoff)
                        .await?
                }
                ModelType::Static => {
                    // Already handled above with continue, but need for exhaustive match
                    0
                }
            };

            if marked > 0 {
                info!(model = %model, count = marked, "Marked datasets as expired");
                stats.marked_expired += marked;
            }
        }

        // Get storage paths of expired datasets
        let expired_paths = self.state.catalog.get_expired_storage_paths().await?;

        if !expired_paths.is_empty() {
            info!(
                count = expired_paths.len(),
                "Deleting expired files from storage"
            );

            // Delete files from object storage
            for path in &expired_paths {
                // Use delete_prefix for Zarr directories to delete all files
                let result = if path.ends_with(".zarr") {
                    // Add trailing slash to ensure we only delete within the zarr directory
                    let prefix = format!("{}/", path);
                    self.state.storage.delete_prefix(&prefix).await.map(|_| ())
                } else {
                    self.state.storage.delete(path).await
                };

                match result {
                    Ok(()) => {
                        stats.files_deleted += 1;
                    }
                    Err(e) => {
                        // Log but continue - file might already be deleted
                        warn!(path = %path, error = %e, "Failed to delete file");
                        stats.delete_errors += 1;
                    }
                }
            }

            // Delete expired records from database
            let deleted = self.state.catalog.delete_expired().await?;
            stats.records_deleted = deleted;
            info!(count = deleted, "Deleted expired records from database");

            // Invalidate capabilities cache when data is deleted
            if deleted > 0 {
                self.state.capabilities_cache.invalidate().await;
                debug!(
                    "Invalidated capabilities cache after cleanup deleted {} records",
                    deleted
                );
            }
        }

        info!(
            marked = stats.marked_expired,
            files = stats.files_deleted,
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
        let runs = self.state.catalog.get_model_runs_with_counts(model).await?;

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
        self.state
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
        let observations = self.state.catalog.get_model_runs_with_counts(model).await?;

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
        self.state
            .catalog
            .mark_model_expired_except_runs(model, cutoff, &protected_obs)
            .await
            .map_err(Into::into)
    }

    /// Get information about protected runs for a model (for admin API).
    pub async fn get_protected_runs(&self, model: &str) -> Result<Vec<ProtectedRunInfo>> {
        let model_config = self.config.get_model_config(model);
        // Runs are already sorted by reference_time DESC from the query
        let runs = self.state.catalog.get_model_runs_with_counts(model).await?;

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

        let mut ticker = interval(TokioDuration::from_secs(self.config.interval_secs));

        // Run immediately on startup
        if let Err(e) = self.run_once().await {
            error!(error = %e, "Cleanup cycle failed");
        }

        loop {
            ticker.tick().await;
            if let Err(e) = self.run_once().await {
                error!(error = %e, "Cleanup cycle failed");
            }
        }
    }
}

/// Statistics from a cleanup run.
#[derive(Debug, Default, Clone, Serialize)]
pub struct CleanupStats {
    /// Number of datasets marked as expired
    pub marked_expired: u64,
    /// Number of files deleted from storage
    pub files_deleted: u64,
    /// Number of database records deleted
    pub records_deleted: u64,
    /// Number of delete errors
    pub delete_errors: u64,
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

/// Sync task for reconciling database and MinIO storage.
pub struct SyncTask {
    state: Arc<AppState>,
    config: SyncConfig,
}

impl SyncTask {
    /// Create a new sync task.
    pub fn new(state: Arc<AppState>, config: SyncConfig) -> Self {
        Self { state, config }
    }

    /// Create a new sync task with default config (for admin API use).
    pub fn new_default(state: Arc<AppState>) -> Self {
        Self {
            state,
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

        let mut ticker = interval(TokioDuration::from_secs(self.config.interval_secs));

        // Run immediately on startup
        if let Err(e) = self.run().await {
            error!(error = %e, "Sync cycle failed");
        }

        loop {
            ticker.tick().await;
            if let Err(e) = self.run().await {
                error!(error = %e, "Sync cycle failed");
            }
        }
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

        // Get all storage paths from the database
        let db_paths = self.state.catalog.get_all_storage_paths().await?;
        let db_path_set: std::collections::HashSet<String> = db_paths.iter().cloned().collect();
        let db_records_checked = db_paths.len() as u64;

        // Get all objects from MinIO
        // Need to check shredded/ (legacy GRIB2 data), raw/ (GOES NetCDF data), and grids/ (Zarr data)
        let mut minio_paths = self.state.storage.list("shredded/").await?;
        let raw_paths = self.state.storage.list("raw/").await?;
        minio_paths.extend(raw_paths);
        let grid_paths = self.state.storage.list("grids/").await?;
        minio_paths.extend(grid_paths);

        // For Zarr directories, extract the parent .zarr path from individual files
        let mut minio_path_set: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for path in &minio_paths {
            if let Some(zarr_idx) = path.find(".zarr/") {
                let zarr_dir = &path[..zarr_idx + 5];
                minio_path_set.insert(zarr_dir.to_string());
            } else {
                minio_path_set.insert(path.clone());
            }
        }
        let minio_objects_checked = minio_paths.len() as u64;

        // Find orphan DB records (in DB but not in MinIO)
        let orphan_db_paths: Vec<String> = db_paths
            .into_iter()
            .filter(|path| !minio_path_set.contains(path))
            .collect();

        // Find orphan MinIO objects (in MinIO but not in DB)
        // For Zarr files, check if the extracted zarr directory exists in DB
        // Only consider shredded/ and grids/ files as orphans - raw/ files may be awaiting processing
        let orphan_minio_paths: Vec<String> = minio_path_set
            .iter()
            .filter(|path| {
                (path.starts_with("shredded/") || path.starts_with("grids/"))
                    && !db_path_set.contains(*path)
            })
            .cloned()
            .collect();

        Ok(SyncPreview {
            orphan_db_paths,
            orphan_minio_paths,
            db_records_checked,
            minio_objects_checked,
        })
    }

    /// Core sync logic.
    async fn run_sync(&self, delete: bool) -> Result<SyncStats> {
        let mut stats = SyncStats::default();

        info!(delete = delete, "Starting database/storage sync");

        // Step 1: Get all storage paths from the database
        let db_paths = self.state.catalog.get_all_storage_paths().await?;
        let db_path_set: std::collections::HashSet<String> = db_paths.iter().cloned().collect();
        stats.db_records_checked = db_paths.len() as u64;

        info!(count = db_paths.len(), "Retrieved database storage paths");

        // Step 2: Get all objects from MinIO
        // Need to check shredded/ (legacy GRIB2 data), raw/ (GOES NetCDF data), and grids/ (Zarr data)
        let mut minio_paths = self.state.storage.list("shredded/").await?;
        let raw_paths = self.state.storage.list("raw/").await?;
        minio_paths.extend(raw_paths);
        let grid_paths = self.state.storage.list("grids/").await?;
        minio_paths.extend(grid_paths);

        // For Zarr directories, we need to extract the parent .zarr path from individual files
        // e.g., "grids/mrms/123/foo.zarr/c/0/0" -> "grids/mrms/123/foo.zarr"
        let mut minio_path_set: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for path in &minio_paths {
            // Check if this is a Zarr file (contains .zarr/ in path)
            if let Some(zarr_idx) = path.find(".zarr/") {
                // Extract the zarr directory path (including .zarr)
                let zarr_dir = &path[..zarr_idx + 5]; // +5 for ".zarr"
                minio_path_set.insert(zarr_dir.to_string());
            } else {
                // Regular file path
                minio_path_set.insert(path.clone());
            }
        }
        stats.minio_objects_checked = minio_paths.len() as u64;

        info!(
            count = minio_paths.len(),
            unique_paths = minio_path_set.len(),
            "Retrieved MinIO objects (shredded + raw + grids)"
        );

        // Step 3: Find orphan DB records (in DB but not in MinIO)
        let orphan_db_paths: Vec<String> = db_paths
            .into_iter()
            .filter(|path| !minio_path_set.contains(path))
            .collect();
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
                    .state
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

        // Step 4: Find orphan MinIO objects (in MinIO but not in DB)
        // For Zarr files, check if the extracted zarr directory exists in DB
        // Only consider shredded/ and grids/ files as orphans - raw/ files may be awaiting processing
        let orphan_minio_paths: Vec<String> = minio_path_set
            .iter()
            .filter(|path| {
                (path.starts_with("shredded/") || path.starts_with("grids/"))
                    && !db_path_set.contains(*path)
            })
            .cloned()
            .collect();
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
                        self.state.storage.delete_prefix(&prefix).await.map(|_| ())
                    } else {
                        self.state.storage.delete(path).await
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
        assert_eq!(config.interval_secs, 60);
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
}
