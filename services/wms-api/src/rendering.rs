//! Shared weather data rendering logic.

use renderer::gradient;
use renderer::barbs::{self, BarbConfig};
use renderer::contour;
use renderer::style::{StyleConfig, apply_style_gradient, ContourStyle};
use storage::{Catalog, CatalogEntry, ObjectStorage};
use std::path::Path;
use std::time::Instant;
use tracing::{info, debug, warn};
use projection::{LambertConformal, Geostationary};
use crate::metrics::MetricsCollector;

/// Render weather data from GRIB2 grid to PNG.
///
/// # Arguments
/// - `storage`: Object storage for retrieving GRIB2 files
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
    storage: &ObjectStorage,
    catalog: &Catalog,
    metrics: &MetricsCollector,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
) -> Result<Vec<u8>, String> {
    render_weather_data_with_style(storage, catalog, metrics, model, parameter, forecast_hour, width, height, bbox, None, false).await
}

/// Render weather data with optional style configuration.
///
/// # Arguments
/// - `storage`: Object storage for retrieving GRIB2 files
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
    storage: &ObjectStorage,
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
        storage, catalog, metrics, model, parameter, forecast_hour, None, width, height, bbox, style_name, use_mercator
    ).await
}

/// Render weather data with optional style configuration and level.
///
/// # Arguments
/// - `storage`: Object storage for retrieving GRIB2 files
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
    storage: &ObjectStorage,
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
        storage, catalog, metrics, model, parameter, forecast_hour, None, level, width, height, bbox, style_name, use_mercator
    ).await
}

