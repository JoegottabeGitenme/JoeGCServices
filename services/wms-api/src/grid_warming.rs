//! Grid data cache warming module.
//!
//! This module proactively warms the GridDataCache by parsing and caching
//! recent observation data (GOES, HRRR, MRMS) before tile requests arrive.
//! This eliminates the cold-start latency of NetCDF/GRIB2 parsing.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};

use storage::{CachedGridData, GoesProjectionParams as StorageGoesProjectionParams};
use crate::state::AppState;

/// Precaching configuration from model YAML files.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PrecacheConfig {
    /// Whether precaching is enabled for this model
    #[serde(default)]
    pub enabled: bool,
    
    /// Number of recent observations to keep warm
    #[serde(default = "default_keep_recent")]
    pub keep_recent: usize,
    
    /// Warm cache immediately when new data is ingested
    #[serde(default)]
    pub warm_on_ingest: bool,
    
    /// Background polling interval in seconds
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    
    /// Parameters to precache (empty = all)
    #[serde(default)]
    pub parameters: Vec<String>,
}

fn default_keep_recent() -> usize { 10 }
fn default_poll_interval() -> u64 { 60 }

/// Structure matching model YAML files for precaching config.
#[derive(Debug, Deserialize)]
struct ModelConfigFile {
    model: ModelConfigModel,
    #[serde(default)]
    precaching: Option<PrecacheConfig>,
}

#[derive(Debug, Deserialize)]
struct ModelConfigModel {
    id: String,
}

/// Grid warmer for proactive cache population.
pub struct GridWarmer {
    state: Arc<AppState>,
    /// Per-model precaching configuration
    configs: HashMap<String, PrecacheConfig>,
    /// Track what we've already warmed to avoid redundant work
    warmed_keys: tokio::sync::RwLock<HashSet<String>>,
}

impl GridWarmer {
    /// Create a new grid warmer by loading configs from YAML files.
    pub fn new(state: Arc<AppState>, config_dir: &str) -> Self {
        let configs = Self::load_precache_configs(config_dir);
        
        info!(
            models_with_precaching = configs.iter()
                .filter(|(_, c)| c.enabled)
                .count(),
            "Initialized GridWarmer"
        );
        
        for (model, config) in &configs {
            if config.enabled {
                info!(
                    model = %model,
                    keep_recent = config.keep_recent,
                    warm_on_ingest = config.warm_on_ingest,
                    poll_interval_secs = config.poll_interval_secs,
                    "Precaching enabled"
                );
            }
        }
        
        Self {
            state,
            configs,
            warmed_keys: tokio::sync::RwLock::new(HashSet::new()),
        }
    }
    
