//! Cache warming module for pre-rendering tiles at startup.
//!
//! This module warms the L1 and L2 caches by pre-rendering tiles for zoom levels 0-4
//! across all configured layers and forecast hours. This dramatically improves cold
//! start performance by ensuring frequently accessed tiles are already cached.

use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn, debug};
use wms_common::TileCoord;

use crate::state::AppState;
use crate::rendering;

/// Cache warming configuration.
#[derive(Clone, Debug)]
pub struct WarmingConfig {
    /// Enable cache warming at startup
    pub enabled: bool,
    /// Maximum zoom level to warm (0-4 recommended)
    pub max_zoom: u32,
    /// Forecast hours to warm (e.g., [0, 3, 6])
    pub forecast_hours: Vec<u32>,
    /// Layers to warm with their styles
    pub layers: Vec<WarmingLayer>,
    /// Number of concurrent warming tasks
    pub concurrency: usize,
}

/// Layer configuration for cache warming.
#[derive(Clone, Debug)]
pub struct WarmingLayer {
    /// Layer name (e.g., "gfs_TMP")
    pub name: String,
    /// Style name (e.g., "temperature")
    pub style: String,
}

/// Result of warming a single tile.
#[derive(Debug)]
#[allow(dead_code)]
enum WarmResult {
    /// Tile was rendered successfully
    Rendered,
    /// Tile was already in cache
    AlreadyCached,
    /// Tile warming failed
    Failed(String),
}

/// Cache warmer for pre-rendering tiles.
pub struct CacheWarmer {
    state: Arc<AppState>,
    config: WarmingConfig,
}

impl CacheWarmer {
    /// Create a new cache warmer.
    pub fn new(state: Arc<AppState>, config: WarmingConfig) -> Self {
        Self { state, config }
    }
    
    /// Run initial cache warming at startup.
    pub async fn warm_startup(&self) {
        if !self.config.enabled {
            info!("Cache warming disabled");
            return;
        }
        
        info!(
            max_zoom = self.config.max_zoom,
            layers = self.config.layers.len(),
            hours = ?self.config.forecast_hours,
            concurrency = self.config.concurrency,
            "Starting cache warming"
        );
        
        let start = Instant::now();
        
        // Generate all tiles to warm
        let tiles = self.generate_warming_tiles();
        let total_tiles = tiles.len();
        
        info!(total_tiles = total_tiles, "Generated warming tile list");
        
        // Process with limited concurrency using semaphore
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.concurrency));
        let mut handles = Vec::new();
        
        for (layer, style, coord, hour) in tiles {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let state = self.state.clone();
            
            let handle = tokio::spawn(async move {
                let result = warm_single_tile(&state, &layer, &style, coord, hour).await;
                drop(permit); // Release permit
                result
            });
            
            handles.push(handle);
        }
        
        // Collect results
        let mut success = 0;
        let mut cached = 0;
        let mut failed = 0;
        
        for handle in handles {
            match handle.await {
                Ok(WarmResult::Rendered) => success += 1,
                Ok(WarmResult::AlreadyCached) => cached += 1,
                Ok(WarmResult::Failed(_)) => failed += 1,
                Err(e) => {
                    warn!(error = %e, "Warming task panicked");
                    failed += 1;
                }
            }
            
            // Log progress every 100 tiles
            let completed = success + cached + failed;
            if completed % 100 == 0 {
                info!(
                    progress = format!("{}/{}", completed, total_tiles),
                    rendered = success,
                    cached = cached,
                    failed = failed,
                    "Warming progress"
                );
            }
        }
        
        let duration = start.elapsed();
        let tiles_per_sec = total_tiles as f64 / duration.as_secs_f64();
        
        info!(
            duration_secs = duration.as_secs(),
            duration_ms = duration.as_millis(),
            total = total_tiles,
            rendered = success,
            already_cached = cached,
            failed = failed,
            tiles_per_sec = format!("{:.1}", tiles_per_sec),
            "Cache warming complete"
        );
    }
    
    /// Generate list of all tiles to warm.
    fn generate_warming_tiles(&self) -> Vec<(String, String, TileCoord, u32)> {
        let mut tiles = Vec::new();
        
        for layer in &self.config.layers {
            for hour in &self.config.forecast_hours {
                for z in 0..=self.config.max_zoom {
                    let max_xy = 2u32.pow(z);
                    for x in 0..max_xy {
                        for y in 0..max_xy {
                            tiles.push((
                                layer.name.clone(),
                                layer.style.clone(),
                                TileCoord::new(z, x, y),
                                *hour,
                            ));
                        }
                    }
                }
            }
        }
        
        tiles
    }
}

