//! Grid resampling for EDR area queries.
//!
//! This module provides functions for resampling grid data to different
//! output resolutions and properly reprojecting from native grid projections
//! (Lambert Conformal, Polar Stereographic, Mercator) to geographic coordinates.
//!
//! For projected grids, we use the proper projection math (geo_to_grid) to
//! correctly transform coordinates, matching the approach used in WMS rendering.

use grid_processor::{BoundingBox, GridRegion, ProjectionType};
use projection::{LambertConformal, Mercator, PolarStereographic};

/// Resample a grid region from its native projection to a regular geographic grid.
///
/// This creates a new GridRegion with uniformly-spaced lat/lon coordinates,
/// properly reprojecting data from projected coordinate systems.
///
/// # Arguments
/// * `region` - Source grid region in native projection coordinates
/// * `projection` - The projection type of the source grid
/// * `model` - Model name (used to select specific projection parameters)
/// * `output_resolution` - Desired output resolution in degrees (lon, lat)
///
/// # Returns
/// A new GridRegion with data properly reprojected to WGS84
pub fn resample_to_geographic(
    region: &GridRegion,
    projection: ProjectionType,
    model: &str,
    output_resolution: Option<(f64, f64)>,
) -> GridRegion {
    // If geographic projection, no reprojection needed
    if !projection.requires_projection_transform() {
        return region.clone();
    }

    // Determine output grid dimensions
    // Use the requested resolution or estimate based on the source grid
    let (out_res_lon, out_res_lat) = output_resolution.unwrap_or_else(|| {
        // Estimate resolution from bbox size and native grid dimensions
        // This preserves approximate detail level
        (
            region.bbox.width() / region.width as f64,
            region.bbox.height() / region.height as f64,
        )
    });

    let output_width = ((region.bbox.width() / out_res_lon).ceil() as usize).max(1);
    let output_height = ((region.bbox.height() / out_res_lat).ceil() as usize).max(1);

    // Dispatch to projection-specific resampling
    match projection {
        ProjectionType::LambertConformal => resample_lambert_to_geographic(
            &region.data,
            region.width,
            region.height,
            output_width,
            output_height,
            &region.bbox,
            model,
        ),
        ProjectionType::PolarStereographic => resample_polar_stereo_to_geographic(
            &region.data,
            region.width,
            region.height,
            output_width,
            output_height,
            &region.bbox,
            model,
        ),
        ProjectionType::Mercator => resample_mercator_to_geographic(
            &region.data,
            region.width,
            region.height,
            output_width,
            output_height,
            &region.bbox,
            model,
        ),
        // Geographic and Geostationary don't need reprojection here
        // (GOES data is pre-projected to geographic in our Zarr storage)
        _ => region.clone(),
    }
}

