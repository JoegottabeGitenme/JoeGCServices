//! Shared weather data rendering logic.

use renderer::gradient;
use renderer::barbs::{self, BarbConfig};
use renderer::contour;
use renderer::numbers::{self, NumbersConfig, GridPointNumbersConfig};
use renderer::style::{StyleConfig, apply_style_gradient, ContourStyle};
use storage::{Catalog, CatalogEntry, GribCache, GridDataCache, CachedGridData};
use std::path::Path;
use std::time::Instant;
use tracing::{info, debug, warn, error, instrument};
use projection::{LambertConformal, Geostationary};
use crate::metrics::{MetricsCollector, DataSourceType};
use crate::state::{GridProcessorFactory, ProjectionLuts};

/// Render weather data from GRIB2 grid to PNG.
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `model`: Weather model name
/// - `parameter`: Parameter name (e.g., "TMP", "WIND_SPEED")
/// - `forecast_hour`: Optional forecast hour; if None, uses latest
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box [min_lon, min_lat, max_lon, max_lat]
///
/// # Returns
/// PNG image data as bytes
pub async fn render_weather_data(
    grib_cache: &GribCache,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
) -> Result<Vec<u8>, String> {
    render_weather_data_with_style(grib_cache, catalog, metrics, model, parameter, forecast_hour, width, height, bbox, None, false).await
}

/// Render weather data with optional style configuration.
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `model`: Weather model name
/// - `parameter`: Parameter name (e.g., "TMP", "WIND_SPEED")
/// - `forecast_hour`: Optional forecast hour; if None, uses latest
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box [min_lon, min_lat, max_lon, max_lat]; if None, renders full globe
/// - `style_name`: Optional style name to apply from configuration
/// - `use_mercator`: Use Web Mercator projection for resampling
///
/// # Returns
/// PNG image data as bytes
pub async fn render_weather_data_with_style(
    grib_cache: &GribCache,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    style_name: Option<&str>,
    use_mercator: bool,
) -> Result<Vec<u8>, String> {
    render_weather_data_with_level(
        grib_cache, catalog, metrics, model, parameter, forecast_hour, None, width, height, bbox, style_name, use_mercator
    ).await
}

/// Render weather data with optional style configuration and level.
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `model`: Weather model name
/// - `parameter`: Parameter name (e.g., "TMP", "WIND_SPEED")
/// - `forecast_hour`: Optional forecast hour; if None, uses latest
/// - `level`: Optional level/elevation (e.g., "500 mb", "2 m above ground")
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box [min_lon, min_lat, max_lon, max_lat]; if None, renders full globe
/// - `style_name`: Optional style name to apply from configuration
/// - `use_mercator`: Use Web Mercator projection for resampling
///
/// # Returns
/// PNG image data as bytes
pub async fn render_weather_data_with_level(
    grib_cache: &GribCache,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    style_name: Option<&str>,
    use_mercator: bool,
) -> Result<Vec<u8>, String> {
    render_weather_data_with_time(
        grib_cache, None, catalog, metrics, model, parameter, forecast_hour, None, level, width, height, bbox, style_name, use_mercator
    ).await
}

/// Render weather data with support for both forecast hours and observation times.
///
/// This is the main rendering function that supports:
/// - Forecast models (GFS, HRRR): Use `forecast_hour` parameter
/// - Observation data (MRMS, GOES): Use `observation_time` parameter
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving files
/// - `catalog`: Catalog for finding datasets
/// - `model`: Weather model name
/// - `parameter`: Parameter name
/// - `forecast_hour`: Optional forecast hour for forecast models
/// - `observation_time`: Optional observation time (ISO8601) for observation data
/// - `level`: Optional vertical level
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box
/// - `style_name`: Optional style name
/// - `use_mercator`: Use Web Mercator projection
pub async fn render_weather_data_with_time(
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    observation_time: Option<chrono::DateTime<chrono::Utc>>,
    level: Option<&str>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    style_name: Option<&str>,
    use_mercator: bool,
) -> Result<Vec<u8>, String> {
    // Call the LUT-aware version without tile coords, LUT, or Zarr factory
    // (This path is for legacy callers that don't have Zarr support)
    render_weather_data_with_lut(
        grib_cache, grid_cache, catalog, metrics, model, parameter,
        forecast_hour, observation_time, level, width, height, bbox,
        style_name, use_mercator, None, None, None,
    ).await
}

/// Render weather data with optional projection LUT for fast GOES rendering.
///
/// This is the full-featured rendering function that supports:
/// - Forecast models (GFS, HRRR): Use `forecast_hour` parameter
/// - Observation data (MRMS, GOES): Use `observation_time` parameter
/// - Pre-computed projection LUTs for fast GOES tile rendering
/// - Zarr-format grid data with efficient chunked reads (when zarr_metadata is present)
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving files
/// - `grid_processor_factory`: Optional factory for Zarr-based grid access
/// - `catalog`: Catalog for finding datasets
/// - `model`: Weather model name
/// - `parameter`: Parameter name
/// - `forecast_hour`: Optional forecast hour for forecast models
/// - `observation_time`: Optional observation time (ISO8601) for observation data
/// - `level`: Optional vertical level
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box
/// - `style_name`: Optional style name
/// - `use_mercator`: Use Web Mercator projection
/// - `tile_coords`: Optional tile coordinates (z, x, y) for LUT lookup
/// - `projection_luts`: Optional pre-computed projection LUTs
pub async fn render_weather_data_with_lut(
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    observation_time: Option<chrono::DateTime<chrono::Utc>>,
    level: Option<&str>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    style_name: Option<&str>,
    use_mercator: bool,
    tile_coords: Option<(u32, u32, u32)>,  // (z, x, y) for LUT lookup
    projection_luts: Option<&ProjectionLuts>,
    grid_processor_factory: Option<&GridProcessorFactory>,
) -> Result<Vec<u8>, String> {
    // Record model-specific request
    let render_start = Instant::now();
    let weather_model = crate::metrics::WeatherModel::from_model(model);
    if let Some(wm) = weather_model {
        metrics.record_model_request(wm, parameter);
    }
    
    // Get dataset based on time specification
    let entry = {
        if let Some(obs_time) = observation_time {
            // Observation mode: find dataset closest to requested observation time
            info!(model = model, parameter = parameter, observation_time = ?obs_time, "Looking up observation data by time");
            let entry = catalog
                .find_by_time(model, parameter, obs_time)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No observation data found for {}/{} at time {:?}", model, parameter, obs_time))?;
            info!(
                model = model, 
                parameter = parameter, 
                requested_time = ?obs_time,
                found_time = ?entry.reference_time,
                storage_path = %entry.storage_path,
                "Found observation dataset"
            );
            entry
        } else {
        // Forecast mode: use forecast_hour and level
        match (forecast_hour, level) {
            (Some(hour), Some(lev)) => {
                // Find dataset with matching forecast hour and level
                catalog
                    .find_by_forecast_hour_and_level(model, parameter, hour, lev)
                    .await
                    .map_err(|e| format!("Catalog query failed: {}", e))?
                    .ok_or_else(|| format!("No data found for {}/{} at hour {} level {}", model, parameter, hour, lev))?
            }
            (Some(hour), None) => {
                // Find dataset with matching forecast hour (any level - defaults to first available)
                catalog
                    .find_by_forecast_hour(model, parameter, hour)
                    .await
                    .map_err(|e| format!("Catalog query failed: {}", e))?
                    .ok_or_else(|| format!("No data found for {}/{} at hour {}", model, parameter, hour))?
            }
            (None, Some(lev)) => {
                // Get latest run with earliest forecast hour at specific level
                catalog
                    .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
                    .await
                    .map_err(|e| format!("Catalog query failed: {}", e))?
                    .ok_or_else(|| format!("No data found for {}/{} at level {}", model, parameter, lev))?
            }
            (None, None) => {
                // Get latest run with earliest forecast hour (any level)
                catalog
                    .get_latest_run_earliest_forecast(model, parameter)
                    .await
                    .map_err(|e| format!("Catalog query failed: {}", e))?
                    .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?
            }
        }
    }};

    // Load grid data from cache/storage (handles GRIB2, NetCDF, and Zarr formats)
    // If zarr_metadata is present and grid_processor_factory is available, use efficient
    // chunked reads from Zarr; otherwise fall back to legacy loaders
    info!(
        parameter = parameter,
        level = %entry.level,
        storage_path = %entry.storage_path,
        model = model,
        has_zarr = entry.zarr_metadata.is_some(),
        "Loading grid data"
    );
    
    let start = Instant::now();
    let grid_result = load_grid_data_with_zarr_support(
        grid_processor_factory,
        grib_cache,
        grid_cache,
        metrics,
        &entry,
        parameter,
        bbox,
    ).await?;
    let load_duration = start.elapsed();
    metrics.record_grib_load(load_duration.as_micros() as u64).await;
    
    // Record model-specific fetch metrics
    if let Some(wm) = weather_model {
        metrics.record_model_fetch(wm, entry.file_size, load_duration.as_micros() as u64);
        if let Some(fhr) = forecast_hour {
            metrics.record_model_forecast_hour(wm, fhr);
        }
    }
    
    let grid_data = grid_result.data;
    let grid_width = grid_result.width;
    let grid_height = grid_result.height;
    let goes_projection = grid_result.goes_projection;

    if grid_data.len() != grid_width * grid_height {
        return Err(format!(
            "Grid data size mismatch: {} vs {}x{}",
            grid_data.len(),
            grid_width,
            grid_height
        ));
    }

    // Find GLOBAL data min/max for scaling (from FULL grid, not subset)
    // This ensures all tiles use the same color scale for continuity
    let (min_val, max_val) = grid_data
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &val| {
            (min.min(val), max.max(val))
        });

    // For tile continuity, resample directly from the geographic grid using coordinates
    // This ensures adjacent tiles sample from the exact same grid points at boundaries
    let rendered_width = width as usize;
    let rendered_height = height as usize;
    
    // Use actual bbox from grid data if available (for Zarr partial reads),
    // otherwise fall back to entry.bbox (for GRIB2/NetCDF full grid reads)
    let data_bounds = grid_result.bbox.unwrap_or_else(|| [
        entry.bbox.min_x as f32,
        entry.bbox.min_y as f32,
        entry.bbox.max_x as f32,
        entry.bbox.max_y as f32,
    ]);
    
    let start = Instant::now();
    let resampled_data = {
        if let Some(output_bbox) = bbox {
            // Try to use LUT for GOES models if available
            let lut_result = try_resample_with_lut(
                model,
                tile_coords,
                projection_luts,
                &grid_data,
                grid_width,
            );
            
            if let Some(resampled) = lut_result {
                debug!(model = model, "Used projection LUT for fast resampling");
                resampled
            } else {
                // Fall back to computing projection on-the-fly
                resample_grid_for_bbox_with_proj(
                    &grid_data,
                    grid_width,
                    grid_height,
                    rendered_width,
                    rendered_height,
                    output_bbox,
                    data_bounds,
                    use_mercator,
                    model,
                    goes_projection.as_ref(),
                    grid_result.grid_uses_360,
                )
            }
        } else {
            // No bbox - resample entire data grid
            if grid_width != rendered_width || grid_height != rendered_height {
                renderer::gradient::resample_grid(&grid_data, grid_width, grid_height, rendered_width, rendered_height)
            } else {
                grid_data.clone()
            }
        }
    };
    let resample_duration = start.elapsed();
    let resample_us = resample_duration.as_micros() as u64;
    metrics.record_resample(resample_us).await;
    
    // Record model-specific resample timing
    if let Some(wm) = weather_model {
        metrics.record_model_resample(wm, resample_us);
    }

    // Apply color rendering (gradient/wind barbs/etc)
    let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
    
    let rgba_data = {
        if let Some(style_name) = style_name {
        // Try to load and apply custom style
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            Path::new(&style_config_dir).join("temperature.json")
        } else if parameter.contains("WIND") || parameter.contains("GUST") || parameter.contains("SPEED") {
            Path::new(&style_config_dir).join("wind.json")
        } else if parameter.contains("PRES") || parameter.contains("PRESS") || parameter.contains("PRMSL") {
            Path::new(&style_config_dir).join("mslp.json")
        } else if parameter.contains("RH") || parameter.contains("HUMID") {
            Path::new(&style_config_dir).join("humidity.json")
        } else if parameter.contains("REFL") {
            Path::new(&style_config_dir).join("reflectivity.json")
        } else if parameter.contains("PRECIP_RATE") {
            Path::new(&style_config_dir).join("precip_rate.json")
        } else if parameter.contains("QPE") {
            Path::new(&style_config_dir).join("precipitation.json")
        } else if parameter.contains("CMI_C01") || parameter.contains("CMI_C02") || parameter.contains("CMI_C03") {
            // GOES visible bands (1-3)
            Path::new(&style_config_dir).join("goes_visible.json")
        } else if parameter.starts_with("CMI_C") {
            // GOES IR bands (7-16) and others
            Path::new(&style_config_dir).join("goes_ir.json")
        } else {
            Path::new(&style_config_dir).join("temperature.json")
        };

        match StyleConfig::from_file(style_file.to_str().unwrap_or("")) {
            Ok(config) => {
                if let Some(style) = config.get_style(style_name) {
                    // Apply style-based rendering
                    apply_style_gradient(&resampled_data, rendered_width, rendered_height, style)
                } else {
                    // Fall back to parameter-based rendering if style not found
                    render_by_parameter(&resampled_data, parameter, min_val, max_val, rendered_width, rendered_height)
                }
            }
            Err(_) => {
                // Fall back to parameter-based rendering if style loading fails
                render_by_parameter(&resampled_data, parameter, min_val, max_val, rendered_width, rendered_height)
            }
        }
        } else {
            // Use parameter-based rendering without custom style
            render_by_parameter(&resampled_data, parameter, min_val, max_val, rendered_width, rendered_height)
        }
    };

    // Convert to PNG
    let start = Instant::now();
    let png = {
        renderer::png::create_png(&rgba_data, rendered_width, rendered_height)
            .map_err(|e| format!("PNG encoding failed: {}", e))?
    };
    let png_duration = start.elapsed();
    metrics.record_png_encode(png_duration.as_micros() as u64).await;
    
    // Record model-specific render completion metrics
    let total_render_duration = render_start.elapsed();
    if let Some(wm) = weather_model {
        metrics.record_model_render(wm, parameter, total_render_duration.as_micros() as u64, true);
        metrics.record_model_png_encode(wm, png_duration.as_micros() as u64);
    }

    Ok(png)
}

/// Render data based on parameter type using built-in color scales
fn render_by_parameter(
    data: &[f32],
    parameter: &str,
    min_val: f32,
    max_val: f32,
    width: usize,
    height: usize,
) -> Vec<u8> {
    if parameter.contains("TMP") || parameter.contains("TEMP") {
        // Temperature in Kelvin, convert to Celsius for rendering
        let celsius_data: Vec<f32> = data.iter().map(|k| k - 273.15).collect();
        let min_c = min_val - 273.15;
        let max_c = max_val - 273.15;

        renderer::gradient::render_temperature(&celsius_data, width, height, min_c, max_c)
    } else if parameter.contains("WIND") || parameter.contains("GUST") || parameter.contains("SPEED") {
        // Wind speed in m/s
        renderer::gradient::render_wind_speed(data, width, height, min_val, max_val)
    } else if parameter.contains("PRES") || parameter.contains("PRESS") || parameter.contains("PRMSL") {
        // Pressure in Pa, convert to hPa (PRMSL = Pressure Reduced to Mean Sea Level)
        let hpa_data: Vec<f32> = data.iter().map(|pa| pa / 100.0).collect();
        let min_hpa = min_val / 100.0;
        let max_hpa = max_val / 100.0;

        renderer::gradient::render_pressure(&hpa_data, width, height, min_hpa, max_hpa)
    } else if parameter.contains("RH") || parameter.contains("HUMID") {
        // Relative humidity in percent (0-100)
        renderer::gradient::render_humidity(data, width, height, min_val, max_val)
    } else if parameter.contains("REFL") {
        // Radar reflectivity in dBZ
        render_reflectivity(data, width, height)
    } else if parameter.contains("PRECIP_RATE") {
        // Precipitation rate in mm/hr (or kg/m^2/s which is ~same for rain)
        render_precip_rate(data, width, height)
    } else if parameter.contains("QPE") {
        // Quantitative Precipitation Estimate (accumulated mm)
        render_precipitation_accumulation(data, width, height)
    } else if parameter.contains("CMI_C01") || parameter.contains("CMI_C02") || parameter.contains("CMI_C03") {
        // GOES visible bands - grayscale reflectance (0-1 range)
        render_goes_visible(data, width, height)
    } else if parameter.starts_with("CMI_C") {
        // GOES IR bands - brightness temperature (Kelvin)
        render_goes_ir(data, width, height)
    } else {
        // Generic gradient rendering
        renderer::gradient::render_grid(
            data,
            width,
            height,
            min_val,
            max_val,
            |norm| {
                // Generic blue-red gradient
                let hue = (1.0 - norm) * 240.0; // Blue to red
                let rgb = hsv_to_rgb(hue, 1.0, 1.0);
                gradient::Color::new(rgb.0, rgb.1, rgb.2, 255)
            },
        )
    }
}

/// Render radar reflectivity (dBZ) with standard NWS colors
fn render_reflectivity(data: &[f32], width: usize, height: usize) -> Vec<u8> {
    renderer::gradient::render_grid(
        data,
        width,
        height,
        -10.0,  // min dBZ
        75.0,   // max dBZ
        |norm| {
            // NWS standard radar color scale
            let dbz = norm * 85.0 - 10.0; // Map 0-1 to -10 to 75 dBZ
            
            if dbz < 5.0 {
                // Below threshold - transparent
                gradient::Color::new(0, 0, 0, 0)
            } else if dbz < 10.0 {
                gradient::Color::new(100, 100, 100, 200)
            } else if dbz < 15.0 {
                gradient::Color::new(75, 75, 75, 220)
            } else if dbz < 20.0 {
                gradient::Color::new(0, 236, 236, 255)  // Light cyan
            } else if dbz < 25.0 {
                gradient::Color::new(1, 160, 246, 255)  // Cyan-blue
            } else if dbz < 30.0 {
                gradient::Color::new(0, 0, 246, 255)    // Blue
            } else if dbz < 35.0 {
                gradient::Color::new(0, 255, 0, 255)    // Bright green
            } else if dbz < 40.0 {
                gradient::Color::new(0, 200, 0, 255)    // Green
            } else if dbz < 45.0 {
                gradient::Color::new(0, 144, 0, 255)    // Dark green
            } else if dbz < 50.0 {
                gradient::Color::new(255, 255, 0, 255)  // Yellow
            } else if dbz < 55.0 {
                gradient::Color::new(231, 192, 0, 255)  // Gold
            } else if dbz < 60.0 {
                gradient::Color::new(255, 144, 0, 255)  // Orange
            } else if dbz < 65.0 {
                gradient::Color::new(255, 0, 0, 255)    // Red
            } else if dbz < 70.0 {
                gradient::Color::new(214, 0, 0, 255)    // Dark red
            } else if dbz < 75.0 {
                gradient::Color::new(192, 0, 0, 255)    // Maroon
            } else {
                gradient::Color::new(255, 0, 255, 255)  // Magenta (extreme)
            }
        },
    )
}

