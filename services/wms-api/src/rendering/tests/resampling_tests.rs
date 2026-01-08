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

/// Test NDFD Lambert resampling returns correct grid indices
/// This test checks that the projection maps coordinates correctly
/// and that the western side of the grid is not mirrored.
#[test]
fn test_ndfd_lambert_grid_indices() {
    use projection::LambertConformal;

    let proj = LambertConformal::ndfd();

    // Test specific coordinates across the CONUS
    let test_points = [
        // (lat, lon, expected_i_approx, expected_j_approx, name)
        (34.0, -118.2, 239.0, 572.0, "Los Angeles"), // West
        (40.7, -74.0, 1810.0, 856.0, "New York"),    // East
        (38.5, -95.0, 1088.0, 700.0, "Kansas (center)"), // Center
        (47.6, -122.3, 216.0, 1210.0, "Seattle"),    // NW
        (25.8, -80.2, 1670.0, 171.0, "Miami"),       // SE
    ];

    println!("\nTesting geo_to_grid for key locations:");
    for (lat, lon, expected_i, expected_j, name) in test_points {
        let (i, j) = proj.geo_to_grid(lat, lon);
        let i_err = (i - expected_i).abs();
        let j_err = (j - expected_j).abs();

        println!(
            "  {}: ({:.1}, {:.1}) -> i={:.1}, j={:.1} (expected i≈{:.0}, j≈{:.0})",
            name, lat, lon, i, j, expected_i, expected_j
        );

        // Allow some tolerance (within 50 grid points)
        assert!(i_err < 50.0, "{} i error too large: {:.1}", name, i_err);
        assert!(j_err < 50.0, "{} j error too large: {:.1}", name, j_err);
    }

    // Critical test: ensure western coordinates don't mirror to eastern indices
    // LA should have smaller i than NY
    let (la_i, _) = proj.geo_to_grid(34.0, -118.2);
    let (ny_i, _) = proj.geo_to_grid(40.7, -74.0);

    println!("\nMirroring check:");
    println!("  LA (west, -118.2°): i={:.1}", la_i);
    println!("  NY (east, -74.0°): i={:.1}", ny_i);
    assert!(
        la_i < ny_i,
        "LA (west) should have smaller i than NY (east)"
    );
    assert!(la_i < 500.0, "LA i should be in western half of grid");
    assert!(ny_i > 1500.0, "NY i should be in eastern half of grid");
}

// ============================================================================
// Lambert Conformal pyramid level scaling tests
// ============================================================================

/// Test that Lambert resampling correctly handles pyramid level scaling for HRRR
/// This test verifies the fix for the bug where pyramid level data (e.g., 449x264)
/// was being indexed using native resolution coordinates (1799x1059), causing
/// data to render in a tiny rectangle instead of full CONUS coverage.
#[test]
fn test_hrrr_lambert_pyramid_level_scaling() {
    use crate::rendering::resampling::resample_grid_for_bbox_with_proj;
    use projection::LambertConformal;

    let proj = LambertConformal::hrrr();

    // Native HRRR dimensions: 1799 x 1059
    // Pyramid level 2 dimensions: 449 x 264 (scale factor 4)
    let native_width = proj.nx;
    let native_height = proj.ny;
    let pyramid_width = 449usize;
    let pyramid_height = 264usize;

    assert_eq!(native_width, 1799, "HRRR native width should be 1799");
    assert_eq!(native_height, 1059, "HRRR native height should be 1059");

    // Create pyramid level 2 data with a gradient pattern
    // Value = (i + j) to create a recognizable pattern
    let mut pyramid_data = vec![f32::NAN; pyramid_width * pyramid_height];
    for j in 0..pyramid_height {
        for i in 0..pyramid_width {
            pyramid_data[j * pyramid_width + i] = (i + j) as f32;
        }
    }

    // Request a tile that covers the center of CONUS
    // This area should definitely have data if pyramid scaling is correct
    let output_bbox = [-105.0f32, 35.0, -95.0, 45.0]; // Central US (Kansas/Oklahoma area)
    let output_width = 256;
    let output_height = 256;

    let result = resample_grid_for_bbox_with_proj(
        &pyramid_data,
        pyramid_width,
        pyramid_height,
        output_width,
        output_height,
        output_bbox,
        output_bbox, // data_bounds (not used for Lambert)
        false,       // use_mercator
        "hrrr",
        None,  // goes_projection
        false, // grid_uses_360
    );

    assert_eq!(result.len(), output_width * output_height);

    // Count valid (non-NaN) pixels
    let valid_count = result.iter().filter(|v| !v.is_nan()).count();
    let valid_percent = (valid_count as f64 / result.len() as f64) * 100.0;

    println!("HRRR pyramid level scaling test:");
    println!("  Native dims: {}x{}", native_width, native_height);
    println!("  Pyramid dims: {}x{}", pyramid_width, pyramid_height);
    println!("  Output bbox: {:?}", output_bbox);
    println!(
        "  Valid pixels: {} / {} ({:.1}%)",
        valid_count,
        result.len(),
        valid_percent
    );

    // The center of CONUS should have substantial coverage
    // Before the fix, this would be ~0% because indices were out of bounds
    // After the fix, this should be >80% (accounting for edges outside HRRR coverage)
    assert!(
        valid_percent > 50.0,
        "Expected >50% valid pixels in central US, got {:.1}%. \
         This suggests pyramid level scaling is not working correctly.",
        valid_percent
    );
}

