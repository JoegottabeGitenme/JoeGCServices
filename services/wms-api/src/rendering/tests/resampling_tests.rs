//! Tests for resampling functions.

use crate::rendering::resampling::{
    bilinear_interpolate, lat_to_mercator_y, mercator_y_to_lat, resample_for_mercator,
    resample_from_geographic,
};

// ============================================================================
// Web Mercator conversion tests
// ============================================================================

#[test]
fn test_lat_to_mercator_y_equator() {
    let y = lat_to_mercator_y(0.0);
    assert!((y - 0.0).abs() < 0.001, "Equator should map to Y=0");
}

#[test]
fn test_lat_to_mercator_y_positive_lat() {
    let y = lat_to_mercator_y(45.0);
    assert!(y > 0.0, "Positive latitude should have positive Y");
}

#[test]
fn test_lat_to_mercator_y_negative_lat() {
    let y = lat_to_mercator_y(-45.0);
    assert!(y < 0.0, "Negative latitude should have negative Y");
}

#[test]
fn test_mercator_y_to_lat_equator() {
    let lat = mercator_y_to_lat(0.0);
    assert!((lat - 0.0).abs() < 0.001, "Y=0 should map to equator");
}

#[test]
fn test_mercator_roundtrip() {
    for lat in [-60.0, -30.0, 0.0, 30.0, 60.0] {
        let y = lat_to_mercator_y(lat);
        let lat_back = mercator_y_to_lat(y);
        assert!(
            (lat - lat_back).abs() < 0.0001,
            "Roundtrip failed for lat={}: got {}",
            lat,
            lat_back
        );
    }
}

#[test]
fn test_mercator_symmetry() {
    let y_pos = lat_to_mercator_y(45.0);
    let y_neg = lat_to_mercator_y(-45.0);
    assert!(
        (y_pos + y_neg).abs() < 0.001,
        "Mercator Y should be symmetric around equator"
    );
}

// ============================================================================
// Bilinear interpolation tests
// ============================================================================

#[test]
fn test_bilinear_interpolate_corner() {
    // 2x2 grid with values 0, 1, 2, 3
    let grid = vec![0.0f32, 1.0, 2.0, 3.0];
    let result = bilinear_interpolate(&grid, 2, 2, 0.0, 0.0, false).unwrap();
    assert!((result - 0.0).abs() < 0.001, "Corner should be exact value");
}

#[test]
fn test_bilinear_interpolate_center() {
    // 2x2 grid with values 0, 1, 2, 3
    let grid = vec![0.0f32, 1.0, 2.0, 3.0];
    let result = bilinear_interpolate(&grid, 2, 2, 0.5, 0.5, false).unwrap();
    // Center should be average of all 4: (0+1+2+3)/4 = 1.5
    assert!(
        (result - 1.5).abs() < 0.001,
        "Center should be average: got {}",
        result
    );
}

#[test]
fn test_bilinear_interpolate_horizontal_midpoint() {
    let grid = vec![0.0f32, 2.0, 0.0, 2.0];
    let result = bilinear_interpolate(&grid, 2, 2, 0.5, 0.0, false).unwrap();
    assert!(
        (result - 1.0).abs() < 0.001,
        "Horizontal midpoint should be 1.0: got {}",
        result
    );
}

#[test]
fn test_bilinear_interpolate_vertical_midpoint() {
    let grid = vec![0.0f32, 0.0, 2.0, 2.0];
    let result = bilinear_interpolate(&grid, 2, 2, 0.0, 0.5, false).unwrap();
    assert!(
        (result - 1.0).abs() < 0.001,
        "Vertical midpoint should be 1.0: got {}",
        result
    );
}

#[test]
fn test_bilinear_interpolate_with_wrap() {
    // Test wrap_longitude functionality
    let grid = vec![1.0f32, 2.0, 3.0, 4.0];
    let result = bilinear_interpolate(&grid, 2, 2, 1.5, 0.0, true).unwrap();
    // With wrap, x=1.5 should interpolate between x=1 and x=0
    // At y=0: v21=2.0, v11=1.0 (wrapped), so (2.0 * 0.5 + 1.0 * 0.5) = 1.5
    assert!(
        (result - 1.5).abs() < 0.001,
        "Wrapped interpolation should be 1.5: got {}",
        result
    );
}

// ============================================================================
// Geographic resampling tests
// ============================================================================

