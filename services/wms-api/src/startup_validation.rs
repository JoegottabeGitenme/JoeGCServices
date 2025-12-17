//! Startup validation module for verifying data ingestion and warming caches.
//!
//! This module performs validation renders at startup to:
//! 1. Verify that data has been ingested correctly for each model
//! 2. Ensure the rendering pipeline is working end-to-end
//! 3. Warm the caches by pre-rendering representative tiles
//! 4. Report on data availability status
//!
//! Unlike cache warming which renders many tiles at low zoom levels,
//! startup validation focuses on rendering a small set of representative
//! tiles at various zoom levels to validate the full pipeline.

use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn, error, debug};
use wms_common::TileCoord;
use serde::Serialize;

use crate::state::AppState;
use crate::rendering;

/// Configuration for startup validation.
#[derive(Clone, Debug)]
pub struct StartupValidationConfig {
    /// Enable startup validation
    pub enabled: bool,
    /// Number of concurrent validation tasks
    pub concurrency: usize,
    /// Whether to fail startup if validation fails
    pub fail_on_error: bool,
    /// Zoom levels to test (e.g., [2, 4, 6])
    pub test_zoom_levels: Vec<u32>,
    /// Skip validation for specific models
    pub skip_models: Vec<String>,
}

impl Default for StartupValidationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            concurrency: 4,
            fail_on_error: false,
            test_zoom_levels: vec![2, 4, 6],
            skip_models: Vec::new(),
        }
    }
}

impl StartupValidationConfig {
    /// Parse configuration from environment variables.
    pub fn from_env() -> Self {
        use std::env;
        
        fn parse_bool(key: &str, default: bool) -> bool {
            env::var(key)
                .ok()
                .map(|v| v.to_lowercase() == "true" || v == "1")
                .unwrap_or(default)
        }
        
        fn parse_usize(key: &str, default: usize) -> usize {
            env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        
        Self {
            enabled: parse_bool("ENABLE_STARTUP_VALIDATION", true),
            concurrency: parse_usize("STARTUP_VALIDATION_CONCURRENCY", 4),
            fail_on_error: parse_bool("STARTUP_VALIDATION_FAIL_ON_ERROR", false),
            test_zoom_levels: env::var("STARTUP_VALIDATION_ZOOM_LEVELS")
                .ok()
                .map(|v| {
                    v.split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .collect()
                })
                .unwrap_or_else(|| vec![2, 4, 6]),
            skip_models: env::var("STARTUP_VALIDATION_SKIP_MODELS")
                .ok()
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_lowercase())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// Result of a single validation test.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationTestResult {
    pub model: String,
    pub parameter: String,
    pub layer: String,
    pub style: String,
    pub zoom: u32,
    pub success: bool,
    pub render_time_ms: u64,
    pub tile_size_bytes: usize,
    pub error: Option<String>,
}

/// Summary of startup validation results.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationSummary {
    pub total_tests: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration_ms: u64,
    pub models_available: Vec<String>,
    pub models_missing: Vec<String>,
    pub results: Vec<ValidationTestResult>,
}

/// Model/layer configuration for validation.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ValidationTarget {
    model: String,
    parameter: String,
    style: String,
    /// Bounding box for test tile [min_x, min_y, max_x, max_y] in lat/lon
    test_bbox: [f32; 4],
    /// Description for logging
    description: String,
}

/// Startup validator.
pub struct StartupValidator {
    state: Arc<AppState>,
    config: StartupValidationConfig,
}

impl StartupValidator {
    /// Create a new startup validator.
    pub fn new(state: Arc<AppState>, config: StartupValidationConfig) -> Self {
        Self { state, config }
    }
    
