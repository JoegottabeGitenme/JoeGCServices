//! Grid resampling and projection transformations.
//!
//! This module provides functions for resampling weather data grids between
//! different coordinate systems and projections:
//!
//! - Geographic (lat/lon) to geographic
//! - Geographic to Web Mercator (EPSG:3857)
//! - Lambert Conformal (HRRR) to geographic/Mercator
//! - Geostationary (GOES) to geographic/Mercator
//!
//! All resampling uses bilinear interpolation for smooth results.

use projection::{LambertConformal, Geostationary};
use tracing::debug;

use super::types::GoesProjectionParams;
use crate::state::ProjectionLuts;

// ============================================================================
// Web Mercator coordinate conversions
// ============================================================================

/// Convert latitude to Web Mercator Y coordinate
pub fn lat_to_mercator_y(lat: f64) -> f64 {
    let lat_rad = lat.to_radians();
    let y = ((std::f64::consts::PI / 4.0) + (lat_rad / 2.0)).tan().ln();
    y * 6378137.0  // Earth radius in meters
}

/// Convert Web Mercator Y coordinate to latitude
pub fn mercator_y_to_lat(y: f64) -> f64 {
    let y_normalized = y / 6378137.0;  // Normalize by Earth radius
    (2.0 * y_normalized.exp().atan() - std::f64::consts::PI / 2.0).to_degrees()
}

// ============================================================================
// Bilinear interpolation
// ============================================================================

/// Perform bilinear interpolation at grid coordinates
pub fn bilinear_interpolate(
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

// ============================================================================
// Geographic grid resampling
// ============================================================================

/// Resample from geographic grid to a specific bbox and output size
/// 
/// This ensures consistent sampling across tile boundaries.
/// Supports both global and regional datasets by respecting data_bounds.
/// Pixels outside data_bounds are set to NaN for transparent rendering.
/// 
/// For Web Mercator output, use `resample_for_mercator` instead.
pub fn resample_from_geographic(
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
pub fn resample_for_mercator(
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

// ============================================================================
// Model-aware resampling dispatchers
// ============================================================================

/// Resample grid data for a given bbox, with model-aware projection handling
pub fn resample_grid_for_bbox(
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
pub fn resample_grid_for_bbox_with_proj(
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
        debug!(
            model = model,
            use_mercator = use_mercator,
            data_width = data_width,
            data_height = data_height,
            output_width = output_width,
            output_height = output_height,
            output_bbox = ?output_bbox,
            "Using Lambert Conformal resampling for HRRR"
        );
        if use_mercator {
            resample_lambert_to_mercator(data, data_width, data_height, output_width, output_height, output_bbox)
        } else {
            resample_lambert_to_geographic(data, data_width, data_height, output_width, output_height, output_bbox)
        }
    } else if model == "goes16" || model == "goes18" || model == "goes" {
        // GOES satellite data handling
        // If goes_projection is present, data is in native geostationary projection (raw NetCDF)
        // If goes_projection is None, data has been pre-projected to geographic (Zarr)
        if let Some(params) = goes_projection {
            // Native geostationary data - use projection transform
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
            // Pre-projected geographic data (from Zarr) - treat as regular lat/lon grid
            // This is the path for GOES data that was reprojected during ingestion
            debug!(
                model = model,
                data_width = data_width,
                data_height = data_height,
                "GOES data from Zarr (pre-projected to geographic), using geographic resampling"
            );
            if use_mercator {
                resample_for_mercator(data, data_width, data_height, output_width, output_height, output_bbox, data_bounds, grid_uses_360)
            } else {
                resample_from_geographic(data, data_width, data_height, output_width, output_height, output_bbox, data_bounds, grid_uses_360)
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
#[allow(unused_variables)]
pub fn try_resample_with_lut(
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
}

/// Model-aware resampling for geographic output (used for wind barbs and other non-Mercator rendering)
/// 
/// This wraps the projection-specific resampling functions to handle different grid types.
pub fn resample_for_model_geographic(
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

// ============================================================================
// Lambert Conformal projection resampling (HRRR)
// ============================================================================

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

// ============================================================================
// Geostationary projection resampling (GOES)
// ============================================================================

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