/// Render precipitation rate (mm/hr) with blue-green-yellow-red colors
fn render_precip_rate(data: &[f32], width: usize, height: usize) -> Vec<u8> {
    renderer::gradient::render_grid(
        data,
        width,
        height,
        0.0,     // min mm/hr
        100.0,   // max mm/hr
        |norm| {
            let rate = norm * 100.0;  // Map to 0-100 mm/hr
            
            if rate < 0.1 {
                gradient::Color::new(0, 0, 0, 0)  // Transparent for trace
            } else if rate < 0.5 {
                gradient::Color::new(200, 200, 200, 180)  // Light gray
            } else if rate < 1.0 {
                gradient::Color::new(170, 210, 255, 200)  // Very light blue
            } else if rate < 2.5 {
                gradient::Color::new(100, 170, 255, 220)  // Light blue
            } else if rate < 5.0 {
                gradient::Color::new(50, 130, 255, 255)   // Blue
            } else if rate < 10.0 {
                gradient::Color::new(0, 90, 255, 255)     // Dark blue
            } else if rate < 15.0 {
                gradient::Color::new(0, 200, 100, 255)    // Teal
            } else if rate < 20.0 {
                gradient::Color::new(0, 255, 0, 255)      // Green
            } else if rate < 30.0 {
                gradient::Color::new(150, 255, 0, 255)    // Yellow-green
            } else if rate < 40.0 {
                gradient::Color::new(255, 255, 0, 255)    // Yellow
            } else if rate < 50.0 {
                gradient::Color::new(255, 200, 0, 255)    // Gold
            } else if rate < 75.0 {
                gradient::Color::new(255, 140, 0, 255)    // Orange
            } else {
                gradient::Color::new(255, 50, 0, 255)     // Red
            }
        },
    )
}

/// Render precipitation accumulation (QPE) in mm with blue-green-yellow-red colors
fn render_precipitation_accumulation(data: &[f32], width: usize, height: usize) -> Vec<u8> {
    renderer::gradient::render_grid(
        data,
        width,
        height,
        0.0,     // min mm
        150.0,   // max mm
        |norm| {
            let mm = norm * 150.0;  // Map to 0-150 mm
            
            if mm < 0.25 {
                gradient::Color::new(0, 0, 0, 0)  // Transparent for trace
            } else if mm < 1.0 {
                gradient::Color::new(200, 200, 200, 150)  // Light gray
            } else if mm < 2.5 {
                gradient::Color::new(170, 220, 255, 180)  // Very light blue
            } else if mm < 5.0 {
                gradient::Color::new(130, 190, 255, 200)  // Light blue
            } else if mm < 10.0 {
                gradient::Color::new(80, 150, 255, 220)   // Blue
            } else if mm < 15.0 {
                gradient::Color::new(30, 100, 255, 240)   // Dark blue
            } else if mm < 20.0 {
                gradient::Color::new(0, 60, 200, 255)     // Navy
            } else if mm < 25.0 {
                gradient::Color::new(0, 180, 100, 255)    // Teal
            } else if mm < 35.0 {
                gradient::Color::new(0, 230, 0, 255)      // Green
            } else if mm < 50.0 {
                gradient::Color::new(150, 255, 0, 255)    // Yellow-green
            } else if mm < 75.0 {
                gradient::Color::new(255, 255, 0, 255)    // Yellow
            } else if mm < 100.0 {
                gradient::Color::new(255, 190, 0, 255)    // Gold
            } else if mm < 125.0 {
                gradient::Color::new(255, 100, 0, 255)    // Orange
            } else {
                gradient::Color::new(255, 50, 0, 255)     // Red
            }
        },
    )
}

/// Render GOES visible band (reflectance factor 0-1) as grayscale
fn render_goes_visible(data: &[f32], width: usize, height: usize) -> Vec<u8> {
    renderer::gradient::render_grid(
        data,
        width,
        height,
        0.0,    // min reflectance
        1.0,    // max reflectance
        |norm| {
            // Linear grayscale mapping
            let val = (norm * 255.0) as u8;
            gradient::Color::new(val, val, val, 255)
        },
    )
}

/// Render GOES IR band (brightness temperature in Kelvin) with enhanced colors
fn render_goes_ir(data: &[f32], width: usize, height: usize) -> Vec<u8> {
    renderer::gradient::render_grid(
        data,
        width,
        height,
        180.0,  // min temp (very cold cloud tops)
        330.0,  // max temp (warm surface)
        |norm| {
            // Map normalized value to temperature for color mapping
            let temp_k = norm * 150.0 + 180.0; // 180K to 330K
            
            // Enhanced IR colorscale - cold=bright, warm=dark (standard IR display)
            if temp_k < 195.0 {
                gradient::Color::new(0, 0, 0, 255)           // Very cold - black
            } else if temp_k < 205.0 {
                gradient::Color::new(80, 0, 80, 255)         // Purple
            } else if temp_k < 213.0 {
                gradient::Color::new(128, 0, 255, 255)       // Violet
            } else if temp_k < 220.0 {
                gradient::Color::new(0, 0, 255, 255)         // Blue
            } else if temp_k < 228.0 {
                gradient::Color::new(0, 128, 255, 255)       // Cyan-blue
            } else if temp_k < 235.0 {
                gradient::Color::new(0, 255, 255, 255)       // Cyan
            } else if temp_k < 243.0 {
                gradient::Color::new(0, 255, 128, 255)       // Cyan-green
            } else if temp_k < 250.0 {
                gradient::Color::new(0, 255, 0, 255)         // Green
            } else if temp_k < 258.0 {
                gradient::Color::new(255, 255, 0, 255)       // Yellow
            } else if temp_k < 265.0 {
                gradient::Color::new(255, 200, 0, 255)       // Gold
            } else if temp_k < 273.0 {
                gradient::Color::new(255, 150, 0, 255)       // Orange
            } else if temp_k < 283.0 {
                gradient::Color::new(255, 100, 0, 255)       // Dark orange
            } else if temp_k < 293.0 {
                gradient::Color::new(255, 50, 0, 255)        // Red-orange
            } else if temp_k < 303.0 {
                gradient::Color::new(255, 0, 0, 255)         // Red
            } else if temp_k < 313.0 {
                gradient::Color::new(200, 0, 0, 255)         // Dark red
            } else if temp_k < 323.0 {
                gradient::Color::new(150, 0, 0, 255)         // Maroon
            } else {
                gradient::Color::new(100, 50, 50, 255)       // Warm surface - brownish
            }
        },
    )
}

/// Resample from global geographic grid to a specific bbox and output size
/// Resample from geographic grid to a specific bbox and output size
/// This ensures consistent sampling across tile boundaries
/// 
/// Supports both global and regional datasets by respecting data_bounds.
/// Pixels outside data_bounds are set to NaN for transparent rendering.
/// 
/// For Web Mercator output, use `resample_for_mercator` instead.
fn resample_from_geographic(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
    data_bounds: [f32; 4],
    grid_uses_360: bool,
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;
    let [data_min_lon, data_min_lat, data_max_lon, data_max_lat] = data_bounds;
    
    // Use the explicit grid_uses_360 flag rather than inferring from data_bounds.
    // This is important because partial reads may return data with bbox that doesn't
    // span > 180, but the underlying grid still uses 0-360 convention.
    let data_uses_360 = grid_uses_360;
    
    // Data grid resolution
    let data_lon_range = data_max_lon - data_min_lon;
    let data_lat_range = data_max_lat - data_min_lat;
    
    let mut output = vec![f32::NAN; output_width * output_height];
    
    // For each output pixel, calculate its geographic position and sample from data grid
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Calculate geographic coordinates of this output pixel (pixel center)
            let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
            let y_ratio = (out_y as f32 + 0.5) / output_height as f32;
            
            let lon = out_min_lon + x_ratio * (out_max_lon - out_min_lon);
            let lat = out_max_lat - y_ratio * (out_max_lat - out_min_lat); // Y is inverted
            
            // Check if this is a global grid (covers nearly 360 degrees)
            let is_global_grid = data_uses_360 && (data_max_lon - data_min_lon) > 359.0;
            
            // Normalize longitude for data grids that use 0-360 convention
            // For tiles that span negative longitudes, we need to add 360 to map them
            // to the 0-360 grid.
            let norm_lon = if data_uses_360 && lon < 0.0 {
                lon + 360.0
            } else if data_uses_360 && lon >= 0.0 && lon < 1.0 && data_min_lon > 180.0 {
                // Special case: data is from the "wrapped" region (e.g., 343-360)
                // and we're at lon near 0, which should map to near 360
                lon + 360.0
            } else {
                lon
            };
            
            // For global grids, handle the gap between grid end (e.g., 359.75°) and 360°
            // by treating longitudes in this gap as valid and wrapping interpolation
            let in_wrap_gap = is_global_grid && norm_lon > data_max_lon && norm_lon < 360.0;
            
            // Check if this pixel is within data bounds (with special handling for wrap gap)
            if !in_wrap_gap {
                if norm_lon < data_min_lon || norm_lon > data_max_lon || lat < data_min_lat || lat > data_max_lat {
                    // Outside data coverage - leave as NaN for transparent rendering
                    continue;
                }
            } else {
                // In wrap gap - only check latitude bounds
                if lat < data_min_lat || lat > data_max_lat {
                    continue;
                }
            }
            
            // Convert to data grid coordinates (continuous, not indices)
            // For the wrap gap, calculate position relative to the last grid cell
            let grid_x = if in_wrap_gap {
                // In the gap between last grid point and 360°
                // Map to position past the last column, interpolation will wrap to column 0
                let gap_start = data_max_lon;
                let gap_size = 360.0 - data_max_lon + data_min_lon; // Gap wraps around
                let pos_in_gap = norm_lon - gap_start;
                (data_width as f32 - 1.0) + (pos_in_gap / gap_size) as f32
            } else {
                (norm_lon - data_min_lon) / data_lon_range * data_width as f32
            };
            let grid_y = (data_max_lat - lat) / data_lat_range * data_height as f32;
            
            // Bilinear interpolation from data grid
            let x1 = grid_x.floor() as usize;
            let y1 = grid_y.floor() as usize;
            
            // For global grids (0-360), wrap x2 around instead of clamping
            // This ensures smooth interpolation across the prime meridian
            let x2 = if is_global_grid && x1 + 1 >= data_width {
                0 // Wrap to column 0
            } else {
                (x1 + 1).min(data_width - 1)
            };
            let y2 = (y1 + 1).min(data_height - 1);
            
            // Bounds check
            if x1 >= data_width || y1 >= data_height {
                continue;
            }
            
            let dx = grid_x - x1 as f32;
            let dy = grid_y - y1 as f32;
            
            // Sample four surrounding grid points
            let v11 = data.get(y1 * data_width + x1).copied().unwrap_or(f32::NAN);
            let v21 = data.get(y1 * data_width + x2).copied().unwrap_or(f32::NAN);
            let v12 = data.get(y2 * data_width + x1).copied().unwrap_or(f32::NAN);
            let v22 = data.get(y2 * data_width + x2).copied().unwrap_or(f32::NAN);
            
            // Skip interpolation if any corner is NaN
            if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
                continue;
            }
            
            // Bilinear interpolation
            let v1 = v11 * (1.0 - dx) + v21 * dx;
            let v2 = v12 * (1.0 - dx) + v22 * dx;
            let value = v1 * (1.0 - dy) + v2 * dy;
            
            output[out_y * output_width + out_x] = value;
        }
    }
    
    output
}

/// Resample from geographic grid for Web Mercator (EPSG:3857) output
/// 
/// In Web Mercator, the Y axis has non-linear latitude spacing. This function
/// accounts for that by converting pixel Y positions to Mercator Y, then to latitude.
/// 
/// Supports both global and regional datasets by respecting data_bounds.
/// Pixels outside data_bounds are set to NaN for transparent rendering.
fn resample_for_mercator(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],  // [min_lon, min_lat, max_lon, max_lat] in WGS84
    data_bounds: [f32; 4],
    grid_uses_360: bool,
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;
    let [data_min_lon, data_min_lat, data_max_lon, data_max_lat] = data_bounds;
    
    // Use the explicit grid_uses_360 flag rather than inferring from data_bounds.
    // This is important because partial reads may return data with bbox that doesn't
    // span > 180, but the underlying grid still uses 0-360 convention.
    let data_uses_360 = grid_uses_360;
    
    // Convert lat bounds to Mercator Y coordinates
    let min_merc_y = lat_to_mercator_y(out_min_lat as f64);
    let max_merc_y = lat_to_mercator_y(out_max_lat as f64);
    
    // Data grid resolution
    let data_lon_range = data_max_lon - data_min_lon;
    let data_lat_range = data_max_lat - data_min_lat;
    
    let mut output = vec![f32::NAN; output_width * output_height];
    
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Calculate position in output image (pixel center)
            let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
            let y_ratio = (out_y as f32 + 0.5) / output_height as f32;
            
            // Longitude is linear
            let lon = out_min_lon + x_ratio * (out_max_lon - out_min_lon);
            
            // Y position is in Mercator space, need to convert to latitude
            // y_ratio 0 = top = max_merc_y, y_ratio 1 = bottom = min_merc_y
            let merc_y = max_merc_y - y_ratio as f64 * (max_merc_y - min_merc_y);
            let lat = mercator_y_to_lat(merc_y) as f32;
            
            // Check if this is a global grid (covers nearly 360 degrees)
            let is_global_grid = data_uses_360 && (data_max_lon - data_min_lon) > 359.0;
            
            // Normalize longitude for data grids that use 0-360 convention
            // For tiles that span negative longitudes, we need to add 360 to map them
            // to the 0-360 grid.
            let norm_lon = if data_uses_360 && lon < 0.0 {
                lon + 360.0
            } else if data_uses_360 && lon >= 0.0 && lon < 1.0 && data_min_lon > 180.0 {
                // Special case: data is from the "wrapped" region (e.g., 343-360)
                // and we're at lon near 0, which should map to near 360
                lon + 360.0
            } else {
                lon
            };
            
            // For global grids, handle the gap between grid end (e.g., 359.75°) and 360°
            // by treating longitudes in this gap as valid and wrapping interpolation
            let in_wrap_gap = is_global_grid && norm_lon > data_max_lon && norm_lon < 360.0;
            
            // Check if this pixel is within data bounds (with special handling for wrap gap)
            if !in_wrap_gap {
                if norm_lon < data_min_lon || norm_lon > data_max_lon || lat < data_min_lat || lat > data_max_lat {
                    // Outside data coverage - leave as NaN for transparent rendering
                    continue;
                }
            } else {
                // In wrap gap - only check latitude bounds
                if lat < data_min_lat || lat > data_max_lat {
                    continue;
                }
            }
            
            // Convert to data grid coordinates
            // For the wrap gap, calculate position relative to the last grid cell
            let grid_x = if in_wrap_gap {
                // In the gap between last grid point and 360°
                // Map to position past the last column, interpolation will wrap to column 0
                let gap_start = data_max_lon;
                let gap_size = 360.0 - data_max_lon + data_min_lon; // Gap wraps around
                let pos_in_gap = norm_lon - gap_start;
                (data_width as f32 - 1.0) + (pos_in_gap / gap_size) as f32
            } else {
                (norm_lon - data_min_lon) / data_lon_range * data_width as f32
            };
            let grid_y = (data_max_lat - lat) / data_lat_range * data_height as f32;
            
            // Bilinear interpolation
            let x1 = grid_x.floor() as usize;
            let y1 = grid_y.floor() as usize;
            
            // For global grids (0-360), wrap x2 around instead of clamping
            // This ensures smooth interpolation across the prime meridian
            let x2 = if is_global_grid && x1 + 1 >= data_width {
                0 // Wrap to column 0
            } else {
                (x1 + 1).min(data_width - 1)
            };
            let y2 = (y1 + 1).min(data_height - 1);
            
            // Bounds check
            if x1 >= data_width || y1 >= data_height {
                continue;
            }
            
            let dx = grid_x - x1 as f32;
            let dy = grid_y - y1 as f32;
            
            let v11 = data.get(y1 * data_width + x1).copied().unwrap_or(f32::NAN);
            let v21 = data.get(y1 * data_width + x2).copied().unwrap_or(f32::NAN);
            let v12 = data.get(y2 * data_width + x1).copied().unwrap_or(f32::NAN);
            let v22 = data.get(y2 * data_width + x2).copied().unwrap_or(f32::NAN);
            
            // Skip interpolation if any corner is NaN
            if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
                continue;
            }
            
            let v1 = v11 * (1.0 - dx) + v21 * dx;
            let v2 = v12 * (1.0 - dx) + v22 * dx;
            let value = v1 * (1.0 - dy) + v2 * dy;
            
            output[out_y * output_width + out_x] = value;
        }
    }
    
    output
}

/// Convert latitude to Web Mercator Y coordinate
fn lat_to_mercator_y(lat: f64) -> f64 {
    let lat_rad = lat.to_radians();
    let y = ((std::f64::consts::PI / 4.0) + (lat_rad / 2.0)).tan().ln();
    y * 6378137.0  // Earth radius in meters
}

/// Convert Web Mercator Y coordinate to latitude
fn mercator_y_to_lat(y: f64) -> f64 {
    let y_normalized = y / 6378137.0;  // Normalize by Earth radius
    (2.0 * y_normalized.exp().atan() - std::f64::consts::PI / 2.0).to_degrees()
}

/// Resample grid data for a given bbox, with model-aware projection handling
fn resample_grid_for_bbox(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
    data_bounds: [f32; 4],
    use_mercator: bool,
    model: &str,
    grid_uses_360: bool,
) -> Vec<f32> {
    resample_grid_for_bbox_with_proj(
        data, data_width, data_height, output_width, output_height,
        output_bbox, data_bounds, use_mercator, model, None, grid_uses_360
    )
}

/// Resample grid data for a given bbox, with optional GOES projection parameters
fn resample_grid_for_bbox_with_proj(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
    data_bounds: [f32; 4],
    use_mercator: bool,
    model: &str,
    goes_projection: Option<&GoesProjectionParams>,
    grid_uses_360: bool,
) -> Vec<f32> {
    // Use Lambert Conformal resampling for HRRR (native projection)
    if model == "hrrr" {
        if use_mercator {
            resample_lambert_to_mercator(data, data_width, data_height, output_width, output_height, output_bbox)
        } else {
            resample_lambert_to_geographic(data, data_width, data_height, output_width, output_height, output_bbox)
        }
    } else if model == "goes16" || model == "goes18" || model == "goes" {
        // Use Geostationary projection for GOES satellite data
        // Prefer dynamic projection parameters if available
        if let Some(params) = goes_projection {
            let proj = Geostationary::from_goes(
                params.perspective_point_height,
                params.semi_major_axis,
                params.semi_minor_axis,
                params.longitude_origin,
                params.x_origin,
                params.y_origin,
                params.dx,
                params.dy,
                data_width,
                data_height,
            );
            if use_mercator {
                resample_geostationary_to_mercator_with_proj(data, data_width, data_height, output_width, output_height, output_bbox, &proj)
            } else {
                resample_geostationary_to_geographic_with_proj(data, data_width, data_height, output_width, output_height, output_bbox, &proj)
            }
        } else {
            // Fallback to preset projections
            let satellite_lon = if model == "goes18" { -137.2 } else { -75.0 };
            if use_mercator {
                resample_geostationary_to_mercator(data, data_width, data_height, output_width, output_height, output_bbox, satellite_lon)
            } else {
                resample_geostationary_to_geographic(data, data_width, data_height, output_width, output_height, output_bbox, satellite_lon)
            }
        }
    } else {
        // GFS and other models use geographic (lat/lon) grids
        if use_mercator {
            resample_for_mercator(data, data_width, data_height, output_width, output_height, output_bbox, data_bounds, grid_uses_360)
        } else {
            resample_from_geographic(data, data_width, data_height, output_width, output_height, output_bbox, data_bounds, grid_uses_360)
        }
    }
}