#[test]
fn test_resample_from_geographic_identity() {
    // Create a simple 4x4 grid
    let data: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let bbox = [-180.0f32, -90.0, 180.0, 90.0];

    let result = resample_from_geographic(&data, 4, 4, 4, 4, bbox, bbox, false);

    // For identity transform, values should be similar (not exact due to pixel center sampling)
    assert_eq!(result.len(), 16);
    // Check corners are approximately correct
    assert!(!result[0].is_nan(), "Top-left should have value");
    assert!(!result[3].is_nan(), "Top-right should have value");
}

#[test]
fn test_resample_from_geographic_subset() {
    // Create a simple 4x4 grid
    let data: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let data_bbox = [-180.0f32, -90.0, 180.0, 90.0];
    let output_bbox = [-90.0f32, -45.0, 90.0, 45.0];

    let result = resample_from_geographic(&data, 4, 4, 4, 4, output_bbox, data_bbox, false);

    // Should get valid values for subset
    assert_eq!(result.len(), 16);
    let valid_count = result.iter().filter(|v| !v.is_nan()).count();
    assert!(valid_count > 0, "Should have some valid values in subset");
}

#[test]
fn test_resample_from_geographic_out_of_bounds() {
    // Create a simple 4x4 grid covering a small region
    let data: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let data_bbox = [0.0f32, 0.0, 10.0, 10.0];
    let output_bbox = [50.0f32, 50.0, 60.0, 60.0]; // Completely outside

    let result = resample_from_geographic(&data, 4, 4, 4, 4, output_bbox, data_bbox, false);

    // All values should be NaN since output is outside data bounds
    let nan_count = result.iter().filter(|v| v.is_nan()).count();
    assert_eq!(nan_count, 16, "All values should be NaN for out-of-bounds");
}

#[test]
fn test_resample_from_geographic_360_longitude() {
    // Test handling of 0-360 longitude grids (like GFS)
    let data: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let data_bbox = [0.0f32, -90.0, 360.0, 90.0];
    let output_bbox = [-90.0f32, -45.0, 0.0, 45.0]; // Western hemisphere

    let result = resample_from_geographic(&data, 4, 4, 4, 4, output_bbox, data_bbox, true);

    // Should get valid values for western hemisphere from 0-360 grid
    assert_eq!(result.len(), 16);
    let valid_count = result.iter().filter(|v| !v.is_nan()).count();
    assert!(
        valid_count > 0,
        "Should have valid values for western hemisphere from 0-360 grid"
    );
}

// ============================================================================
// Mercator resampling tests
// ============================================================================

#[test]
fn test_resample_for_mercator_equatorial() {
    // Create a simple 4x4 grid
    let data: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let data_bbox = [-180.0f32, -85.0, 180.0, 85.0];
    let output_bbox = [-45.0f32, -20.0, 45.0, 20.0]; // Equatorial region

    let result = resample_for_mercator(&data, 4, 4, 4, 4, output_bbox, data_bbox, false);

    assert_eq!(result.len(), 16);
    let valid_count = result.iter().filter(|v| !v.is_nan()).count();
    assert!(
        valid_count > 0,
        "Should have valid values in equatorial region"
    );
}

#[test]
fn test_resample_for_mercator_high_latitude() {
    // Create a simple 4x4 grid
    let data: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let data_bbox = [-180.0f32, -85.0, 180.0, 85.0];
    let output_bbox = [-45.0f32, 60.0, 45.0, 80.0]; // High latitude

    let result = resample_for_mercator(&data, 4, 4, 4, 4, output_bbox, data_bbox, false);

    assert_eq!(result.len(), 16);
    let valid_count = result.iter().filter(|v| !v.is_nan()).count();
    assert!(
        valid_count > 0,
        "Should have valid values at high latitudes"
    );
}

// ============================================================================
// NDFD Lambert Conformal resampling tests
// ============================================================================

