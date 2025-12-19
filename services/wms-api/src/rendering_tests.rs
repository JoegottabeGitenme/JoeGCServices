//! Tests for the rendering module.
//!
//! These tests cover the pure functions in rendering.rs:
//! - HSV to RGB color conversion
//! - Mercator projection conversions
//! - Bilinear interpolation
//! - Parameter value conversion
//! - Geographic and Mercator resampling

use super::rendering::*;

// ========================================================================
// HSV to RGB conversion tests
// ========================================================================

#[test]
fn test_hsv_to_rgb_red() {
    // Red: H=0, S=1, V=1
    let (r, g, b) = hsv_to_rgb(0.0, 1.0, 1.0);
    assert_eq!(r, 255);
    assert_eq!(g, 0);
    assert_eq!(b, 0);
}

#[test]
fn test_hsv_to_rgb_green() {
    // Green: H=120, S=1, V=1
    let (r, g, b) = hsv_to_rgb(120.0, 1.0, 1.0);
    assert_eq!(r, 0);
    assert_eq!(g, 255);
    assert_eq!(b, 0);
}

#[test]
fn test_hsv_to_rgb_blue() {
    // Blue: H=240, S=1, V=1
    let (r, g, b) = hsv_to_rgb(240.0, 1.0, 1.0);
    assert_eq!(r, 0);
    assert_eq!(g, 0);
    assert_eq!(b, 255);
}

#[test]
fn test_hsv_to_rgb_white() {
    // White: H=any, S=0, V=1
    let (r, g, b) = hsv_to_rgb(0.0, 0.0, 1.0);
    assert_eq!(r, 255);
    assert_eq!(g, 255);
    assert_eq!(b, 255);
}

#[test]
fn test_hsv_to_rgb_black() {
    // Black: H=any, S=any, V=0
    let (r, g, b) = hsv_to_rgb(0.0, 1.0, 0.0);
    assert_eq!(r, 0);
    assert_eq!(g, 0);
    assert_eq!(b, 0);
}

#[test]
fn test_hsv_to_rgb_yellow() {
    // Yellow: H=60, S=1, V=1
    let (r, g, b) = hsv_to_rgb(60.0, 1.0, 1.0);
    assert_eq!(r, 255);
    assert_eq!(g, 255);
    assert_eq!(b, 0);
}

#[test]
fn test_hsv_to_rgb_cyan() {
    // Cyan: H=180, S=1, V=1
    let (r, g, b) = hsv_to_rgb(180.0, 1.0, 1.0);
    assert_eq!(r, 0);
    assert_eq!(g, 255);
    assert_eq!(b, 255);
}

#[test]
fn test_hsv_to_rgb_magenta() {
    // Magenta: H=300, S=1, V=1
    let (r, g, b) = hsv_to_rgb(300.0, 1.0, 1.0);
    assert_eq!(r, 255);
    assert_eq!(g, 0);
    assert_eq!(b, 255);
}

#[test]
fn test_hsv_to_rgb_hue_wrapping() {
    // H=360 should be equivalent to H=0 (red)
    let (r, g, b) = hsv_to_rgb(360.0, 1.0, 1.0);
    assert_eq!(r, 255);
    assert_eq!(g, 0);
    assert_eq!(b, 0);
}

// ========================================================================
// Mercator projection tests
// ========================================================================

#[test]
fn test_lat_to_mercator_y_equator() {
    // Equator should map to 0
    let y = lat_to_mercator_y(0.0);
    assert!((y - 0.0).abs() < 0.001, "Equator should map to y=0, got {}", y);
}

#[test]
fn test_lat_to_mercator_y_positive_lat() {
    // Positive latitude should give positive Y
    let y = lat_to_mercator_y(45.0);
    assert!(y > 0.0, "45°N should give positive Y");
}

#[test]
fn test_lat_to_mercator_y_negative_lat() {
    // Negative latitude should give negative Y
    let y = lat_to_mercator_y(-45.0);
    assert!(y < 0.0, "45°S should give negative Y");
}

#[test]
fn test_mercator_y_to_lat_equator() {
    // Y=0 should map to equator
    let lat = mercator_y_to_lat(0.0);
    assert!((lat - 0.0).abs() < 0.001, "y=0 should map to equator, got {}", lat);
}