/// Try to resample GOES data using a pre-computed LUT.
/// 
/// Returns Some(resampled_data) if a LUT is available for this tile,
/// or None if we should fall back to computing the projection.
/// 
/// NOTE: LUT is currently disabled due to a mismatch between hardcoded projection
/// parameters in the pre-computed LUTs and the actual dynamic parameters from
/// NetCDF files. The hardcoded values (x_origin: -0.101360, y_origin: 0.128226)
/// differ from actual file values (x_offset: -0.101353, y_offset: 0.128233),
/// causing visible pixel misalignment between zoom levels 0-7 (LUT) and 8+ (on-the-fly).
/// 
/// TODO: Regenerate LUTs using actual projection parameters from NetCDF files,
/// or implement on-demand LUT generation with caching per unique projection params.
fn try_resample_with_lut(
    _model: &str,
    _tile_coords: Option<(u32, u32, u32)>,
    _projection_luts: Option<&ProjectionLuts>,
    _data: &[f32],
    _data_width: usize,
) -> Option<Vec<f32>> {
    // DISABLED: LUT projection parameters don't match actual NetCDF file parameters,
    // causing pixel misalignment at zoom level boundaries. Using on-the-fly projection
    // computation ensures consistent results across all zoom levels.
    // 
    // The mismatch is:
    // - LUT hardcoded:  x_origin=-0.101360, y_origin=0.128226, dx=0.000028
    // - NetCDF actual:  x_offset=-0.101353, y_offset=0.128233, x_scale=1.4e-05
    //
    // To re-enable, the LUT generation must use the exact same projection parameters
    // as the NetCDF files being rendered.
    None
    
    // Original code (disabled):
    // // Only applicable for GOES models
    // if model != "goes16" && model != "goes18" && model != "goes" {
    //     return None;
    // }
    // 
    // // Need both tile coords and LUT cache
    // let (z, x, y) = tile_coords?;
    // let luts = projection_luts?;
    // 
    // // Get the LUT cache for this satellite
    // let cache = luts.get(model)?;
    // 
    // // Check if we have a pre-computed LUT for this tile
    // let lut = cache.get(z, x, y)?;
    // 
    // // Use the LUT for fast resampling
    // Some(resample_with_lut(data, data_width, lut))
}

/// Resample from Lambert Conformal grid (HRRR) to geographic output
/// 
/// This handles the projection transformation from HRRR's native Lambert Conformal
/// grid to a regular lat/lon grid for WMS output.
fn resample_lambert_to_geographic(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;
    
    // Create HRRR projection
    let proj = LambertConformal::hrrr();
    
    let mut output = vec![f32::NAN; output_width * output_height];
    
    // For each output pixel, find the corresponding grid point in the Lambert grid
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Calculate geographic coordinates of this output pixel (pixel center)
            let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
            let y_ratio = (out_y as f32 + 0.5) / output_height as f32;
            
            let lon = out_min_lon + x_ratio * (out_max_lon - out_min_lon);
            let lat = out_max_lat - y_ratio * (out_max_lat - out_min_lat); // Y is inverted
            
            // Convert geographic to Lambert grid indices
            let (grid_i, grid_j) = proj.geo_to_grid(lat as f64, lon as f64);
            
            // Check if within grid bounds
            if grid_i < 0.0 || grid_i >= data_width as f64 - 1.0 ||
               grid_j < 0.0 || grid_j >= data_height as f64 - 1.0 {
                // Outside HRRR coverage - leave as NaN
                continue;
            }
            
            // Bilinear interpolation
            let i1 = grid_i.floor() as usize;
            let j1 = grid_j.floor() as usize;
            let i2 = (i1 + 1).min(data_width - 1);
            let j2 = (j1 + 1).min(data_height - 1);
            
            let di = grid_i - i1 as f64;
            let dj = grid_j - j1 as f64;
            
            // Sample four surrounding grid points
            let v11 = data.get(j1 * data_width + i1).copied().unwrap_or(f32::NAN);
            let v21 = data.get(j1 * data_width + i2).copied().unwrap_or(f32::NAN);
            let v12 = data.get(j2 * data_width + i1).copied().unwrap_or(f32::NAN);
            let v22 = data.get(j2 * data_width + i2).copied().unwrap_or(f32::NAN);
            
            // Skip if any corner is NaN
            if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
                continue;
            }
            
            // Bilinear interpolation
            let di = di as f32;
            let dj = dj as f32;
            let v1 = v11 * (1.0 - di) + v21 * di;
            let v2 = v12 * (1.0 - di) + v22 * di;
            let value = v1 * (1.0 - dj) + v2 * dj;
            
            output[out_y * output_width + out_x] = value;
        }
    }
    
    output
}

/// Resample from Lambert Conformal grid (HRRR) to Web Mercator output
/// 
/// This handles the projection transformation from HRRR's native Lambert Conformal
/// grid to Web Mercator (EPSG:3857) for WMTS tiles.
/// 
/// Note: output_bbox is in WGS84 degrees [min_lon, min_lat, max_lon, max_lat],
/// but the output Y-axis uses Mercator (non-linear latitude) spacing.
fn resample_lambert_to_mercator(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;
    
    // Create HRRR projection
    let proj = LambertConformal::hrrr();
    
    // Convert lat bounds to Mercator Y coordinates for proper Y-axis spacing
    let min_merc_y = lat_to_mercator_y(out_min_lat as f64);
    let max_merc_y = lat_to_mercator_y(out_max_lat as f64);
    
    let mut output = vec![f32::NAN; output_width * output_height];
    
    // For each output pixel
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Calculate position in output image (pixel center)
            let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
            let y_ratio = (out_y as f32 + 0.5) / output_height as f32;
            
            // Longitude is linear in degrees
            let lon = out_min_lon + x_ratio * (out_max_lon - out_min_lon);
            
            // Y position uses Mercator spacing, then convert back to latitude
            // y_ratio 0 = top = max_merc_y, y_ratio 1 = bottom = min_merc_y
            let merc_y = max_merc_y - y_ratio as f64 * (max_merc_y - min_merc_y);
            let lat = mercator_y_to_lat(merc_y);
            
            // Convert geographic to Lambert grid indices
            let (grid_i, grid_j) = proj.geo_to_grid(lat, lon as f64);
            
            // Check if within grid bounds
            if grid_i < 0.0 || grid_i >= data_width as f64 - 1.0 ||
               grid_j < 0.0 || grid_j >= data_height as f64 - 1.0 {
                // Outside HRRR coverage - leave as NaN
                continue;
            }
            
            // Bilinear interpolation
            let i1 = grid_i.floor() as usize;
            let j1 = grid_j.floor() as usize;
            let i2 = (i1 + 1).min(data_width - 1);
            let j2 = (j1 + 1).min(data_height - 1);
            
            let di = grid_i - i1 as f64;
            let dj = grid_j - j1 as f64;
            
            // Sample four surrounding grid points
            let v11 = data.get(j1 * data_width + i1).copied().unwrap_or(f32::NAN);
            let v21 = data.get(j1 * data_width + i2).copied().unwrap_or(f32::NAN);
            let v12 = data.get(j2 * data_width + i1).copied().unwrap_or(f32::NAN);
            let v22 = data.get(j2 * data_width + i2).copied().unwrap_or(f32::NAN);
            
            // Skip if any corner is NaN
            if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
                continue;
            }
            
            // Bilinear interpolation
            let di = di as f32;
            let dj = dj as f32;
            let v1 = v11 * (1.0 - di) + v21 * di;
            let v2 = v12 * (1.0 - di) + v22 * di;
            let value = v1 * (1.0 - dj) + v2 * dj;
            
            output[out_y * output_width + out_x] = value;
        }
    }
    
    output
}

/// Resample from Geostationary grid (GOES) to geographic output
/// 
/// This handles the projection transformation from GOES native geostationary
/// grid to a regular lat/lon grid for WMS output.
fn resample_geostationary_to_geographic(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
    satellite_lon: f64,
) -> Vec<f32> {
    // Create GOES projection based on satellite position (fallback if no dynamic projection)
    let proj = if satellite_lon < -100.0 {
        Geostationary::goes18_conus()
    } else {
        Geostationary::goes16_conus()
    };
    resample_geostationary_to_geographic_with_proj(data, data_width, data_height, output_width, output_height, output_bbox, &proj)
}

/// Resample from Geostationary grid (GOES) to geographic output with custom projection
fn resample_geostationary_to_geographic_with_proj(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
    proj: &Geostationary,
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;
    
    let mut output = vec![f32::NAN; output_width * output_height];
    
    // For each output pixel, find the corresponding grid point in the geostationary grid
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Calculate geographic coordinates of this output pixel (pixel center)
            let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
            let y_ratio = (out_y as f32 + 0.5) / output_height as f32;
            
            let lon = out_min_lon + x_ratio * (out_max_lon - out_min_lon);
            let lat = out_max_lat - y_ratio * (out_max_lat - out_min_lat); // Y is inverted
            
            // Convert geographic to geostationary grid indices
            let grid_coords = proj.geo_to_grid(lat as f64, lon as f64);
            
            let (grid_i, grid_j) = match grid_coords {
                Some((i, j)) => (i, j),
                None => continue, // Point not visible from satellite
            };
            
            // Check if within grid bounds
            if grid_i < 0.0 || grid_i >= data_width as f64 - 1.0 ||
               grid_j < 0.0 || grid_j >= data_height as f64 - 1.0 {
                // Outside GOES coverage - leave as NaN
                continue;
            }
            
            // Bilinear interpolation
            let i1 = grid_i.floor() as usize;
            let j1 = grid_j.floor() as usize;
            let i2 = (i1 + 1).min(data_width - 1);
            let j2 = (j1 + 1).min(data_height - 1);
            
            let di = grid_i - i1 as f64;
            let dj = grid_j - j1 as f64;
            
            // Sample four surrounding grid points
            let v11 = data.get(j1 * data_width + i1).copied().unwrap_or(f32::NAN);
            let v21 = data.get(j1 * data_width + i2).copied().unwrap_or(f32::NAN);
            let v12 = data.get(j2 * data_width + i1).copied().unwrap_or(f32::NAN);
            let v22 = data.get(j2 * data_width + i2).copied().unwrap_or(f32::NAN);
            
            // Skip if any corner is NaN
            if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
                continue;
            }
            
            // Bilinear interpolation
            let di = di as f32;
            let dj = dj as f32;
            let v1 = v11 * (1.0 - di) + v21 * di;
            let v2 = v12 * (1.0 - di) + v22 * di;
            let value = v1 * (1.0 - dj) + v2 * dj;
            
            output[out_y * output_width + out_x] = value;
        }
    }
    
    output
}

/// Resample from Geostationary grid (GOES) to Web Mercator output
/// 
/// This handles the projection transformation from GOES native geostationary
/// grid to Web Mercator (EPSG:3857) for WMTS tiles.
fn resample_geostationary_to_mercator(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
    satellite_lon: f64,
) -> Vec<f32> {
    // Create GOES projection based on satellite position (fallback if no dynamic projection)
    let proj = if satellite_lon < -100.0 {
        Geostationary::goes18_conus()
    } else {
        Geostationary::goes16_conus()
    };
    resample_geostationary_to_mercator_with_proj(data, data_width, data_height, output_width, output_height, output_bbox, &proj)
}

/// Resample from Geostationary grid (GOES) to Web Mercator output with custom projection
/// 
/// Note: output_bbox is in WGS84 degrees [min_lon, min_lat, max_lon, max_lat],
/// but the output Y-axis uses Mercator (non-linear latitude) spacing.
fn resample_geostationary_to_mercator_with_proj(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
    proj: &Geostationary,
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;
    
    // Convert lat bounds to Mercator Y coordinates for proper Y-axis spacing
    let min_merc_y = lat_to_mercator_y(out_min_lat as f64);
    let max_merc_y = lat_to_mercator_y(out_max_lat as f64);
    
    let mut output = vec![f32::NAN; output_width * output_height];
    
    // For each output pixel
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Calculate position in output image (pixel center)
            let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
            let y_ratio = (out_y as f32 + 0.5) / output_height as f32;
            
            // Longitude is linear in degrees
            let lon = out_min_lon + x_ratio * (out_max_lon - out_min_lon);
            
            // Y position uses Mercator spacing, then convert back to latitude
            // y_ratio 0 = top = max_merc_y, y_ratio 1 = bottom = min_merc_y
            let merc_y = max_merc_y - y_ratio as f64 * (max_merc_y - min_merc_y);
            let lat = mercator_y_to_lat(merc_y);
            
            // Convert geographic to geostationary grid indices
            let grid_coords = proj.geo_to_grid(lat, lon as f64);
            
            let (grid_i, grid_j) = match grid_coords {
                Some((i, j)) => (i, j),
                None => continue, // Point not visible from satellite
            };
            
            // Check if within grid bounds
            if grid_i < 0.0 || grid_i >= data_width as f64 - 1.0 ||
               grid_j < 0.0 || grid_j >= data_height as f64 - 1.0 {
                // Outside GOES coverage - leave as NaN
                continue;
            }
            
            // Bilinear interpolation
            let i1 = grid_i.floor() as usize;
            let j1 = grid_j.floor() as usize;
            let i2 = (i1 + 1).min(data_width - 1);
            let j2 = (j1 + 1).min(data_height - 1);
            
            let di = grid_i - i1 as f64;
            let dj = grid_j - j1 as f64;
            
            // Sample four surrounding grid points
            let v11 = data.get(j1 * data_width + i1).copied().unwrap_or(f32::NAN);
            let v21 = data.get(j1 * data_width + i2).copied().unwrap_or(f32::NAN);
            let v12 = data.get(j2 * data_width + i1).copied().unwrap_or(f32::NAN);
            let v22 = data.get(j2 * data_width + i2).copied().unwrap_or(f32::NAN);
            
            // Skip if any corner is NaN
            if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
                continue;
            }
            
            // Bilinear interpolation
            let di = di as f32;
            let dj = dj as f32;
            let v1 = v11 * (1.0 - di) + v21 * di;
            let v2 = v12 * (1.0 - di) + v22 * di;
            let value = v1 * (1.0 - dj) + v2 * dj;
            
            output[out_y * output_width + out_x] = value;
        }
    }
    
    output
}

/// Model-aware resampling for geographic output (used for wind barbs and other non-Mercator rendering)
/// 
/// This wraps the projection-specific resampling functions to handle different grid types.
fn resample_for_model_geographic(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
    data_bounds: [f32; 4],
    model: &str,
    grid_uses_360: bool,
) -> Vec<f32> {
    if model == "hrrr" {
        resample_lambert_to_geographic(data, data_width, data_height, output_width, output_height, output_bbox)
    } else if model == "goes16" || model == "goes18" || model == "goes" {
        let satellite_lon = if model == "goes18" { -137.2 } else { -75.0 };
        resample_geostationary_to_geographic(data, data_width, data_height, output_width, output_height, output_bbox, satellite_lon)
    } else {
        resample_from_geographic(data, data_width, data_height, output_width, output_height, output_bbox, data_bounds, grid_uses_360)
    }
}

/// Convert HSV to RGB (simplified version)
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Find a specific parameter in a GRIB2 file, optionally matching by level
///
/// GRIB files can contain multiple parameters at different levels. This function searches
/// through all messages to find the one matching the requested parameter and level.
/// If level is None, returns the first matching parameter.
fn find_parameter_in_grib(grib_data: bytes::Bytes, parameter: &str, level: Option<&str>) -> Result<grib2_parser::Grib2Message, String> {
    let mut reader = grib2_parser::Grib2Reader::new(grib_data);
    let mut first_param_match: Option<grib2_parser::Grib2Message> = None;
    
    while let Some(msg) = reader
        .next_message()
        .map_err(|e| format!("GRIB2 parse error: {}", e))?
    {
        let msg_param = msg.parameter();
        
        info!(
            found_param = msg_param,
            wanted_param = parameter,
            discipline = msg.indicator.discipline,
            category = msg.product_definition.parameter_category,
            number = msg.product_definition.parameter_number,
            "Checking GRIB2 message parameter"
        );
        
        // Check parameter match (exact or MRMS fallback)
        // MRMS files lose discipline 209 during shredding, need fallback matching
        let param_matches = msg_param == parameter || match parameter {
            // MRMS fallback mappings (discipline 209 → discipline 0 after shredding)
            "REFL" => msg_param == "P0_9_0",           // MergedReflectivityQC
            "PRECIP_RATE" => msg_param == "P0_6_1",    // PrecipRate
            "QPE" => msg_param == "P0_1_8",            // Quantitative Precipitation Estimate
            _ => false,
        };
        
        if param_matches {
            // If level is specified, check it matches
            if let Some(target_level) = level {
                let msg_level = msg.product_definition.level_description.as_str();
                if msg_level == target_level {
                    info!(
                        param = parameter,
                        level = msg_level,
                        "Found exact parameter+level match in GRIB"
                    );
                    return Ok(msg);
                }
                // Save first parameter match as fallback
                if first_param_match.is_none() {
                    info!(
                        param = parameter,
                        found_level = msg_level,
                        wanted_level = target_level,
                        "First param match (wrong level), saving as fallback"
                    );
                    first_param_match = Some(msg);
                }
            } else {
                // No level specified, return first parameter match
                return Ok(msg);
            }
        }
    }
    
    // Return first parameter match if no exact level match found
    if let Some(msg) = first_param_match {
        info!(
            param = parameter,
            requested_level = level,
            "No exact level match, using first parameter match"
        );
        return Ok(msg);
    }
    
    Err(format!("Parameter {} not found in GRIB2 file", parameter))
}

/// Grid data extracted from either GRIB2, NetCDF, or Zarr files
pub(crate) struct GridData {
    pub data: Vec<f32>,
    pub width: usize,
    pub height: usize,
    /// Actual bounding box of the returned data (may be subset of full grid)
    pub bbox: Option<[f32; 4]>,
    /// For GOES data: dynamic geostationary projection parameters
    pub goes_projection: Option<GoesProjectionParams>,
    /// Whether the underlying grid uses 0-360 longitude convention (like GFS).
    /// This is needed for proper coordinate handling even when bbox is a partial region.
    pub grid_uses_360: bool,
}

/// Dynamic GOES projection parameters extracted from NetCDF file
#[derive(Clone, Debug)]
pub(crate) struct GoesProjectionParams {
    x_origin: f64,
    y_origin: f64,
    dx: f64,
    dy: f64,
    perspective_point_height: f64,
    semi_major_axis: f64,
    semi_minor_axis: f64,
    longitude_origin: f64,
}

/// Load grid data from a storage path, handling GRIB2, NetCDF, and Zarr files.
/// 
/// This function automatically routes to the appropriate loader:
/// - If `zarr_metadata` is present in the entry -> use Zarr loader (efficient chunked reads)
/// - If file is NetCDF (.nc or GOES) -> use NetCDF loader
/// - Otherwise -> use GRIB2 loader
/// 
/// When `cache_grib2` is true and `grid_cache` is provided, parsed GRIB2 grids
/// will also be cached to avoid redundant decompression for adjacent tiles.
async fn load_grid_data(
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    metrics: &MetricsCollector,
    entry: &CatalogEntry,
    parameter: &str,
) -> Result<GridData, String> {
    load_grid_data_with_options(grib_cache, grid_cache, metrics, entry, parameter, true).await
}

/// Load grid data with support for Zarr format when zarr_metadata is available.
///
/// This is the preferred method for loading grid data as it automatically
/// uses efficient chunked reads for Zarr-format data.
///
/// # Arguments
/// * `factory` - Optional GridProcessorFactory for Zarr data access
/// * `grib_cache` - GRIB cache for legacy format fallback
/// * `grid_cache` - Grid data cache for parsed data
/// * `metrics` - Metrics collector
/// * `entry` - Catalog entry
/// * `parameter` - Parameter name
/// * `bbox` - Optional bounding box (for Zarr partial reads)
pub async fn load_grid_data_with_zarr_support(
    factory: Option<&GridProcessorFactory>,
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    metrics: &MetricsCollector,
    entry: &CatalogEntry,
    parameter: &str,
    bbox: Option<[f32; 4]>,
) -> Result<GridData, String> {
    // Determine loader type for logging
    let loader_type = if entry.zarr_metadata.is_some() && factory.is_some() {
        "zarr"
    } else if entry.storage_path.ends_with(".nc") || entry.parameter.starts_with("CMI_") {
        "netcdf"
    } else {
        "grib2"
    };
    
    debug!(
        model = %entry.model,
        parameter = %entry.parameter,
        level = %entry.level,
        storage_path = %entry.storage_path,
        loader = loader_type,
        has_zarr_meta = entry.zarr_metadata.is_some(),
        "Loading grid data"
    );
    
    // Check if this entry has Zarr metadata and we have a factory
    if entry.zarr_metadata.is_some() {
        if let Some(factory) = factory {
            info!(
                model = %entry.model,
                parameter = %entry.parameter,
                storage_path = %entry.storage_path,
                loader = "zarr",
                "Using Zarr loader for grid data"
            );
            return load_grid_data_from_zarr(factory, entry, bbox).await;
        } else {
            warn!(
                storage_path = %entry.storage_path,
                "Entry has zarr_metadata but no GridProcessorFactory available, falling back to legacy loader"
            );
        }
    }
    
    // Fall back to legacy loaders
    debug!(
        model = %entry.model,
        parameter = %entry.parameter,
        loader = loader_type,
        "Using legacy loader for grid data"
    );
    load_grid_data_with_options(grib_cache, grid_cache, metrics, entry, parameter, true).await
}