#[test]
fn test_ndfd_lambert_resampling_geographic_coordinates() {
    use crate::rendering::resampling::resample_grid_for_bbox_with_proj;

    // Create a synthetic NDFD grid with known values at specific locations
    // NDFD: 2145 x 1377, grid indices:
    // - LA (34°N, 118.2°W): i=239, j=572
    // - NY (40.7°N, 74°W): i=1810, j=856
    // - Miami (25.8°N, 80.2°W): i=1670, j=171
    // - Kansas City (38.5°N, 98°W): i=982, j=702 (approx)

    let ndfd_width = 2145;
    let ndfd_height = 1377;
    let mut ndfd_data = vec![f32::NAN; ndfd_width * ndfd_height];

    // Set specific values at known grid locations
    // Value = latitude * 10 (easy to verify)
    // Fill a larger region (50x50) around each point to ensure bilinear interpolation works
    let set_value = |data: &mut Vec<f32>, i: usize, j: usize, val: f32| {
        let radius = 50i32;
        for di in -radius..=radius {
            for dj in -radius..=radius {
                let ni = (i as i32 + di) as usize;
                let nj = (j as i32 + dj) as usize;
                if ni < ndfd_width && nj < ndfd_height {
                    let nidx = nj * ndfd_width + ni;
                    if nidx < data.len() {
                        data[nidx] = val;
                    }
                }
            }
        }
    };

    // LA at (239, 572) - value 340 (34.0°N * 10)
    set_value(&mut ndfd_data, 239, 572, 340.0);

    // NY at (1810, 856) - value 407 (40.7°N * 10)
    set_value(&mut ndfd_data, 1810, 856, 407.0);

    // Miami at (1670, 171) - value 258 (25.8°N * 10)
    set_value(&mut ndfd_data, 1670, 171, 258.0);

    // Kansas at (982, 702) - value 385 (38.5°N * 10)
    set_value(&mut ndfd_data, 982, 702, 385.0);

    // Test resampling a small area around Los Angeles
    // LA is at ~34°N, 118.2°W
    let la_bbox = [-119.0f32, 33.0, -117.0, 35.0]; // 2° x 2° box around LA
    let output_width = 10;
    let output_height = 10;

    let result = resample_grid_for_bbox_with_proj(
        &ndfd_data,
        ndfd_width,
        ndfd_height,
        output_width,
        output_height,
        la_bbox,
        la_bbox, // data_bounds (not used for Lambert)
        false,   // use_mercator
        "ndfd",
        None,  // goes_projection
        false, // grid_uses_360
    );

    assert_eq!(result.len(), output_width * output_height);

    // The center of the output should have a value close to LA's value (340)
    let center_idx = (output_height / 2) * output_width + (output_width / 2);
    let center_value = result[center_idx];

    println!("LA bbox resampling result:");
    println!("  Center value: {}", center_value);

    // Debug: print all non-NaN values
    let valid_values: Vec<_> = result
        .iter()
        .enumerate()
        .filter(|(_, v)| !v.is_nan())
        .collect();
    println!("  Valid values in LA result: {:?}", valid_values);

    // Debug: Check what grid indices LA maps to
    let proj = projection::LambertConformal::ndfd();
    let (la_i, la_j) = proj.geo_to_grid(34.0, -118.2);
    println!(
        "  LA (34°N, 118.2°W) maps to grid: i={:.1}, j={:.1}",
        la_i, la_j
    );

    // Debug: test several points in the LA bbox
    for (lat, lon) in [(33.5, -118.5), (34.0, -118.0), (34.5, -117.5)] {
        let (i, j) = proj.geo_to_grid(lat, lon);
        println!("  ({:.1}°N, {:.1}°W) -> i={:.1}, j={:.1}", lat, -lon, i, j);
        let in_bounds =
            i >= 0.0 && i < (ndfd_width - 1) as f64 && j >= 0.0 && j < (ndfd_height - 1) as f64;
        println!(
            "    In bounds: {} (w={}, h={})",
            in_bounds, ndfd_width, ndfd_height
        );
    }

    // Check if we set the value at the right place
    let la_idx = 572 * ndfd_width + 239;
    println!(
        "  Value at LA grid position (239, 572): {}",
        ndfd_data[la_idx]
    );

    // Check if we got a valid value (not NaN)
    assert!(
        !center_value.is_nan() || !valid_values.is_empty(),
        "LA bbox should have at least some valid values, got all NaN. LA maps to ({:.1}, {:.1})",
        la_i,
        la_j
    );

    // The value should be close to 340 (LA's latitude * 10)
    // Allow some tolerance for interpolation
    assert!(
        (center_value - 340.0).abs() < 50.0,
        "LA region should have value near 340, got {}",
        center_value
    );

    // Test resampling around New York
    let ny_bbox = [-75.0f32, 40.0, -73.0, 42.0];
    let ny_result = resample_grid_for_bbox_with_proj(
        &ndfd_data,
        ndfd_width,
        ndfd_height,
        output_width,
        output_height,
        ny_bbox,
        ny_bbox,
        false,
        "ndfd",
        None,
        false,
    );

    let ny_center_idx = (output_height / 2) * output_width + (output_width / 2);
    let ny_center_value = ny_result[ny_center_idx];

    println!("NY bbox resampling result:");
    println!("  Center value: {}", ny_center_value);

    assert!(
        !ny_center_value.is_nan(),
        "Center of NY bbox should have valid value, got NaN"
    );

    // NY value should be near 407, not 340 (which would indicate mirroring)
    assert!(
        (ny_center_value - 407.0).abs() < 50.0,
        "NY region should have value near 407 (not 340 if mirrored), got {}",
        ny_center_value
    );

    // CRITICAL: If the image is horizontally mirrored, NY would show LA's value (340)
    // and LA would show NY's value (407). Let's verify this isn't happening.
    assert!(
        (center_value - 407.0).abs() > 30.0,
        "LA region should NOT have NY's value (407), got {} - possible horizontal mirror!",
        center_value
    );
}