#[test]
fn test_mercator_roundtrip() {
    // Converting lat to Y and back should give the same lat
    for lat in [-60.0, -30.0, 0.0, 30.0, 60.0] {
        let y = lat_to_mercator_y(lat);
        let recovered_lat = mercator_y_to_lat(y);
        assert!(
            (lat - recovered_lat).abs() < 0.0001,
            "Roundtrip failed for lat {}: got {}",
            lat,
            recovered_lat
        );
    }
}

#[test]
fn test_mercator_symmetry() {
    // +lat and -lat should give equal magnitude but opposite sign Y
    let y_pos = lat_to_mercator_y(45.0);
    let y_neg = lat_to_mercator_y(-45.0);
    assert!(
        (y_pos + y_neg).abs() < 0.001,
        "Y values should be symmetric: {} vs {}",
        y_pos,
        y_neg
    );
}

// ========================================================================
// Bilinear interpolation tests
// ========================================================================

#[test]
fn test_bilinear_interpolate_corner() {
    // 2x2 grid with values at corners
    let data = vec![1.0, 2.0, 3.0, 4.0];
    // At corner (0,0) should give exact value
    let result = bilinear_interpolate(&data, 2, 2, 0.0, 0.0, false).unwrap();
    assert!((result - 1.0).abs() < 0.001, "Expected 1.0, got {}", result);
}

#[test]
fn test_bilinear_interpolate_center() {
    // 2x2 grid: average of all four corners
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let result = bilinear_interpolate(&data, 2, 2, 0.5, 0.5, false).unwrap();
    // (1+2+3+4)/4 = 2.5
    assert!((result - 2.5).abs() < 0.001, "Expected 2.5, got {}", result);
}

#[test]
fn test_bilinear_interpolate_horizontal_midpoint() {
    // 2x2 grid: midpoint along top edge
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let result = bilinear_interpolate(&data, 2, 2, 0.5, 0.0, false).unwrap();
    // Average of 1 and 2 = 1.5
    assert!((result - 1.5).abs() < 0.001, "Expected 1.5, got {}", result);
}

#[test]
fn test_bilinear_interpolate_vertical_midpoint() {
    // 2x2 grid: midpoint along left edge
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let result = bilinear_interpolate(&data, 2, 2, 0.0, 0.5, false).unwrap();
    // Average of 1 and 3 = 2.0
    assert!((result - 2.0).abs() < 0.001, "Expected 2.0, got {}", result);
}

#[test]
fn test_bilinear_interpolate_with_wrap() {
    // 4x2 grid, test wrapping at longitude boundary
    let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    // At x=3.5 with wrap, should interpolate between column 3 and column 0
    let result = bilinear_interpolate(&data, 4, 2, 3.5, 0.0, true).unwrap();
    // Should be average of 4 and 1 = 2.5
    assert!((result - 2.5).abs() < 0.001, "Expected 2.5, got {}", result);
}

// ========================================================================
// convert_parameter_value tests
// ========================================================================

#[test]
fn test_convert_temperature_kelvin_to_celsius() {
    let (value, unit, raw_unit, name) = convert_parameter_value("TMP", 300.0);
    assert!((value - 26.85).abs() < 0.01, "Expected ~26.85°C, got {}", value);
    assert_eq!(unit, "°C");
    assert_eq!(raw_unit, "K");
    assert_eq!(name, "Temperature");
}

#[test]
fn test_convert_temperature_freezing() {
    let (value, _, _, _) = convert_parameter_value("TEMP", 273.15);
    assert!((value - 0.0).abs() < 0.01, "Expected 0°C, got {}", value);
}

#[test]
fn test_convert_pressure_pa_to_hpa() {
    let (value, unit, raw_unit, name) = convert_parameter_value("PRES", 101325.0);
    assert!((value - 1013.25).abs() < 0.01, "Expected 1013.25 hPa, got {}", value);
    assert_eq!(unit, "hPa");
    assert_eq!(raw_unit, "Pa");
    assert_eq!(name, "Pressure");
}

#[test]
fn test_convert_prmsl() {
    // PRMSL (Pressure Reduced to Mean Sea Level) should also convert
    let (value, unit, _, _) = convert_parameter_value("PRMSL", 101500.0);
    assert!((value - 1015.0).abs() < 0.01);
    assert_eq!(unit, "hPa");
}

