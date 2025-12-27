//! Tests for colorscales rendering functions.

use crate::rendering::colorscales::{hsv_to_rgb, render_with_style_file};

// ============================================================================
// HSV to RGB conversion tests
// ============================================================================

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
    // Hue should wrap at 360
    let (r1, g1, b1) = hsv_to_rgb(0.0, 1.0, 1.0);
    let (r2, g2, b2) = hsv_to_rgb(360.0, 1.0, 1.0);
    assert_eq!(r1, r2);
    assert_eq!(g1, g2);
    assert_eq!(b1, b2);
}

// ============================================================================
// render_with_style_file tests
// ============================================================================

#[test]
fn test_render_with_style_file_missing_file() {
    // Should return an error when file doesn't exist
    let data = vec![50.0f32; 100];
    let result = render_with_style_file(&data, "/nonexistent/path/style.json", None, 10, 10);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Failed to load style file"),
        "Error should mention loading failure: {}",
        err
    );
}

#[test]
fn test_render_with_style_file_missing_style_name() {
    // Should return an error when style name not found (using temp file)
    let temp_dir = std::env::temp_dir();
    let style_path = temp_dir.join("test_style_missing_name.json");

    // Create a minimal valid style file without the requested style
    let style_content = r##"{
        "version": "1.0",
        "metadata": { "name": "Test", "description": "Test style" },
        "styles": {
            "default": {
                "name": "Default Style",
                "type": "gradient",
                "default": true,
                "range": { "min": 0.0, "max": 100.0 },
                "stops": [
                    { "value": 0.0, "color": "#0000FF" },
                    { "value": 100.0, "color": "#FF0000" }
                ]
            }
        }
    }"##;
    std::fs::write(&style_path, style_content).unwrap();

    let data = vec![25.0f32; 400]; // 20x20 grid
    let result = render_with_style_file(
        &data,
        style_path.to_str().unwrap(),
        Some("nonexistent_style"),
        20,
        20,
    );

    // Cleanup
    let _ = std::fs::remove_file(&style_path);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("not found"),
        "Error should mention style not found: {}",
        err
    );
}

#[test]
fn test_render_with_style_file_success() {
    // Should successfully render with a valid style file
    let temp_dir = std::env::temp_dir();
    let style_path = temp_dir.join("test_style_success.json");

    // Create a valid style file matching the actual schema
    let style_content = r##"{
        "version": "1.0",
        "metadata": { "name": "Test", "description": "Test style" },
        "styles": {
            "gradient": {
                "name": "Test Gradient",
                "type": "gradient",
                "default": true,
                "range": { "min": 0.0, "max": 100.0 },
                "stops": [
                    { "value": 0.0, "color": "#0000FF" },
                    { "value": 50.0, "color": "#00FF00" },
                    { "value": 100.0, "color": "#FF0000" }
                ]
            }
        }
    }"##;
    std::fs::write(&style_path, style_content).unwrap();

    let data = vec![50.0f32; 400]; // 20x20 grid
    let result = render_with_style_file(&data, style_path.to_str().unwrap(), None, 20, 20);

    // Cleanup
    let _ = std::fs::remove_file(&style_path);

    assert!(result.is_ok());
    // RGBA output: 20*20*4 = 1600 bytes
    assert_eq!(result.unwrap().len(), 1600);
}