/// Warm a single tile by rendering it and storing in cache.
async fn warm_single_tile(
    state: &Arc<AppState>,
    layer: &str,
    style: &str,
    coord: TileCoord,
    forecast_hour: u32,
) -> WarmResult {
    use storage::CacheKey;
    use wms_common::{BoundingBox, CrsCode};
    
    // Build cache key with time dimension
    let dimension_suffix = Some(format!("t{}", forecast_hour));
    
    let cache_key = CacheKey::new(
        layer,
        style,
        CrsCode::Epsg3857,
        BoundingBox::new(coord.x as f64, coord.y as f64, coord.z as f64, 0.0),
        256,
        256,
        dimension_suffix.clone(),
        "png",
    );
    
    // Build string key for L1 cache
    let cache_key_str = format!(
        "{}:{}:EPSG:3857:{}_{}_{}:{}",
        layer,
        style,
        coord.z,
        coord.x,
        coord.y,
        dimension_suffix.as_deref().unwrap_or("current")
    );
    
    // Check if already in L1 cache
    if state.tile_memory_cache.get(&cache_key_str).await.is_some() {
        debug!(layer = %layer, z = coord.z, x = coord.x, y = coord.y, "Already in L1 cache");
        return WarmResult::AlreadyCached;
    }
    
    // Check if already in L2 cache
    {
        let mut cache = state.cache.lock().await;
        if let Ok(Some(_)) = cache.get(&cache_key).await {
            debug!(layer = %layer, z = coord.z, x = coord.x, y = coord.y, "Already in L2 cache");
            return WarmResult::AlreadyCached;
        }
    }
    
    // Not cached - render the tile
    debug!(layer = %layer, z = coord.z, x = coord.x, y = coord.y, hour = forecast_hour, "Rendering tile for warming");
    
    // Get tile bounding box
    let latlon_bbox = wms_common::tile::tile_to_latlon_bounds(&coord);
    let bbox_array = [
        latlon_bbox.min_x as f32,
        latlon_bbox.min_y as f32,
        latlon_bbox.max_x as f32,
        latlon_bbox.max_y as f32,
    ];
    
    // Parse layer name (format: "model_parameter")
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return WarmResult::Failed("Invalid layer format".to_string());
    }
    
    let model = parts[0];
    // Uppercase parameter to match database storage
    let parameter = parts[1..].join("_").to_uppercase();
    
    // Get default level from layer config for consistent data selection
    let default_level: Option<String> = {
        let configs = state.layer_configs.read().await;
        configs
            .get_layer_by_param(model, &parameter)
            .and_then(|l| l.default_level())
            .map(|s| s.to_string())
    };
    
    // Render the tile based on layer type
    let result = if parameter == "WIND_BARBS" {
        rendering::render_wind_barbs_tile_with_level(
            &state.catalog,
            &state.grid_processor_factory,
            model,
            Some(coord),
            256,
            256,
            bbox_array,
            Some(forecast_hour),
            default_level.as_deref(), // Use default level
        )
        .await
    } else if style == "isolines" {
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = format!("{}/temperature.json", style_config_dir);
        rendering::render_isolines_tile_with_level(
            &state.catalog,
            &state.grid_processor_factory,
            model,
            &parameter,
            Some(coord), // tile_coord
            256,
            256,
            bbox_array,
            &style_file,
            "isolines",  // style name within the file
            Some(forecast_hour),
            default_level.as_deref(), // Use default level
            true, // use_mercator
        )
        .await
    } else {
        // Get style file from layer config registry (single source of truth)
        let style_file = state.layer_configs.read().await.get_style_file_for_parameter(model, &parameter);
        // Check if model requires full grid reads (non-geographic projection)
        let requires_full_grid = state.model_dimensions.requires_full_grid(model);
        
        rendering::render_weather_data_with_level(
            &state.catalog,
            &state.metrics,
            &state.grid_processor_factory,
            model,
            &parameter,
            Some(forecast_hour),
            default_level.as_deref(), // Use default level for consistent data selection
            256,
            256,
            Some(bbox_array),
            &style_file,
            Some(style), // style_name
            true, // use_mercator
            requires_full_grid,
        )
        .await
    };
    
    match result {
        Ok(tile_data) => {
            // Store in L1 cache
            let data_bytes = bytes::Bytes::from(tile_data.clone());
            state.tile_memory_cache.set(&cache_key_str, data_bytes, None).await;
            
            // Store in L2 cache
            {
                let mut cache = state.cache.lock().await;
                if let Err(e) = cache.set(&cache_key, &tile_data, None).await {
                    warn!(error = %e, layer = %layer, "Failed to store in L2 cache");
                }
            }
            
            debug!(layer = %layer, z = coord.z, x = coord.x, y = coord.y, "Tile warmed successfully");
            WarmResult::Rendered
        }
        Err(e) => {
            debug!(error = %e, layer = %layer, z = coord.z, x = coord.x, y = coord.y, "Failed to warm tile");
            WarmResult::Failed(e.to_string())
        }
    }
}

impl Default for WarmingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_zoom: 4,
            forecast_hours: vec![0],
            layers: Vec::new(),
            concurrency: 10,
        }
    }
}
