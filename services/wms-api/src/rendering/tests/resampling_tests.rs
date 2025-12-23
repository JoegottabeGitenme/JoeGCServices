//! Tests for resampling functions.

use crate::rendering::resampling::{
    lat_to_mercator_y, mercator_y_to_lat, bilinear_interpolate,
    resample_from_geographic, resample_for_mercator,
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
    assert!(valid_count > 0, "Should have valid values in equatorial region");
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
    assert!(valid_count > 0, "Should have valid values at high latitudes");
}