#[test]
fn test_convert_wind_speed() {
    let (value, unit, _, name) = convert_parameter_value("WIND_SPEED", 10.0);
    assert!((value - 10.0).abs() < 0.01);
    assert_eq!(unit, "m/s");
    assert_eq!(name, "Wind Speed");
}

#[test]
fn test_convert_ugrd() {
    let (value, unit, _, name) = convert_parameter_value("UGRD", 5.5);
    assert!((value - 5.5).abs() < 0.01);
    assert_eq!(unit, "m/s");
    assert_eq!(name, "U Wind Component");
}

#[test]
fn test_convert_vgrd() {
    let (value, unit, _, name) = convert_parameter_value("VGRD", -3.2);
    assert!((value - -3.2).abs() < 0.01);
    assert_eq!(unit, "m/s");
    assert_eq!(name, "V Wind Component");
}

#[test]
fn test_convert_relative_humidity() {
    let (value, unit, _, name) = convert_parameter_value("RH", 65.0);
    assert!((value - 65.0).abs() < 0.01);
    assert_eq!(unit, "%");
    assert_eq!(name, "Relative Humidity");
}

#[test]
fn test_convert_geopotential_height() {
    let (value, unit, raw_unit, name) = convert_parameter_value("HGT", 6000.0);
    // 6000 * 0.0167 = 100.2 dkm
    assert!((value - 100.2).abs() < 0.1, "Expected ~100 dkm, got {}", value);
    assert_eq!(unit, "dkm");
    assert_eq!(raw_unit, "gpm");
    assert_eq!(name, "Geopotential Height");
}

#[test]
fn test_convert_generic_parameter() {
    let (value, unit, _, name) = convert_parameter_value("UNKNOWN_PARAM", 42.0);
    assert!((value - 42.0).abs() < 0.01);
    assert_eq!(unit, "");
    assert_eq!(name, "UNKNOWN_PARAM");
}

// ========================================================================
// resample_from_geographic tests
// ========================================================================

#[test]
fn test_resample_from_geographic_identity() {
    // Simple 2x2 grid covering 0-1 in lat/lon
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let data_bounds = [0.0, 0.0, 1.0, 1.0];
    let output_bbox = [0.0, 0.0, 1.0, 1.0];
    
    let result = resample_from_geographic(
        &data, 2, 2, 2, 2, output_bbox, data_bounds, false
    );
    
    // Should get approximately the same values (interpolated at pixel centers)
    assert_eq!(result.len(), 4);
    // All values should be valid (not NaN)
    for val in &result {
        assert!(!val.is_nan(), "Unexpected NaN in result");
    }
}

#[test]
fn test_resample_from_geographic_subset() {
    // 4x4 grid, extract center 2x2
    let mut data = vec![0.0; 16];
    for y in 0..4 {
        for x in 0..4 {
            data[y * 4 + x] = (x + y * 10) as f32;
        }
    }
    // Data bounds: lon 0-4, lat 0-4
    let data_bounds = [0.0, 0.0, 4.0, 4.0];
    // Output bbox: center region lon 1-3, lat 1-3
    let output_bbox = [1.0, 1.0, 3.0, 3.0];
    
    let result = resample_from_geographic(
        &data, 4, 4, 2, 2, output_bbox, data_bounds, false
    );
    
    assert_eq!(result.len(), 4);
    // Values should be from the center of the grid
    for val in &result {
        assert!(!val.is_nan(), "Unexpected NaN in subset result");
    }
}

#[test]
fn test_resample_from_geographic_out_of_bounds() {
    // 2x2 grid covering [0,0] to [10,10]
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let data_bounds = [0.0, 0.0, 10.0, 10.0];
    // Request region completely outside data bounds
    let output_bbox = [20.0, 20.0, 30.0, 30.0];
    
    let result = resample_from_geographic(
        &data, 2, 2, 2, 2, output_bbox, data_bounds, false
    );
    
    // All values should be NaN (outside data coverage)
    for val in &result {
        assert!(val.is_nan(), "Expected NaN for out-of-bounds region");
    }
}

