//! Reprojection utilities for converting grids between coordinate systems.
//!
//! This module provides functions for reprojecting satellite data from native
//! projections (e.g., geostationary) to regular geographic (lat/lon) grids.

use projection::Geostationary;

use super::bilinear_interpolate;
use crate::types::BoundingBox;

/// Reproject geostationary satellite data to a regular lat/lon grid.
///
/// This function converts data from native geostationary projection (scan angles)
/// to a regular geographic grid. The output grid maintains the same number of
/// pixels as the input, preserving all native data points.
///
/// # Arguments
/// * `data` - Source grid data in row-major order (top-to-bottom, left-to-right)
/// * `data_width` - Source grid width (number of columns)
/// * `data_height` - Source grid height (number of rows)
/// * `proj` - Geostationary projection parameters from the satellite data
///
/// # Returns
/// A tuple of `(reprojected_data, output_width, output_height, output_bbox)`:
/// * `reprojected_data` - Grid data reprojected to geographic coordinates
/// * `output_width` - Output grid width (same as input)
/// * `output_height` - Output grid height (same as input)
/// * `output_bbox` - Geographic bounding box of the output grid
///
/// # Notes
/// * Pixels outside the satellite's view are set to NaN
/// * The output resolution varies spatially due to the projection transformation
/// * Bilinear interpolation is used for smooth resampling
///
/// # Example
/// ```
/// use projection::Geostationary;
/// use grid_processor::projection::reproject::reproject_geostationary_to_geographic;
///
/// // Use GOES-16 CONUS projection with known valid parameters
/// let proj = Geostationary::goes16_conus();
/// let width = proj.nx;
/// let height = proj.ny;
///
/// // Create sample data
/// let data: Vec<f32> = vec![0.0; width * height];
///
/// let (reprojected, out_width, out_height, bbox) =
///     reproject_geostationary_to_geographic(&data, width, height, &proj);
///
/// assert_eq!(out_width, width);
/// assert_eq!(out_height, height);
/// ```
pub fn reproject_geostationary_to_geographic(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    proj: &Geostationary,
) -> (Vec<f32>, usize, usize, BoundingBox) {
    // Compute geographic bounds from projection by sampling the grid edges
    let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();

    // Output dimensions match input (preserves pixel count)
    let output_width = data_width;
    let output_height = data_height;

    // Calculate pixel spacing in geographic coordinates
    let lon_step = (max_lon - min_lon) / output_width as f64;
    let lat_step = (max_lat - min_lat) / output_height as f64;

    let mut output = vec![f32::NAN; output_width * output_height];

    // For each output pixel in the geographic grid
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Calculate geographic coordinates (pixel center)
            // X increases left-to-right (west to east, min_lon to max_lon)
            // Y increases top-to-bottom in output array, but decreases in lat (max_lat to min_lat)
            let lon = min_lon + (out_x as f64 + 0.5) * lon_step;
            let lat = max_lat - (out_y as f64 + 0.5) * lat_step;

            // Convert geographic coordinates to source grid indices
            // geo_to_grid returns Option<(i, j)> where i is x-index, j is y-index
            if let Some((grid_i, grid_j)) = proj.geo_to_grid(lat, lon) {
                // Check bounds with margin for bilinear interpolation (need 4 surrounding pixels)
                if grid_i >= 0.0
                    && grid_i < (data_width - 1) as f64
                    && grid_j >= 0.0
                    && grid_j < (data_height - 1) as f64
                {
                    // Use bilinear interpolation for smooth resampling
                    output[out_y * output_width + out_x] =
                        bilinear_interpolate(data, data_width, data_height, grid_i, grid_j);
                }
            }
            // Points outside satellite view remain NaN (initialized above)
        }
    }

    let bbox = BoundingBox::new(min_lon, min_lat, max_lon, max_lat);
    (output, output_width, output_height, bbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reproject_preserves_dimensions() {
        // Use the standard GOES-16 CONUS projection which has known valid parameters
        let proj = Geostationary::goes16_conus();
        let width = proj.nx;
        let height = proj.ny;

        // Create test data matching the projection dimensions
        let data: Vec<f32> = (0..width * height).map(|i| i as f32).collect();

        let (output, out_width, out_height, bbox) =
            reproject_geostationary_to_geographic(&data, width, height, &proj);

        // Output dimensions should match input
        assert_eq!(out_width, width);
        assert_eq!(out_height, height);
        assert_eq!(output.len(), width * height);

        // Bbox should be valid
        assert!(bbox.min_lon < bbox.max_lon, "min_lon ({}) should be < max_lon ({})", bbox.min_lon, bbox.max_lon);
        assert!(bbox.min_lat < bbox.max_lat, "min_lat ({}) should be < max_lat ({})", bbox.min_lat, bbox.max_lat);
    }

    #[test]
    fn test_reproject_handles_nan_input() {
        // Use the standard GOES-16 CONUS projection
        let proj = Geostationary::goes16_conus();
        let width = proj.nx;
        let height = proj.ny;

        // Create test data with some NaN values
        let mut data: Vec<f32> = (0..width * height).map(|i| i as f32).collect();

        // Set some values to NaN
        data[45] = f32::NAN;
        data[46] = f32::NAN;
        data[1000] = f32::NAN;

        let (output, _, _, _) = reproject_geostationary_to_geographic(&data, width, height, &proj);

        // Should not panic, and output should have some valid values
        let valid_count = output.iter().filter(|v| !v.is_nan()).count();
        assert!(valid_count > 0, "Should have some valid output values");
    }
}
