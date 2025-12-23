//! Tests for PNG encoding functionality.
//!
//! Tests the indexed PNG and RGBA PNG encoders, including:
//! - Palette extraction (sequential and parallel)
//! - PNG format selection (auto mode)
//! - File size comparisons for weather-like data

use renderer::png::{create_png, create_png_auto};
use std::collections::HashSet;

// ============================================================================
// Helper functions
// ============================================================================

/// Pack RGBA bytes into a u32 for color counting
fn pack_color(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

/// Create a weather-like color palette (temperature colors)
fn weather_palette() -> [(u8, u8, u8); 20] {
    [
        (102, 0, 214), // -40C purple
        (0, 51, 255),  // -30C blue
        (0, 128, 255), // -20C light blue
        (0, 191, 255), // -10C cyan
        (0, 255, 255), // 0C cyan
        (0, 255, 191), // 5C teal
        (0, 255, 128), // 10C green
        (0, 255, 0),   // 15C bright green
        (128, 255, 0), // 20C yellow-green
        (191, 255, 0), // 22C
        (255, 255, 0), // 25C yellow
        (255, 220, 0), // 27C
        (255, 191, 0), // 30C orange
        (255, 128, 0), // 33C
        (255, 64, 0),  // 36C red-orange
        (255, 0, 0),   // 40C red
        (214, 0, 0),   // 42C dark red
        (178, 0, 0),   // 45C
        (139, 0, 0),   // 48C maroon
        (100, 0, 0),   // 50C dark maroon
    ]
}

/// Generate weather-like pixel data with a limited palette
fn generate_weather_pixels(width: usize, height: usize) -> Vec<u8> {
    let palette = weather_palette();
    let mut pixels = Vec::with_capacity(width * height * 4);

    for y in 0..height {
        for x in 0..width {
            // Map position to palette index (quantized like real weather data)
            let temp_idx =
                ((x as f32 / width as f32 * 0.3 + y as f32 / height as f32 * 0.7) * 19.0) as usize;
            let (r, g, b) = palette[temp_idx.min(19)];
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    pixels
}

/// Count unique colors in pixel data
fn count_unique_colors(pixels: &[u8]) -> usize {
    let mut unique: HashSet<u32> = HashSet::new();
    for chunk in pixels.chunks_exact(4) {
        unique.insert(pack_color(chunk[0], chunk[1], chunk[2], chunk[3]));
    }
    unique.len()
}

// ============================================================================
// Basic PNG creation tests
// ============================================================================

#[test]
fn test_create_png_simple() {
    // Simple 2x2 image with 2 colors
    let pixels = [
        255, 0, 0, 255, // red
        0, 255, 0, 255, // green
        0, 255, 0, 255, // green
        255, 0, 0, 255, // red
    ];

    let result = create_png_auto(&pixels, 2, 2);
    assert!(result.is_ok());

    let png = result.unwrap();
    // Check PNG signature
    assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
}

#[test]
fn test_create_png_rgba() {
    // Simple 2x2 RGBA image
    let pixels = [
        255, 0, 0, 255, // red
        0, 255, 0, 255, // green
        0, 0, 255, 255, // blue
        255, 255, 0, 255, // yellow
    ];

    let result = create_png(&pixels, 2, 2);
    assert!(result.is_ok());

    let png = result.unwrap();
    // Check PNG signature
    assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
}

#[test]
fn test_create_png_with_transparency() {
    // 2x2 image with transparent pixels
    let pixels = [
        255, 0, 0, 255, // red, opaque
        0, 0, 0, 0, // transparent
        0, 255, 0, 128, // green, semi-transparent
        0, 0, 255, 255, // blue, opaque
    ];

    let result = create_png_auto(&pixels, 2, 2);
    assert!(result.is_ok());
}

// ============================================================================
// Format selection tests
// ============================================================================

#[test]
fn test_create_png_rgba_fallback_many_colors() {
    // Create image with >256 unique colors
    let mut pixels = Vec::with_capacity(300 * 4);
    for i in 0..300 {
        pixels.push((i % 256) as u8); // R
        pixels.push(((i / 2) % 256) as u8); // G
        pixels.push(((i / 3) % 256) as u8); // B
        pixels.push(255); // A
    }

    // Should fall back to RGBA (more than 256 colors)
    let result = create_png_auto(&pixels, 300, 1);
    assert!(result.is_ok());
}

#[test]
fn test_create_png_auto_weather_like() {
    // Simulate weather tile: gradient with limited colors
    // 16x16 tile with temperature-like gradient (~20 unique colors)
    let mut pixels = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16 {
        for x in 0..16 {
            // Quantized gradient (step of 16 gives ~16 unique colors per channel)
            let r = ((x * 16) as u8).wrapping_mul(16);
            let g = 128;
            let b = ((y * 16) as u8).wrapping_mul(16);
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }

    let indexed = create_png_auto(&pixels, 16, 16).unwrap();
    let rgba = create_png(&pixels, 16, 16).unwrap();

    // Indexed should be smaller for weather-like data
    assert!(
        indexed.len() < rgba.len(),
        "Indexed PNG ({} bytes) should be smaller than RGBA ({} bytes)",
        indexed.len(),
        rgba.len()
    );
}

// ============================================================================
// Large image tests (parallel processing)
// ============================================================================

#[test]
fn test_large_image_parallel_extraction() {
    // Generate a larger image that triggers parallel extraction
    // 128x128 = 16384 pixels, above PARALLEL_THRESHOLD (4096)
    let mut pixels = Vec::with_capacity(128 * 128 * 4);
    for y in 0..128 {
        for x in 0..128 {
            // Limited color palette (~50 colors)
            let color_idx = ((x / 8) + (y / 8)) % 50;
            let r = (color_idx * 5) as u8;
            let g = (100 + color_idx * 3) as u8;
            let b = (200 - color_idx * 2) as u8;
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }

    let result = create_png_auto(&pixels, 128, 128);
    assert!(result.is_ok());

    let png = result.unwrap();
    // Should be a valid PNG
    assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
}

// ============================================================================
// File size comparison tests
// ============================================================================

#[test]
fn test_file_size_comparison_256x256() {
    let pixels = generate_weather_pixels(256, 256);
    let unique_count = count_unique_colors(&pixels);

    let indexed = create_png_auto(&pixels, 256, 256).unwrap();
    let rgba = create_png(&pixels, 256, 256).unwrap();

    println!("\n=== 256x256 Weather Tile ===");
    println!("Unique colors: {}", unique_count);
    println!(
        "RGBA PNG:    {:>6} bytes ({:.1} KB)",
        rgba.len(),
        rgba.len() as f64 / 1024.0
    );
    println!(
        "Indexed PNG: {:>6} bytes ({:.1} KB)",
        indexed.len(),
        indexed.len() as f64 / 1024.0
    );
    println!(
        "Savings:     {:.1}%",
        (1.0 - indexed.len() as f64 / rgba.len() as f64) * 100.0
    );

    // When colors fit in palette, indexed should be smaller or equal
    if unique_count <= 256 {
        assert!(
            indexed.len() <= rgba.len(),
            "Indexed should be <= RGBA when colors fit"
        );
    }
}

#[test]
fn test_file_size_comparison_512x512() {
    let pixels = generate_weather_pixels(512, 512);
    let unique_count = count_unique_colors(&pixels);

    let indexed = create_png_auto(&pixels, 512, 512).unwrap();
    let rgba = create_png(&pixels, 512, 512).unwrap();

    println!("\n=== 512x512 Weather Tile ===");
    println!("Unique colors: {}", unique_count);
    println!(
        "RGBA PNG:    {:>6} bytes ({:.1} KB)",
        rgba.len(),
        rgba.len() as f64 / 1024.0
    );
    println!(
        "Indexed PNG: {:>6} bytes ({:.1} KB)",
        indexed.len(),
        indexed.len() as f64 / 1024.0
    );
    println!(
        "Savings:     {:.1}%",
        (1.0 - indexed.len() as f64 / rgba.len() as f64) * 100.0
    );

    if unique_count <= 256 {
        assert!(
            indexed.len() <= rgba.len(),
            "Indexed should be <= RGBA when colors fit"
        );
    }
}

// ============================================================================
// Edge case tests
// ============================================================================

#[test]
fn test_single_pixel() {
    let pixels = [255, 0, 0, 255]; // single red pixel
    let result = create_png_auto(&pixels, 1, 1);
    assert!(result.is_ok());
}

#[test]
fn test_single_color_image() {
    // 100x100 image with a single color
    let mut pixels = Vec::with_capacity(100 * 100 * 4);
    for _ in 0..(100 * 100) {
        pixels.extend_from_slice(&[128, 64, 32, 255]);
    }

    let result = create_png_auto(&pixels, 100, 100);
    assert!(result.is_ok());

    // Single color should result in very small file
    let png = result.unwrap();
    assert!(
        png.len() < 1000,
        "Single color 100x100 should be very small"
    );
}

#[test]
fn test_all_transparent() {
    // 10x10 fully transparent image
    let pixels = vec![0u8; 10 * 10 * 4];

    let result = create_png_auto(&pixels, 10, 10);
    assert!(result.is_ok());
}

#[test]
fn test_grayscale_gradient() {
    // 256x1 grayscale gradient (exactly 256 unique colors)
    let mut pixels = Vec::with_capacity(256 * 4);
    for i in 0..256 {
        let v = i as u8;
        pixels.extend_from_slice(&[v, v, v, 255]);
    }

    // Should still be able to use indexed (exactly 256 colors)
    let result = create_png_auto(&pixels, 256, 1);
    assert!(result.is_ok());
}

#[test]
fn test_grayscale_gradient_plus_one() {
    // 257 colors (one more than max palette size)
    let mut pixels = Vec::with_capacity(257 * 4);
    for i in 0..256 {
        let v = i as u8;
        pixels.extend_from_slice(&[v, v, v, 255]);
    }
    // Add one more unique color
    pixels.extend_from_slice(&[128, 0, 0, 255]); // red-ish

    // Should fall back to RGBA
    let result = create_png_auto(&pixels, 257, 1);
    assert!(result.is_ok());
}