/// Render weather data with support for both forecast hours and observation times.
///
/// This is the main rendering function that supports:
/// - Forecast models (GFS, HRRR): Use `forecast_hour` parameter
/// - Observation data (MRMS, GOES): Use `observation_time` parameter
///
/// # Arguments
/// - `storage`: Object storage for retrieving files
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
    storage: &ObjectStorage,
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
    // Get dataset based on time specification
    let entry = {
        if let Some(obs_time) = observation_time {
            // Observation mode: find dataset closest to requested observation time
            info!(model = model, parameter = parameter, observation_time = ?obs_time, "Looking up observation data by time");
            catalog
                .find_by_time(model, parameter, obs_time)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No observation data found for {}/{} at time {:?}", model, parameter, obs_time))?
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

    // Load grid data from storage (handles both GRIB2 and NetCDF formats)
    info!(
        parameter = parameter,
        level = %entry.level,
        storage_path = %entry.storage_path,
        model = model,
        "Loading grid data"
    );
    
    let start = Instant::now();
    let grid_result = load_grid_data(storage, metrics, &entry, parameter).await?;
    let load_duration = start.elapsed();
    metrics.record_grib_load(load_duration.as_micros() as u64).await;
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
    
    // Convert entry.bbox to array format for resampling
    let data_bounds = [
        entry.bbox.min_x as f32,
        entry.bbox.min_y as f32,
        entry.bbox.max_x as f32,
        entry.bbox.max_y as f32,
    ];
    
    let start = Instant::now();
    let resampled_data = {
        if let Some(output_bbox) = bbox {
            // Resample from data grid to output bbox
            // Use Mercator projection when rendering for Web Mercator display
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
            )
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
    metrics.record_resample(resample_duration.as_micros() as u64).await;

    // Apply color rendering (gradient/wind barbs/etc)
    let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
    
    let rgba_data = {
        if let Some(style_name) = style_name {
        // Try to load and apply custom style
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            Path::new(&style_config_dir).join("temperature.json")
        } else if parameter.contains("WIND") || parameter.contains("GUST") || parameter.contains("SPEED") {
            Path::new(&style_config_dir).join("wind.json")
        } else if parameter.contains("PRES") || parameter.contains("PRESS") {
            Path::new(&style_config_dir).join("atmospheric.json")
        } else if parameter.contains("RH") || parameter.contains("HUMID") {
            Path::new(&style_config_dir).join("precipitation.json")
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
    } else if parameter.contains("PRES") || parameter.contains("PRESS") {
        // Pressure in Pa, convert to hPa
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
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;
    let [data_min_lon, data_min_lat, data_max_lon, data_max_lat] = data_bounds;
    
    // Check if data uses 0-360 longitude convention (like GFS)
    let data_uses_360 = data_min_lon >= 0.0 && data_max_lon > 180.0;
    
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
            
            // Normalize longitude for data grids that use 0-360 convention
            let norm_lon = if data_uses_360 && lon < 0.0 {
                lon + 360.0
            } else {
                lon
            };
            
            // Check if this pixel is within data bounds
            if norm_lon < data_min_lon || norm_lon > data_max_lon || lat < data_min_lat || lat > data_max_lat {
                // Outside data coverage - leave as NaN for transparent rendering
                continue;
            }
            
            // Convert to data grid coordinates (continuous, not indices)
            let grid_x = (norm_lon - data_min_lon) / data_lon_range * data_width as f32;
            let grid_y = (data_max_lat - lat) / data_lat_range * data_height as f32;
            
            // Bilinear interpolation from data grid
            let x1 = grid_x.floor() as usize;
            let y1 = grid_y.floor() as usize;
            let x2 = (x1 + 1).min(data_width - 1);
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
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;
    let [data_min_lon, data_min_lat, data_max_lon, data_max_lat] = data_bounds;
    
    // Check if data uses 0-360 longitude convention (like GFS)
    let data_uses_360 = data_min_lon >= 0.0 && data_max_lon > 180.0;
    
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
            
            // Normalize longitude for data grids that use 0-360 convention
            let norm_lon = if data_uses_360 && lon < 0.0 {
                lon + 360.0
            } else {
                lon
            };
            
            // Check if this pixel is within data bounds
            if norm_lon < data_min_lon || norm_lon > data_max_lon || lat < data_min_lat || lat > data_max_lat {
                // Outside data coverage - leave as NaN for transparent rendering
                continue;
            }
            
            // Convert to data grid coordinates
            let grid_x = (norm_lon - data_min_lon) / data_lon_range * data_width as f32;
            let grid_y = (data_max_lat - lat) / data_lat_range * data_height as f32;
            
            // Bilinear interpolation
            let x1 = grid_x.floor() as usize;
            let y1 = grid_y.floor() as usize;
            let x2 = (x1 + 1).min(data_width - 1);
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
) -> Vec<f32> {
    resample_grid_for_bbox_with_proj(
        data, data_width, data_height, output_width, output_height,
        output_bbox, data_bounds, use_mercator, model, None
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
            resample_for_mercator(data, data_width, data_height, output_width, output_height, output_bbox, data_bounds)
        } else {
            resample_from_geographic(data, data_width, data_height, output_width, output_height, output_bbox, data_bounds)
        }
    }
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
) -> Vec<f32> {
    if model == "hrrr" {
        resample_lambert_to_geographic(data, data_width, data_height, output_width, output_height, output_bbox)
    } else if model == "goes16" || model == "goes18" || model == "goes" {
        let satellite_lon = if model == "goes18" { -137.2 } else { -75.0 };
        resample_geostationary_to_geographic(data, data_width, data_height, output_width, output_height, output_bbox, satellite_lon)
    } else {
        resample_from_geographic(data, data_width, data_height, output_width, output_height, output_bbox, data_bounds)
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
        
        // Check parameter match
        if msg_param == parameter {
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

/// Grid data extracted from either GRIB2 or NetCDF files
struct GridData {
    data: Vec<f32>,
    width: usize,
    height: usize,
    /// For GOES data: dynamic geostationary projection parameters
    goes_projection: Option<GoesProjectionParams>,
}

/// Dynamic GOES projection parameters extracted from NetCDF file
#[derive(Clone, Debug)]
struct GoesProjectionParams {
    x_origin: f64,
    y_origin: f64,
    dx: f64,
    dy: f64,
    perspective_point_height: f64,
    semi_major_axis: f64,
    semi_minor_axis: f64,
    longitude_origin: f64,
}

/// Load grid data from a storage path, handling both GRIB2 and NetCDF files
/// 
/// For GRIB2 files, searches for the matching parameter and level.
/// For NetCDF (GOES) files, reads the CMI variable directly.
async fn load_grid_data(
    storage: &ObjectStorage,
    metrics: &MetricsCollector,
    entry: &CatalogEntry,
    parameter: &str,
) -> Result<GridData, String> {
    // Load file from storage
    let file_data = storage
        .get(&entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load file: {}", e))?;
    
    // Check file type by extension or magic bytes
    let is_netcdf = entry.storage_path.ends_with(".nc") || 
                    entry.parameter.starts_with("CMI_") ||
                    entry.model.starts_with("goes");
    
    if is_netcdf {
        // Handle NetCDF (GOES) data
        load_netcdf_grid_data(storage, entry).await
    } else {
        // Handle GRIB2 data
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
        metrics.record_grib_parse(parse_duration.as_micros() as u64).await;
        
        let (grid_height, grid_width) = msg.grid_dims();
        
        Ok(GridData {
            data: grid_data,
            width: grid_width as usize,
            height: grid_height as usize,
            goes_projection: None,
        })
    }
}

/// Load GOES NetCDF data from storage
/// 
/// This uses ncdump to extract data since we don't have direct HDF5 support.
/// Downloads the file from S3 storage to a temp location first.
async fn load_netcdf_grid_data(
    storage: &ObjectStorage,
    entry: &CatalogEntry,
) -> Result<GridData, String> {
    use std::process::Command;
    use std::io::Write;
    
    info!(
        storage_path = %entry.storage_path,
        parameter = %entry.parameter,
        "Loading GOES NetCDF data from storage"
    );
    
    // Download file from S3 to temp location
    let file_data = storage
        .get(&entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load NetCDF file from storage: {}", e))?;
    
    // Write to temp file for ncdump to read
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!("goes_temp_{}.nc", uuid::Uuid::new_v4()));
    let file_path = temp_file.to_string_lossy().to_string();
    
    std::fs::write(&temp_file, &file_data[..])
        .map_err(|e| format!("Failed to write temp file: {}", e))?;
    
    info!(
        temp_file = %file_path,
        size = file_data.len(),
        "Downloaded NetCDF to temp file"
    );
    
    // Use ncdump to get header information
    let header_output = Command::new("ncdump")
        .arg("-h")
        .arg(&file_path)
        .output()
        .map_err(|e| format!("Failed to run ncdump -h: {}", e))?;
    
    if !header_output.status.success() {
        return Err(format!(
            "ncdump -h failed: {}",
            String::from_utf8_lossy(&header_output.stderr)
        ));
    }
    
    let header = String::from_utf8_lossy(&header_output.stdout);
    
    // Parse dimensions from header
    let width = parse_dimension(&header, "x")?;
    let height = parse_dimension(&header, "y")?;
    
    // Parse scale/offset for CMI
    let scale_factor = parse_attribute(&header, "scale_factor").unwrap_or(1.0) as f32;
    let add_offset = parse_attribute(&header, "add_offset").unwrap_or(0.0) as f32;
    let fill_value = parse_attribute(&header, "_FillValue").unwrap_or(-1.0) as i16;
    
    // Parse projection parameters for x and y coordinates
    // x: scale_factor and add_offset give us x_origin and dx
    // y: scale_factor and add_offset give us y_origin and dy
    let x_scale = parse_coordinate_attribute(&header, "x", "scale_factor").unwrap_or(1.4e-05);
    let x_offset = parse_coordinate_attribute(&header, "x", "add_offset").unwrap_or(-0.101353);
    let y_scale = parse_coordinate_attribute(&header, "y", "scale_factor").unwrap_or(-1.4e-05);
    let y_offset = parse_coordinate_attribute(&header, "y", "add_offset").unwrap_or(0.128233);
    
    // Parse GOES projection parameters
    let perspective_point_height = parse_attribute(&header, "perspective_point_height").unwrap_or(35786023.0);
    let semi_major_axis = parse_attribute(&header, "semi_major_axis").unwrap_or(6378137.0);
    let semi_minor_axis = parse_attribute(&header, "semi_minor_axis").unwrap_or(6356752.31414);
    let longitude_origin = parse_attribute(&header, "longitude_of_projection_origin").unwrap_or(-75.0);
    
    info!(
        width = width,
        height = height,
        scale_factor = scale_factor,
        add_offset = add_offset,
        x_scale = x_scale,
        x_offset = x_offset,
        y_scale = y_scale,
        y_offset = y_offset,
        longitude_origin = longitude_origin,
        "Parsed NetCDF metadata"
    );
    
    // Use ncdump to extract CMI data
    let data_output = Command::new("ncdump")
        .arg("-v")
        .arg("CMI")
        .arg("-p")
        .arg("9,17")
        .arg(&file_path)
        .output()
        .map_err(|e| format!("Failed to run ncdump -v CMI: {}", e))?;
    
    if !data_output.status.success() {
        return Err(format!(
            "ncdump -v CMI failed: {}",
            String::from_utf8_lossy(&data_output.stderr)
        ));
    }
    
    let output_str = String::from_utf8_lossy(&data_output.stdout);
    
    // Find the data section
    let data_start = output_str.find("CMI =")
        .ok_or_else(|| "CMI data section not found".to_string())?;
    
    let data_section = &output_str[data_start..];
    
    // Parse the numeric values
    let mut data = Vec::with_capacity(width * height);
    let mut in_data = false;
    
    for line in data_section.lines() {
        if line.contains("CMI =") {
            in_data = true;
            continue;
        }
        if !in_data {
            continue;
        }
        if line.contains(';') && !line.contains(',') {
            break;
        }
        
        for part in line.split(',') {
            let trimmed = part.trim().trim_end_matches(';');
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(val) = trimmed.parse::<i16>() {
                let scaled = if val == fill_value {
                    f32::NAN
                } else {
                    val as f32 * scale_factor + add_offset
                };
                data.push(scaled);
            } else if trimmed == "_" {
                data.push(f32::NAN);
            }
        }
    }
    
    info!(
        data_len = data.len(),
        expected_len = width * height,
        "Parsed CMI data"
    );
    
    // Verify we got the expected amount of data
    if data.len() != width * height {
        warn!(
            "Data length mismatch: got {} expected {}",
            data.len(),
            width * height
        );
        // If we got less data, pad with NaN
        while data.len() < width * height {
            data.push(f32::NAN);
        }
        // If we got more, truncate
        data.truncate(width * height);
    }
    
    // Clean up temp file
    if let Err(e) = std::fs::remove_file(&temp_file) {
        warn!("Failed to remove temp file {}: {}", temp_file.display(), e);
    }
    
    // Create projection parameters
    let goes_projection = GoesProjectionParams {
        x_origin: x_offset,
        y_origin: y_offset,
        dx: x_scale,
        dy: y_scale,
        perspective_point_height,
        semi_major_axis,
        semi_minor_axis,
        longitude_origin,
    };
    
    Ok(GridData {
        data,
        width,
        height,
        goes_projection: Some(goes_projection),
    })
}

/// Parse coordinate variable attribute (e.g., x:scale_factor or y:add_offset)
fn parse_coordinate_attribute(header: &str, coord: &str, attr: &str) -> Result<f64, String> {
    // Look for lines like: "x:scale_factor = 1.4e-05f ;"
    let pattern = format!("{}:{} = ", coord, attr);
    for line in header.lines() {
        if line.trim().contains(&pattern) {
            let parts: Vec<&str> = line.split('=').collect();
            if parts.len() >= 2 {
                let num_str = parts[1].trim()
                    .trim_end_matches(';')
                    .trim_end_matches('f')
                    .trim();
                let clean = num_str.replace('f', "");
                return clean.parse()
                    .map_err(|_| format!("Failed to parse {}:{}: '{}'", coord, attr, num_str));
            }
        }
    }
    Err(format!("{}:{} not found", coord, attr))
}

/// Parse dimension from ncdump header (e.g., "x = 5000")
fn parse_dimension(header: &str, name: &str) -> Result<usize, String> {
    let pattern = format!("{} = ", name);
    for line in header.lines() {
        let trimmed = line.trim();
        if trimmed.contains(&pattern) && !trimmed.contains(":") {
            let parts: Vec<&str> = trimmed.split('=').collect();
            if parts.len() >= 2 {
                let num_str = parts[1].trim().trim_end_matches(';').trim();
                return num_str.parse()
                    .map_err(|_| format!("Failed to parse dimension {}", name));
            }
        }
    }
    Err(format!("dimension {} not found", name))
}

/// Parse attribute from ncdump header
fn parse_attribute(header: &str, name: &str) -> Result<f64, String> {
    let pattern = format!(":{} = ", name);
    for line in header.lines() {
        if line.contains(&pattern) {
            let parts: Vec<&str> = line.split('=').collect();
            if parts.len() >= 2 {
                let num_str = parts[1].trim()
                    .trim_end_matches(';')
                    .trim_end_matches('f')
                    .trim_end_matches('s')
                    .trim();
                let clean = num_str.replace('f', "").replace('s', "");
                return clean.parse()
                    .map_err(|_| format!("Failed to parse attribute {}: '{}'", name, num_str));
            }
        }
    }
    Err(format!("attribute {} not found", name))
}

/// Render wind barbs combining U and V component data using expanded tile rendering.
/// This renders a 3x3 grid of tiles and crops the center to ensure seamless boundaries.
///
/// # Arguments
/// - `storage`: Object storage for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `model`: Weather model name (e.g., "gfs")
/// - `tile_coord`: Optional tile coordinate for expanded rendering
/// - `width`: Output image width (single tile)
/// - `height`: Output image height (single tile)
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat] for the single tile
///
/// # Returns
/// PNG image data as bytes
pub async fn render_wind_barbs_tile(
    storage: &ObjectStorage,
    catalog: &Catalog,
    model: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    forecast_hour: Option<u32>,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::{ExpandedTileConfig, expanded_tile_bbox, crop_center_tile};
    
    // Load U and V component data
    let (u_data, v_data, grid_width, grid_height, data_bounds) = load_wind_components(storage, catalog, model, forecast_hour).await?;
    
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
        render_width, render_height, render_bbox, data_bounds, model
    );
    let v_resampled = resample_for_model_geographic(
        &v_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model
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
/// - `storage`: Object storage for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
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
    storage: &ObjectStorage,
    catalog: &Catalog,
    model: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::{ExpandedTileConfig, expanded_tile_bbox, crop_center_tile};
    
    // Load U and V component data with level support
    let (u_data, v_data, grid_width, grid_height, data_bounds) = load_wind_components_with_level(
        storage, catalog, model, forecast_hour, level
    ).await?;
    
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
        render_width, render_height, render_bbox, data_bounds, model
    );
    let v_resampled = resample_for_model_geographic(
        &v_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model
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

/// Load U and V wind component data from storage with optional level
async fn load_wind_components_with_level(
    storage: &ObjectStorage,
    catalog: &Catalog,
    model: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<(Vec<f32>, Vec<f32>, usize, usize, [f32; 4]), String> {
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

    // Load GRIB2 files
    let u_grib = storage
        .get(&u_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

    let v_grib = storage
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
    
    Ok((u_data, v_data, grid_width as usize, grid_height as usize, data_bounds))
}

/// Load U and V wind component data from storage
async fn load_wind_components(
    storage: &ObjectStorage,
    catalog: &Catalog,
    model: &str,
    forecast_hour: Option<u32>,
) -> Result<(Vec<f32>, Vec<f32>, usize, usize, [f32; 4]), String> {
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

    // Load GRIB2 files
    let u_grib = storage
        .get(&u_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

    let v_grib = storage
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

    Ok((u_data, v_data, grid_width as usize, grid_height as usize, data_bounds))
}

/// Render wind barbs combining U and V component data
///
/// # Arguments
/// - `storage`: Object storage for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `model`: Weather model name (e.g., "gfs")
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box [min_lon, min_lat, max_lon, max_lat]
/// - `barb_spacing`: Optional spacing between barbs in pixels (default: 50)
///
/// # Returns
/// PNG image data as bytes
pub async fn render_wind_barbs_layer(
    storage: &ObjectStorage,
    catalog: &Catalog,
    model: &str,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    barb_spacing: Option<usize>,
    forecast_hour: Option<u32>,
) -> Result<Vec<u8>, String> {
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

    // Load GRIB2 files
    let u_grib = storage
        .get(&u_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

    let v_grib = storage
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
    let grid_width = grid_width as usize;
    let grid_height = grid_height as usize;
    
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
    
    // Get data bounds from catalog entry (both U and V should have same bounds)
    let data_bounds = [
        u_entry.bbox.min_x as f32,
        u_entry.bbox.min_y as f32,
        u_entry.bbox.max_x as f32,
        u_entry.bbox.max_y as f32,
    ];

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
        let u_resampled = resample_for_model_geographic(&u_data, grid_width, grid_height, output_width, output_height, bbox, data_bounds, model);
        let v_resampled = resample_for_model_geographic(&v_data, grid_width, grid_height, output_width, output_height, bbox, data_bounds, model);
        
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
    renderer::png::create_png(&barb_pixels, render_width, render_height)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

/// Query data value at a specific point for GetFeatureInfo
///
/// # Arguments
/// - `storage`: Object storage for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
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
    storage: &ObjectStorage,
    catalog: &Catalog,
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
    let (lon, lat) = if crs.contains("3857") {
        // Web Mercator - convert bbox and then pixel position
        let [min_x, min_y, max_x, max_y] = bbox;
        let (min_lon, min_lat) = mercator_to_wgs84(min_x, min_y);
        let (max_lon, max_lat) = mercator_to_wgs84(max_x, max_y);
        pixel_to_geographic(i, j, width, height, [min_lon, min_lat, max_lon, max_lat])
    } else {
        // Assume WGS84 (EPSG:4326)
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
        return query_wind_barbs_value(storage, catalog, model, lon, lat, forecast_hour, level).await;
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
    
    // Load GRIB2 file from storage
    let grib_data = storage
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
    let value = sample_grid_value(&grid_data, grid_width, grid_height, lon, lat)?;
    
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
    storage: &ObjectStorage,
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
    
    // Load GRIB2 files
    let u_grib = storage.get(&u_entry.storage_path).await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;
    let v_grib = storage.get(&v_entry.storage_path).await
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
    let u = sample_grid_value(&u_data, grid_width, grid_height, lon, lat)?;
    let v = sample_grid_value(&v_data, grid_width, grid_height, lon, lat)?;
    
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
) -> Result<f32, String> {
    // GRIB grid: lat 90 to -90, lon 0 to 360
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
    
    // Bilinear interpolation
    let x1 = grid_x.floor() as usize;
    let y1 = grid_y.floor() as usize;
    let x2 = if x1 + 1 >= grid_width { 0 } else { x1 + 1 }; // Longitude wrap-around
    let y2 = (y1 + 1).min(grid_height - 1); // Latitude clamp
    
    let dx = grid_x - x1 as f64;
    let dy = grid_y - y1 as f64;
    
    // Sample four surrounding grid points
    let v11 = grid_data.get(y1 * grid_width + x1).copied().unwrap_or(0.0);
    let v21 = grid_data.get(y1 * grid_width + x2).copied().unwrap_or(0.0);
    let v12 = grid_data.get(y2 * grid_width + x1).copied().unwrap_or(0.0);
    let v22 = grid_data.get(y2 * grid_width + x2).copied().unwrap_or(0.0);
    
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
    storage: &ObjectStorage,
    catalog: &Catalog,
    model: &str,
    parameter: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    style_path: &str,
    forecast_hour: Option<u32>,
    use_mercator: bool,
) -> Result<Vec<u8>, String> {
    render_isolines_tile_with_level(
        storage, catalog, model, parameter, tile_coord, width, height, bbox,
        style_path, forecast_hour, None, use_mercator
    ).await
}

/// Render isolines (contour lines) for a single tile with optional level.
pub async fn render_isolines_tile_with_level(
    storage: &ObjectStorage,
    catalog: &Catalog,
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
    use wms_common::tile::{ExpandedTileConfig, expanded_tile_bbox, crop_center_tile};
    
    // Load style configuration
    let style_config = ContourStyle::from_file(style_path)
        .map_err(|e| format!("Failed to load contour style: {}", e))?;
    
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
    
    // Load GRIB2 file from storage
    let grib_data = storage
        .get(&entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load GRIB2 file: {}", e))?;
    
    // Parse GRIB2 and find parameter with matching level
    let msg = find_parameter_in_grib(grib_data, parameter, Some(&entry.level))?;
    
    // Unpack grid data
    let grid_data = msg
        .unpack_data()
        .map_err(|e| format!("Unpacking failed: {}", e))?;
    
    let (grid_height, grid_width) = msg.grid_dims();
    let grid_width = grid_width as usize;
    let grid_height = grid_height as usize;
    
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
        "Loaded grid data for isolines"
    );
    
    // For isolines, we don't use expanded rendering because:
    // 1. Contours are continuous and don't need alignment across tiles like wind barbs
    // 2. Expanded rendering at low zoom can cause the bbox to span the entire world,
    //    leading to projection distortion when cropping in pixel space
    // Instead, we render each tile independently
    let (render_bbox, render_width, render_height, needs_crop) = (bbox, width as usize, height as usize, None);
    
    // Get data bounds from catalog entry
    let data_bounds = [
        entry.bbox.min_x as f32,
        entry.bbox.min_y as f32,
        entry.bbox.max_x as f32,
        entry.bbox.max_y as f32,
    ];
    
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
    );
    
    // Generate contour levels from style config
    let levels = style_config.generate_levels(min_val, max_val);
    
    info!(
        num_levels = levels.len(),
        first_level = levels.first().copied().unwrap_or(0.0),
        last_level = levels.last().copied().unwrap_or(0.0),
        "Generated contour levels"
    );
    
    // Build ContourConfig
    let contour_config = contour::ContourConfig {
        levels,
        line_width: style_config.contour.line_width,
        line_color: style_config.contour.line_color,
        smoothing_passes: style_config.contour.smoothing_passes.unwrap_or(1),
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
