//! Grid value sampling for point queries.
//!
//! This module provides functions for sampling weather data grid values at
//! specific geographic points, used primarily for GetFeatureInfo requests.
//!
//! Key features:
//! - Model-aware projection handling (GFS global, HRRR Lambert, MRMS regional, GOES geostationary)
//! - Bilinear interpolation for smooth value queries
//! - Unit conversion for display values

use projection::{LambertConformal, Geostationary};
use storage::{GribCache, Catalog};
use tracing::info;

use super::types::GoesProjectionParams;
use super::loaders::{load_grid_data, query_point_from_zarr, find_parameter_in_grib};
use super::resampling::bilinear_interpolate;
use crate::metrics::MetricsCollector;
use crate::state::GridProcessorFactory;

// ============================================================================
// Public query functions
// ============================================================================

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
    _metrics: &MetricsCollector,
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
    // Uppercase parameter to match database storage
    let parameter = parts[1..].join("_").to_uppercase();
    
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
    } else if is_netcdf && grid_processor_factory.is_some() {
        // Handle NetCDF (GOES satellite) data via Zarr path
        let factory = grid_processor_factory.unwrap();
        let grid_result = load_grid_data(factory, &entry, None, None).await?;
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
pub async fn query_wind_barbs_value(
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

// ============================================================================
// Grid sampling functions
// ============================================================================

/// Sample a grid value at a geographic point using bilinear interpolation
pub fn sample_grid_value(
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
pub fn sample_lambert_grid_value(
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
pub fn sample_mrms_grid_value(
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
pub fn sample_grid_value_with_projection(
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

// ============================================================================
// Unit conversion
// ============================================================================

/// Convert parameter value to display format with appropriate units
pub fn convert_parameter_value(parameter: &str, value: f32) -> (f64, String, String, String) {
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
    } else if parameter.contains("HGT") {
        // Geopotential height: raw units to decameters (dkm)
        // Data appears to be ~60x larger than expected, so divide by 60 (~0.0167)
        let dkm = value * 0.0167;
        (dkm as f64, "dkm".to_string(), "gpm".to_string(), "Geopotential Height".to_string())
    } else {
        // Generic parameter
        (value as f64, "".to_string(), "".to_string(), parameter.to_string())
    }
}