/// Resample from Lambert Conformal grid to geographic output.
///
/// This uses proper projection math (geo_to_grid) to correctly transform
/// geographic coordinates to Lambert grid indices, matching the approach
/// used in WMS rendering. This ensures data alignment is correct.
///
/// Lambert grids (HRRR, NDFD, NBM-CONUS) use RowOrigin::South, meaning
/// row 0 is at the southern edge of the grid.
fn resample_lambert_to_geographic(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    bbox: &BoundingBox,
    model: &str,
) -> GridRegion {
    let mut output = vec![f32::NAN; output_width * output_height];

    let out_res_lon = bbox.width() / output_width as f64;
    let out_res_lat = bbox.height() / output_height as f64;

    // Select the appropriate Lambert projection based on model
    let proj = match model {
        "ndfd" => LambertConformal::ndfd(),
        "nbm-conus" => LambertConformal::nbm_conus(),
        // HRRR and NAM CONUS nest share the same grid
        _ => LambertConformal::hrrr(),
    };

    // Calculate scale factors if data dimensions differ from native projection dimensions.
    // This handles cases where pyramid levels or partial reads are used.
    let scale_x = data_width as f64 / proj.nx as f64;
    let scale_y = data_height as f64 / proj.ny as f64;

    // For each output pixel, find the corresponding point in the input data
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Calculate geographic coordinates of this output pixel (pixel center)
            // Output uses standard image convention: row 0 = north
            let lon = bbox.min_lon + (out_x as f64 + 0.5) * out_res_lon;
            let lat = bbox.max_lat - (out_y as f64 + 0.5) * out_res_lat;

            // Convert geographic to Lambert grid indices using proper projection math
            // proj.geo_to_grid returns indices for the native resolution grid
            let (native_i, native_j) = proj.geo_to_grid(lat, lon);

            // Scale indices to match actual data dimensions
            let grid_i = native_i * scale_x;
            let grid_j = native_j * scale_y;

            // Check if within data bounds
            if grid_i < 0.0
                || grid_i >= data_width as f64 - 1.0
                || grid_j < 0.0
                || grid_j >= data_height as f64 - 1.0
            {
                continue;
            }

            // Bilinear interpolation
            let value = bilinear_interpolate(data, data_width, data_height, grid_i, grid_j);
            output[out_y * output_width + out_x] = value;
        }
    }

    GridRegion::new(
        output,
        output_width,
        output_height,
        *bbox,
        (out_res_lon, out_res_lat),
    )
}

/// Resample from Polar Stereographic grid to geographic output.
///
/// This uses proper projection math (geo_to_grid) to correctly transform
/// geographic coordinates to Polar Stereographic grid indices.
///
/// Polar Stereographic grids (NBM-Alaska) use RowOrigin::South.
fn resample_polar_stereo_to_geographic(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    bbox: &BoundingBox,
    model: &str,
) -> GridRegion {
    let mut output = vec![f32::NAN; output_width * output_height];

    let out_res_lon = bbox.width() / output_width as f64;
    let out_res_lat = bbox.height() / output_height as f64;

    // Select the appropriate Polar Stereographic projection based on model
    let proj = match model {
        "nbm-alaska" => PolarStereographic::nbm_alaska(),
        _ => PolarStereographic::nbm_alaska(), // Default to NBM Alaska
    };

    // Calculate scale factors if data dimensions differ from native projection dimensions
    let scale_x = data_width as f64 / proj.nx as f64;
    let scale_y = data_height as f64 / proj.ny as f64;

    for out_y in 0..output_height {
        for out_x in 0..output_width {
            let lon = bbox.min_lon + (out_x as f64 + 0.5) * out_res_lon;
            let lat = bbox.max_lat - (out_y as f64 + 0.5) * out_res_lat;

            // Convert geographic to Polar Stereographic grid indices using proper projection math
            let (native_i, native_j) = proj.geo_to_grid(lat, lon);

            // Scale indices to match actual data dimensions
            let grid_i = native_i * scale_x;
            let grid_j = native_j * scale_y;

            if grid_i < 0.0
                || grid_i >= data_width as f64 - 1.0
                || grid_j < 0.0
                || grid_j >= data_height as f64 - 1.0
            {
                continue;
            }

            let value = bilinear_interpolate(data, data_width, data_height, grid_i, grid_j);
            output[out_y * output_width + out_x] = value;
        }
    }

    GridRegion::new(
        output,
        output_width,
        output_height,
        *bbox,
        (out_res_lon, out_res_lat),
    )
}

