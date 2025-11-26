//! Shared weather data rendering logic.

use renderer::gradient;
use renderer::barbs::{self, BarbConfig};
use renderer::style::{StyleConfig, apply_style_gradient};
use storage::{Catalog, ObjectStorage};
use std::path::Path;
use tracing::{info, debug, warn};

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
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
) -> Result<Vec<u8>, String> {
    render_weather_data_with_style(storage, catalog, model, parameter, forecast_hour, width, height, bbox, None).await
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
///
/// # Returns
/// PNG image data as bytes
pub async fn render_weather_data_with_style(
    storage: &ObjectStorage,
    catalog: &Catalog,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    style_name: Option<&str>,
) -> Result<Vec<u8>, String> {
    // Get dataset for this parameter
    let entry = if let Some(hour) = forecast_hour {
        // Find dataset with matching forecast hour
        catalog
            .find_by_forecast_hour(model, parameter, hour)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| format!("No data found for {}/{} at hour {}", model, parameter, hour))?
    } else {
        // Get latest dataset
        catalog
            .get_latest(model, parameter)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?
    };

    // Load GRIB2 file from storage
    let grib_data = storage
        .get(&entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load GRIB2 file: {}", e))?;

    // Parse GRIB2 and find matching message
    // Strategy: Look for exact match first, then substring match
    let mut reader = grib2_parser::Grib2Reader::new(grib_data);
    let mut exact_match = None;
    let mut substring_matches = Vec::new();

    while let Some(msg) = reader
        .next_message()
        .map_err(|e| format!("GRIB2 parse error: {}", e))?
    {
        let msg_param = msg.parameter();
        
        // Look for exact match
        if msg_param == parameter {
            exact_match = Some(msg);
            break; // Exact match takes priority
        }
        
        // Look for substring matches
        if msg_param.contains(parameter) || parameter.contains(msg_param) {
            substring_matches.push(msg);
        }
    }

    let msg = if let Some(msg) = exact_match {
        msg
    } else if !substring_matches.is_empty() {
        // Use the first substring match
        substring_matches.into_iter().next().unwrap()
    } else {
        return Err(format!("Parameter {} not found in GRIB2", parameter));
    };

    // Unpack grid data
    let grid_data = msg
        .unpack_data()
        .map_err(|e| format!("Unpacking failed: {}", e))?;

    let (grid_height, grid_width) = msg.grid_dims();
    let grid_width = grid_width as usize;
    let grid_height = grid_height as usize;

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

    // For tile continuity, resample directly from the global grid using geographic coordinates
    // This ensures adjacent tiles sample from the exact same grid points at boundaries
    let rendered_width = width as usize;
    let rendered_height = height as usize;
    
    let resampled_data = if let Some(bbox) = bbox {
        // Resample directly from global grid using bbox
        resample_from_geographic(
            &grid_data,
            grid_width,
            grid_height,
            rendered_width,
            rendered_height,
            bbox,
        )
    } else {
        // No bbox - resample entire global grid
        if grid_width != rendered_width || grid_height != rendered_height {
            renderer::gradient::resample_grid(&grid_data, grid_width, grid_height, rendered_width, rendered_height)
        } else {
            grid_data.clone()
        }
    };

    // Try to load and apply custom style
    let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
    
    let rgba_data = if let Some(style_name) = style_name {
        // Try to load and apply custom style
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            Path::new(&style_config_dir).join("temperature.json")
        } else if parameter.contains("WIND") || parameter.contains("GUST") || parameter.contains("SPEED") {
            Path::new(&style_config_dir).join("wind.json")
        } else if parameter.contains("PRES") || parameter.contains("PRESS") {
            Path::new(&style_config_dir).join("atmospheric.json")
        } else if parameter.contains("RH") || parameter.contains("HUMID") {
            Path::new(&style_config_dir).join("precipitation.json")
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
    };

    // Convert to PNG
    let png = renderer::png::create_png(&rgba_data, rendered_width, rendered_height)
        .map_err(|e| format!("PNG encoding failed: {}", e))?;

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

/// Resample from global geographic grid to a specific bbox and output size
/// This ensures consistent sampling across tile boundaries
fn resample_from_geographic(
    global_data: &[f32],
    global_width: usize,
    global_height: usize,
    output_width: usize,
    output_height: usize,
    bbox: [f32; 4],
) -> Vec<f32> {
    let [min_lon, min_lat, max_lon, max_lat] = bbox;
    
    // GRIB grid: lat 90 to -90, lon 0 to 360
    // Grid resolution
    let lon_step = 360.0 / global_width as f32;
    let lat_step = 180.0 / global_height as f32;
    
    let mut output = vec![0.0f32; output_width * output_height];
    
    // For each output pixel, calculate its geographic position and sample from global grid
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Calculate geographic coordinates of this output pixel (pixel center)
            let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
            let y_ratio = (out_y as f32 + 0.5) / output_height as f32;
            
            let lon = min_lon + x_ratio * (max_lon - min_lon);
            let lat = max_lat - y_ratio * (max_lat - min_lat); // Y is inverted
            
            // Normalize longitude to 0-360
            let norm_lon = if lon < 0.0 { lon + 360.0 } else { lon };
            
            // Convert to global grid coordinates (continuous, not indices)
            let grid_x = norm_lon / lon_step;
            let grid_y = (90.0 - lat) / lat_step;
            
            // Bilinear interpolation from global grid
            let x1 = grid_x.floor() as usize;
            let y1 = grid_y.floor() as usize;
            // Handle longitude wrap-around (360° = 0°)
            let x2 = if x1 + 1 >= global_width { 0 } else { x1 + 1 };
            // Latitude clamps at poles (no wrap-around)
            let y2 = (y1 + 1).min(global_height - 1);
            
            let dx = grid_x - x1 as f32;
            let dy = grid_y - y1 as f32;
            
            // Sample four surrounding grid points
            let v11 = global_data.get(y1 * global_width + x1).copied().unwrap_or(0.0);
            let v21 = global_data.get(y1 * global_width + x2).copied().unwrap_or(0.0);
            let v12 = global_data.get(y2 * global_width + x1).copied().unwrap_or(0.0);
            let v22 = global_data.get(y2 * global_width + x2).copied().unwrap_or(0.0);
            
            // Bilinear interpolation
            let v1 = v11 * (1.0 - dx) + v21 * dx;
            let v2 = v12 * (1.0 - dx) + v22 * dx;
            let value = v1 * (1.0 - dy) + v2 * dy;
            
            output[out_y * output_width + out_x] = value;
        }
    }
    
    output
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
) -> Result<Vec<u8>, String> {
    // Load U component (UGRD) - latest available
    let u_entry = catalog
        .get_latest(model, "UGRD")
        .await
        .map_err(|e| format!("Failed to get UGRD: {}", e))?
        .ok_or_else(|| "No UGRD data available".to_string())?;

    // Load V component (VGRD) - latest available
    let v_entry = catalog
        .get_latest(model, "VGRD")
        .await
        .map_err(|e| format!("Failed to get VGRD: {}", e))?
        .ok_or_else(|| "No VGRD data available".to_string())?;

    // Load GRIB2 files
    let u_grib = storage
        .get(&u_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

    let v_grib = storage
        .get(&v_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load VGRD file: {}", e))?;

    // Parse GRIB2 messages
    let mut u_reader = grib2_parser::Grib2Reader::new(u_grib);
    let u_msg = u_reader
        .next_message()
        .map_err(|e| format!("GRIB2 parse error (U): {}", e))?
        .ok_or_else(|| "No U-component message found".to_string())?;

    let mut v_reader = grib2_parser::Grib2Reader::new(v_grib);
    let v_msg = v_reader
        .next_message()
        .map_err(|e| format!("GRIB2 parse error (V): {}", e))?
        .ok_or_else(|| "No V-component message found".to_string())?;

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
        
        // Resample from geographic coordinates
        let u_resampled = resample_from_geographic(&u_data, grid_width, grid_height, output_width, output_height, bbox);
        let v_resampled = resample_from_geographic(&v_data, grid_width, grid_height, output_width, output_height, bbox);
        
        // Debug: check resampled data
        let u_non_zero = u_resampled.iter().filter(|&&v| v != 0.0 && !v.is_nan()).count();
        let v_non_zero = v_resampled.iter().filter(|&&v| v != 0.0 && !v.is_nan()).count();
        info!(
            u_non_zero = u_non_zero,
            v_non_zero = v_non_zero,
            total = u_resampled.len(),
            "Resampled wind data stats"
        );
        
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
    let mut config = BarbConfig::default();
    config.spacing = spacing as u32;
    
    info!(
        render_width = render_width,
        render_height = render_height,
        barb_spacing = config.spacing,
        barb_size = config.size,
        "Rendering wind barbs"
    );
    
    let barb_pixels = barbs::render_wind_barbs(
        &u_to_render,
        &v_to_render,
        render_width,
        render_height,
        &config,
    );
    
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