    /// Run startup validation and return summary.
    pub async fn validate(&self) -> ValidationSummary {
        if !self.config.enabled {
            info!("Startup validation disabled");
            return ValidationSummary {
                total_tests: 0,
                passed: 0,
                failed: 0,
                skipped: 0,
                duration_ms: 0,
                models_available: Vec::new(),
                models_missing: Vec::new(),
                results: Vec::new(),
            };
        }
        
        let start = Instant::now();
        
        info!(
            concurrency = self.config.concurrency,
            zoom_levels = ?self.config.test_zoom_levels,
            "Starting startup validation"
        );
        
        // Discover available data from catalog
        let available_models = self.discover_available_models().await;
        
        info!(
            available = ?available_models,
            "Discovered available models"
        );
        
        // Define validation targets for each model
        let targets = self.build_validation_targets(&available_models);
        
        // Run validation tests
        let results = self.run_validation_tests(targets).await;
        
        // Build summary
        let passed = results.iter().filter(|r| r.success).count();
        let failed = results.iter().filter(|r| !r.success && r.error.is_some()).count();
        let skipped = results.iter().filter(|r| !r.success && r.error.is_none()).count();
        
        let all_known_models = ["gfs", "hrrr", "goes16", "goes18", "mrms"];
        let models_missing: Vec<String> = all_known_models
            .iter()
            .filter(|m| !available_models.contains(&m.to_string()))
            .map(|s| s.to_string())
            .collect();
        
        let duration = start.elapsed();
        
        let summary = ValidationSummary {
            total_tests: results.len(),
            passed,
            failed,
            skipped,
            duration_ms: duration.as_millis() as u64,
            models_available: available_models,
            models_missing,
            results,
        };
        
        // Log summary
        if failed > 0 {
            warn!(
                total = summary.total_tests,
                passed = summary.passed,
                failed = summary.failed,
                duration_ms = summary.duration_ms,
                "Startup validation completed with failures"
            );
        } else {
            info!(
                total = summary.total_tests,
                passed = summary.passed,
                skipped = summary.skipped,
                duration_ms = summary.duration_ms,
                models = ?summary.models_available,
                "Startup validation completed successfully"
            );
        }
        
        summary
    }
    
    /// Discover which models have available data in the catalog.
    async fn discover_available_models(&self) -> Vec<String> {
        let mut models = Vec::new();
        
        // Query catalog for available datasets grouped by model
        match self.state.catalog.get_available_models().await {
            Ok(available) => {
                models = available;
            }
            Err(e) => {
                warn!(error = %e, "Failed to query catalog for available models");
                // Fall back to checking known models individually
                for model in &["gfs", "hrrr", "goes16", "goes18", "mrms"] {
                    if self.check_model_has_data(model).await {
                        models.push(model.to_string());
                    }
                }
            }
        }
        
        // Filter out skipped models
        models.retain(|m| !self.config.skip_models.contains(&m.to_lowercase()));
        
        models
    }
    
    /// Check if a specific model has any data available.
    async fn check_model_has_data(&self, model: &str) -> bool {
        match self.state.catalog.get_latest_dataset(model, None).await {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(e) => {
                debug!(model = %model, error = %e, "Failed to check model data");
                false
            }
        }
    }
    