#[test]
fn test_resample_from_geographic_360_longitude() {
    // Grid using 0-360 longitude convention (like GFS)
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let data_bounds = [0.0, -90.0, 360.0, 90.0];
    // Request tile in western hemisphere (negative longitude)
    let output_bbox = [-45.0, 30.0, -30.0, 45.0];
    
    let result = resample_from_geographic(
        &data, 2, 2, 2, 2, output_bbox, data_bounds, true // grid_uses_360 = true
    );
    
    // Should handle the coordinate conversion
    assert_eq!(result.len(), 4);
}

// ========================================================================
// resample_for_mercator tests  
// ========================================================================

#[test]
fn test_resample_for_mercator_equatorial() {
    // Simple grid, small equatorial region
    let data = vec![10.0, 20.0, 30.0, 40.0];
    let data_bounds = [-10.0, -10.0, 10.0, 10.0];
    let output_bbox = [-5.0, -5.0, 5.0, 5.0];
    
    let result = resample_for_mercator(
        &data, 2, 2, 2, 2, output_bbox, data_bounds, false
    );
    
    assert_eq!(result.len(), 4);
    // Near equator, Mercator is nearly linear, so results should be similar
    // to geographic resampling
    for val in &result {
        assert!(!val.is_nan(), "Unexpected NaN in Mercator resample");
    }
}

#[test]
fn test_resample_for_mercator_high_latitude() {
    // Grid covering high latitude region
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let data_bounds = [-180.0, 40.0, 180.0, 80.0];
    let output_bbox = [-10.0, 50.0, 10.0, 70.0];
    
    let result = resample_for_mercator(
        &data, 2, 2, 4, 4, output_bbox, data_bounds, false
    );
    
    assert_eq!(result.len(), 16);
    // At high latitudes, Mercator spacing is non-linear
    // Just verify we get valid results
    let valid_count = result.iter().filter(|v| !v.is_nan()).count();
    assert!(valid_count > 0, "Should have some valid values");
}

// ========================================================================
// render_by_parameter tests (basic validation)
// ========================================================================

#[test]
fn test_render_by_parameter_temperature_output_size() {
    // Just verify correct output size for temperature rendering
    let data = vec![273.15, 280.0, 290.0, 300.0]; // Kelvin values
    let result = render_by_parameter(&data, "TMP", 273.15, 300.0, 2, 2);
    // Should produce RGBA data: 4 pixels * 4 bytes
    assert_eq!(result.len(), 4 * 4);
}

#[test]
fn test_render_by_parameter_wind_output_size() {
    let data = vec![0.0, 5.0, 10.0, 15.0]; // m/s
    let result = render_by_parameter(&data, "WIND_SPEED", 0.0, 15.0, 2, 2);
    assert_eq!(result.len(), 4 * 4);
}

#[test]
fn test_render_by_parameter_pressure_output_size() {
    let data = vec![100000.0, 101000.0, 102000.0, 103000.0]; // Pa
    let result = render_by_parameter(&data, "PRES", 100000.0, 103000.0, 2, 2);
    assert_eq!(result.len(), 4 * 4);
}

#[test]
fn test_render_by_parameter_reflectivity_output_size() {
    let data = vec![0.0, 20.0, 40.0, 60.0]; // dBZ
    let result = render_by_parameter(&data, "REFL", -10.0, 75.0, 2, 2);
    assert_eq!(result.len(), 4 * 4);
}

#[test]
fn test_render_by_parameter_goes_visible_output_size() {
    let data = vec![0.0, 0.3, 0.6, 1.0]; // reflectance 0-1
    let result = render_by_parameter(&data, "CMI_C02", 0.0, 1.0, 2, 2);
    assert_eq!(result.len(), 4 * 4);
}

#[test]
fn test_render_by_parameter_generic_output_size() {
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let result = render_by_parameter(&data, "UNKNOWN", 1.0, 4.0, 2, 2);
    assert_eq!(result.len(), 4 * 4);
}

// ========================================================================
// GridData struct tests
// ========================================================================

#[test]
fn test_grid_data_size_validation() {
    // Verify the grid data mismatch check logic
    let data = vec![1.0; 100];
    let width = 10;
    let height = 10;
    assert_eq!(data.len(), width * height, "Data should match dimensions");
    
    let bad_width = 5;
    let bad_height = 10;
    assert_ne!(data.len(), bad_width * bad_height, "Should detect mismatch");
}
