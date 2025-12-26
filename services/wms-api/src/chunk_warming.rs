//! Chunk cache warming module.
//!
//! This module proactively warms the chunk cache by reading Zarr data at
//! configurable zoom levels before tile requests arrive. This eliminates
//! the cold-start latency of chunk fetching and decompression.
//!
//! Key features:
//! - Configurable zoom levels for warming (e.g., warm overviews at zoom 0-4)
//! - Parameter filtering to focus on most-used data
//! - Background polling for new data
//! - On-ingest warming for immediate cache population

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};

use crate::rendering::loaders::load_grid_data;
use crate::state::AppState;
use storage::CatalogEntry;

/// Precaching configuration from model YAML files.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PrecacheConfig {
    /// Whether precaching is enabled for this model
    #[serde(default)]
    pub enabled: bool,

    /// Number of recent observations/forecasts to keep warm
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

    /// Zoom levels to warm chunks for (e.g., [0, 2, 4] for overview tiles)
    /// Each zoom level corresponds to an output tile size that determines
    /// which pyramid level is read, thus which chunks get cached.
    #[serde(default = "default_zoom_levels")]
    pub zoom_levels: Vec<u32>,
}

fn default_keep_recent() -> usize {
    10
}
fn default_poll_interval() -> u64 {
    60
}
fn default_zoom_levels() -> Vec<u32> {
    vec![0, 2, 4]
}

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

/// Chunk warmer for proactive cache population.
pub struct ChunkWarmer {
    state: Arc<AppState>,
    /// Per-model precaching configuration
    configs: HashMap<String, PrecacheConfig>,
    /// Track what we've already warmed to avoid redundant work
    warmed_keys: tokio::sync::RwLock<HashSet<String>>,
}

