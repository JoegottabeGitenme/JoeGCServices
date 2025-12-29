//! Data retention and cleanup background task.
//!
//! This module handles automatic cleanup of expired datasets based on
//! retention settings in model configuration files.

use anyhow::Result;
use chrono::{Duration, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::time::{interval, Duration as TokioDuration};
use tracing::{error, info, warn};

use crate::state::AppState;

/// Configuration for the cleanup task.
#[derive(Debug, Clone)]
pub struct CleanupConfig {
    /// Whether cleanup is enabled
    pub enabled: bool,
    /// How often to run cleanup (in seconds)
    pub interval_secs: u64,
    /// Per-model retention settings
    pub model_retentions: HashMap<String, u32>,
    /// Default retention if model not specified (in hours)
    pub default_retention_hours: u32,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 3600, // Run every hour
            model_retentions: HashMap::new(),
            default_retention_hours: 24, // 24 hours default
        }
    }
}

/// Structure matching retention section in model YAML files.
#[derive(Debug, Deserialize)]
struct ModelConfigRetention {
    hours: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ModelConfigFile {
    model: ModelConfigModel,
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
        let model_retentions = Self::load_model_retentions(config_dir);

        Self {
            enabled,
            interval_secs,
            model_retentions,
            default_retention_hours,
        }
    }

    /// Load retention settings from model YAML files.
    fn load_model_retentions(config_dir: &str) -> HashMap<String, u32> {
        let mut retentions = HashMap::new();
        let models_dir = Path::new(config_dir).join("models");

        if !models_dir.exists() {
            warn!("Model config directory not found: {:?}", models_dir);
            return retentions;
        }

        // Read all YAML files in the models directory
        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
                    if let Ok(contents) = std::fs::read_to_string(&path) {
                        match serde_yaml::from_str::<ModelConfigFile>(&contents) {
                            Ok(config) => {
                                if let Some(retention) = config.retention {
                                    if let Some(hours) = retention.hours {
                                        info!(
                                            model = %config.model.id,
                                            retention_hours = hours,
                                            "Loaded retention config"
                                        );
                                        retentions.insert(config.model.id, hours);
                                    }
                                }
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

        retentions
    }

    /// Get retention hours for a model.
    pub fn get_retention_hours(&self, model: &str) -> u32 {
        *self
            .model_retentions
            .get(model)
            .unwrap_or(&self.default_retention_hours)
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

    /// Run the cleanup task once.
    pub async fn run_once(&self) -> Result<CleanupStats> {
        let mut stats = CleanupStats::default();

        info!("Starting cleanup cycle");

        // Get list of models from the database
        let models = self.state.catalog.list_models().await?;

        for model in &models {
            let retention_hours = self.config.get_retention_hours(model);
            let cutoff = Utc::now() - Duration::hours(retention_hours as i64);

            info!(
                model = %model,
                retention_hours = retention_hours,
                cutoff = %cutoff,
                "Checking for expired datasets"
            );

            // Mark datasets as expired
            let marked = self.state.catalog.mark_model_expired(model, cutoff).await?;
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
                tracing::debug!(
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
#[derive(Debug, Default, Clone, serde::Serialize)]
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
#[derive(Debug, Default, Clone, serde::Serialize)]
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
#[derive(Debug, Default, Clone, serde::Serialize)]
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
}
