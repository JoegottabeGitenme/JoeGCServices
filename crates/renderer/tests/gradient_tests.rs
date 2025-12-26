//! Tests for gradient/heatmap rendering module.

use renderer::gradient::{interpolate_color, render_grid, resample_grid, subset_grid, Color};

// ============================================================================
// subset_grid tests
// ============================================================================

#[test]
fn test_subset_grid_full_globe() {
    // Create a simple 4x4 global grid (90 to -90 lat, 0 to 360 lon)
    // Each value encodes its position: row * 10 + col
    let data: Vec<f32> = (0..16).map(|i| i as f32).collect();
    let grid_width = 4;
    let grid_height = 4;

    // Request the full globe
    let bbox = [0.0, -90.0, 360.0, 90.0];
    let (subset, width, height) = subset_grid(&data, grid_width, grid_height, &bbox);

    // Should return the full grid
    assert_eq!(width, grid_width);
    assert_eq!(height, grid_height);
    assert_eq!(subset.len(), data.len());
}

#[test]
fn test_subset_grid_northern_hemisphere() {
    // 8x4 grid covering globe
    let grid_width = 8;
    let grid_height = 4;
    let data: Vec<f32> = (0..(grid_width * grid_height)).map(|i| i as f32).collect();

    // Request northern hemisphere only (lat 0 to 90)
    let bbox = [0.0, 0.0, 360.0, 90.0];
    let (_subset, width, height) = subset_grid(&data, grid_width, grid_height, &bbox);

    // Should get approximately top half of the grid
    assert!(height <= grid_height);
    assert!(height >= 1);
    assert_eq!(width, grid_width);
}

#[test]
fn test_subset_grid_negative_longitude() {
    // Test handling of negative longitudes (should normalize to 0-360)
    let grid_width = 8;
    let grid_height = 4;
    let data: Vec<f32> = (0..(grid_width * grid_height)).map(|i| i as f32).collect();

    // Request with negative longitudes (-180 to 0)
    let bbox = [-180.0, -45.0, 0.0, 45.0];
    let (subset, width, height) = subset_grid(&data, grid_width, grid_height, &bbox);

    // Should still return valid subset
    assert!(width > 0);
    assert!(height > 0);
    assert_eq!(subset.len(), width * height);
}

#[test]
fn test_subset_grid_small_region() {
    // 360x180 grid (1 degree resolution)
    let grid_width = 360;
    let grid_height = 180;
    let data: Vec<f32> = (0..(grid_width * grid_height))
        .map(|i| (i % 256) as f32)
        .collect();

    // Request a small region (roughly US)
    let bbox = [-125.0, 25.0, -65.0, 50.0];
    let (_, width, height) = subset_grid(&data, grid_width, grid_height, &bbox);

    // Should be significantly smaller than full grid
    assert!(width < grid_width);
    assert!(height < grid_height);
    assert!(width > 0);
    assert!(height > 0);
}

#[test]
fn test_subset_grid_edge_cases() {
    let grid_width = 10;
    let grid_height = 10;
    let data: Vec<f32> = vec![1.0; grid_width * grid_height];

    // Very small bbox at edge
    let bbox = [0.0, 85.0, 10.0, 90.0];
    let (subset, width, height) = subset_grid(&data, grid_width, grid_height, &bbox);
    assert!(width > 0);
    assert!(height > 0);
    assert_eq!(subset.len(), width * height);

    // Bbox at the equator
    let bbox = [0.0, -5.0, 360.0, 5.0];
    let (_, width, height) = subset_grid(&data, grid_width, grid_height, &bbox);
    assert!(width > 0);
    assert!(height > 0);
}

#[test]
fn test_subset_grid_data_integrity() {
    // Create grid where each cell has unique value based on position
    let grid_width = 10;
    let grid_height = 10;
    let data: Vec<f32> = (0..(grid_width * grid_height)).map(|i| i as f32).collect();

    // Get a subset and verify values are from the original grid
    let bbox = [90.0, -45.0, 180.0, 45.0];
    let (subset, _width, _height) = subset_grid(&data, grid_width, grid_height, &bbox);

    // All values in subset should exist in original data
    for val in &subset {
        assert!(
            data.contains(val),
            "Subset contains value not in original: {}",
            val
        );
    }
}