/// Load grid data with explicit control over GRIB2 caching
async fn load_grid_data_with_options(
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    metrics: &MetricsCollector,
    entry: &CatalogEntry,
    parameter: &str,
    cache_grib2: bool,
) -> Result<GridData, String> {
    use std::sync::Arc;
    
    // Check file type by extension or magic bytes
    let is_netcdf = entry.storage_path.ends_with(".nc") || 
                    entry.parameter.starts_with("CMI_") ||
                    entry.model.starts_with("goes");
    
    // Determine data source type for per-layer metrics
    let source_type = DataSourceType::from_model(&entry.model);
    
    if is_netcdf {
        // Handle NetCDF (GOES) data - use grid cache if available
        load_netcdf_grid_data(grib_cache, grid_cache, metrics, entry, &source_type).await
    } else {
        // Handle GRIB2 data
        // Build cache key: storage_path + level (to handle multi-level files)
        let cache_key = format!("{}:{}", entry.storage_path, entry.level);
        
        // Check grid cache first if GRIB2 caching is enabled
        if cache_grib2 {
            if let Some(cache) = grid_cache {
                if let Some(cached) = cache.get(&cache_key).await {
                    info!(
                        storage_path = %entry.storage_path,
                        level = %entry.level,
                        "Grid cache HIT for GRIB2 data"
                    );
                    // Record cache hit for this data source
                    metrics.record_data_source_cache_hit(&source_type).await;
                    
                    // Record model-specific grid cache hit
                    if let Some(wm) = crate::metrics::WeatherModel::from_model(&entry.model) {
                        metrics.record_model_grid_cache_hit(wm);
                    }
                    
                    // Check if grid uses 0-360 longitude (like GFS)
                    let grid_uses_360 = entry.bbox.min_x >= 0.0 && entry.bbox.max_x > 180.0;
                    return Ok(GridData {
                        data: (*cached.data).clone(),
                        width: cached.width,
                        height: cached.height,
                        bbox: None, // GRIB2 cached data uses full grid bbox from entry
                        goes_projection: None, // GRIB2 doesn't use GOES projection
                        grid_uses_360,
                    });
                }
            }
        }
        
        // Cache miss - record grid cache miss for model
        if let Some(wm) = crate::metrics::WeatherModel::from_model(&entry.model) {
            metrics.record_model_grid_cache_miss(wm);
        }
        
        // Load GRIB2 file from cache/storage with hit/miss status
        let cache_result = grib_cache
            .get_with_status(&entry.storage_path)
            .await
            .map_err(|e| format!("Failed to load file: {}", e))?;
        
        // Record model-specific file cache hit/miss
        if let Some(wm) = crate::metrics::WeatherModel::from_model(&entry.model) {
            if cache_result.was_cache_hit {
                metrics.record_model_cache_hit(wm);
            } else {
                metrics.record_model_cache_miss(wm);
            }
        }
        
        let file_data = cache_result.data;
        
        // For MRMS (shredded single-message files), just read the first message
        // since each file contains only one parameter
        let is_shredded = entry.storage_path.contains("shredded/") || entry.model == "mrms";
        
        let start = Instant::now();
        let msg = {
            if is_shredded {
                // Read first (and only) message from shredded file
                let mut reader = grib2_parser::Grib2Reader::new(file_data);
                reader.next_message()
                    .map_err(|e| format!("GRIB2 parse error: {}", e))?
                    .ok_or_else(|| "No message found in GRIB2 file".to_string())?
            } else {
                // Search for specific parameter in multi-message file
                find_parameter_in_grib(file_data, parameter, Some(&entry.level))?
            }
        };
        
        let grid_data = msg
            .unpack_data()
            .map_err(|e| format!("Unpacking failed: {}", e))?;
        
        let parse_duration = start.elapsed();
        let parse_us = parse_duration.as_micros() as u64;
        
        // Record both global and per-source parse times
        metrics.record_grib_parse(parse_us).await;
        metrics.record_data_source_parse(&source_type, parse_us).await;
        
        let (grid_height, grid_width) = msg.grid_dims();
        let grid_points = (grid_width as u64) * (grid_height as u64);
        
        // Record model-specific parse time and grid size
        if let Some(wm) = crate::metrics::WeatherModel::from_model(&entry.model) {
            metrics.record_model_parse(wm, parse_us, grid_points);
        }
        
        // Store in grid cache if GRIB2 caching is enabled
        if cache_grib2 {
            if let Some(cache) = grid_cache {
                let cached_data = CachedGridData {
                    data: Arc::new(grid_data.clone()),
                    width: grid_width as usize,
                    height: grid_height as usize,
                    goes_projection: None,
                };
                cache.insert(cache_key.clone(), cached_data).await;
                info!(
                    storage_path = %entry.storage_path,
                    level = %entry.level,
                    width = grid_width,
                    height = grid_height,
                    parse_ms = parse_us as f64 / 1000.0,
                    "Cached parsed GRIB2 grid data"
                );
            }
        }
        
        // Check if grid uses 0-360 longitude (like GFS)
        let grid_uses_360 = entry.bbox.min_x >= 0.0 && entry.bbox.max_x > 180.0;
        Ok(GridData {
            data: grid_data,
            width: grid_width as usize,
            height: grid_height as usize,
            bbox: None, // GRIB2 returns full grid, use entry.bbox
            goes_projection: None,
            grid_uses_360,
        })
    }
}

/// Load grid data from a Zarr file stored in MinIO.
///
/// This function uses the new GridProcessor abstraction for efficient partial reads
/// from Zarr V3 formatted data. It only loads the chunks needed for the requested
/// bounding box, significantly reducing data transfer for tile requests.
///
/// # Arguments
/// * `factory` - GridProcessorFactory containing MinIO client and shared cache
/// * `entry` - Catalog entry with zarr_metadata
/// * `bbox` - Optional bounding box to load (if None, loads full grid)
///
/// # Returns
/// GridData containing the grid values and dimensions
#[instrument(skip(factory, entry), fields(
    model = %entry.model,
    parameter = %entry.parameter,
    level = %entry.level,
    storage_path = %entry.storage_path,
))]
async fn load_grid_data_from_zarr(
    factory: &GridProcessorFactory,
    entry: &CatalogEntry,
    bbox: Option<[f32; 4]>,
) -> Result<GridData, String> {
    use grid_processor::{
        BoundingBox as GpBoundingBox,
        GridProcessor, ZarrGridProcessor, ZarrMetadata,
        MinioConfig, create_minio_storage,
    };

    // Parse zarr_metadata from catalog entry
    let zarr_json = entry.zarr_metadata.as_ref()
        .ok_or_else(|| {
            error!(model = %entry.model, parameter = %entry.parameter, "No zarr_metadata in catalog entry");
            "No zarr_metadata in catalog entry".to_string()
        })?;
    
    let zarr_meta = ZarrMetadata::from_json(zarr_json)
        .map_err(|e| {
            error!(model = %entry.model, parameter = %entry.parameter, error = %e, "Failed to parse zarr_metadata");
            format!("Failed to parse zarr_metadata: {}", e)
        })?;
    
    info!(
        storage_path = %entry.storage_path,
        shape = ?zarr_meta.shape,
        chunk_shape = ?zarr_meta.chunk_shape,
        "Loading grid data from Zarr"
    );
    
    // Build storage path - the storage_path in catalog points to the Zarr directory
    // e.g., "grids/gfs/20241212_00z/TMP_2m_f006.zarr"
    // zarrs expects paths to start with / for object_store backends
    let zarr_path = if entry.storage_path.starts_with('/') {
        entry.storage_path.clone()
    } else {
        format!("/{}", entry.storage_path)
    };
    
    // Create MinIO storage using the helper (uses correct object_store version)
    let minio_config = MinioConfig::from_env();
    let store = create_minio_storage(&minio_config)
        .map_err(|e| {
            error!(
                error = %e,
                endpoint = %minio_config.endpoint,
                bucket = %minio_config.bucket,
                "Failed to create MinIO storage"
            );
            format!("Failed to create MinIO storage: {}", e)
        })?;
    
    // Convert zarr_meta to GridMetadata for the processor
    let grid_metadata = grid_processor::GridMetadata {
        model: zarr_meta.model.clone(),
        parameter: zarr_meta.parameter.clone(),
        level: zarr_meta.level.clone(),
        units: zarr_meta.units.clone(),
        reference_time: zarr_meta.reference_time,
        forecast_hour: zarr_meta.forecast_hour,
        bbox: zarr_meta.bbox,
        shape: zarr_meta.shape,
        chunk_shape: zarr_meta.chunk_shape,
        num_chunks: zarr_meta.num_chunks,
        fill_value: zarr_meta.fill_value,
    };
    
    // Create processor with metadata from catalog (avoids metadata fetch from MinIO)
    let processor = ZarrGridProcessor::with_metadata(
        store,
        &zarr_path,
        grid_metadata.clone(),
        factory.chunk_cache(),
        factory.config().clone(),
    ).map_err(|e| {
        error!(
            error = %e,
            zarr_path = %zarr_path,
            shape = ?grid_metadata.shape,
            chunk_shape = ?grid_metadata.chunk_shape,
            "Failed to open Zarr array"
        );
        format!("Failed to open Zarr: {}", e)
    })?;
    
    // Determine bbox to read
    let read_bbox = if let Some(bbox_arr) = bbox {
        GpBoundingBox::new(
            bbox_arr[0] as f64,
            bbox_arr[1] as f64, 
            bbox_arr[2] as f64,
            bbox_arr[3] as f64,
        )
    } else {
        // Read full grid
        zarr_meta.bbox
    };
    
    // Read the region
    let start = Instant::now();
    let region = processor.read_region(&read_bbox).await
        .map_err(|e| {
            error!(
                error = %e,
                zarr_path = %zarr_path,
                bbox = ?read_bbox,
                "Failed to read Zarr region"
            );
            format!("Failed to read Zarr region: {}", e)
        })?;
    let read_duration = start.elapsed();
    
    // Return actual bbox from the region (important for partial reads)
    let actual_bbox = [
        region.bbox.min_lon as f32,
        region.bbox.min_lat as f32,
        region.bbox.max_lon as f32,
        region.bbox.max_lat as f32,
    ];
    
    info!(
        width = region.width,
        height = region.height,
        data_points = region.data.len(),
        read_ms = read_duration.as_millis(),
        actual_bbox_min_lon = actual_bbox[0],
        actual_bbox_max_lon = actual_bbox[2],
        "Loaded Zarr region"
    );
    
    // Check if source grid uses 0-360 longitude (like GFS)
    // This must be based on the full grid bbox, not the partial region bbox
    let grid_uses_360 = zarr_meta.bbox.min_lon >= 0.0 && zarr_meta.bbox.max_lon > 180.0;
    
    Ok(GridData {
        data: region.data,
        width: region.width,
        height: region.height,
        bbox: Some(actual_bbox),
        goes_projection: None, // Zarr data is always pre-projected to geographic
        grid_uses_360,
    })
}

/// Query a single point value from Zarr storage.
/// 
/// This is optimized for GetFeatureInfo requests - it reads only the single chunk
/// containing the requested point, making it much more efficient than loading
/// the entire grid.
/// 
/// # Arguments
/// * `factory` - Grid processor factory with chunk cache
/// * `entry` - Catalog entry with zarr_metadata
/// * `lon` - Longitude in degrees (-180 to 180 or 0 to 360)
/// * `lat` - Latitude in degrees (-90 to 90)
/// 
/// # Returns
/// * `Ok(Some(value))` - The data value at the point
/// * `Ok(None)` - Point is outside grid bounds or contains fill/NaN value
/// * `Err(...)` - Failed to read data
#[tracing::instrument(skip(factory, entry), fields(
    model = %entry.model,
    parameter = %entry.parameter,
    level = %entry.level,
    storage_path = %entry.storage_path,
))]
pub async fn query_point_from_zarr(
    factory: &GridProcessorFactory,
    entry: &CatalogEntry,
    lon: f64,
    lat: f64,
) -> Result<Option<f32>, String> {
    use grid_processor::{
        GridProcessor, ZarrGridProcessor, ZarrMetadata,
        MinioConfig, create_minio_storage,
    };

    // Parse zarr_metadata from catalog entry
    let zarr_json = entry.zarr_metadata.as_ref()
        .ok_or_else(|| {
            error!(model = %entry.model, parameter = %entry.parameter, "No zarr_metadata in catalog entry");
            "No zarr_metadata in catalog entry".to_string()
        })?;
    
    let zarr_meta = ZarrMetadata::from_json(zarr_json)
        .map_err(|e| {
            error!(model = %entry.model, parameter = %entry.parameter, error = %e, "Failed to parse zarr_metadata");
            format!("Failed to parse zarr_metadata: {}", e)
        })?;
    
    // Check if source grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = zarr_meta.bbox.min_lon >= 0.0 && zarr_meta.bbox.max_lon > 180.0;
    
    // Normalize longitude to match grid coordinate system
    let query_lon = if grid_uses_360 && lon < 0.0 {
        lon + 360.0
    } else if !grid_uses_360 && lon > 180.0 {
        lon - 360.0
    } else {
        lon
    };
    
    debug!(
        lon = lon,
        lat = lat,
        query_lon = query_lon,
        grid_uses_360 = grid_uses_360,
        grid_bbox = ?zarr_meta.bbox,
        "Querying point from Zarr"
    );
    
    // Build storage path
    let zarr_path = if entry.storage_path.starts_with('/') {
        entry.storage_path.clone()
    } else {
        format!("/{}", entry.storage_path)
    };
    
    // Create MinIO storage
    let minio_config = MinioConfig::from_env();
    let store = create_minio_storage(&minio_config)
        .map_err(|e| {
            error!(error = %e, "Failed to create MinIO storage");
            format!("Failed to create MinIO storage: {}", e)
        })?;
    
    // Convert zarr_meta to GridMetadata for the processor
    let grid_metadata = grid_processor::GridMetadata {
        model: zarr_meta.model.clone(),
        parameter: zarr_meta.parameter.clone(),
        level: zarr_meta.level.clone(),
        units: zarr_meta.units.clone(),
        reference_time: zarr_meta.reference_time,
        forecast_hour: zarr_meta.forecast_hour,
        bbox: zarr_meta.bbox,
        shape: zarr_meta.shape,
        chunk_shape: zarr_meta.chunk_shape,
        num_chunks: zarr_meta.num_chunks,
        fill_value: zarr_meta.fill_value,
    };
    
    // Create processor with metadata from catalog
    let processor = ZarrGridProcessor::with_metadata(
        store,
        &zarr_path,
        grid_metadata,
        factory.chunk_cache(),
        factory.config().clone(),
    ).map_err(|e| {
        error!(error = %e, zarr_path = %zarr_path, "Failed to open Zarr array");
        format!("Failed to open Zarr: {}", e)
    })?;
    
    // Query the point value (reads only the chunk containing this point)
    let start = Instant::now();
    let value = processor.read_point(query_lon, lat).await
        .map_err(|e| {
            error!(error = %e, lon = query_lon, lat = lat, "Failed to read point from Zarr");
            format!("Failed to read point: {}", e)
        })?;
    let read_duration = start.elapsed();
    
    info!(
        lon = lon,
        lat = lat,
        value = ?value,
        read_ms = read_duration.as_millis(),
        "Zarr point query complete"
    );
    
    Ok(value)
}