    /// Build validation targets based on available models.
    fn build_validation_targets(&self, available_models: &[String]) -> Vec<ValidationTarget> {
        let mut targets = Vec::new();
        
        for model in available_models {
            match model.as_str() {
                "gfs" => {
                    // GFS: Global model - test temperature and wind
                    targets.push(ValidationTarget {
                        model: "gfs".to_string(),
                        parameter: "TMP".to_string(),
                        style: "temperature".to_string(),
                        test_bbox: [-130.0, 20.0, -60.0, 55.0], // CONUS
                        description: "GFS Temperature".to_string(),
                    });
                    targets.push(ValidationTarget {
                        model: "gfs".to_string(),
                        parameter: "WIND_BARBS".to_string(),
                        style: "default".to_string(),
                        test_bbox: [-130.0, 20.0, -60.0, 55.0],
                        description: "GFS Wind Barbs".to_string(),
                    });
                    targets.push(ValidationTarget {
                        model: "gfs".to_string(),
                        parameter: "PRMSL".to_string(),
                        style: "atmospheric".to_string(),
                        test_bbox: [-180.0, -60.0, 180.0, 60.0], // Global
                        description: "GFS Pressure".to_string(),
                    });
                }
                "hrrr" => {
                    // HRRR: CONUS high-resolution
                    targets.push(ValidationTarget {
                        model: "hrrr".to_string(),
                        parameter: "TMP".to_string(),
                        style: "temperature".to_string(),
                        test_bbox: [-125.0, 21.0, -60.0, 50.0], // CONUS
                        description: "HRRR Temperature".to_string(),
                    });
                    targets.push(ValidationTarget {
                        model: "hrrr".to_string(),
                        parameter: "WIND_BARBS".to_string(),
                        style: "default".to_string(),
                        test_bbox: [-125.0, 21.0, -60.0, 50.0],
                        description: "HRRR Wind Barbs".to_string(),
                    });
                }
                "goes16" | "goes18" => {
                    // GOES: Satellite imagery
                    let goes_model = model.clone();
                    targets.push(ValidationTarget {
                        model: goes_model.clone(),
                        parameter: "CMI_C02".to_string(),
                        style: "goes_visible".to_string(),
                        test_bbox: [-140.0, 15.0, -55.0, 55.0], // CONUS Full Disk view
                        description: format!("{} Visible", model.to_uppercase()),
                    });
                    targets.push(ValidationTarget {
                        model: goes_model,
                        parameter: "CMI_C13".to_string(),
                        style: "goes_ir".to_string(),
                        test_bbox: [-140.0, 15.0, -55.0, 55.0],
                        description: format!("{} Infrared", model.to_uppercase()),
                    });
                }
                "mrms" => {
                    // MRMS: Radar
                    targets.push(ValidationTarget {
                        model: "mrms".to_string(),
                        parameter: "REFL".to_string(),
                        style: "reflectivity".to_string(),
                        test_bbox: [-130.0, 20.0, -60.0, 55.0], // CONUS
                        description: "MRMS Reflectivity".to_string(),
                    });
                    targets.push(ValidationTarget {
                        model: "mrms".to_string(),
                        parameter: "PRECIP_RATE".to_string(),
                        style: "precip_rate".to_string(),
                        test_bbox: [-130.0, 20.0, -60.0, 55.0],
                        description: "MRMS Precip Rate".to_string(),
                    });
                }
                _ => {
                    debug!(model = %model, "Unknown model, skipping validation targets");
                }
            }
        }
        
        targets
    }
    
    /// Run validation tests for all targets.
    async fn run_validation_tests(&self, targets: Vec<ValidationTarget>) -> Vec<ValidationTestResult> {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.concurrency));
        let mut handles = Vec::new();
        
        for target in targets {
            for &zoom in &self.config.test_zoom_levels {
                let permit = semaphore.clone().acquire_owned().await.unwrap();
                let state = self.state.clone();
                let target = target.clone();
                
                let handle = tokio::spawn(async move {
                    let result = run_single_validation(&state, &target, zoom).await;
                    drop(permit);
                    result
                });
                
                handles.push(handle);
            }
        }
        
        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => {
                    error!(error = %e, "Validation task panicked");
                }
            }
        }
        
        results
    }
}