/// Resample from Mercator grid to geographic output.
///
/// This uses proper projection math (geo_to_grid) to correctly transform
/// geographic coordinates to Mercator grid indices.
///
/// Mercator grids (NBM-Hawaii, NBM-PuertoRico, NBM-Guam) use RowOrigin::South.
fn resample_mercator_to_geographic(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    bbox: &BoundingBox,
    model: &str,
) -> GridRegion {
    let mut output = vec![f32::NAN; output_width * output_height];

    let out_res_lon = bbox.width() / output_width as f64;
    let out_res_lat = bbox.height() / output_height as f64;

    // Select the appropriate Mercator projection based on model
    let proj = match model {
        "nbm-hawaii" => Mercator::nbm_hawaii(),
        "nbm-puertorico" => Mercator::nbm_puertorico(),
        "nbm-guam" => Mercator::nbm_guam(),
        _ => Mercator::nbm_hawaii(), // Default to NBM Hawaii
    };

    // Calculate scale factors if data dimensions differ from native projection dimensions
    let scale_x = data_width as f64 / proj.nx as f64;
    let scale_y = data_height as f64 / proj.ny as f64;

    for out_y in 0..output_height {
        for out_x in 0..output_width {
            let lon = bbox.min_lon + (out_x as f64 + 0.5) * out_res_lon;
            let lat = bbox.max_lat - (out_y as f64 + 0.5) * out_res_lat;

            // Convert geographic to Mercator grid indices using proper projection math
            let (native_i, native_j) = proj.geo_to_grid(lat, lon);

            // Scale indices to match actual data dimensions
            let grid_i = native_i * scale_x;
            let grid_j = native_j * scale_y;

            if grid_i < 0.0
                || grid_i >= data_width as f64 - 1.0
                || grid_j < 0.0
                || grid_j >= data_height as f64 - 1.0
            {
                continue;
            }

            let value = bilinear_interpolate(data, data_width, data_height, grid_i, grid_j);
            output[out_y * output_width + out_x] = value;
        }
    }

    GridRegion::new(
        output,
        output_width,
        output_height,
        *bbox,
        (out_res_lon, out_res_lat),
    )
}

/// Extract a subregion from a 0-360 global grid for a request that crosses the prime meridian.
///
/// When a request bbox like (-10, 45, 10, 55) is made against a 0-360 grid (like GFS),
/// the grid reader returns the full global grid. This function extracts just the
/// requested region using bilinear interpolation with proper wrapping at the prime meridian.
///
/// # Arguments
/// * `region` - The full global grid region (bbox should be 0-360)
/// * `request_bbox` - The original request bbox in -180/180 coordinates
///
/// # Returns
/// A new GridRegion covering just the requested area with -180/180 coordinates
pub fn extract_prime_meridian_region(
    region: &GridRegion,
    request_bbox: &BoundingBox,
) -> GridRegion {
    // Check if this is a 0-360 grid
    if !region.bbox.uses_0_360_longitude() {
        return region.clone();
    }

    // Check if request crosses prime meridian (negative min_lon, positive max_lon)
    if request_bbox.min_lon >= 0.0 || request_bbox.max_lon <= 0.0 {
        return region.clone();
    }

    let (res_lon, res_lat) = region.resolution;
    let grid_width = region.width;
    let grid_height = region.height;

    // Calculate output dimensions
    let request_width = request_bbox.width();
    let request_height = request_bbox.height();
    let out_width = ((request_width / res_lon).ceil() as usize).max(1);
    let out_height = ((request_height / res_lat).ceil() as usize).max(1);

    let mut output = vec![f32::NAN; out_width * out_height];

    // Check if this is a global grid (covers ~360 degrees)
    let is_global = (region.bbox.max_lon - region.bbox.min_lon) > 359.0;

    // For each output pixel, use bilinear interpolation from source grid
    for out_y in 0..out_height {
        for out_x in 0..out_width {
            // Calculate the lon/lat for this output pixel (using -180/180 coordinates)
            let lon = request_bbox.min_lon + (out_x as f64 + 0.5) * res_lon;
            let lat = request_bbox.max_lat - (out_y as f64 + 0.5) * res_lat;

            // Convert to 0-360 coordinates for source lookup
            let src_lon = if lon < 0.0 { lon + 360.0 } else { lon };

            // Calculate continuous grid coordinates for bilinear interpolation
            let grid_x = (src_lon - region.bbox.min_lon) / res_lon;
            let grid_y = (region.bbox.max_lat - lat) / res_lat;

            // Get integer indices
            let x1 = grid_x.floor() as isize;
            let y1 = grid_y.floor() as isize;

            // For global grids, wrap x2 around at the prime meridian
            let x2 = if is_global && x1 + 1 >= grid_width as isize {
                0 // Wrap to column 0
            } else {
                (x1 + 1).min(grid_width as isize - 1)
            };
            let y2 = (y1 + 1).min(grid_height as isize - 1);

            // Bounds check for y (x wraps for global grids)
            if y1 < 0 || y1 >= grid_height as isize {
                continue;
            }
            if x1 < 0 || (!is_global && x1 >= grid_width as isize) {
                continue;
            }

            // Handle x1 wrapping for global grids
            let x1_idx = if x1 >= grid_width as isize {
                (x1 as usize) % grid_width
            } else {
                x1 as usize
            };

            let x2_idx = x2 as usize;
            let y1_idx = y1 as usize;
            let y2_idx = y2 as usize;

            // Fractional parts for interpolation
            let dx = (grid_x - x1 as f64) as f32;
            let dy = (grid_y - y1 as f64) as f32;

            // Sample four surrounding grid points
            let v11 = region
                .data
                .get(y1_idx * grid_width + x1_idx)
                .copied()
                .unwrap_or(f32::NAN);
            let v21 = region
                .data
                .get(y1_idx * grid_width + x2_idx)
                .copied()
                .unwrap_or(f32::NAN);
            let v12 = region
                .data
                .get(y2_idx * grid_width + x1_idx)
                .copied()
                .unwrap_or(f32::NAN);
            let v22 = region
                .data
                .get(y2_idx * grid_width + x2_idx)
                .copied()
                .unwrap_or(f32::NAN);

            // Skip if any corner is NaN
            if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
                continue;
            }

            // Bilinear interpolation
            let v1 = v11 * (1.0 - dx) + v21 * dx;
            let v2 = v12 * (1.0 - dx) + v22 * dx;
            let value = v1 * (1.0 - dy) + v2 * dy;

            let out_idx = out_y * out_width + out_x;
            output[out_idx] = value;
        }
    }

    // Return with -180/180 coordinates
    GridRegion::new(
        output,
        out_width,
        out_height,
        *request_bbox,
        (res_lon, res_lat),
    )
}