/// Load GOES NetCDF data from storage with parsed grid caching
/// 
/// This uses ncdump to extract data since we don't have direct HDF5 support.
/// Downloads the file from S3 storage to a temp location first.
/// 
/// The grid_cache parameter enables caching of parsed grid data to avoid
/// repeated expensive ncdump parsing for the same file.
async fn load_netcdf_grid_data(
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    metrics: &MetricsCollector,
    entry: &CatalogEntry,
    source_type: &DataSourceType,
) -> Result<GridData, String> {
    use std::sync::Arc;
    
    // Check grid cache first (if available)
    // Include reference_time in cache key to ensure different observation times don't collide
    let cache_key = format!("{}:{}", entry.storage_path, entry.reference_time.timestamp());
    if let Some(cache) = grid_cache {
        if let Some(cached) = cache.get(&cache_key).await {
            info!(
                storage_path = %entry.storage_path,
                reference_time = ?entry.reference_time,
                cache_key = %cache_key,
                "Grid cache HIT for GOES data"
            );
            // Record cache hit for this data source
            metrics.record_data_source_cache_hit(source_type).await;
            
            // Record GOES-specific cache hit
            if let Some(sat) = crate::metrics::GoesSatellite::from_model(&entry.model) {
                metrics.record_goes_cache_hit(sat);
            }
            
            // Convert from CachedGridData back to GridData
            return Ok(GridData {
                data: (*cached.data).clone(),
                width: cached.width,
                height: cached.height,
                bbox: None, // GOES/NetCDF returns full grid, use entry.bbox
                goes_projection: cached.goes_projection.map(|p| GoesProjectionParams {
                    x_origin: p.x_origin,
                    y_origin: p.y_origin,
                    dx: p.dx,
                    dy: p.dy,
                    perspective_point_height: p.perspective_point_height,
                    semi_major_axis: p.semi_major_axis,
                    semi_minor_axis: p.semi_minor_axis,
                    longitude_origin: p.longitude_origin,
                }),
                grid_uses_360: false, // GOES uses geostationary projection, not 0-360
            });
        }
    }
    
    info!(
        storage_path = %entry.storage_path,
        parameter = %entry.parameter,
        "Loading GOES NetCDF data from cache/storage (grid cache MISS)"
    );
    
    // Determine GOES satellite for metrics
    let goes_satellite = crate::metrics::GoesSatellite::from_model(&entry.model);
    
    // Record GOES cache miss
    if let Some(sat) = goes_satellite {
        metrics.record_goes_cache_miss(sat);
    }
    
    // Load file from cache (or storage on cache miss)
    let fetch_start = Instant::now();
    let file_data = grib_cache
        .get(&entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load NetCDF file from cache/storage: {}", e))?;
    let fetch_us = fetch_start.elapsed().as_micros() as u64;
    
    // Record GOES fetch metrics
    if let Some(sat) = goes_satellite {
        metrics.record_goes_fetch(sat, file_data.len() as u64, fetch_us);
    }
    
    info!(
        size = file_data.len(),
        fetch_ms = fetch_us as f64 / 1000.0,
        "Loaded NetCDF file from cache/storage"
    );
    
    // Use native netcdf library to parse the file (5-10x faster than ncdump)
    let parse_start = Instant::now();
    let (data, width, height, projection, x_offset, y_offset, x_scale, y_scale) = 
        netcdf_parser::load_goes_netcdf_from_bytes(&file_data)
            .map_err(|e| format!("Failed to parse NetCDF: {}", e))?;
    let parse_us = parse_start.elapsed().as_micros() as u64;
    
    // Record per-source parse time
    metrics.record_data_source_parse(source_type, parse_us).await;
    
    // Record GOES-specific parse metrics
    if let Some(sat) = goes_satellite {
        metrics.record_goes_parse(sat, parse_us, width as u32, height as u32);
    }
    
    info!(
        width = width,
        height = height,
        x_scale = x_scale,
        x_offset = x_offset,
        y_scale = y_scale,
        y_offset = y_offset,
        longitude_origin = projection.longitude_origin,
        parse_ms = parse_us as f64 / 1000.0,
        "Parsed NetCDF using native library"
    );
    
    // Create projection parameters from parsed data (cast f32 to f64)
    let goes_projection = GoesProjectionParams {
        x_origin: x_offset as f64,
        y_origin: y_offset as f64,
        dx: x_scale as f64,
        dy: y_scale as f64,
        perspective_point_height: projection.perspective_point_height,
        semi_major_axis: projection.semi_major_axis,
        semi_minor_axis: projection.semi_minor_axis,
        longitude_origin: projection.longitude_origin,
    };
    
    // Store in grid cache for future requests
    if let Some(cache) = grid_cache {
        let cached_data = CachedGridData {
            data: Arc::new(data.clone()),
            width,
            height,
            goes_projection: Some(storage::GoesProjectionParams {
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
        cache.insert(cache_key.clone(), cached_data).await;
        info!(
            storage_path = %entry.storage_path,
            reference_time = ?entry.reference_time,
            cache_key = %cache_key,
            width = width,
            height = height,
            "Cached parsed GOES grid data"
        );
    }
    
    Ok(GridData {
        data,
        width,
        height,
        bbox: None, // GOES/NetCDF returns full grid, use entry.bbox
        goes_projection: Some(goes_projection),
        grid_uses_360: false, // GOES uses geostationary projection, not 0-360
    })
}



/// Render wind barbs combining U and V component data using expanded tile rendering.
/// This renders a 3x3 grid of tiles and crops the center to ensure seamless boundaries.
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `grid_processor_factory`: Optional factory for Zarr data access
/// - `model`: Weather model name (e.g., "gfs")
/// - `tile_coord`: Optional tile coordinate for expanded rendering
/// - `width`: Output image width (single tile)
/// - `height`: Output image height (single tile)
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat] for the single tile
///
/// # Returns
/// PNG image data as bytes
pub async fn render_wind_barbs_tile(
    grib_cache: &GribCache,
    catalog: &Catalog,
    grid_processor_factory: Option<&GridProcessorFactory>,
    model: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    forecast_hour: Option<u32>,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::{ExpandedTileConfig, expanded_tile_bbox, crop_center_tile};
    
    // First get the catalog entries to check for Zarr metadata
    let u_entry = get_wind_entry(catalog, model, "UGRD", forecast_hour, None).await?;
    let v_entry = get_wind_entry(catalog, model, "VGRD", forecast_hour, None).await?;
    
    // Determine if we can use Zarr (both entries have zarr_metadata and we have a factory)
    let use_zarr = u_entry.zarr_metadata.is_some() 
        && v_entry.zarr_metadata.is_some()
        && grid_processor_factory.is_some();
    
    // Load U and V component data - use Zarr if available, otherwise GRIB
    let (u_data, v_data, grid_width, grid_height, data_bounds, grid_uses_360) = if use_zarr {
        let factory = grid_processor_factory.unwrap();
        info!(
            model = model,
            u_path = %u_entry.storage_path,
            v_path = %v_entry.storage_path,
            "Loading wind components from Zarr"
        );
        load_wind_components_from_zarr(factory, &u_entry, &v_entry, None).await?
    } else {
        info!(
            model = model,
            "Loading wind components from GRIB (Zarr not available)"
        );
        load_wind_components(grib_cache, catalog, model, forecast_hour).await?
    };
    
    // Determine if we should use expanded rendering
    let (render_bbox, render_width, render_height, needs_crop) = if let Some(coord) = tile_coord {
        let config = ExpandedTileConfig::tiles_3x3();
        let expanded_bbox = expanded_tile_bbox(&coord, &config);
        
        // Calculate actual expanded dimensions
        let (exp_w, exp_h) = wms_common::tile::actual_expanded_dimensions(&coord, &config);
        
        (
            [
                expanded_bbox.min_x as f32,
                expanded_bbox.min_y as f32,
                expanded_bbox.max_x as f32,
                expanded_bbox.max_y as f32,
            ],
            exp_w as usize,
            exp_h as usize,
            Some((coord, config)),
        )
    } else {
        (bbox, width as usize, height as usize, None)
    };
    
    info!(
        render_width = render_width,
        render_height = render_height,
        bbox_min_lon = render_bbox[0],
        bbox_max_lon = render_bbox[2],
        expanded = needs_crop.is_some(),
        "Rendering wind barbs"
    );
    
    // Resample data to the render bbox (model-aware for proper projection handling)
    let u_resampled = resample_for_model_geographic(
        &u_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model, grid_uses_360
    );
    let v_resampled = resample_for_model_geographic(
        &v_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model, grid_uses_360
    );
    
    // Render wind barbs with geographic alignment
    let config = barbs::BarbConfig::default();
    let barb_pixels = barbs::render_wind_barbs_aligned(
        &u_resampled,
        &v_resampled,
        render_width,
        render_height,
        render_bbox,
        &config,
    );
    
    // Crop to center tile if we used expanded rendering
    let final_pixels = if let Some((coord, tile_config)) = needs_crop {
        crop_center_tile(&barb_pixels, render_width as u32, &coord, &tile_config)
    } else {
        barb_pixels
    };
    
    // Encode as PNG
    renderer::png::create_png(&final_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

/// Render wind barbs combining U and V component data with optional level/elevation.
/// This renders a 3x3 grid of tiles and crops the center to ensure seamless boundaries.
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `grid_processor_factory`: Optional factory for Zarr data access
/// - `model`: Weather model name (e.g., "gfs")
/// - `tile_coord`: Optional tile coordinate for expanded rendering
/// - `width`: Output image width (single tile)
/// - `height`: Output image height (single tile)
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat] for the single tile
/// - `forecast_hour`: Optional forecast hour; if None, uses latest
/// - `level`: Optional vertical level/elevation (e.g., "500 mb", "10 m above ground")
///
/// # Returns
/// PNG image data as bytes
pub async fn render_wind_barbs_tile_with_level(
    grib_cache: &GribCache,
    catalog: &Catalog,
    grid_processor_factory: Option<&GridProcessorFactory>,
    model: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::{ExpandedTileConfig, expanded_tile_bbox, crop_center_tile};
    
    // First get the catalog entries to check for Zarr metadata
    let u_entry = get_wind_entry(catalog, model, "UGRD", forecast_hour, level).await?;
    let v_entry = get_wind_entry(catalog, model, "VGRD", forecast_hour, level).await?;
    
    // Determine if we can use Zarr (both entries have zarr_metadata and we have a factory)
    let use_zarr = u_entry.zarr_metadata.is_some() 
        && v_entry.zarr_metadata.is_some()
        && grid_processor_factory.is_some();
    
    // Load U and V component data - use Zarr if available, otherwise GRIB
    let (u_data, v_data, grid_width, grid_height, data_bounds, grid_uses_360) = if use_zarr {
        let factory = grid_processor_factory.unwrap();
        info!(
            model = model,
            u_path = %u_entry.storage_path,
            v_path = %v_entry.storage_path,
            "Loading wind components from Zarr"
        );
        load_wind_components_from_zarr(factory, &u_entry, &v_entry, None).await?
    } else {
        info!(
            model = model,
            "Loading wind components from GRIB (Zarr not available)"
        );
        load_wind_components_with_level(grib_cache, catalog, model, forecast_hour, level).await?
    };
    
    // Determine if we should use expanded rendering
    let (render_bbox, render_width, render_height, needs_crop) = if let Some(coord) = tile_coord {
        let config = ExpandedTileConfig::tiles_3x3();
        let expanded_bbox = expanded_tile_bbox(&coord, &config);
        
        // Calculate actual expanded dimensions
        let (exp_w, exp_h) = wms_common::tile::actual_expanded_dimensions(&coord, &config);
        
        (
            [
                expanded_bbox.min_x as f32,
                expanded_bbox.min_y as f32,
                expanded_bbox.max_x as f32,
                expanded_bbox.max_y as f32,
            ],
            exp_w as usize,
            exp_h as usize,
            Some((coord, config)),
        )
    } else {
        (bbox, width as usize, height as usize, None)
    };
    
    info!(
        render_width = render_width,
        render_height = render_height,
        bbox_min_lon = render_bbox[0],
        bbox_max_lon = render_bbox[2],
        expanded = needs_crop.is_some(),
        level = ?level,
        "Rendering wind barbs with level"
    );
    
    // Resample data to the render bbox (model-aware for proper projection handling)
    let u_resampled = resample_for_model_geographic(
        &u_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model, grid_uses_360
    );
    let v_resampled = resample_for_model_geographic(
        &v_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model, grid_uses_360
    );
    
    // Render wind barbs with geographic alignment
    let config = barbs::BarbConfig::default();
    let barb_pixels = barbs::render_wind_barbs_aligned(
        &u_resampled,
        &v_resampled,
        render_width,
        render_height,
        render_bbox,
        &config,
    );
    
    // Crop to center tile if we used expanded rendering
    let final_pixels = if let Some((coord, tile_config)) = needs_crop {
        crop_center_tile(&barb_pixels, render_width as u32, &coord, &tile_config)
    } else {
        barb_pixels
    };
    
    // Encode as PNG
    renderer::png::create_png(&final_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

/// Get a wind component catalog entry (UGRD or VGRD) for the given parameters.
/// This is a helper function to look up catalog entries for wind components.
async fn get_wind_entry(
    catalog: &Catalog,
    model: &str,
    parameter: &str,  // "UGRD" or "VGRD"
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<CatalogEntry, String> {
    match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, parameter, hour, lev)
                .await
                .map_err(|e| format!("Failed to get {}: {}", parameter, e))?
                .ok_or_else(|| format!("No {} data available for hour {} level {}", parameter, hour, lev))
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, parameter, hour)
                .await
                .map_err(|e| format!("Failed to get {}: {}", parameter, e))?
                .ok_or_else(|| format!("No {} data available for hour {}", parameter, hour))
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
                .await
                .map_err(|e| format!("Failed to get {}: {}", parameter, e))?
                .ok_or_else(|| format!("No {} data available at level {}", parameter, lev))
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, parameter)
                .await
                .map_err(|e| format!("Failed to get {}: {}", parameter, e))?
                .ok_or_else(|| format!("No {} data available", parameter))
        }
    }
}

/// Load U and V wind component data from cache/storage with optional level
async fn load_wind_components_with_level(
    grib_cache: &GribCache,
    catalog: &Catalog,
    model: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<(Vec<f32>, Vec<f32>, usize, usize, [f32; 4], bool), String> {
    // Load U component (UGRD) with level support
    let u_entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, "UGRD", hour, lev)
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| format!("No UGRD data available for hour {} level {}", hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, "UGRD", hour)
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| format!("No UGRD data available for hour {}", hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, "UGRD", lev)
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| format!("No UGRD data available at level {}", lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, "UGRD")
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| "No UGRD data available".to_string())?
        }
    };

    // Load V component (VGRD) with level support
    let v_entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, "VGRD", hour, lev)
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| format!("No VGRD data available for hour {} level {}", hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, "VGRD", hour)
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| format!("No VGRD data available for hour {}", hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, "VGRD", lev)
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| format!("No VGRD data available at level {}", lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, "VGRD")
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| "No VGRD data available".to_string())?
        }
    };

    // Load GRIB2 files from cache
    let u_grib = grib_cache
        .get(&u_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

    let v_grib = grib_cache
        .get(&v_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load VGRD file: {}", e))?;

    // Parse GRIB2 messages - use level from catalog entry to find correct message
    let u_msg = find_parameter_in_grib(u_grib, "UGRD", Some(&u_entry.level))?;
    let v_msg = find_parameter_in_grib(v_grib, "VGRD", Some(&v_entry.level))?;

    // Unpack grid data
    let u_data = u_msg
        .unpack_data()
        .map_err(|e| format!("Unpacking U failed: {}", e))?;

    let v_data = v_msg
        .unpack_data()
        .map_err(|e| format!("Unpacking V failed: {}", e))?;

    let (grid_height, grid_width) = u_msg.grid_dims();
    
    info!(
        grid_width = grid_width,
        grid_height = grid_height,
        u_level = %u_entry.level,
        v_level = %v_entry.level,
        "Loaded wind component data with level"
    );

    // Both U and V should have same bbox - use U's bbox
    let data_bounds = [
        u_entry.bbox.min_x as f32,
        u_entry.bbox.min_y as f32,
        u_entry.bbox.max_x as f32,
        u_entry.bbox.max_y as f32,
    ];
    
    // Check if grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = u_entry.bbox.min_x >= 0.0 && u_entry.bbox.max_x > 180.0;
    
    Ok((u_data, v_data, grid_width as usize, grid_height as usize, data_bounds, grid_uses_360))
}

/// Load U and V wind component data from Zarr storage.
/// 
/// This function loads both UGRD and VGRD components from Zarr, enabling efficient
/// chunked reads for wind barb rendering.
/// 
/// # Arguments
/// * `factory` - Grid processor factory with shared chunk cache
/// * `u_entry` - Catalog entry for U component (UGRD)
/// * `v_entry` - Catalog entry for V component (VGRD)
/// * `bbox` - Optional bounding box for partial reads (used for tile rendering)
/// 
/// # Returns
/// Tuple of (u_data, v_data, grid_width, grid_height, data_bounds, grid_uses_360)
async fn load_wind_components_from_zarr(
    factory: &GridProcessorFactory,
    u_entry: &CatalogEntry,
    v_entry: &CatalogEntry,
    bbox: Option<[f32; 4]>,
) -> Result<(Vec<f32>, Vec<f32>, usize, usize, [f32; 4], bool), String> {
    use grid_processor::{
        BoundingBox as GpBoundingBox,
        GridProcessor, ZarrGridProcessor, ZarrMetadata,
        MinioConfig, create_minio_storage,
    };
    
    // Parse zarr_metadata for U component
    let u_zarr_json = u_entry.zarr_metadata.as_ref()
        .ok_or_else(|| "No zarr_metadata in U (UGRD) catalog entry".to_string())?;
    let u_zarr_meta = ZarrMetadata::from_json(u_zarr_json)
        .map_err(|e| format!("Failed to parse U zarr_metadata: {}", e))?;
    
    // Parse zarr_metadata for V component
    let v_zarr_json = v_entry.zarr_metadata.as_ref()
        .ok_or_else(|| "No zarr_metadata in V (VGRD) catalog entry".to_string())?;
    let v_zarr_meta = ZarrMetadata::from_json(v_zarr_json)
        .map_err(|e| format!("Failed to parse V zarr_metadata: {}", e))?;
    
    // Verify both grids have the same shape
    if u_zarr_meta.shape != v_zarr_meta.shape {
        return Err(format!(
            "U and V grid shapes don't match: {:?} vs {:?}",
            u_zarr_meta.shape, v_zarr_meta.shape
        ));
    }
    
    info!(
        u_path = %u_entry.storage_path,
        v_path = %v_entry.storage_path,
        shape = ?u_zarr_meta.shape,
        chunk_shape = ?u_zarr_meta.chunk_shape,
        bbox = ?bbox,
        "Loading wind components from Zarr"
    );
    
    // Create MinIO storage
    let minio_config = MinioConfig::from_env();
    let store = create_minio_storage(&minio_config)
        .map_err(|e| format!("Failed to create MinIO storage: {}", e))?;
    
    // Build storage paths
    let u_zarr_path = if u_entry.storage_path.starts_with('/') {
        u_entry.storage_path.clone()
    } else {
        format!("/{}", u_entry.storage_path)
    };
    let v_zarr_path = if v_entry.storage_path.starts_with('/') {
        v_entry.storage_path.clone()
    } else {
        format!("/{}", v_entry.storage_path)
    };
    
    // Create GridMetadata for U processor
    let u_grid_metadata = grid_processor::GridMetadata {
        model: u_zarr_meta.model.clone(),
        parameter: u_zarr_meta.parameter.clone(),
        level: u_zarr_meta.level.clone(),
        units: u_zarr_meta.units.clone(),
        reference_time: u_zarr_meta.reference_time,
        forecast_hour: u_zarr_meta.forecast_hour,
        bbox: u_zarr_meta.bbox,
        shape: u_zarr_meta.shape,
        chunk_shape: u_zarr_meta.chunk_shape,
        num_chunks: u_zarr_meta.num_chunks,
        fill_value: u_zarr_meta.fill_value,
    };
    
    // Create U processor
    let u_processor = ZarrGridProcessor::with_metadata(
        store.clone(),
        &u_zarr_path,
        u_grid_metadata,
        factory.chunk_cache(),
        factory.config().clone(),
    ).map_err(|e| format!("Failed to open U Zarr: {}", e))?;
    
    // Create GridMetadata for V processor
    let v_grid_metadata = grid_processor::GridMetadata {
        model: v_zarr_meta.model.clone(),
        parameter: v_zarr_meta.parameter.clone(),
        level: v_zarr_meta.level.clone(),
        units: v_zarr_meta.units.clone(),
        reference_time: v_zarr_meta.reference_time,
        forecast_hour: v_zarr_meta.forecast_hour,
        bbox: v_zarr_meta.bbox,
        shape: v_zarr_meta.shape,
        chunk_shape: v_zarr_meta.chunk_shape,
        num_chunks: v_zarr_meta.num_chunks,
        fill_value: v_zarr_meta.fill_value,
    };
    
    // Create V processor
    let v_processor = ZarrGridProcessor::with_metadata(
        store,
        &v_zarr_path,
        v_grid_metadata,
        factory.chunk_cache(),
        factory.config().clone(),
    ).map_err(|e| format!("Failed to open V Zarr: {}", e))?;
    
    // Determine bbox to read
    let read_bbox = if let Some(bbox_arr) = bbox {
        GpBoundingBox::new(
            bbox_arr[0] as f64,
            bbox_arr[1] as f64,
            bbox_arr[2] as f64,
            bbox_arr[3] as f64,
        )
    } else {
        // Read full grid
        u_zarr_meta.bbox
    };
    
    // Read both regions
    let start = Instant::now();
    let u_region = u_processor.read_region(&read_bbox).await
        .map_err(|e| format!("Failed to read U Zarr region: {}", e))?;
    let v_region = v_processor.read_region(&read_bbox).await
        .map_err(|e| format!("Failed to read V Zarr region: {}", e))?;
    let read_duration = start.elapsed();
    
    // Verify regions have same dimensions
    if u_region.width != v_region.width || u_region.height != v_region.height {
        return Err(format!(
            "U and V region dimensions don't match: {}x{} vs {}x{}",
            u_region.width, u_region.height, v_region.width, v_region.height
        ));
    }
    
    let grid_width = u_region.width;
    let grid_height = u_region.height;
    
    // Use the bbox from the region (important for partial reads)
    let data_bounds = [
        u_region.bbox.min_lon as f32,
        u_region.bbox.min_lat as f32,
        u_region.bbox.max_lon as f32,
        u_region.bbox.max_lat as f32,
    ];
    
    // Check if source grid uses 0-360 longitude (like GFS)
    // This must be based on the full grid bbox, not the partial region bbox
    let grid_uses_360 = u_zarr_meta.bbox.min_lon >= 0.0 && u_zarr_meta.bbox.max_lon > 180.0;
    
    info!(
        grid_width = grid_width,
        grid_height = grid_height,
        u_level = %u_entry.level,
        v_level = %v_entry.level,
        read_ms = read_duration.as_millis(),
        grid_uses_360 = grid_uses_360,
        "Loaded wind components from Zarr"
    );
    
    Ok((u_region.data, v_region.data, grid_width, grid_height, data_bounds, grid_uses_360))
}

/// Load U and V wind component data from cache/storage
async fn load_wind_components(
    grib_cache: &GribCache,
    catalog: &Catalog,
    model: &str,
    forecast_hour: Option<u32>,
) -> Result<(Vec<f32>, Vec<f32>, usize, usize, [f32; 4], bool), String> {
    // Load U component (UGRD)
    let u_entry = if let Some(hour) = forecast_hour {
        catalog
            .find_by_forecast_hour(model, "UGRD", hour)
            .await
            .map_err(|e| format!("Failed to get UGRD: {}", e))?
            .ok_or_else(|| format!("No UGRD data available for hour {}", hour))?
    } else {
        catalog
            .get_latest_run_earliest_forecast(model, "UGRD")
            .await
            .map_err(|e| format!("Failed to get UGRD: {}", e))?
            .ok_or_else(|| "No UGRD data available".to_string())?
    };

    // Load V component (VGRD)
    let v_entry = if let Some(hour) = forecast_hour {
        catalog
            .find_by_forecast_hour(model, "VGRD", hour)
            .await
            .map_err(|e| format!("Failed to get VGRD: {}", e))?
            .ok_or_else(|| format!("No VGRD data available for hour {}", hour))?
    } else {
        catalog
            .get_latest_run_earliest_forecast(model, "VGRD")
            .await
            .map_err(|e| format!("Failed to get VGRD: {}", e))?
            .ok_or_else(|| "No VGRD data available".to_string())?
    };

    // Load GRIB2 files from cache
    let u_grib = grib_cache
        .get(&u_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

    let v_grib = grib_cache
        .get(&v_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load VGRD file: {}", e))?;

    // Parse GRIB2 messages - use level from catalog entry to find correct message
    let u_msg = find_parameter_in_grib(u_grib, "UGRD", Some(&u_entry.level))?;
    let v_msg = find_parameter_in_grib(v_grib, "VGRD", Some(&v_entry.level))?;

    // Unpack grid data
    let u_data = u_msg
        .unpack_data()
        .map_err(|e| format!("Unpacking U failed: {}", e))?;

    let v_data = v_msg
        .unpack_data()
        .map_err(|e| format!("Unpacking V failed: {}", e))?;

    let (grid_height, grid_width) = u_msg.grid_dims();
    
    info!(
        grid_width = grid_width,
        grid_height = grid_height,
        u_level = %u_entry.level,
        v_level = %v_entry.level,
        "Loaded wind component data"
    );

    // Both U and V should have same bbox - use U's bbox
    let data_bounds = [
        u_entry.bbox.min_x as f32,
        u_entry.bbox.min_y as f32,
        u_entry.bbox.max_x as f32,
        u_entry.bbox.max_y as f32,
    ];

    // Check if grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = u_entry.bbox.min_x >= 0.0 && u_entry.bbox.max_x > 180.0;

    Ok((u_data, v_data, grid_width as usize, grid_height as usize, data_bounds, grid_uses_360))
}

/// Render wind barbs combining U and V component data
///
/// # Arguments
/// - `storage`: Object storage for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `grid_processor_factory`: Optional factory for Zarr data access
/// - `model`: Weather model name (e.g., "gfs")
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box [min_lon, min_lat, max_lon, max_lat]
/// - `barb_spacing`: Optional spacing between barbs in pixels (default: 50)
///
/// # Returns
/// PNG image data as bytes
pub async fn render_wind_barbs_layer(
    grib_cache: &GribCache,
    catalog: &Catalog,
    grid_processor_factory: Option<&GridProcessorFactory>,
    model: &str,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    barb_spacing: Option<usize>,
    forecast_hour: Option<u32>,
) -> Result<Vec<u8>, String> {
    // Get catalog entries for U and V components
    let u_entry = get_wind_entry(catalog, model, "UGRD", forecast_hour, None).await?;
    let v_entry = get_wind_entry(catalog, model, "VGRD", forecast_hour, None).await?;
    
    // Determine if we can use Zarr (both entries have zarr_metadata and we have a factory)
    let use_zarr = u_entry.zarr_metadata.is_some() 
        && v_entry.zarr_metadata.is_some()
        && grid_processor_factory.is_some();
    
    // Load U and V component data - use Zarr if available, otherwise GRIB
    let (u_data, v_data, grid_width, grid_height, data_bounds, grid_uses_360) = if use_zarr {
        let factory = grid_processor_factory.unwrap();
        info!(
            model = model,
            u_path = %u_entry.storage_path,
            v_path = %v_entry.storage_path,
            "Loading wind components from Zarr for layer render"
        );
        load_wind_components_from_zarr(factory, &u_entry, &v_entry, bbox).await?
    } else {
        info!(
            model = model,
            "Loading wind components from GRIB for layer render"
        );
        // Fall back to GRIB loading
        let u_grib = grib_cache
            .get(&u_entry.storage_path)
            .await
            .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

        let v_grib = grib_cache
            .get(&v_entry.storage_path)
            .await
            .map_err(|e| format!("Failed to load VGRD file: {}", e))?;

        // Parse GRIB2 messages - use level from catalog entry to find correct message
        let u_msg = find_parameter_in_grib(u_grib, "UGRD", Some(&u_entry.level))?;
        let v_msg = find_parameter_in_grib(v_grib, "VGRD", Some(&v_entry.level))?;

        // Unpack grid data
        let u_data = u_msg
            .unpack_data()
            .map_err(|e| format!("Unpacking U failed: {}", e))?;

        let v_data = v_msg
            .unpack_data()
            .map_err(|e| format!("Unpacking V failed: {}", e))?;

        let (gh, gw) = u_msg.grid_dims();
        let data_bounds = [
            u_entry.bbox.min_x as f32,
            u_entry.bbox.min_y as f32,
            u_entry.bbox.max_x as f32,
            u_entry.bbox.max_y as f32,
        ];
        let grid_uses_360 = u_entry.bbox.min_x >= 0.0 && u_entry.bbox.max_x > 180.0;
        
        (u_data, v_data, gw as usize, gh as usize, data_bounds, grid_uses_360)
    };
    
    // Debug: Log data statistics
    let u_stats: (f32, f32, f32) = u_data.iter()
        .filter(|v| !v.is_nan())
        .fold((f32::MAX, f32::MIN, 0.0), |(min, max, sum), &v| {
            (min.min(v), max.max(v), sum + v)
        });
    let v_stats: (f32, f32, f32) = v_data.iter()
        .filter(|v| !v.is_nan())
        .fold((f32::MAX, f32::MIN, 0.0), |(min, max, sum), &v| {
            (min.min(v), max.max(v), sum + v)
        });
    
    info!(
        grid_width = grid_width,
        grid_height = grid_height,
        u_min = u_stats.0,
        u_max = u_stats.1,
        v_min = v_stats.0,
        v_max = v_stats.1,
        "Loaded wind component data"
    );

    // Validate data sizes
    if u_data.len() != grid_width * grid_height {
        return Err(format!(
            "U grid size mismatch: {} vs {}x{}",
            u_data.len(),
            grid_width,
            grid_height
        ));
    }

    if v_data.len() != grid_width * grid_height {
        return Err(format!(
            "V grid size mismatch: {} vs {}x{}",
            v_data.len(),
            grid_width,
            grid_height
        ));
    }

    // Prepare rendering parameters
    let output_width = width as usize;
    let output_height = height as usize;
    let spacing = barb_spacing.unwrap_or(50);

    // If bbox is specified, we need to resample to that region
    let (u_to_render, v_to_render, render_width, render_height) = if let Some(bbox) = bbox {
        info!(
            bbox_min_lon = bbox[0],
            bbox_min_lat = bbox[1],
            bbox_max_lon = bbox[2],
            bbox_max_lat = bbox[3],
            output_width = output_width,
            output_height = output_height,
            "Resampling wind data for bbox"
        );
        
        // Resample from geographic coordinates (model-aware for proper projection handling)
        let u_resampled = resample_for_model_geographic(&u_data, grid_width, grid_height, output_width, output_height, bbox, data_bounds, model, grid_uses_360);
        let v_resampled = resample_for_model_geographic(&v_data, grid_width, grid_height, output_width, output_height, bbox, data_bounds, model, grid_uses_360);
        
        // Debug: check resampled data statistics
        let u_min = u_resampled.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MAX, f32::min);
        let u_max = u_resampled.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
        let v_min = v_resampled.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MAX, f32::min);
        let v_max = v_resampled.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
        
        // Check if data has variation
        let u_range = u_max - u_min;
        let v_range = v_max - v_min;
        
        info!(
            u_min = format!("{:.2}", u_min),
            u_max = format!("{:.2}", u_max),
            u_range = format!("{:.2}", u_range),
            v_min = format!("{:.2}", v_min),
            v_max = format!("{:.2}", v_max),
            v_range = format!("{:.2}", v_range),
            "Resampled wind data range"
        );
        
        if u_range < 0.1 && v_range < 0.1 {
            info!("WARNING: Wind data appears uniform across region - all barbs will point same direction");
        }
        
        // Sample some positions to verify variation
        let positions = renderer::barbs::calculate_barb_positions(output_width, output_height, spacing as u32);
        if positions.len() >= 4 {
            for (i, (x, y)) in positions.iter().take(4).enumerate() {
                let idx = y * output_width + x;
                if idx < u_resampled.len() {
                    let u = u_resampled[idx];
                    let v = v_resampled[idx];
                    let (speed, dir) = renderer::barbs::uv_to_speed_direction(u, v);
                    debug!(
                        pos = i,
                        x = x,
                        y = y,
                        u = u,
                        v = v,
                        speed = speed,
                        dir_deg = dir.to_degrees(),
                        "Barb position sample"
                    );
                }
            }
        }
        
        (u_resampled, v_resampled, output_width, output_height)
    } else {
        // Full globe or resample to output size
        let u_resampled = if grid_width != output_width || grid_height != output_height {
            gradient::resample_grid(&u_data, grid_width, grid_height, output_width, output_height)
        } else {
            u_data.clone()
        };

        let v_resampled = if grid_width != output_width || grid_height != output_height {
            gradient::resample_grid(&v_data, grid_width, grid_height, output_width, output_height)
        } else {
            v_data.clone()
        };

        (u_resampled, v_resampled, output_width, output_height)
    };

    // Render wind barbs
    let config = BarbConfig::default();
    
    info!(
        render_width = render_width,
        render_height = render_height,
        barb_spacing = config.spacing,
        barb_size = config.size,
        "Rendering wind barbs"
    );
    
    // Use geographically-aligned positioning when bbox is available
    // This ensures barbs align across tile boundaries
    let barb_pixels = if let Some(bbox) = bbox {
        barbs::render_wind_barbs_aligned(
            &u_to_render,
            &v_to_render,
            render_width,
            render_height,
            bbox,
            &config,
        )
    } else {
        barbs::render_wind_barbs(
            &u_to_render,
            &v_to_render,
            render_width,
            render_height,
            &config,
        )
    };
    
    // Debug: check rendered pixels
    let non_transparent = barb_pixels.chunks(4).filter(|c| c[3] > 0).count();
    info!(
        non_transparent = non_transparent,
        total_pixels = render_width * render_height,
        "Wind barb rendering complete"
    );

    // Encode as PNG
    renderer::png::create_png(&barb_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

/// Render numeric values at grid points for a parameter.
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving GRIB2 files
/// - `grid_cache`: Optional grid data cache for parsed grids
/// - `catalog`: Catalog for finding datasets
/// - `metrics`: Metrics collector for performance tracking
/// - `grid_processor_factory`: Optional factory for Zarr-based grid access
/// - `model`: Weather model name (e.g., "gfs")
/// - `parameter`: Parameter name (e.g., "TMP")
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat]
/// - `style_path`: Path to style configuration file (for color mapping)
/// - `forecast_hour`: Optional forecast hour
/// - `level`: Optional vertical level
/// - `use_mercator`: Use Web Mercator projection
///
/// # Returns
/// PNG image data as bytes
pub async fn render_numbers_tile(
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    grid_processor_factory: Option<&GridProcessorFactory>,
    model: &str,
    parameter: &str,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    style_path: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
    use_mercator: bool,
) -> Result<Vec<u8>, String> {
    // Load style configuration for color mapping
    let style_json = std::fs::read_to_string(style_path)
        .map_err(|e| format!("Failed to read style file: {}", e))?;
    let style_config: StyleConfig = serde_json::from_str(&style_json)
        .map_err(|e| format!("Failed to parse style JSON: {}", e))?;

    // Get the first style definition (there should only be one in the file)
    let style_def = style_config.styles.values().next()
        .ok_or_else(|| "No style definition found in style file".to_string())?;

    // Get dataset for this parameter, optionally at a specific level
    let entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, parameter, hour, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {} level {}", model, parameter, hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, parameter, hour)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {}", model, parameter, hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at level {}", model, parameter, lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, parameter)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?
        }
    };

    // Load grid data using Zarr-aware loader (handles Zarr, NetCDF, and GRIB2)
    let grid_result = load_grid_data_with_zarr_support(
        grid_processor_factory,
        grib_cache,
        grid_cache,
        metrics,
        &entry,
        parameter,
        Some(bbox),
    ).await?;
    
    let (grid_data, grid_width, grid_height) = (grid_result.data, grid_result.width, grid_result.height);

    info!(
        parameter = parameter,
        grid_width = grid_width,
        grid_height = grid_height,
        has_zarr = entry.zarr_metadata.is_some(),
        "Loaded grid data for numbers rendering"
    );

    // Get data bounds from catalog entry
    let data_bounds = [
        entry.bbox.min_x as f32,
        entry.bbox.min_y as f32,
        entry.bbox.max_x as f32,
        entry.bbox.max_y as f32,
    ];
    
    // Check if grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = entry.bbox.min_x >= 0.0 && entry.bbox.max_x > 180.0;

    // Resample data to the render bbox
    let resampled_data = resample_grid_for_bbox(
        &grid_data,
        grid_width,
        grid_height,
        width as usize,
        height as usize,
        bbox,
        data_bounds,
        use_mercator,
        model,
        grid_uses_360,
    );

    // Convert flat array to 2D grid for numbers renderer
    let grid_2d: Vec<Vec<f32>> = (0..height as usize)
        .map(|y| {
            (0..width as usize)
                .map(|x| resampled_data[y * width as usize + x])
                .collect()
        })
        .collect();

    // Determine unit conversion (e.g., K to C for temperature)
    let unit_conversion = if parameter.contains("TMP") || parameter.contains("TEMP") {
        Some(273.15) // Kelvin to Celsius
    } else {
        None
    };

    // Build NumbersConfig
    let numbers_config = NumbersConfig {
        spacing: 50, // 50 pixels between numbers
        font_size: 12.0,
        color_stops: style_def.stops.clone(),
        unit_conversion,
    };

    // Render numbers
    let img = numbers::render_numbers(&grid_2d, width, height, &numbers_config);

    // Convert RgbaImage to flat RGBA buffer
    let pixels = img.into_raw();

    // Encode as PNG
    renderer::png::create_png(&pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

/// Render numbers by directly querying Zarr data at exact grid point locations.
/// This ensures the displayed values exactly match what GetFeatureInfo would return.
async fn render_numbers_direct_zarr(
    factory: &GridProcessorFactory,
    entry: &CatalogEntry,
    width: u32,
    height: u32,
    render_bbox: [f64; 4],
    visible_bbox: Option<[f64; 4]>,
    color_stops: &[renderer::numbers::ColorStop],
    unit_transform: numbers::UnitTransform,
    min_pixel_spacing: u32,
) -> Result<image::RgbaImage, String> {
    use grid_processor::{
        GridProcessor, ZarrGridProcessor, ZarrMetadata,
        MinioConfig, create_minio_storage,
    };
    
    // Parse zarr_metadata
    let zarr_json = entry.zarr_metadata.as_ref()
        .ok_or_else(|| "No zarr_metadata in catalog entry".to_string())?;
    
    let zarr_meta = ZarrMetadata::from_json(zarr_json)
        .map_err(|e| format!("Failed to parse zarr_metadata: {}", e))?;
    
    // Extract grid info
    let source_bbox = [
        zarr_meta.bbox.min_lon,
        zarr_meta.bbox.min_lat,
        zarr_meta.bbox.max_lon,
        zarr_meta.bbox.max_lat,
    ];
    let source_dims = zarr_meta.shape;
    let source_uses_360 = zarr_meta.bbox.min_lon >= 0.0 && zarr_meta.bbox.max_lon > 180.0;
    
    // Build storage path
    let zarr_path = if entry.storage_path.starts_with('/') {
        entry.storage_path.clone()
    } else {
        format!("/{}", entry.storage_path)
    };
    
    // Create MinIO storage
    let minio_config = MinioConfig::from_env();
    let store = create_minio_storage(&minio_config)
        .map_err(|e| format!("Failed to create MinIO storage: {}", e))?;
    
    // Create GridMetadata for the processor
    let grid_metadata = grid_processor::GridMetadata {
        model: entry.model.clone(),
        parameter: entry.parameter.clone(),
        level: zarr_meta.level.clone(),
        units: zarr_meta.units.clone(),
        reference_time: zarr_meta.reference_time,
        forecast_hour: zarr_meta.forecast_hour,
        bbox: zarr_meta.bbox,
        shape: zarr_meta.shape,
        chunk_shape: zarr_meta.chunk_shape,
        num_chunks: zarr_meta.num_chunks,
        fill_value: zarr_meta.fill_value,
    };
    
    // Create processor
    let processor = ZarrGridProcessor::with_metadata(
        store,
        &zarr_path,
        grid_metadata,
        factory.chunk_cache(),
        factory.config().clone(),
    ).map_err(|e| format!("Failed to open Zarr: {}", e))?;
    
    // Image types
    use image::{ImageBuffer, Rgba, RgbaImage};
    use imageproc::drawing::draw_text_mut;
    use rusttype::{Font, Scale};
    
    // Font setup - use the same font as the renderer crate
    let font_data: &[u8] = include_bytes!("../../../crates/renderer/assets/DejaVuSansMono.ttf");
    let font = Font::try_from_bytes(font_data).expect("Failed to load font");
    let font_size = 12.0f32;
    let scale = Scale::uniform(font_size);
    
    let mut img: RgbaImage = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 0]));
    
    let [render_min_lon, render_min_lat, render_max_lon, render_max_lat] = render_bbox;
    let render_lon_range = render_max_lon - render_min_lon;
    let render_lat_range = render_max_lat - render_min_lat;
    
    let [source_min_lon, source_min_lat, source_max_lon, source_max_lat] = source_bbox;
    let (source_width, source_height) = source_dims;
    
    // Calculate source grid resolution
    let source_lon_step = (source_max_lon - source_min_lon) / (source_width - 1) as f64;
    let source_lat_step = (source_max_lat - source_min_lat) / (source_height - 1) as f64;
    
    // Get visible area
    let [vis_min_lon, vis_min_lat, vis_max_lon, vis_max_lat] = visible_bbox.unwrap_or(render_bbox);
    
    // Helper to convert longitude from -180/180 to 0-360 format if needed
    let to_source_lon = |lon: f64| -> f64 {
        if source_uses_360 && lon < 0.0 { lon + 360.0 } else { lon }
    };
    
    // Helper to convert longitude from 0-360 to -180/180 format
    let from_source_lon = |lon: f64| -> f64 {
        if source_uses_360 && lon > 180.0 { lon - 360.0 } else { lon }
    };
    
    // Convert render bbox to source coordinates
    let render_min_lon_src = to_source_lon(render_min_lon);
    let render_max_lon_src = to_source_lon(render_max_lon);
    
    // Calculate step to skip grid points if they would be too close together
    let pixels_per_source_lon = (width as f64 / render_lon_range) * source_lon_step;
    let pixels_per_source_lat = (height as f64 / render_lat_range) * source_lat_step;
    
    let step_x = ((min_pixel_spacing as f64 / pixels_per_source_lon).ceil() as usize).max(1);
    let step_y = ((min_pixel_spacing as f64 / pixels_per_source_lat).ceil() as usize).max(1);
    
    // Find source grid indices that fall within the render bbox
    let start_i = ((render_min_lon_src - source_min_lon) / source_lon_step).floor() as i64;
    let end_i = ((render_max_lon_src - source_min_lon) / source_lon_step).ceil() as i64;
    let start_j = ((render_min_lat - source_min_lat) / source_lat_step).floor() as i64;
    let end_j = ((render_max_lat - source_min_lat) / source_lat_step).ceil() as i64;
    
    // Align to step boundaries for consistent placement across tiles
    let start_i = (start_i / step_x as i64) * step_x as i64;
    let start_j = (start_j / step_y as i64) * step_y as i64;
    
    tracing::info!(
        source_dims = ?source_dims,
        source_lon_step = source_lon_step,
        source_lat_step = source_lat_step,
        step_x = step_x,
        step_y = step_y,
        start_i = start_i,
        end_i = end_i,
        start_j = start_j,
        end_j = end_j,
        "Rendering numbers with direct Zarr queries"
    );
    
    // Iterate over source grid points
    let mut j = start_j;
    while j <= end_j {
        if j < 0 || j >= source_height as i64 {
            j += step_y as i64;
            continue;
        }
        
        let mut i = start_i;
        while i <= end_i {
            if i < 0 || i >= source_width as i64 {
                i += step_x as i64;
                continue;
            }
            
            // Calculate geographic position of this grid point (in source coordinates)
            let geo_lon_src = source_min_lon + (i as f64 * source_lon_step);
            let geo_lat = source_min_lat + (j as f64 * source_lat_step);
            
            // Convert to display coordinates (-180 to 180)
            let geo_lon = from_source_lon(geo_lon_src);
            
            // Check if this position is in the visible area
            let text_buffer_lon = source_lon_step * 0.5;
            let text_buffer_lat = source_lat_step * 0.5;
            if geo_lon < vis_min_lon - text_buffer_lon || geo_lon > vis_max_lon + text_buffer_lon ||
               geo_lat < vis_min_lat - text_buffer_lat || geo_lat > vis_max_lat + text_buffer_lat {
                i += step_x as i64;
                continue;
            }
            
            // Convert geographic to pixel coordinates
            let px = ((geo_lon - render_min_lon) / render_lon_range * width as f64) as i32;
            let py = ((render_max_lat - geo_lat) / render_lat_range * height as f64) as i32;
            
            // Skip if outside render area
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                i += step_x as i64;
                continue;
            }
            
            // Query the exact value from Zarr at this grid cell
            // Note: Zarr row is from north to south, so we need to flip the j index
            let row = (source_height - 1) as i64 - j;
            let value = match processor.read_grid_cell(i as usize, row as usize).await {
                Ok(Some(v)) => v,
                Ok(None) => {
                    i += step_x as i64;
                    continue;
                }
                Err(e) => {
                    tracing::debug!(error = %e, col = i, row = row, "Failed to read grid cell");
                    i += step_x as i64;
                    continue;
                }
            };
            
            // Apply unit transformation
            let display_value = unit_transform.apply(value);
            
            // Format the value
            let text = renderer::numbers::format_value(display_value);
            
            // Get color for this value (use transformed value for color mapping)
            let color = renderer::numbers::get_color_for_value(display_value, color_stops);
            
            // Calculate text dimensions for centering
            let char_width = (font_size * 0.6) as i32;
            let text_width = (text.len() as i32) * char_width;
            let text_height = font_size as i32;
            
            // Center the text on the grid point
            let centered_px = px - text_width / 2;
            let centered_py = py - text_height / 2;
            
            // Draw background
            let bg_color = Rgba([255, 255, 255, 220]);
            let padding = 2i32;
            for dy in -padding..(text_height + padding) {
                for dx in -padding..(text_width + padding) {
                    let bx = centered_px + dx;
                    let by = centered_py + dy;
                    if bx >= 0 && bx < width as i32 && by >= 0 && by < height as i32 {
                        img.put_pixel(bx as u32, by as u32, bg_color);
                    }
                }
            }
            
            // Draw the text
            draw_text_mut(&mut img, color, centered_px, centered_py, scale, &font, &text);
            
            i += step_x as i64;
        }
        j += step_y as i64;
    }
    
    Ok(img)
}

