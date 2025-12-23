//! Tests for style-based rendering functionality.
//!
//! Tests the style definition parsing, palette computation, and gradient rendering.

use renderer::style::{apply_style_gradient, apply_transform, StyleConfig};

// ============================================================================
// Color parsing tests
// ============================================================================

#[test]
fn test_hex_to_rgb() {
    // Test with hash prefix
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "stops": [
                    {"value": 0, "color": "#FF0000"},
                    {"value": 100, "color": "#00FF00"}
                ]
            }
        }
    }"##;
    let config = StyleConfig::from_json(json).unwrap();

    let style = config.get_style("test").unwrap();
    assert!(!style.stops.is_empty());
}

// ============================================================================
// Transform tests
// ============================================================================

#[test]
fn test_linear_transform() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "gradient": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "transform": {
                    "type": "linear",
                    "scale": 0.0167,
                    "offset": 0
                },
                "stops": [
                    {"value": 0, "color": "#000000"},
                    {"value": 100, "color": "#FFFFFF"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("gradient").unwrap();

    // Verify transform is parsed
    assert!(style.transform.is_some());
    let t = style.transform.as_ref().unwrap();
    assert_eq!(t.transform_type, "linear");
    assert_eq!(t.scale, Some(0.0167));
    assert_eq!(t.offset, Some(0.0));

    // Test transform application
    let raw_value: f32 = 35500.0;
    let transformed = apply_transform(raw_value, style.transform.as_ref());
    let expected = 35500.0 * 0.0167;
    assert!(
        (transformed - expected).abs() < 0.01,
        "Expected {}, got {}",
        expected,
        transformed
    );
}

#[test]
fn test_linear_transform_with_offset() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "transform": {
                    "type": "linear",
                    "scale": 1.0,
                    "offset": -273.15
                },
                "stops": [
                    {"value": -50, "color": "#0000FF"},
                    {"value": 50, "color": "#FF0000"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();

    // Test Kelvin to Celsius conversion
    let kelvin: f32 = 300.0;
    let celsius = apply_transform(kelvin, style.transform.as_ref());
    assert!(
        (celsius - 26.85).abs() < 0.01,
        "Expected ~26.85C, got {}",
        celsius
    );
}

// ============================================================================
// Palette computation tests
// ============================================================================

#[test]
fn test_precomputed_palette_matches_rgba() {
    // Create a simple gradient style
    let json = r##"{
        "version": "1.0",
        "styles": {
            "gradient": {
                "default": true,
                "name": "Test Gradient",
                "type": "gradient",
                "range": {"min": 0.0, "max": 100.0},
                "stops": [
                    {"value": 0, "color": "#0000FF"},
                    {"value": 50, "color": "#00FF00"},
                    {"value": 100, "color": "#FF0000"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("gradient").unwrap();
    let palette = style.compute_palette().expect("Should compute palette");

    // Test with specific values
    let test_values = vec![0.0f32, 25.0, 50.0, 75.0, 100.0];

    for &val in &test_values {
        // Get RGBA color
        let rgba = apply_style_gradient(&[val], 1, 1, style);
        let rgba_color = (rgba[0], rgba[1], rgba[2], rgba[3]);

        // Get indexed color - compute manually
        let t = (val - palette.min_value) / (palette.max_value - palette.min_value);
        let lut_idx = (t * 4095.0) as usize;
        let palette_idx = palette.value_to_index[lut_idx.min(4095)];
        let indexed_color = palette.colors[palette_idx as usize];

        // Colors should be similar (may not be exact due to quantization)
        let r_diff = (rgba_color.0 as i32 - indexed_color.0 as i32).abs();
        let g_diff = (rgba_color.1 as i32 - indexed_color.1 as i32).abs();
        let b_diff = (rgba_color.2 as i32 - indexed_color.2 as i32).abs();

        assert!(
            r_diff <= 5,
            "Red mismatch at {}: {} vs {}",
            val,
            rgba_color.0,
            indexed_color.0
        );
        assert!(
            g_diff <= 5,
            "Green mismatch at {}: {} vs {}",
            val,
            rgba_color.1,
            indexed_color.1
        );
        assert!(
            b_diff <= 5,
            "Blue mismatch at {}: {} vs {}",
            val,
            rgba_color.2,
            indexed_color.2
        );
    }
}

#[test]
fn test_palette_range_extraction() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "temp": {
                "default": true,
                "name": "Temperature",
                "type": "gradient",
                "range": {"min": -40.0, "max": 50.0},
                "stops": [
                    {"value": -40, "color": "#0000FF"},
                    {"value": 50, "color": "#FF0000"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("temp").unwrap();
    let palette = style.compute_palette().unwrap();

    assert!(
        (palette.min_value - (-40.0)).abs() < 0.001,
        "Min value should be -40, got {}",
        palette.min_value
    );
    assert!(
        (palette.max_value - 50.0).abs() < 0.001,
        "Max value should be 50, got {}",
        palette.max_value
    );
}

// ============================================================================
// Style configuration parsing tests
// ============================================================================

#[test]
fn test_parse_style_config() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "temperature": {
                "default": true,
                "name": "Temperature",
                "type": "gradient",
                "range": {"min": 220, "max": 320},
                "stops": [
                    {"value": 220, "color": "#9400D3"},
                    {"value": 270, "color": "#0000FF"},
                    {"value": 290, "color": "#00FF00"},
                    {"value": 310, "color": "#FF0000"},
                    {"value": 320, "color": "#8B0000"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("temperature").unwrap();

    assert_eq!(style.name, "Temperature");
    assert_eq!(style.style_type, "gradient");
    assert_eq!(style.stops.len(), 5);
}

#[test]
fn test_parse_multiple_styles() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "style1": {
                "default": true,
                "name": "Style One",
                "type": "gradient",
                "stops": [{"value": 0, "color": "#000000"}, {"value": 100, "color": "#FFFFFF"}]
            },
            "style2": {
                "name": "Style Two",
                "type": "gradient",
                "stops": [{"value": 0, "color": "#FF0000"}, {"value": 100, "color": "#0000FF"}]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();

    assert!(config.get_style("style1").is_some());
    assert!(config.get_style("style2").is_some());
    assert!(config.get_style("nonexistent").is_none());
}

#[test]
fn test_default_style() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "style1": {
                "name": "Style One",
                "type": "gradient",
                "stops": [{"value": 0, "color": "#000000"}, {"value": 100, "color": "#FFFFFF"}]
            },
            "style2": {
                "default": true,
                "name": "Style Two",
                "type": "gradient",
                "stops": [{"value": 0, "color": "#FF0000"}, {"value": 100, "color": "#0000FF"}]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let (_, default) = config.get_default_style().unwrap();

    assert_eq!(default.name, "Style Two");
}

// ============================================================================
// Gradient rendering tests
// ============================================================================

#[test]
fn test_apply_style_gradient_basic() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "range": {"min": 0.0, "max": 100.0},
                "stops": [
                    {"value": 0, "color": "#000000"},
                    {"value": 100, "color": "#FFFFFF"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();

    // Test single value at 50%
    let data = [50.0f32];
    let rgba = apply_style_gradient(&data, 1, 1, style);

    // At 50%, should be approximately gray (128, 128, 128)
    assert!(
        rgba[0] >= 120 && rgba[0] <= 135,
        "Red should be ~128, got {}",
        rgba[0]
    );
    assert!(
        rgba[1] >= 120 && rgba[1] <= 135,
        "Green should be ~128, got {}",
        rgba[1]
    );
    assert!(
        rgba[2] >= 120 && rgba[2] <= 135,
        "Blue should be ~128, got {}",
        rgba[2]
    );
    assert_eq!(rgba[3], 255, "Alpha should be 255");
}

#[test]
fn test_apply_style_gradient_nan_handling() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "stops": [
                    {"value": 0, "color": "#FF0000"},
                    {"value": 100, "color": "#0000FF"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();

    // Test with NaN values
    let data = [f32::NAN, 50.0, f32::NAN];
    let rgba = apply_style_gradient(&data, 3, 1, style);

    // NaN values should be transparent (alpha = 0)
    assert_eq!(rgba[3], 0, "NaN should produce transparent pixel");
    assert_eq!(rgba[7], 255, "Valid value should produce opaque pixel");
    assert_eq!(rgba[11], 0, "NaN should produce transparent pixel");
}

#[test]
fn test_apply_style_gradient_multi_row() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "range": {"min": 0.0, "max": 100.0},
                "stops": [
                    {"value": 0, "color": "#0000FF"},
                    {"value": 100, "color": "#FF0000"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();

    // Create 2x2 grid
    let data = [0.0f32, 100.0, 50.0, 50.0];
    let rgba = apply_style_gradient(&data, 2, 2, style);

    // Should have 4 pixels * 4 bytes = 16 bytes
    assert_eq!(rgba.len(), 16);

    // First pixel (0) should be blue-ish
    assert!(
        rgba[2] > rgba[0],
        "First pixel should be more blue than red"
    );

    // Second pixel (100) should be red-ish
    assert!(
        rgba[4] > rgba[6],
        "Second pixel should be more red than blue"
    );
}