impl ChunkWarmer {
    /// Create a new chunk warmer by loading configs from YAML files.
    pub fn new(state: Arc<AppState>, config_dir: &str) -> Self {
        let configs = Self::load_precache_configs(config_dir);

        info!(
            models_with_precaching = configs.iter().filter(|(_, c)| c.enabled).count(),
            "Initialized ChunkWarmer"
        );

        for (model, config) in &configs {
            if config.enabled {
                info!(
                    model = %model,
                    keep_recent = config.keep_recent,
                    warm_on_ingest = config.warm_on_ingest,
                    poll_interval_secs = config.poll_interval_secs,
                    zoom_levels = ?config.zoom_levels,
                    "Chunk precaching enabled"
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

    /// Warm a specific dataset by reading at configured zoom levels.
    /// This loads and caches chunks through the normal Zarr read path.
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

        // Build cache key
        let cache_key = format!(
            "{}:{}:{}",
            storage_path,
            reference_time.timestamp(),
            config
                .zoom_levels
                .iter()
                .map(|z| z.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        // Check if already warmed
        {
            let warmed = self.warmed_keys.read().await;
            if warmed.contains(&cache_key) {
                debug!(cache_key = %cache_key, "Already warmed, skipping");
                return;
            }
        }

        info!(
            model = %model,
            parameter = %parameter,
            storage_path = %storage_path,
            zoom_levels = ?config.zoom_levels,
            "Warming chunk cache"
        );

        let start = Instant::now();

        // Query catalog for this specific entry
        let entry = match self
            .state
            .catalog
            .find_by_time(model, parameter, reference_time)
            .await
        {
            Ok(Some(e)) => e,
            Ok(None) => {
                debug!(
                    model = %model,
                    parameter = %parameter,
                    reference_time = ?reference_time,
                    "No catalog entry found for warming"
                );
                return;
            }
            Err(e) => {
                warn!(
                    model = %model,
                    parameter = %parameter,
                    error = %e,
                    "Catalog query failed during warming"
                );
                return;
            }
        };

        // Check if entry has zarr_metadata
        if entry.zarr_metadata.is_none() {
            debug!(
                model = %model,
                parameter = %parameter,
                "Entry has no zarr_metadata, skipping warm"
            );
            return;
        }

        // Warm at each configured zoom level
        let mut success_count = 0;
        for &zoom in &config.zoom_levels {
            match self.warm_at_zoom(&entry, zoom).await {
                Ok(()) => success_count += 1,
                Err(e) => {
                    debug!(
                        model = %model,
                        parameter = %parameter,
                        zoom = zoom,
                        error = %e,
                        "Failed to warm at zoom level"
                    );
                }
            }
        }

        let elapsed = start.elapsed();
        info!(
            model = %model,
            parameter = %parameter,
            cache_key = %cache_key,
            zoom_levels_warmed = success_count,
            zoom_levels_total = config.zoom_levels.len(),
            elapsed_ms = elapsed.as_millis(),
            "Chunk cache warmed"
        );

        self.warmed_keys.write().await.insert(cache_key);
    }

    /// Warm chunks for a specific zoom level by reading the full extent.
    async fn warm_at_zoom(&self, entry: &CatalogEntry, zoom: u32) -> Result<(), String> {
        // Calculate output size based on zoom level
        // This determines which pyramid level gets read
        let output_size = self.zoom_level_to_output_size(zoom, entry);

        // Calculate bbox for full extent
        let bbox = Some([
            entry.bbox.min_x as f32,
            entry.bbox.min_y as f32,
            entry.bbox.max_x as f32,
            entry.bbox.max_y as f32,
        ]);

        // Read through the normal Zarr path - this populates the chunk cache
        // Check if model requires full grid reads (non-geographic projection)
        let requires_full_grid = self.state.model_dimensions.requires_full_grid(&entry.model);

        let start = Instant::now();
        let result = load_grid_data(
            &self.state.grid_processor_factory,
            entry,
            bbox,
            Some(output_size),
            requires_full_grid,
        )
        .await?;

        let read_duration = start.elapsed();

        debug!(
            storage_path = %entry.storage_path,
            zoom = zoom,
            output_size = ?output_size,
            result_width = result.width,
            result_height = result.height,
            read_ms = read_duration.as_millis(),
            "Warmed chunks at zoom level"
        );

        Ok(())
    }

    /// Convert a web map zoom level to an output tile size.
    /// This determines which pyramid level gets read and cached.
    fn zoom_level_to_output_size(&self, zoom: u32, entry: &CatalogEntry) -> (usize, usize) {
        // At zoom 0, approximate output as 256x256 for the world
        // Each zoom level doubles the resolution
        let base_size = 256usize << zoom;

        // Scale based on bbox extent relative to world
        let lon_extent = entry.bbox.max_x - entry.bbox.min_x;
        let lat_extent = entry.bbox.max_y - entry.bbox.min_y;

        let width = ((lon_extent / 360.0) * base_size as f64) as usize;
        let height = ((lat_extent / 180.0) * base_size as f64) as usize;

        // Ensure minimum size
        (width.max(256), height.max(256))
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
        let entries = self
            .state
            .catalog
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
            zoom_levels = ?config.zoom_levels,
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

            // Check if entry has zarr_metadata
            if entry.zarr_metadata.is_none() {
                skipped += 1;
                continue;
            }

            // Build cache key
            let cache_key = format!(
                "{}:{}:{}",
                entry.storage_path,
                entry.reference_time.timestamp(),
                config
                    .zoom_levels
                    .iter()
                    .map(|z| z.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );

            // Skip if already warmed
            if self.warmed_keys.read().await.contains(&cache_key) {
                skipped += 1;
                continue;
            }

            // Warm this entry
            self.warm_dataset(
                model,
                &entry.parameter,
                &entry.storage_path,
                entry.reference_time,
            )
            .await;

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
        let min_poll_secs = self
            .configs
            .iter()
            .filter(|(_, c)| c.enabled)
            .map(|(_, c)| c.poll_interval_secs)
            .min()
            .unwrap_or(60);

        info!(
            poll_interval_secs = min_poll_secs,
            "Starting chunk warming background task"
        );

        // Initial warm on startup
        self.warm_recent_all().await;

        let mut ticker = interval(Duration::from_secs(min_poll_secs));

        loop {
            ticker.tick().await;

            debug!("Chunk warming poll tick");
            self.warm_recent_all().await;

            // Periodically clean up the warmed_keys set to prevent unbounded growth
            self.cleanup_warmed_keys().await;
        }
    }

    /// Clean up warmed_keys set by removing old entries.
    /// Since we track by storage_path+timestamp, old entries naturally become stale
    /// when the catalog no longer contains them.
    async fn cleanup_warmed_keys(&self) {
        let mut warmed = self.warmed_keys.write().await;
        let before = warmed.len();

        // Keep set size bounded - remove oldest entries if too large
        const MAX_WARMED_KEYS: usize = 10000;
        if warmed.len() > MAX_WARMED_KEYS {
            // Just clear half the entries (simple approach)
            let to_keep: Vec<_> = warmed.iter().take(MAX_WARMED_KEYS / 2).cloned().collect();
            warmed.clear();
            for key in to_keep {
                warmed.insert(key);
            }
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
}