/// Render numeric values at grid points with tile buffer for seamless edges.
/// This renders a 3x3 grid of tiles and crops the center to ensure numbers
/// at tile boundaries are not clipped.
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving GRIB2 files
/// - `grid_cache`: Optional grid data cache for parsed grids
/// - `catalog`: Catalog for finding datasets
/// - `metrics`: Metrics collector for performance tracking
/// - `grid_processor_factory`: Optional factory for Zarr-based grid access
/// - `model`: Weather model name (e.g., "gfs")
/// - `parameter`: Parameter name (e.g., "TMP")
/// - `tile_coord`: Tile coordinate for expanded rendering
/// - `width`: Output image width (single tile)
/// - `height`: Output image height (single tile)
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat] for the single tile
/// - `style_path`: Path to style configuration file (for color mapping)
/// - `forecast_hour`: Optional forecast hour
/// - `level`: Optional vertical level
/// - `use_mercator`: Use Web Mercator projection
///
/// # Returns
/// PNG image data as bytes
pub async fn render_numbers_tile_with_buffer(
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    grid_processor_factory: Option<&GridProcessorFactory>,
    model: &str,
    parameter: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    style_path: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
    use_mercator: bool,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::{ExpandedTileConfig, expanded_tile_bbox, crop_center_tile, actual_expanded_dimensions, tile_bbox};
    
    // Load style configuration for color mapping
    let style_json = std::fs::read_to_string(style_path)
        .map_err(|e| format!("Failed to read style file: {}", e))?;
    let style_config: StyleConfig = serde_json::from_str(&style_json)
        .map_err(|e| format!("Failed to parse style JSON: {}", e))?;

    // Get the first style definition (there should only be one in the file)
    let style_def = style_config.styles.values().next()
        .ok_or_else(|| "No style definition found in style file".to_string())?;

    // Get dataset for this parameter, optionally at a specific level
    let entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, parameter, hour, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {} level {}", model, parameter, hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, parameter, hour)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {}", model, parameter, hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at level {}", model, parameter, lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, parameter)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?
        }
    };

    // Determine if we should use expanded rendering
    let (render_bbox, render_width, render_height, needs_crop) = if let Some(coord) = tile_coord {
        let config = ExpandedTileConfig::tiles_3x3();
        let expanded_bbox = expanded_tile_bbox(&coord, &config);
        
        // Calculate actual expanded dimensions
        let (exp_w, exp_h) = actual_expanded_dimensions(&coord, &config);
        
        (
            [
                expanded_bbox.min_x as f32,
                expanded_bbox.min_y as f32,
                expanded_bbox.max_x as f32,
                expanded_bbox.max_y as f32,
            ],
            exp_w as usize,
            exp_h as usize,
            Some((coord, config)),
        )
    } else {
        (bbox, width as usize, height as usize, None)
    };

    info!(
        render_width = render_width,
        render_height = render_height,
        bbox_min_lon = render_bbox[0],
        bbox_max_lon = render_bbox[2],
        expanded = needs_crop.is_some(),
        "Rendering numbers tile"
    );

    // Load grid data using Zarr-aware loader
    let grid_result = load_grid_data_with_zarr_support(
        grid_processor_factory,
        grib_cache,
        grid_cache,
        metrics,
        &entry,
        parameter,
        Some(render_bbox),
    ).await?;
    
    let (grid_data, grid_width, grid_height) = (grid_result.data, grid_result.width, grid_result.height);

    // Get data bounds from catalog entry
    let data_bounds = [
        entry.bbox.min_x as f32,
        entry.bbox.min_y as f32,
        entry.bbox.max_x as f32,
        entry.bbox.max_y as f32,
    ];
    
    // Check if grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = entry.bbox.min_x >= 0.0 && entry.bbox.max_x > 180.0;
    
    // Get full source grid dimensions from zarr_metadata if available
    // This is needed for placing numbers at exact grid point locations
    let source_grid_dims: Option<(usize, usize)> = if let Some(ref zarr_json) = entry.zarr_metadata {
        use grid_processor::ZarrMetadata;
        ZarrMetadata::from_json(zarr_json).ok().map(|meta| meta.shape)
    } else {
        None
    };

    // Resample data to the render bbox
    let resampled_data = resample_grid_for_bbox(
        &grid_data,
        grid_width,
        grid_height,
        render_width,
        render_height,
        render_bbox,
        data_bounds,
        use_mercator,
        model,
        grid_uses_360,
    );

    // Convert flat array to 2D grid for numbers renderer
    let grid_2d: Vec<Vec<f32>> = (0..render_height)
        .map(|y| {
            (0..render_width)
                .map(|x| resampled_data[y * render_width + x])
                .collect()
        })
        .collect();

    // Determine unit transform from style config
    // The transform field specifies how to convert native units (e.g., Pa) to display units (e.g., hPa)
    let unit_transform = style_def.transform.as_ref().map(|t| {
        match t.transform_type.to_lowercase().as_str() {
            "k_to_c" | "kelvin_to_celsius" => numbers::UnitTransform::Subtract(273.15),
            "pa_to_hpa" => numbers::UnitTransform::Divide(100.0),
            "m_to_km" => numbers::UnitTransform::Divide(1000.0),
            "mps_to_knots" => numbers::UnitTransform::Divide(0.514444), // multiply by ~1.94
            _ => numbers::UnitTransform::None,
        }
    }).unwrap_or_else(|| {
        // Fallback: detect from parameter name for backwards compatibility
        if parameter.contains("TMP") || parameter.contains("TEMP") {
            numbers::UnitTransform::Subtract(273.15)
        } else if parameter.contains("PRES") || parameter.contains("PRESS") || parameter.contains("PRMSL") {
            numbers::UnitTransform::Divide(100.0)
        } else {
            numbers::UnitTransform::None
        }
    });
    
    // Legacy format for backwards compatibility (None means use unit_transform)
    let unit_conversion: Option<f32> = None;

    // Render numbers at exact source grid point locations
    // For accurate display, we query the Zarr data directly at each grid point
    let final_pixels = if let Some((coord, tile_config)) = needs_crop {
        // Calculate center tile bbox for visibility filtering
        let center_bbox = wms_common::tile::tile_bbox(&coord);
        let visible_bbox = [
            center_bbox.min_x,
            center_bbox.min_y,
            center_bbox.max_x,
            center_bbox.max_y,
        ];
        
        // Keep source bbox in its original format (0-360 for GFS)
        let source_bbox = [
            data_bounds[0] as f64,
            data_bounds[1] as f64,
            data_bounds[2] as f64,
            data_bounds[3] as f64,
        ];
        
        // Use full source grid dimensions if available (from zarr_metadata)
        let full_source_dims = source_grid_dims.unwrap_or((grid_width, grid_height));
        
        // Try to use direct Zarr queries for exact values
        let img = if let (Some(factory), Some(_zarr_meta)) = (grid_processor_factory, &entry.zarr_metadata) {
            // Render using direct Zarr queries
            match render_numbers_direct_zarr(
                factory,
                &entry,
                render_width as u32,
                render_height as u32,
                [render_bbox[0] as f64, render_bbox[1] as f64, render_bbox[2] as f64, render_bbox[3] as f64],
                Some(visible_bbox),
                &style_def.stops,
                unit_transform,
                40, // min_pixel_spacing
            ).await {
                Ok(img) => img,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to render with direct Zarr queries, falling back to resampled data");
                    // Fall back to resampled data
                    let grid_point_config = numbers::GridPointNumbersConfig {
                        font_size: 12.0,
                        color_stops: style_def.stops.clone(),
                        unit_conversion,
                        unit_transform,
                        render_bbox: [render_bbox[0] as f64, render_bbox[1] as f64, render_bbox[2] as f64, render_bbox[3] as f64],
                        source_bbox,
                        source_dims: full_source_dims,
                        visible_bbox: Some(visible_bbox),
                        min_pixel_spacing: 40,
                        source_uses_360: grid_uses_360,
                    };
                    numbers::render_numbers_at_grid_points(&grid_2d, render_width as u32, render_height as u32, &grid_point_config)
                }
            }
        } else {
            // No Zarr access, use resampled data
            let grid_point_config = numbers::GridPointNumbersConfig {
                font_size: 12.0,
                color_stops: style_def.stops.clone(),
                unit_conversion,
                unit_transform,
                render_bbox: [render_bbox[0] as f64, render_bbox[1] as f64, render_bbox[2] as f64, render_bbox[3] as f64],
                source_bbox,
                source_dims: full_source_dims,
                visible_bbox: Some(visible_bbox),
                min_pixel_spacing: 40,
                source_uses_360: grid_uses_360,
            };
            numbers::render_numbers_at_grid_points(&grid_2d, render_width as u32, render_height as u32, &grid_point_config)
        };
        
        let expanded_pixels = img.into_raw();
        
        // Crop to center tile
        crop_center_tile(&expanded_pixels, render_width as u32, &coord, &tile_config)
    } else {
        // No tile coordinate - fall back to regular rendering
        let numbers_config = NumbersConfig {
            spacing: 50,
            font_size: 12.0,
            color_stops: style_def.stops.clone(),
            unit_conversion,
        };
        let img = numbers::render_numbers(&grid_2d, render_width as u32, render_height as u32, &numbers_config);
        img.into_raw()
    };

    // Encode as PNG
    renderer::png::create_png(&final_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

/// Query data value at a specific point for GetFeatureInfo
///
/// # Arguments
/// - `grib_cache`: Cache for GRIB2/NetCDF file data
/// - `catalog`: Catalog for finding datasets
/// - `metrics`: Metrics collector
/// - `grid_processor_factory`: Optional factory for Zarr-based point queries (more efficient)
/// - `layer`: Layer name (e.g., "gfs_TMP")
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat]
/// - `width`: Map width in pixels
/// - `height`: Map height in pixels
/// - `i`: Pixel column (0-based from left)
/// - `j`: Pixel row (0-based from top)
/// - `crs`: Coordinate reference system (e.g., "EPSG:4326", "EPSG:3857")
/// - `forecast_hour`: Optional forecast hour; if None, uses latest
/// - `level`: Optional vertical level/elevation (e.g., "500 mb", "2 m above ground")
///
/// # Returns
/// Vector of FeatureInfo with data values at the queried point
pub async fn query_point_value(
    grib_cache: &GribCache,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    grid_processor_factory: Option<&GridProcessorFactory>,
    layer: &str,
    bbox: [f64; 4],
    width: u32,
    height: u32,
    i: u32,
    j: u32,
    crs: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<Vec<wms_protocol::FeatureInfo>, String> {
    use wms_protocol::{pixel_to_geographic, mercator_to_wgs84, FeatureInfo, Location};
    
    // Parse layer name (format: "model_parameter")
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return Err("Invalid layer format".to_string());
    }
    
    let model = parts[0];
    let parameter = parts[1..].join("_");
    
    // Convert pixel coordinates to geographic coordinates
    // Note: bbox is already in [min_lon, min_lat, max_lon, max_lat] format
    // (the handler has already converted from WMS 1.3.0 axis order if needed)
    let (lon, lat) = if crs.contains("3857") {
        // Web Mercator - convert bbox from meters to degrees
        let [min_x, min_y, max_x, max_y] = bbox;
        let (min_lon, min_lat) = mercator_to_wgs84(min_x, min_y);
        let (max_lon, max_lat) = mercator_to_wgs84(max_x, max_y);
        pixel_to_geographic(i, j, width, height, [min_lon, min_lat, max_lon, max_lat])
    } else {
        // EPSG:4326 - bbox is already [min_lon, min_lat, max_lon, max_lat]
        pixel_to_geographic(i, j, width, height, bbox)
    };
    
    info!(
        layer = layer,
        lon = lon,
        lat = lat,
        pixel_i = i,
        pixel_j = j,
        level = ?level,
        "GetFeatureInfo query"
    );
    
    // Handle special composite layers
    if parameter == "WIND_BARBS" {
        return query_wind_barbs_value(grib_cache, catalog, model, lon, lat, forecast_hour, level).await;
    }
    
    // Get dataset for this parameter, optionally at a specific level
    let entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, &parameter, hour, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {} level {}", model, parameter, hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, &parameter, hour)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {}", model, parameter, hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, &parameter, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at level {}", model, parameter, lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, &parameter)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?
        }
    };
    
    // Determine data source type and load value
    // Priority: Zarr (most efficient for point queries) > NetCDF > GRIB2
    let has_zarr = entry.zarr_metadata.is_some() && grid_processor_factory.is_some();
    let is_netcdf = entry.storage_path.ends_with(".nc") || 
                    entry.parameter.starts_with("CMI_") ||
                    model.starts_with("goes");
    
    // Load and sample grid data
    let value = if has_zarr {
        // Use Zarr for efficient point query (reads only one chunk)
        let factory = grid_processor_factory.unwrap();
        match query_point_from_zarr(factory, &entry, lon, lat).await? {
            Some(v) => v,
            None => {
                // Point outside grid or fill value - return no-data response
                return Ok(vec![wms_protocol::FeatureInfo {
                    layer_name: layer.to_string(),
                    parameter: parameter.clone(),
                    value: f64::NAN,
                    unit: "N/A".to_string(),
                    raw_value: f64::NAN,
                    raw_unit: "no data".to_string(),
                    location: wms_protocol::Location {
                        longitude: lon,
                        latitude: lat,
                    },
                    forecast_hour: Some(entry.forecast_hour),
                    reference_time: Some(entry.reference_time.to_rfc3339()),
                    level: Some(entry.level.clone()),
                }]);
            }
        }
    } else if is_netcdf {
        // Handle NetCDF (GOES satellite) data
        let grid_result = load_grid_data(grib_cache, None, metrics, &entry, &parameter).await?;
        let grid_data = grid_result.data;
        let grid_width = grid_result.width;
        let grid_height = grid_result.height;
        let goes_projection = grid_result.goes_projection;
        
        // Sample value at the point using projection-aware sampling
        sample_grid_value_with_projection(&grid_data, grid_width, grid_height, lon, lat, model, goes_projection.as_ref())?
    } else {
        // Handle GRIB2 data (legacy fallback)
        let grib_data = grib_cache
            .get(&entry.storage_path)
            .await
            .map_err(|e| format!("Failed to load GRIB2 file: {}", e))?;
        
        // Parse GRIB2 and find parameter with matching level
        let msg = find_parameter_in_grib(grib_data, &parameter, Some(&entry.level))?;
        
        // Unpack grid data
        let grid_data = msg
            .unpack_data()
            .map_err(|e| format!("Unpacking failed: {}", e))?;
        
        let (grid_height, grid_width) = msg.grid_dims();
        let grid_width = grid_width as usize;
        let grid_height = grid_height as usize;
        
        // Sample value at the point using bilinear interpolation
        sample_grid_value(&grid_data, grid_width, grid_height, lon, lat, model)?
    };
    
    // Check for missing/no-data values (MRMS uses -99 and -999)
    const MISSING_VALUE_THRESHOLD: f32 = -90.0;
    if value <= MISSING_VALUE_THRESHOLD || value.is_nan() {
        // Return a "no data" response with NaN value
        // The JSON serialization will handle NaN appropriately
        return Ok(vec![FeatureInfo {
            layer_name: layer.to_string(),
            parameter: parameter.clone(),
            value: f64::NAN,
            unit: "N/A".to_string(),
            raw_value: value as f64,
            raw_unit: "no data".to_string(),
            location: Location {
                longitude: lon,
                latitude: lat,
            },
            forecast_hour: Some(entry.forecast_hour),
            reference_time: Some(entry.reference_time.to_rfc3339()),
            level: Some(entry.level.clone()),
        }]);
    }
    
    // Convert value based on parameter type
    let (display_value, display_unit, raw_unit, param_name) = convert_parameter_value(&parameter, value);
    
    Ok(vec![FeatureInfo {
        layer_name: layer.to_string(),
        parameter: param_name,
        value: display_value,
        unit: display_unit,
        raw_value: value as f64,
        raw_unit,
        location: Location {
            longitude: lon,
            latitude: lat,
        },
        forecast_hour: Some(entry.forecast_hour),
        reference_time: Some(entry.reference_time.to_rfc3339()),
        level: Some(entry.level.clone()),
    }])
}