    /// Load precaching configs from model YAML files.
    fn load_precache_configs(config_dir: &str) -> HashMap<String, PrecacheConfig> {
        let mut configs = HashMap::new();
        let models_dir = Path::new(config_dir).join("models");
        
        if !models_dir.exists() {
            warn!("Model config directory not found: {:?}", models_dir);
            return configs;
        }
        
        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
                    if let Ok(contents) = std::fs::read_to_string(&path) {
                        match serde_yaml::from_str::<ModelConfigFile>(&contents) {
                            Ok(config) => {
                                if let Some(precache) = config.precaching {
                                    configs.insert(config.model.id.clone(), precache);
                                }
                            }
                            Err(e) => {
                                debug!(
                                    path = ?path,
                                    error = %e,
                                    "Failed to parse model config for precaching"
                                );
                            }
                        }
                    }
                }
            }
        }
        
        configs
    }
    
    /// Check if a model has precaching enabled.
    pub fn is_enabled(&self, model: &str) -> bool {
        self.configs.get(model).map(|c| c.enabled).unwrap_or(false)
    }
    
    /// Check if warm_on_ingest is enabled for a model.
    pub fn should_warm_on_ingest(&self, model: &str) -> bool {
        self.configs.get(model)
            .map(|c| c.enabled && c.warm_on_ingest)
            .unwrap_or(false)
    }
    
    /// Warm a specific dataset (called on ingestion).
    /// This loads and parses the data, storing it in GridDataCache.
    pub async fn warm_dataset(
        &self,
        model: &str,
        parameter: &str,
        storage_path: &str,
        reference_time: DateTime<Utc>,
    ) {
        let config = match self.configs.get(model) {
            Some(c) if c.enabled => c,
            _ => return,
        };
        
        // Check parameter filter
        if !config.parameters.is_empty() && !config.parameters.contains(&parameter.to_string()) {
            debug!(
                model = %model,
                parameter = %parameter,
                "Skipping precache - parameter not in filter list"
            );
            return;
        }
        
        // Build cache key (same format as rendering.rs)
        let cache_key = format!("{}:{}", storage_path, reference_time.timestamp());
        
        // Check if already warmed
        {
            let warmed = self.warmed_keys.read().await;
            if warmed.contains(&cache_key) {
                debug!(cache_key = %cache_key, "Already warmed, skipping");
                return;
            }
        }
        
        // Check if already in grid cache
        if self.state.grid_cache.get(&cache_key).await.is_some() {
            debug!(cache_key = %cache_key, "Already in grid cache");
            // Mark as warmed so we don't check again
            self.warmed_keys.write().await.insert(cache_key);
            return;
        }
        
        info!(
            model = %model,
            parameter = %parameter,
            storage_path = %storage_path,
            "Warming grid cache"
        );
        
        let start = Instant::now();
        
        // Load the file from storage
        let file_data = match self.state.grib_cache.get(storage_path).await {
            Ok(data) => data,
            Err(e) => {
                warn!(
                    storage_path = %storage_path,
                    error = %e,
                    "Failed to load file for warming"
                );
                return;
            }
        };
        
        // Parse based on file type
        let result = if storage_path.ends_with(".nc") {
            // NetCDF (GOES)
            self.parse_and_cache_netcdf(&cache_key, &file_data).await
        } else {
            // GRIB2 - not implemented yet for warming
            debug!(storage_path = %storage_path, "GRIB2 warming not yet implemented");
            return;
        };
        
        match result {
            Ok(()) => {
                let elapsed = start.elapsed();
                info!(
                    model = %model,
                    parameter = %parameter,
                    cache_key = %cache_key,
                    elapsed_ms = elapsed.as_millis(),
                    "Grid cache warmed successfully"
                );
                self.warmed_keys.write().await.insert(cache_key);
            }
            Err(e) => {
                warn!(
                    model = %model,
                    parameter = %parameter,
                    error = %e,
                    "Failed to warm grid cache"
                );
            }
        }
    }
    
    /// Parse NetCDF file and store in grid cache.
    async fn parse_and_cache_netcdf(&self, cache_key: &str, file_data: &[u8]) -> Result<()> {
        let (data, width, height, projection, x_offset, y_offset, x_scale, y_scale) = 
            netcdf_parser::load_goes_netcdf_from_bytes(file_data)
                .map_err(|e| anyhow::anyhow!("Failed to parse NetCDF: {}", e))?;
        
        let cached_data = CachedGridData {
            data: Arc::new(data),
            width,
            height,
            goes_projection: Some(StorageGoesProjectionParams {
                x_origin: x_offset as f64,
                y_origin: y_offset as f64,
                dx: x_scale as f64,
                dy: y_scale as f64,
                perspective_point_height: projection.perspective_point_height,
                semi_major_axis: projection.semi_major_axis,
                semi_minor_axis: projection.semi_minor_axis,
                longitude_origin: projection.longitude_origin,
            }),
        };
        
        self.state.grid_cache.insert(cache_key.to_string(), cached_data).await;
        Ok(())
    }
    
    /// Warm all recent observations for enabled models.
    /// Called on startup and periodically.
    pub async fn warm_recent_all(&self) {
        for (model, config) in &self.configs {
            if !config.enabled {
                continue;
            }
            
            if let Err(e) = self.warm_recent_for_model(model, config).await {
                warn!(model = %model, error = %e, "Failed to warm recent data");
            }
        }
    }
    
    /// Warm recent observations for a specific model.
    async fn warm_recent_for_model(&self, model: &str, config: &PrecacheConfig) -> Result<()> {
        // Query catalog for recent entries
        let entries = self.state.catalog
            .get_recent_entries(model, config.keep_recent)
            .await?;
        
        if entries.is_empty() {
            debug!(model = %model, "No recent entries to warm");
            return Ok(());
        }
        
        info!(
            model = %model,
            entries_found = entries.len(),
            keep_recent = config.keep_recent,
            "Warming recent observations"
        );
        
        let mut warmed = 0;
        let mut skipped = 0;
        
        for entry in entries {
            // Check parameter filter
            if !config.parameters.is_empty() && !config.parameters.contains(&entry.parameter) {
                skipped += 1;
                continue;
            }
            
            // Build cache key
            let cache_key = format!("{}:{}", entry.storage_path, entry.reference_time.timestamp());
            
            // Skip if already in cache
            if self.state.grid_cache.get(&cache_key).await.is_some() {
                skipped += 1;
                continue;
            }
            
            // Warm this entry
            self.warm_dataset(
                model,
                &entry.parameter,
                &entry.storage_path,
                entry.reference_time,
            ).await;
            
            warmed += 1;
        }
        
        info!(
            model = %model,
            warmed = warmed,
            skipped = skipped,
            "Finished warming recent observations"
        );
        
        Ok(())
    }
    
    /// Background loop that periodically warms new data.
    pub async fn run_forever(self: Arc<Self>) {
        // Find the minimum poll interval across all enabled models
        let min_poll_secs = self.configs.iter()
            .filter(|(_, c)| c.enabled)
            .map(|(_, c)| c.poll_interval_secs)
            .min()
            .unwrap_or(60);
        
        info!(
            poll_interval_secs = min_poll_secs,
            "Starting grid warming background task"
        );
        
        // Initial warm on startup
        self.warm_recent_all().await;
        
        let mut ticker = interval(Duration::from_secs(min_poll_secs));
        
        loop {
            ticker.tick().await;
            
            debug!("Grid warming poll tick");
            self.warm_recent_all().await;
            
            // Periodically clean up the warmed_keys set to prevent unbounded growth
            // Keep only keys that are still in the grid cache
            self.cleanup_warmed_keys().await;
        }
    }
    
    /// Clean up warmed_keys set by removing entries no longer in cache.
    async fn cleanup_warmed_keys(&self) {
        let mut warmed = self.warmed_keys.write().await;
        let before = warmed.len();
        
        let mut to_remove = Vec::new();
        for key in warmed.iter() {
            if self.state.grid_cache.get(key).await.is_none() {
                to_remove.push(key.clone());
            }
        }
        
        for key in to_remove {
            warmed.remove(&key);
        }
        
        let after = warmed.len();
        if before != after {
            debug!(
                before = before,
                after = after,
                removed = before - after,
                "Cleaned up warmed_keys tracking set"
            );
        }
    }
    
    /// Get stats about the warmer.
    pub async fn stats(&self) -> GridWarmerStats {
        let warmed_count = self.warmed_keys.read().await.len();
        let cache_stats = self.state.grid_cache.stats().await;
        
        GridWarmerStats {
            enabled_models: self.configs.iter().filter(|(_, c)| c.enabled).count(),
            tracked_keys: warmed_count,
            cache_entries: cache_stats.entries,
            cache_hit_rate: cache_stats.hit_rate(),
        }
    }
}

/// Statistics about the grid warmer.
#[derive(Debug, Clone)]
pub struct GridWarmerStats {
    pub enabled_models: usize,
    pub tracked_keys: usize,
    pub cache_entries: usize,
    pub cache_hit_rate: f64,
}