// ============================================================================
// resample_grid tests
// ============================================================================

#[test]
fn test_resample_grid_identity() {
    // When src and dst dimensions match, should return copy of input
    let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
    let result = resample_grid(&data, 3, 3, 3, 3);
    assert_eq!(result, data);
}

#[test]
fn test_resample_grid_upscale() {
    // Simple 2x2 grid upscaled to 4x4
    let data = vec![0.0, 10.0, 10.0, 20.0];
    let result = resample_grid(&data, 2, 2, 4, 4);

    assert_eq!(result.len(), 16);

    // Corner values should match original corners
    assert!((result[0] - 0.0).abs() < 0.01, "Top-left corner");
    assert!((result[3] - 10.0).abs() < 0.01, "Top-right corner");
    assert!((result[12] - 10.0).abs() < 0.01, "Bottom-left corner");
    assert!((result[15] - 20.0).abs() < 0.01, "Bottom-right corner");

    // Center values should be interpolated
    // Value at center should be average of all corners
    let center_approx = (0.0 + 10.0 + 10.0 + 20.0) / 4.0;
    assert!(
        (result[5] - center_approx).abs() < 5.0,
        "Center should be interpolated"
    );
}

#[test]
fn test_resample_grid_downscale() {
    // 4x4 grid downscaled to 2x2
    let data = vec![
        0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    ];
    let result = resample_grid(&data, 4, 4, 2, 2);

    assert_eq!(result.len(), 4);

    // Corners should sample from original corners
    assert!((result[0] - 0.0).abs() < 0.01);
    assert!((result[1] - 3.0).abs() < 0.01);
    assert!((result[2] - 12.0).abs() < 0.01);
    assert!((result[3] - 15.0).abs() < 0.01);
}

#[test]
fn test_resample_grid_non_square() {
    // 4x2 grid resampled to 8x4
    let data = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let result = resample_grid(&data, 4, 2, 8, 4);

    assert_eq!(result.len(), 32);

    // Check that corner values are preserved
    assert!((result[0] - 0.0).abs() < 0.01);
    assert!((result[7] - 3.0).abs() < 0.01);
    assert!((result[24] - 4.0).abs() < 0.01);
    assert!((result[31] - 7.0).abs() < 0.01);
}

#[test]
fn test_resample_grid_preserves_range() {
    // Values should stay within original min/max after interpolation
    let data = vec![0.0, 100.0, 100.0, 0.0];
    let result = resample_grid(&data, 2, 2, 10, 10);

    let min = result.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    let max = result.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));

    assert!(min >= 0.0, "Min should be >= 0");
    assert!(max <= 100.0, "Max should be <= 100");
}

#[test]
fn test_resample_grid_large() {
    // Test with larger grid to ensure parallel processing works
    let src_width = 100;
    let src_height = 100;
    let data: Vec<f32> = (0..(src_width * src_height))
        .map(|i| (i % 256) as f32)
        .collect();

    let result = resample_grid(&data, src_width, src_height, 256, 256);
    assert_eq!(result.len(), 256 * 256);
}

// ============================================================================
// interpolate_color tests
// ============================================================================

#[test]
fn test_interpolate_color_endpoints() {
    let c1 = Color::new(0, 0, 0, 255);
    let c2 = Color::new(255, 255, 255, 255);

    // t=0 should return c1
    let result = interpolate_color(c1, c2, 0.0);
    assert_eq!(result.r, 0);
    assert_eq!(result.g, 0);
    assert_eq!(result.b, 0);

    // t=1 should return c2
    let result = interpolate_color(c1, c2, 1.0);
    assert_eq!(result.r, 255);
    assert_eq!(result.g, 255);
    assert_eq!(result.b, 255);
}

#[test]
fn test_interpolate_color_midpoint() {
    let c1 = Color::new(0, 0, 0, 255);
    let c2 = Color::new(200, 100, 50, 255);

    let result = interpolate_color(c1, c2, 0.5);
    assert_eq!(result.r, 100);
    assert_eq!(result.g, 50);
    assert_eq!(result.b, 25);
}