/// Query wind barbs value (combines UGRD and VGRD)
async fn query_wind_barbs_value(
    grib_cache: &GribCache,
    catalog: &Catalog,
    model: &str,
    lon: f64,
    lat: f64,
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<Vec<wms_protocol::FeatureInfo>, String> {
    use wms_protocol::{FeatureInfo, Location};
    
    // Load U component, optionally at a specific level
    let u_entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, "UGRD", hour, lev)
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| format!("No UGRD data available for hour {} level {}", hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, "UGRD", hour)
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| format!("No UGRD data available for hour {}", hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, "UGRD", lev)
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| format!("No UGRD data available at level {}", lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, "UGRD")
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| "No UGRD data available".to_string())?
        }
    };
    
    // Load V component, optionally at a specific level
    let v_entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, "VGRD", hour, lev)
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| format!("No VGRD data available for hour {} level {}", hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, "VGRD", hour)
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| format!("No VGRD data available for hour {}", hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, "VGRD", lev)
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| format!("No VGRD data available at level {}", lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, "VGRD")
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| "No VGRD data available".to_string())?
        }
    };
    
    // Load GRIB2 files from cache
    let u_grib = grib_cache.get(&u_entry.storage_path).await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;
    let v_grib = grib_cache.get(&v_entry.storage_path).await
        .map_err(|e| format!("Failed to load VGRD file: {}", e))?;
    
    // Parse and unpack - use level from catalog entry
    let u_msg = find_parameter_in_grib(u_grib, "UGRD", Some(&u_entry.level))?;
    let v_msg = find_parameter_in_grib(v_grib, "VGRD", Some(&v_entry.level))?;
    
    let u_data = u_msg.unpack_data().map_err(|e| format!("Unpacking U failed: {}", e))?;
    let v_data = v_msg.unpack_data().map_err(|e| format!("Unpacking V failed: {}", e))?;
    
    let (grid_height, grid_width) = u_msg.grid_dims();
    let grid_width = grid_width as usize;
    let grid_height = grid_height as usize;
    
    // Sample both components
    let u = sample_grid_value(&u_data, grid_width, grid_height, lon, lat, model)?;
    let v = sample_grid_value(&v_data, grid_width, grid_height, lon, lat, model)?;
    
    // Calculate speed and direction
    let speed = (u * u + v * v).sqrt();
    let direction_rad = v.atan2(u);
    let direction_deg = direction_rad.to_degrees();
    // Convert from mathematical angle to meteorological (from north, clockwise)
    let wind_direction = (270.0 - direction_deg).rem_euclid(360.0);
    
    Ok(vec![
        FeatureInfo {
            layer_name: format!("{}_WIND_BARBS", model),
            parameter: "Wind Speed".to_string(),
            value: speed as f64,
            unit: "m/s".to_string(),
            raw_value: speed as f64,
            raw_unit: "m/s".to_string(),
            location: Location { longitude: lon, latitude: lat },
            forecast_hour: Some(u_entry.forecast_hour),
            reference_time: Some(u_entry.reference_time.to_rfc3339()),
            level: Some(u_entry.level.clone()),
        },
        FeatureInfo {
            layer_name: format!("{}_WIND_BARBS", model),
            parameter: "Wind Direction".to_string(),
            value: wind_direction as f64,
            unit: "degrees".to_string(),
            raw_value: wind_direction as f64,
            raw_unit: "degrees".to_string(),
            location: Location { longitude: lon, latitude: lat },
            forecast_hour: Some(u_entry.forecast_hour),
            reference_time: Some(u_entry.reference_time.to_rfc3339()),
            level: Some(u_entry.level.clone()),
        },
    ])
}