/// Test NDFD Lambert resampling with Mercator output
#[test]
fn test_ndfd_lambert_mercator_resampling() {
    use crate::rendering::resampling::resample_grid_for_bbox_with_proj;

    // Create a synthetic NDFD grid with gradient values
    // Value = column index / 10, so west has small values, east has large values
    let ndfd_width = 2145;
    let ndfd_height = 1377;
    let mut ndfd_data = vec![f32::NAN; ndfd_width * ndfd_height];

    // Fill the grid with a simple pattern:
    // Values increase from west (small i) to east (large i)
    // This makes it easy to detect horizontal mirroring
    for j in 0..ndfd_height {
        for i in 0..ndfd_width {
            let idx = j * ndfd_width + i;
            // Value = i / 10, so ranges from 0 (west) to 214.4 (east)
            ndfd_data[idx] = i as f32 / 10.0;
        }
    }

    // Test resampling the full CONUS view with Mercator
    // This is what the web map would request
    let conus_bbox = [-130.0f32, 20.0, -60.0, 55.0]; // Approximate CONUS bounds
    let output_width = 100;
    let output_height = 50;

    let result = resample_grid_for_bbox_with_proj(
        &ndfd_data,
        ndfd_width,
        ndfd_height,
        output_width,
        output_height,
        conus_bbox,
        conus_bbox,
        true, // use_mercator
        "ndfd",
        None,
        false,
    );

    assert_eq!(result.len(), output_width * output_height);

    // Sample values at different longitudes
    // West side (col 10, ~-124°) should have small values (i~200, val~20)
    // Center (col 50, ~-95°) should have medium values (i~1080, val~108)
    // East side (col 90, ~-67°) should have large values (i~1900, val~190)

    let row = output_height / 2; // Middle row (around lat 37°N)

    let west_val = result[row * output_width + 10];
    let center_val = result[row * output_width + 50];
    let east_val = result[row * output_width + 90];

    println!("Mercator resampling gradient test (row {}):", row);
    println!("  West (col 10): {:.1}", west_val);
    println!("  Center (col 50): {:.1}", center_val);
    println!("  East (col 90): {:.1}", east_val);

    // Check that values increase from west to east
    if !west_val.is_nan() && !center_val.is_nan() {
        assert!(
            west_val < center_val,
            "West should have smaller value than center: {:.1} < {:.1}",
            west_val,
            center_val
        );
    }
    if !center_val.is_nan() && !east_val.is_nan() {
        assert!(
            center_val < east_val,
            "Center should have smaller value than east: {:.1} < {:.1}",
            center_val,
            east_val
        );
    }

    // Print a row of values to visualize the pattern
    println!("\n  Row {} values (every 10 columns):", row);
    for col in (0..output_width).step_by(10) {
        let val = result[row * output_width + col];
        print!("{:6.1}", val);
    }
    println!();

    // MIRRORING CHECK: If mirrored, west and east would have swapped values
    // West should be ~20-50, East should be ~150-200
    if !west_val.is_nan() {
        assert!(
            west_val < 100.0,
            "West should have low values (<100), got {:.1} - possible mirror!",
            west_val
        );
    }
    if !east_val.is_nan() {
        assert!(
            east_val > 100.0,
            "East should have high values (>100), got {:.1} - possible mirror!",
            east_val
        );
    }
}