/// Run a single validation test.
async fn run_single_validation(
    state: &Arc<AppState>,
    target: &ValidationTarget,
    zoom: u32,
) -> ValidationTestResult {
    let start = Instant::now();
    let layer = format!("{}_{}", target.model, target.parameter);
    
    debug!(
        layer = %layer,
        style = %target.style,
        zoom = zoom,
        "Running validation test"
    );
    
    // Calculate a representative tile for this bbox and zoom
    let center_lon = (target.test_bbox[0] + target.test_bbox[2]) / 2.0;
    let center_lat = (target.test_bbox[1] + target.test_bbox[3]) / 2.0;
    let coord = lat_lon_to_tile(center_lat as f64, center_lon as f64, zoom);
    
    // Get tile bounding box
    let latlon_bbox = wms_common::tile::tile_to_latlon_bounds(&coord);
    let bbox_array = [
        latlon_bbox.min_x as f32,
        latlon_bbox.min_y as f32,
        latlon_bbox.max_x as f32,
        latlon_bbox.max_y as f32,
    ];
    
    // Get default level from layer config for consistent data selection
    let default_level: Option<String> = {
        let configs = state.layer_configs.read().await;
        configs
            .get_layer_by_param(&target.model, &target.parameter)
            .and_then(|l| l.default_level())
            .map(|s| s.to_string())
    };
    
    // Render the tile
    let result = if target.parameter == "WIND_BARBS" {
        rendering::render_wind_barbs_tile_with_level(
            &state.grib_cache,
            &state.catalog,
            Some(&state.grid_processor_factory),
            &target.model,
            Some(coord),
            256,
            256,
            bbox_array,
            None, // Use latest forecast hour
            default_level.as_deref(), // Use default level
        )
        .await
    } else {
        rendering::render_weather_data_with_level(
            &state.grib_cache,
            &state.catalog,
            &state.metrics,
            &target.model,
            &target.parameter,
            None, // forecast_hour - use latest
            default_level.as_deref(), // Use default level for consistent data selection
            256,
            256,
            Some(bbox_array),
            Some(&target.style),
            true, // use_mercator
        )
        .await
    };
    
    let render_time = start.elapsed();
    
    match result {
        Ok(tile_data) => {
            // Validate PNG format
            let is_valid_png = tile_data.len() >= 8 && 
                tile_data[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
            
            if is_valid_png {
                // Store in cache for warming benefit
                cache_tile(state, &layer, &target.style, &coord, &tile_data).await;
                
                info!(
                    layer = %layer,
                    zoom = zoom,
                    size_bytes = tile_data.len(),
                    render_ms = render_time.as_millis(),
                    "Validation passed"
                );
                
                ValidationTestResult {
                    model: target.model.clone(),
                    parameter: target.parameter.clone(),
                    layer,
                    style: target.style.clone(),
                    zoom,
                    success: true,
                    render_time_ms: render_time.as_millis() as u64,
                    tile_size_bytes: tile_data.len(),
                    error: None,
                }
            } else {
                warn!(
                    layer = %layer,
                    zoom = zoom,
                    "Validation failed: invalid PNG format"
                );
                
                ValidationTestResult {
                    model: target.model.clone(),
                    parameter: target.parameter.clone(),
                    layer,
                    style: target.style.clone(),
                    zoom,
                    success: false,
                    render_time_ms: render_time.as_millis() as u64,
                    tile_size_bytes: tile_data.len(),
                    error: Some("Invalid PNG format".to_string()),
                }
            }
        }
        Err(e) => {
            // Check if this is a "no data" error vs a real failure
            let error_str = e.to_string();
            let is_no_data = error_str.contains("No data") || 
                            error_str.contains("not found") ||
                            error_str.contains("No datasets");
            
            if is_no_data {
                debug!(
                    layer = %layer,
                    zoom = zoom,
                    "No data available for validation"
                );
            } else {
                warn!(
                    layer = %layer,
                    zoom = zoom,
                    error = %e,
                    "Validation render failed"
                );
            }
            
            ValidationTestResult {
                model: target.model.clone(),
                parameter: target.parameter.clone(),
                layer,
                style: target.style.clone(),
                zoom,
                success: false,
                render_time_ms: render_time.as_millis() as u64,
                tile_size_bytes: 0,
                error: Some(error_str),
            }
        }
    }
}

/// Store a validated tile in the cache for warming benefit.
async fn cache_tile(
    state: &Arc<AppState>,
    layer: &str,
    style: &str,
    coord: &TileCoord,
    data: &[u8],
) {
    use storage::CacheKey;
    use wms_common::{BoundingBox, CrsCode};
    
    // Build cache key
    let cache_key_str = format!(
        "{}:{}:EPSG:3857:{}_{}_{}:current",
        layer, style, coord.z, coord.x, coord.y
    );
    
    // Store in L1 cache
    let data_bytes = bytes::Bytes::from(data.to_vec());
    state.tile_memory_cache.set(&cache_key_str, data_bytes, None).await;
    
    // Store in L2 cache
    let cache_key = CacheKey::new(
        layer,
        style,
        CrsCode::Epsg3857,
        BoundingBox::new(coord.x as f64, coord.y as f64, coord.z as f64, 0.0),
        256,
        256,
        None,
        "png",
    );
    
    let mut cache = state.cache.lock().await;
    if let Err(e) = cache.set(&cache_key, data, None).await {
        debug!(error = %e, "Failed to store validation tile in L2 cache");
    }
}

/// Convert lat/lon to tile coordinates at a given zoom level.
fn lat_lon_to_tile(lat: f64, lon: f64, zoom: u32) -> TileCoord {
    let n = 2.0_f64.powi(zoom as i32);
    let x = ((lon + 180.0) / 360.0 * n).floor() as u32;
    let lat_rad = lat.to_radians();
    let y = ((1.0 - lat_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * n).floor() as u32;
    TileCoord::new(zoom, x.min((n as u32) - 1), y.min((n as u32) - 1))
}