/// Sample a grid value at a geographic point using bilinear interpolation
fn sample_grid_value(
    grid_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    lon: f64,
    lat: f64,
    model: &str,
) -> Result<f32, String> {
    // Handle HRRR's Lambert Conformal projection
    if model == "hrrr" {
        return sample_lambert_grid_value(grid_data, grid_width, grid_height, lon, lat);
    }
    
    // Handle MRMS regional lat/lon grid
    if model == "mrms" {
        return sample_mrms_grid_value(grid_data, grid_width, grid_height, lon, lat);
    }
    
    // GFS and other models use global lat/lon grids: lat 90 to -90, lon 0 to 360
    let lon_step = 360.0 / grid_width as f64;
    let lat_step = 180.0 / grid_height as f64;
    
    // Normalize longitude to 0-360
    let norm_lon = if lon < 0.0 { lon + 360.0 } else { lon };
    
    // Convert to grid coordinates
    let grid_x = norm_lon / lon_step;
    let grid_y = (90.0 - lat) / lat_step;
    
    // Bounds check
    if grid_x < 0.0 || grid_y < 0.0 || grid_x >= grid_width as f64 || grid_y >= grid_height as f64 {
        return Err(format!("Point ({}, {}) outside grid bounds", lon, lat));
    }
    
    bilinear_interpolate(grid_data, grid_width, grid_height, grid_x, grid_y, true)
}

/// Sample a Lambert Conformal grid (HRRR) at a geographic point
fn sample_lambert_grid_value(
    grid_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    lon: f64,
    lat: f64,
) -> Result<f32, String> {
    // Create HRRR projection
    let proj = LambertConformal::hrrr();
    
    // Convert geographic coordinates (lat, lon) to Lambert grid coordinates (i, j)
    let (grid_x, grid_y) = proj.geo_to_grid(lat, lon);
    
    // Bounds check
    if grid_x < 0.0 || grid_y < 0.0 || grid_x >= grid_width as f64 || grid_y >= grid_height as f64 {
        return Err(format!("Point ({}, {}) outside HRRR grid bounds (grid coords: {}, {})", lon, lat, grid_x, grid_y));
    }
    
    bilinear_interpolate(grid_data, grid_width, grid_height, grid_x, grid_y, false)
}

/// Sample an MRMS regional lat/lon grid at a geographic point
/// MRMS grid: 7000x3500, lat 54.995° to 20.005°, lon -129.995° to -60.005° (0.01° resolution)
fn sample_mrms_grid_value(
    grid_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    lon: f64,
    lat: f64,
) -> Result<f32, String> {
    // MRMS grid parameters (from GRIB2 grid definition)
    // Grid starts at NW corner: (54.995°N, -129.995°E) = (54.995°N, 230.005°E in 0-360)
    // Resolution: 0.01° in both directions
    // Scan mode 0: rows go from north to south, columns go from west to east
    let first_lat = 54.995;   // Northern edge
    let last_lat = 20.005;    // Southern edge  
    let first_lon = -129.995; // Western edge
    let last_lon = -60.005;   // Eastern edge
    
    // Calculate step sizes from grid dimensions
    let lon_step = (last_lon - first_lon) / (grid_width as f64 - 1.0);  // ~0.01°
    let lat_step = (first_lat - last_lat) / (grid_height as f64 - 1.0); // ~0.01°
    
    // Bounds check
    if lat < last_lat || lat > first_lat || lon < first_lon || lon > last_lon {
        return Err(format!(
            "Point ({}, {}) outside MRMS grid bounds (lon: {} to {}, lat: {} to {})",
            lon, lat, first_lon, last_lon, last_lat, first_lat
        ));
    }
    
    // Convert to grid coordinates
    // Column: distance from west edge divided by lon step
    let grid_x = (lon - first_lon) / lon_step;
    // Row: distance from north edge divided by lat step (rows go south)
    let grid_y = (first_lat - lat) / lat_step;
    
    // Final bounds check on grid coordinates
    if grid_x < 0.0 || grid_y < 0.0 || grid_x >= grid_width as f64 || grid_y >= grid_height as f64 {
        return Err(format!(
            "Point ({}, {}) maps to invalid grid coords ({}, {})",
            lon, lat, grid_x, grid_y
        ));
    }
    
    bilinear_interpolate(grid_data, grid_width, grid_height, grid_x, grid_y, false)
}

/// Sample grid value with projection awareness (for GOES geostationary projection)
fn sample_grid_value_with_projection(
    grid_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    lon: f64,
    lat: f64,
    model: &str,
    goes_projection: Option<&GoesProjectionParams>,
) -> Result<f32, String> {
    // Handle HRRR's Lambert Conformal projection
    if model == "hrrr" {
        return sample_lambert_grid_value(grid_data, grid_width, grid_height, lon, lat);
    }
    
    // Handle MRMS regional lat/lon grid
    if model == "mrms" {
        return sample_mrms_grid_value(grid_data, grid_width, grid_height, lon, lat);
    }
    
    // Handle GOES geostationary projection
    if model == "goes16" || model == "goes18" || model == "goes" {
        if let Some(params) = goes_projection {
            let proj = Geostationary::from_goes(
                params.perspective_point_height,
                params.semi_major_axis,
                params.semi_minor_axis,
                params.longitude_origin,
                params.x_origin,
                params.y_origin,
                params.dx,
                params.dy,
                grid_width,
                grid_height,
            );
            
            // Convert geographic to geostationary grid indices
            let grid_coords = proj.geo_to_grid(lat, lon);
            
            let (grid_x, grid_y) = match grid_coords {
                Some((i, j)) => (i, j),
                None => return Err(format!("Point ({}, {}) not visible from satellite", lon, lat)),
            };
            
            // Bounds check
            if grid_x < 0.0 || grid_y < 0.0 || grid_x >= grid_width as f64 - 1.0 || grid_y >= grid_height as f64 - 1.0 {
                return Err(format!("Point ({}, {}) outside GOES coverage (grid coords: {}, {})", lon, lat, grid_x, grid_y));
            }
            
            return bilinear_interpolate(grid_data, grid_width, grid_height, grid_x, grid_y, false);
        } else {
            // Fallback to preset projection
            let satellite_lon = if model == "goes18" { -137.2 } else { -75.0 };
            let proj = if satellite_lon < -100.0 {
                Geostationary::goes18_conus()
            } else {
                Geostationary::goes16_conus()
            };
            
            let grid_coords = proj.geo_to_grid(lat, lon);
            let (grid_x, grid_y) = match grid_coords {
                Some((i, j)) => (i, j),
                None => return Err(format!("Point ({}, {}) not visible from satellite", lon, lat)),
            };
            
            if grid_x < 0.0 || grid_y < 0.0 || grid_x >= grid_width as f64 - 1.0 || grid_y >= grid_height as f64 - 1.0 {
                return Err(format!("Point ({}, {}) outside GOES coverage", lon, lat));
            }
            
            return bilinear_interpolate(grid_data, grid_width, grid_height, grid_x, grid_y, false);
        }
    }
    
    // Fall back to standard geographic grid sampling
    sample_grid_value(grid_data, grid_width, grid_height, lon, lat, model)
}

/// Perform bilinear interpolation at grid coordinates
fn bilinear_interpolate(
    grid_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    grid_x: f64,
    grid_y: f64,
    wrap_longitude: bool,
) -> Result<f32, String> {
    let x1 = grid_x.floor() as usize;
    let y1 = grid_y.floor() as usize;
    let x2 = if wrap_longitude {
        if x1 + 1 >= grid_width { 0 } else { x1 + 1 }
    } else {
        (x1 + 1).min(grid_width - 1)
    };
    let y2 = (y1 + 1).min(grid_height - 1);
    
    let dx = grid_x - x1 as f64;
    let dy = grid_y - y1 as f64;
    
    // Sample four surrounding grid points
    let v11 = grid_data.get(y1 * grid_width + x1).copied().unwrap_or(f32::NAN);
    let v21 = grid_data.get(y1 * grid_width + x2).copied().unwrap_or(f32::NAN);
    let v12 = grid_data.get(y2 * grid_width + x1).copied().unwrap_or(f32::NAN);
    let v22 = grid_data.get(y2 * grid_width + x2).copied().unwrap_or(f32::NAN);
    
    // Bilinear interpolation
    let v1 = v11 * (1.0 - dx as f32) + v21 * dx as f32;
    let v2 = v12 * (1.0 - dx as f32) + v22 * dx as f32;
    let value = v1 * (1.0 - dy as f32) + v2 * dy as f32;
    
    Ok(value)
}

/// Convert parameter value to display format with appropriate units
fn convert_parameter_value(parameter: &str, value: f32) -> (f64, String, String, String) {
    if parameter.contains("TMP") || parameter.contains("TEMP") {
        // Temperature: Kelvin to Celsius
        let celsius = value - 273.15;
        (celsius as f64, "°C".to_string(), "K".to_string(), "Temperature".to_string())
    } else if parameter.contains("PRES") || parameter.contains("PRMSL") {
        // Pressure: Pa to hPa
        let hpa = value / 100.0;
        (hpa as f64, "hPa".to_string(), "Pa".to_string(), "Pressure".to_string())
    } else if parameter.contains("WIND") || parameter.contains("GUST") || parameter.contains("SPEED") {
        // Wind speed: m/s (no conversion)
        (value as f64, "m/s".to_string(), "m/s".to_string(), "Wind Speed".to_string())
    } else if parameter.contains("RH") || parameter.contains("HUMID") {
        // Relative humidity: % (no conversion)
        (value as f64, "%".to_string(), "%".to_string(), "Relative Humidity".to_string())
    } else if parameter.contains("UGRD") {
        (value as f64, "m/s".to_string(), "m/s".to_string(), "U Wind Component".to_string())
    } else if parameter.contains("VGRD") {
        (value as f64, "m/s".to_string(), "m/s".to_string(), "V Wind Component".to_string())
    } else {
        // Generic parameter
        (value as f64, "".to_string(), "".to_string(), parameter.to_string())
    }
}

/// Render isolines (contours) for a parameter using expanded tile rendering.
/// This renders a 3x3 grid of tiles and crops the center to ensure seamless boundaries.
///
/// # Arguments
/// - `storage`: Object storage for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `model`: Weather model name (e.g., "gfs")
/// - `parameter`: Parameter name (e.g., "TMP")
/// - `tile_coord`: Optional tile coordinate for expanded rendering
/// - `width`: Output image width (single tile)
/// - `height`: Output image height (single tile)
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat] for the single tile
/// - `style_path`: Path to contour style configuration file
///
/// # Returns
/// PNG image data as bytes
pub async fn render_isolines_tile(
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    model: &str,
    parameter: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    style_path: &str,
    style_name: &str,
    forecast_hour: Option<u32>,
    use_mercator: bool,
    grid_processor_factory: Option<&GridProcessorFactory>,
) -> Result<Vec<u8>, String> {
    render_isolines_tile_with_level(
        grib_cache, grid_cache, catalog, metrics, model, parameter, tile_coord, width, height, bbox,
        style_path, style_name, forecast_hour, None, use_mercator, grid_processor_factory
    ).await
}

/// Render isolines (contour lines) for a single tile with optional level.
pub async fn render_isolines_tile_with_level(
    grib_cache: &GribCache,
    grid_cache: Option<&GridDataCache>,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    model: &str,
    parameter: &str,
    _tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    style_path: &str,
    style_name: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
    use_mercator: bool,
    grid_processor_factory: Option<&GridProcessorFactory>,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::crop_center_tile;
    
    // Load style configuration - look up the specific style by name
    let style_config = ContourStyle::from_file_with_style(style_path, style_name)
        .map_err(|e| format!("Failed to load contour style '{}' from {}: {}", style_name, style_path, e))?;
    
    // Get dataset for this parameter, optionally at a specific level
    let entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, parameter, hour, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {} level {}", model, parameter, hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, parameter, hour)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {}", model, parameter, hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at level {}", model, parameter, lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, parameter)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?
        }
    };
    
    // Load grid data using the unified loader (handles GRIB2, NetCDF, and Zarr)
    // Pass None for bbox to get full grid (isolines need global min/max for level generation)
    let grid_result = load_grid_data_with_zarr_support(
        grid_processor_factory,
        grib_cache,
        grid_cache,
        metrics,
        &entry,
        parameter,
        None,  // No bbox subset - we need full grid for contour level calculation
    ).await?;
    
    let grid_data = grid_result.data;
    let grid_width = grid_result.width;
    let grid_height = grid_result.height;
    
    // Find global min/max for level generation
    let (min_val, max_val) = grid_data
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &val| {
            (min.min(val), max.max(val))
        });
    
    info!(
        parameter = parameter,
        min_val = min_val,
        max_val = max_val,
        grid_width = grid_width,
        grid_height = grid_height,
        "Loaded grid data for isolines"
    );
    
    // For isolines, we don't use expanded rendering because:
    // 1. Contours are continuous and don't need alignment across tiles like wind barbs
    // 2. Expanded rendering at low zoom can cause the bbox to span the entire world,
    //    leading to projection distortion when cropping in pixel space
    // Instead, we render each tile independently
    let (render_bbox, render_width, render_height, needs_crop) = (bbox, width as usize, height as usize, None);
    
    // Use actual bbox from grid data if available, otherwise fall back to entry.bbox
    let data_bounds = grid_result.bbox.unwrap_or_else(|| [
        entry.bbox.min_x as f32,
        entry.bbox.min_y as f32,
        entry.bbox.max_x as f32,
        entry.bbox.max_y as f32,
    ]);
    
    // Use grid_uses_360 from result (for proper coordinate handling)
    let grid_uses_360 = grid_result.grid_uses_360;
    
    info!(
        render_width = render_width,
        render_height = render_height,
        bbox_min_lon = render_bbox[0],
        bbox_max_lon = render_bbox[2],
        expanded = needs_crop.is_some(),
        "Rendering isolines"
    );
    
    // Resample data to the render bbox
    // Use Mercator projection when rendering for Web Mercator display
    let resampled_data_raw = resample_grid_for_bbox(
        &grid_data,
        grid_width,
        grid_height,
        render_width,
        render_height,
        render_bbox,
        data_bounds,
        use_mercator,
        model,
        grid_uses_360,
    );
    
    // Apply transform to convert data to display units (e.g., Pa -> hPa, K -> C)
    // This ensures contour levels (defined in display units) match the data
    let resampled_data: Vec<f32> = resampled_data_raw.iter()
        .map(|&v| renderer::style::apply_transform(v, style_config.transform.as_ref()))
        .collect();
    
    // Also transform min/max for level generation
    let transformed_min = renderer::style::apply_transform(min_val, style_config.transform.as_ref());
    let transformed_max = renderer::style::apply_transform(max_val, style_config.transform.as_ref());
    
    // Log resampled data stats for debugging (now in display units)
    let resampled_valid: Vec<f32> = resampled_data.iter().filter(|v| !v.is_nan()).copied().collect();
    let resampled_min = resampled_valid.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    let resampled_max = resampled_valid.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let nan_count = resampled_data.iter().filter(|v| v.is_nan()).count();
    
    info!(
        resampled_min = resampled_min,
        resampled_max = resampled_max,
        valid_count = resampled_valid.len(),
        nan_count = nan_count,
        total = resampled_data.len(),
        transform = ?style_config.transform,
        "Resampled data stats for isolines (in display units)"
    );
    
    // Generate contour levels from style config (using transformed min/max)
    let levels = style_config.generate_levels(transformed_min, transformed_max);
    
    info!(
        num_levels = levels.len(),
        first_level = levels.first().copied().unwrap_or(0.0),
        last_level = levels.last().copied().unwrap_or(0.0),
        "Generated contour levels"
    );
    
    // Build special level configs from style
    // Since data is now in display units, special levels are used directly (no conversion needed)
    let special_levels: Vec<contour::SpecialLevelConfig> = style_config.contour.special_levels
        .as_ref()
        .map(|levels| {
            levels.iter().map(|sl| {
                contour::SpecialLevelConfig {
                    level: sl.value,  // Already in display units
                    line_color: sl.line_color,
                    line_width: sl.line_width,
                    label: sl.label.clone(),
                }
            }).collect()
        })
        .unwrap_or_default();
    
    // No label offset needed since data is already in display units
    let label_unit_offset = 0.0;
    
    // Build ContourConfig
    let contour_config = contour::ContourConfig {
        levels,
        line_width: style_config.contour.line_width,
        line_color: style_config.contour.line_color,
        smoothing_passes: style_config.contour.smoothing_passes.unwrap_or(1),
        labels_enabled: style_config.contour.labels.unwrap_or(false),
        label_font_size: style_config.contour.label_font_size.unwrap_or(10.0),
        label_spacing: style_config.contour.label_spacing.unwrap_or(150.0),
        label_unit_offset,
        special_levels,
    };
    
    // Render contours
    let contour_pixels = contour::render_contours(
        &resampled_data,
        render_width,
        render_height,
        &contour_config,
    );
    
    // Crop to center tile if we used expanded rendering
    let final_pixels = if let Some((coord, tile_config)) = needs_crop {
        crop_center_tile(&contour_pixels, render_width as u32, &coord, &tile_config)
    } else {
        contour_pixels
    };
    
    // Encode as PNG
    renderer::png::create_png(&final_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}