/// Test that Lambert resampling correctly handles pyramid level scaling for NDFD
#[test]
fn test_ndfd_lambert_pyramid_level_scaling() {
    use crate::rendering::resampling::resample_grid_for_bbox_with_proj;
    use projection::LambertConformal;

    let proj = LambertConformal::ndfd();

    // Native NDFD dimensions: 2145 x 1377
    // Simulate pyramid level 2 dimensions: 536 x 344 (scale factor 4)
    let native_width = proj.nx;
    let native_height = proj.ny;
    let pyramid_width = 536usize;
    let pyramid_height = 344usize;

    assert_eq!(native_width, 2145, "NDFD native width should be 2145");
    assert_eq!(native_height, 1377, "NDFD native height should be 1377");

    // Create pyramid level data with a gradient pattern
    let mut pyramid_data = vec![f32::NAN; pyramid_width * pyramid_height];
    for j in 0..pyramid_height {
        for i in 0..pyramid_width {
            pyramid_data[j * pyramid_width + i] = (i + j) as f32;
        }
    }

    // Request a tile that covers the center of CONUS
    let output_bbox = [-100.0f32, 30.0, -90.0, 40.0]; // Texas/Louisiana area
    let output_width = 256;
    let output_height = 256;

    let result = resample_grid_for_bbox_with_proj(
        &pyramid_data,
        pyramid_width,
        pyramid_height,
        output_width,
        output_height,
        output_bbox,
        output_bbox, // data_bounds (not used for Lambert)
        false,       // use_mercator
        "ndfd",
        None,  // goes_projection
        false, // grid_uses_360
    );

    assert_eq!(result.len(), output_width * output_height);

    // Count valid (non-NaN) pixels
    let valid_count = result.iter().filter(|v| !v.is_nan()).count();
    let valid_percent = (valid_count as f64 / result.len() as f64) * 100.0;

    println!("NDFD pyramid level scaling test:");
    println!("  Native dims: {}x{}", native_width, native_height);
    println!("  Pyramid dims: {}x{}", pyramid_width, pyramid_height);
    println!("  Output bbox: {:?}", output_bbox);
    println!(
        "  Valid pixels: {} / {} ({:.1}%)",
        valid_count,
        result.len(),
        valid_percent
    );

    assert!(
        valid_percent > 50.0,
        "Expected >50% valid pixels in south-central US, got {:.1}%. \
         This suggests pyramid level scaling is not working correctly.",
        valid_percent
    );
}

/// Test that Lambert Mercator resampling also handles pyramid level scaling
#[test]
fn test_hrrr_lambert_mercator_pyramid_level_scaling() {
    use crate::rendering::resampling::resample_grid_for_bbox_with_proj;

    // Pyramid level 2 dimensions
    let pyramid_width = 449usize;
    let pyramid_height = 264usize;

    // Create pyramid level data
    let mut pyramid_data = vec![f32::NAN; pyramid_width * pyramid_height];
    for j in 0..pyramid_height {
        for i in 0..pyramid_width {
            pyramid_data[j * pyramid_width + i] = (i + j) as f32;
        }
    }

    // Request a Mercator tile covering central US
    let output_bbox = [-105.0f32, 35.0, -95.0, 45.0];
    let output_width = 256;
    let output_height = 256;

    let result = resample_grid_for_bbox_with_proj(
        &pyramid_data,
        pyramid_width,
        pyramid_height,
        output_width,
        output_height,
        output_bbox,
        output_bbox, // data_bounds (not used for Lambert)
        true,        // use_mercator = true for WMTS tiles
        "hrrr",
        None,
        false,
    );

    assert_eq!(result.len(), output_width * output_height);

    let valid_count = result.iter().filter(|v| !v.is_nan()).count();
    let valid_percent = (valid_count as f64 / result.len() as f64) * 100.0;

    println!("HRRR Lambert Mercator pyramid level scaling test:");
    println!("  Pyramid dims: {}x{}", pyramid_width, pyramid_height);
    println!("  Output bbox: {:?}", output_bbox);
    println!(
        "  Valid pixels: {} / {} ({:.1}%)",
        valid_count,
        result.len(),
        valid_percent
    );

    assert!(
        valid_percent > 50.0,
        "Expected >50% valid pixels for Mercator output, got {:.1}%. \
         This suggests pyramid level scaling is not working in Mercator mode.",
        valid_percent
    );
}

