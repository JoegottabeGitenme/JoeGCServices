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

/// Retention configuration for a model.
#[derive(Debug, Clone)]
pub struct ModelRetention {
    pub model_id: String,
    pub retention_hours: u32,
}

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
                if path.extension().map_or(false, |e| e == "yaml" || e == "yml") {
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
        *self.model_retentions.get(model).unwrap_or(&self.default_retention_hours)
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
            info!(count = expired_paths.len(), "Deleting expired files from storage");

            // Delete files from object storage
            for path in &expired_paths {
                match self.state.storage.delete(path).await {
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
#[derive(Debug, Default)]
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
}