#[test]
fn test_interpolate_color_clamps() {
    let c1 = Color::new(100, 100, 100, 255);
    let c2 = Color::new(200, 200, 200, 255);

    // Values outside 0-1 should be clamped
    let below = interpolate_color(c1, c2, -1.0);
    assert_eq!(below.r, 100);

    let above = interpolate_color(c1, c2, 2.0);
    assert_eq!(above.r, 200);
}

#[test]
fn test_interpolate_color_alpha() {
    let c1 = Color::new(0, 0, 0, 0);
    let c2 = Color::new(0, 0, 0, 255);

    let result = interpolate_color(c1, c2, 0.5);
    assert!((result.a as i32 - 127).abs() <= 1);
}

// ============================================================================
// Color tests
// ============================================================================

#[test]
fn test_color_new() {
    let c = Color::new(10, 20, 30, 40);
    assert_eq!(c.r, 10);
    assert_eq!(c.g, 20);
    assert_eq!(c.b, 30);
    assert_eq!(c.a, 40);
}

#[test]
fn test_color_transparent() {
    let c = Color::transparent();
    assert_eq!(c.r, 0);
    assert_eq!(c.g, 0);
    assert_eq!(c.b, 0);
    assert_eq!(c.a, 0);
}

// ============================================================================
// render_grid tests
// ============================================================================

#[test]
fn test_render_grid_basic() {
    let data = vec![0.0, 0.5, 0.5, 1.0];
    let width = 2;
    let height = 2;

    let pixels = render_grid(&data, width, height, 0.0, 1.0, |t| {
        Color::new((t * 255.0) as u8, 0, 0, 255)
    });

    assert_eq!(pixels.len(), width * height * 4);

    // First pixel (value 0.0) should be black
    assert_eq!(pixels[0], 0); // R
    assert_eq!(pixels[3], 255); // A

    // Last pixel (value 1.0) should be red
    assert_eq!(pixels[12], 255); // R
    assert_eq!(pixels[15], 255); // A
}

#[test]
fn test_render_grid_nan_handling() {
    let data = vec![f32::NAN, 0.5, 0.5, 1.0];
    let width = 2;
    let height = 2;

    let pixels = render_grid(&data, width, height, 0.0, 1.0, |_| {
        Color::new(255, 0, 0, 255)
    });

    // First pixel (NaN) should be transparent
    assert_eq!(pixels[0], 0); // R
    assert_eq!(pixels[1], 0); // G
    assert_eq!(pixels[2], 0); // B
    assert_eq!(pixels[3], 0); // A (transparent)

    // Other pixels should be colored
    assert_eq!(pixels[7], 255); // Second pixel alpha
}

#[test]
fn test_render_grid_normalization() {
    // Data range 100-200, should normalize to 0-1
    let data = vec![100.0, 150.0, 150.0, 200.0];
    let width = 2;
    let height = 2;

    let pixels = render_grid(&data, width, height, 100.0, 200.0, |t| {
        Color::new((t * 255.0) as u8, 0, 0, 255)
    });

    // First pixel (100.0 = 0.0 normalized) should be 0
    assert_eq!(pixels[0], 0);

    // Last pixel (200.0 = 1.0 normalized) should be 255
    assert_eq!(pixels[12], 255);
}

#[test]
fn test_render_grid_flat_range() {
    // All same values - should not divide by zero
    let data = vec![5.0, 5.0, 5.0, 5.0];
    let width = 2;
    let height = 2;

    // Should not panic
    let pixels = render_grid(&data, width, height, 5.0, 5.0, |_| {
        Color::new(128, 128, 128, 255)
    });

    assert_eq!(pixels.len(), 16);
}

#[test]
fn test_render_grid_large() {
    // Test parallel processing with larger grid
    let width = 256;
    let height = 256;
    let data: Vec<f32> = (0..(width * height))
        .map(|i| (i as f32) / (width * height) as f32)
        .collect();

    let pixels = render_grid(&data, width, height, 0.0, 1.0, |t| {
        Color::new((t * 255.0) as u8, 0, 0, 255)
    });

    assert_eq!(pixels.len(), width * height * 4);
}