/// Test that native resolution (no scaling) still works correctly
#[test]
fn test_hrrr_lambert_native_resolution() {
    use crate::rendering::resampling::resample_grid_for_bbox_with_proj;
    use projection::LambertConformal;

    let proj = LambertConformal::hrrr();

    // Use native dimensions (scale factor = 1.0)
    let data_width = proj.nx;
    let data_height = proj.ny;

    // Create native resolution data
    let mut data = vec![f32::NAN; data_width * data_height];
    for j in 0..data_height {
        for i in 0..data_width {
            data[j * data_width + i] = (i + j) as f32;
        }
    }

    // Request a tile covering central US
    let output_bbox = [-105.0f32, 35.0, -95.0, 45.0];
    let output_width = 256;
    let output_height = 256;

    let result = resample_grid_for_bbox_with_proj(
        &data,
        data_width,
        data_height,
        output_width,
        output_height,
        output_bbox,
        output_bbox,
        false,
        "hrrr",
        None,
        false,
    );

    assert_eq!(result.len(), output_width * output_height);

    let valid_count = result.iter().filter(|v| !v.is_nan()).count();
    let valid_percent = (valid_count as f64 / result.len() as f64) * 100.0;

    println!("HRRR native resolution test:");
    println!("  Native dims: {}x{}", data_width, data_height);
    println!("  Output bbox: {:?}", output_bbox);
    println!(
        "  Valid pixels: {} / {} ({:.1}%)",
        valid_count,
        result.len(),
        valid_percent
    );

    // Native resolution should also have good coverage
    assert!(
        valid_percent > 50.0,
        "Expected >50% valid pixels at native resolution, got {:.1}%.",
        valid_percent
    );
}

/// Test pyramid scaling with various scale factors
#[test]
fn test_lambert_pyramid_various_scale_factors() {
    use crate::rendering::resampling::resample_grid_for_bbox_with_proj;
    use projection::LambertConformal;

    let proj = LambertConformal::hrrr();
    let native_width = proj.nx;
    let native_height = proj.ny;

    // Test multiple pyramid levels
    let scale_factors = [1.0, 2.0, 4.0, 8.0];

    let output_bbox = [-100.0f32, 35.0, -95.0, 40.0];
    let output_width = 128;
    let output_height = 128;

    println!("\nTesting various pyramid scale factors:");

    for scale in scale_factors {
        let pyramid_width = (native_width as f64 / scale).ceil() as usize;
        let pyramid_height = (native_height as f64 / scale).ceil() as usize;

        // Create pyramid data
        let mut pyramid_data = vec![f32::NAN; pyramid_width * pyramid_height];
        for j in 0..pyramid_height {
            for i in 0..pyramid_width {
                pyramid_data[j * pyramid_width + i] = 100.0 + scale as f32; // Encode scale in value
            }
        }

        let result = resample_grid_for_bbox_with_proj(
            &pyramid_data,
            pyramid_width,
            pyramid_height,
            output_width,
            output_height,
            output_bbox,
            output_bbox,
            false,
            "hrrr",
            None,
            false,
        );

        let valid_count = result.iter().filter(|v| !v.is_nan()).count();
        let valid_percent = (valid_count as f64 / result.len() as f64) * 100.0;

        // Check that we get the correct value (100 + scale)
        let expected_value = 100.0 + scale as f32;
        let values_correct = result
            .iter()
            .filter(|v| !v.is_nan())
            .all(|&v| (v - expected_value).abs() < 0.1);

        println!(
            "  Scale {}: dims {}x{}, valid {:.1}%, values_correct: {}",
            scale, pyramid_width, pyramid_height, valid_percent, values_correct
        );

        assert!(
            valid_percent > 30.0,
            "Scale {}: Expected >30% valid pixels, got {:.1}%",
            scale,
            valid_percent
        );

        assert!(
            values_correct,
            "Scale {}: Values should be approximately {}",
            scale, expected_value
        );
    }
}