/// Extract and convert a region from a 0-360 grid to -180/180 output coordinates
/// for requests that cross the dateline using extended notation (e.g., 140° to 235°).
///
/// This handles the case where:
/// - The grid uses 0-360 longitude convention (like GFS)
/// - The request uses extended notation with max_lon > 180° (e.g., 140° to 235°)
/// - The output should use standard -180/180 coordinates (e.g., 140° to -125°)
///
/// # Arguments
/// * `region` - The grid region (may be a subset or full grid, bbox in 0-360 or mixed)
/// * `request_bbox` - The original request bbox (may use extended notation)
///
/// # Returns
/// A GridRegion with bbox converted to -180/180 format if needed, otherwise unchanged.
pub fn extract_dateline_region(region: &GridRegion, request_bbox: &BoundingBox) -> GridRegion {
    // Check if request uses extended notation crossing dateline (max_lon > 180)
    if !request_bbox.uses_extended_notation_dateline() {
        return region.clone();
    }

    // Check if the region's bbox also uses extended notation (from the grid read)
    // This happens when the grid read preserved the 0-360 coordinates
    if region.bbox.max_lon <= 180.0 {
        // Region bbox is already in -180/180 format, nothing to do
        return region.clone();
    }

    // Convert the region's bbox from extended notation to crossing notation
    // e.g., 140 to 235 becomes 140 to -125 (where -125 = 235 - 360)
    let output_bbox = BoundingBox::new(
        region.bbox.min_lon,
        region.bbox.min_lat,
        region.bbox.max_lon - 360.0, // Convert 235 → -125
        region.bbox.max_lat,
    );

    // Data is already correct (the grid read returned the right columns)
    // We just need to update the bbox metadata for output
    GridRegion::new(
        region.data.clone(),
        region.width,
        region.height,
        output_bbox,
        region.resolution,
    )
}

/// Bilinear interpolation at continuous grid coordinates.
fn bilinear_interpolate(
    data: &[f32],
    width: usize,
    height: usize,
    grid_x: f64,
    grid_y: f64,
) -> f32 {
    let i1 = grid_x.floor() as usize;
    let j1 = grid_y.floor() as usize;
    let i2 = (i1 + 1).min(width - 1);
    let j2 = (j1 + 1).min(height - 1);

    let di = (grid_x - i1 as f64) as f32;
    let dj = (grid_y - j1 as f64) as f32;

    // Sample four surrounding grid points
    let v11 = data.get(j1 * width + i1).copied().unwrap_or(f32::NAN);
    let v21 = data.get(j1 * width + i2).copied().unwrap_or(f32::NAN);
    let v12 = data.get(j2 * width + i1).copied().unwrap_or(f32::NAN);
    let v22 = data.get(j2 * width + i2).copied().unwrap_or(f32::NAN);

    // Skip if any corner is NaN
    if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
        return f32::NAN;
    }

    // Bilinear interpolation
    let v1 = v11 * (1.0 - di) + v21 * di;
    let v2 = v12 * (1.0 - di) + v22 * di;
    v1 * (1.0 - dj) + v2 * dj
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bilinear_interpolate_center() {
        // 2x2 grid with values 1, 2, 3, 4
        let data = vec![1.0, 2.0, 3.0, 4.0];

        // Center of grid should be average of all 4 corners
        let val = bilinear_interpolate(&data, 2, 2, 0.5, 0.5);
        assert!((val - 2.5).abs() < 0.01); // (1+2+3+4)/4 = 2.5
    }

    #[test]
    fn test_bilinear_interpolate_corners() {
        let data = vec![1.0, 2.0, 3.0, 4.0];

        // At exact grid points, should return exact values
        let v00 = bilinear_interpolate(&data, 2, 2, 0.0, 0.0);
        assert!((v00 - 1.0).abs() < 0.01);

        let v10 = bilinear_interpolate(&data, 2, 2, 1.0, 0.0);
        assert!((v10 - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_bilinear_interpolate_with_nan() {
        let data = vec![1.0, f32::NAN, 3.0, 4.0];

        // Should return NaN if any corner is NaN
        let val = bilinear_interpolate(&data, 2, 2, 0.5, 0.5);
        assert!(val.is_nan());
    }

    #[test]
    fn test_resample_geographic_passthrough() {
        // Geographic projection should return clone
        let region = GridRegion::new(
            vec![1.0, 2.0, 3.0, 4.0],
            2,
            2,
            BoundingBox::new(-100.0, 35.0, -98.0, 37.0),
            (1.0, 1.0),
        );

        let result = resample_to_geographic(&region, ProjectionType::Geographic, "gfs", None);

        assert_eq!(result.width, region.width);
        assert_eq!(result.height, region.height);
        assert_eq!(result.data, region.data);
    }

    #[test]
    fn test_extract_prime_meridian_region() {
        // Create a simple 0-360 global grid (4 columns x 2 rows)
        // Each column represents 90 degrees: 0-90, 90-180, 180-270, 270-360
        // Values are column indices for easy verification
        let data = vec![
            0.0, 1.0, 2.0, 3.0, // row 0 (north)
            0.0, 1.0, 2.0, 3.0, // row 1 (south)
        ];
        let region = GridRegion::new(
            data,
            4,
            2,
            BoundingBox::new(0.0, -90.0, 360.0, 90.0),
            (90.0, 90.0),
        );

        // Request crossing prime meridian: -45 to 45
        let request_bbox = BoundingBox::new(-45.0, -90.0, 45.0, 90.0);
        let result = extract_prime_meridian_region(&region, &request_bbox);

        // Output should cover -45 to 45 (90 degrees = 1 column in this grid)
        // Since request is 90 degrees wide and resolution is 90 degrees,
        // we should get 1 column
        assert_eq!(result.width, 1);
        assert_eq!(result.height, 2);

        // The center of our request (-45 to 45) is 0 degrees
        // In 0-360 coords, 0 degrees maps to column 0 (the 0.0 value)
        // and -45 maps to 315 (column 3, value 3.0)
        // Since we're sampling at center of output pixel, we get column 3 (315 degrees = -45)
        // Actually at -45 + 45 = 0 center, so 0 + 360 = 360 -> column 0

        // The bbox should be in -180/180 coordinates
        assert!((result.bbox.min_lon - (-45.0)).abs() < 0.01);
        assert!((result.bbox.max_lon - 45.0).abs() < 0.01);
    }

    #[test]
    fn test_extract_prime_meridian_non_crossing() {
        // Request that doesn't cross prime meridian should return clone
        let region = GridRegion::new(
            vec![1.0, 2.0, 3.0, 4.0],
            2,
            2,
            BoundingBox::new(0.0, -90.0, 360.0, 90.0),
            (180.0, 90.0),
        );

        // Request entirely in positive longitudes
        let request_bbox = BoundingBox::new(10.0, -45.0, 20.0, 45.0);
        let result = extract_prime_meridian_region(&region, &request_bbox);

        // Should return unchanged since request doesn't cross prime meridian
        assert_eq!(result.width, region.width);
        assert_eq!(result.height, region.height);
    }

    #[test]
    fn test_extract_dateline_region_extended_notation() {
        // Region from 0-360 grid read, covering 140° to 235° (Japan to California)
        let region = GridRegion::new(
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            4,
            2,
            BoundingBox::new(140.0, 25.0, 235.0, 55.0), // Extended notation bbox
            (23.75, 15.0),
        );

        // Request using extended notation
        let request_bbox = BoundingBox::new(140.0, 25.0, 235.0, 55.0);
        let result = extract_dateline_region(&region, &request_bbox);

        // Data should be unchanged
        assert_eq!(result.width, region.width);
        assert_eq!(result.height, region.height);
        assert_eq!(result.data, region.data);

        // Bbox should be converted to -180/180 format
        // 140° stays 140°, 235° becomes -125° (235 - 360 = -125)
        assert!((result.bbox.min_lon - 140.0).abs() < 0.01);
        assert!((result.bbox.max_lon - (-125.0)).abs() < 0.01);
    }

    #[test]
    fn test_extract_dateline_region_non_extended() {
        // Region with normal bbox (no dateline crossing)
        let region = GridRegion::new(
            vec![1.0, 2.0, 3.0, 4.0],
            2,
            2,
            BoundingBox::new(-100.0, 25.0, -90.0, 55.0),
            (5.0, 15.0),
        );

        // Request with normal notation
        let request_bbox = BoundingBox::new(-100.0, 25.0, -90.0, 55.0);
        let result = extract_dateline_region(&region, &request_bbox);

        // Should return unchanged (no extended notation)
        assert_eq!(result.width, region.width);
        assert_eq!(result.height, region.height);
        assert_eq!(result.bbox.min_lon, region.bbox.min_lon);
        assert_eq!(result.bbox.max_lon, region.bbox.max_lon);
    }

    #[test]
    fn test_extract_dateline_region_already_normalized() {
        // Region bbox already in -180/180 format (from some other processing)
        let region = GridRegion::new(
            vec![1.0, 2.0, 3.0, 4.0],
            2,
            2,
            BoundingBox::new(140.0, 25.0, -125.0, 55.0), // Already crossing notation
            (5.0, 15.0),
        );

        // Request using extended notation
        let request_bbox = BoundingBox::new(140.0, 25.0, 235.0, 55.0);
        let result = extract_dateline_region(&region, &request_bbox);

        // Should return unchanged (region bbox already normalized)
        assert_eq!(result.bbox.min_lon, region.bbox.min_lon);
        assert_eq!(result.bbox.max_lon, region.bbox.max_lon);
    }
}
