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

// ============================================================================
// Transform application tests
// ============================================================================

#[test]
fn test_apply_transform_none() {
    let value = 42.0f32;
    let result = apply_transform(value, None);
    assert_eq!(result, value);
}

#[test]
fn test_apply_transform_scale_only() {
    use renderer::style::Transform;

    let transform = Transform {
        transform_type: "linear".to_string(),
        scale: Some(2.0),
        offset: None,
    };

    let result = apply_transform(10.0, Some(&transform));
    assert!((result - 20.0).abs() < 0.001);
}

#[test]
fn test_apply_transform_offset_only() {
    use renderer::style::Transform;

    let transform = Transform {
        transform_type: "linear".to_string(),
        scale: None,
        offset: Some(-273.15),
    };

    // With no scale, should default to scale=1.0
    let result = apply_transform(300.0, Some(&transform));
    assert!((result - 26.85).abs() < 0.01);
}

#[test]
fn test_apply_transform_kelvin_to_celsius() {
    use renderer::style::Transform;

    // Standard K to C: subtract 273.15
    let transform = Transform {
        transform_type: "linear".to_string(),
        scale: Some(1.0),
        offset: Some(-273.15),
    };

    // Freezing point: 273.15 K = 0 C
    assert!((apply_transform(273.15, Some(&transform)) - 0.0).abs() < 0.01);

    // Boiling point: 373.15 K = 100 C
    assert!((apply_transform(373.15, Some(&transform)) - 100.0).abs() < 0.01);
}

#[test]
fn test_apply_transform_pascals_to_hectopascals() {
    use renderer::style::Transform;

    // Pa to hPa: divide by 100 (scale = 0.01)
    let transform = Transform {
        transform_type: "linear".to_string(),
        scale: Some(0.01),
        offset: None,
    };

    let result = apply_transform(101325.0, Some(&transform));
    assert!((result - 1013.25).abs() < 0.01);
}

// ============================================================================
// Palette size and efficiency tests
// ============================================================================

#[test]
fn test_palette_size_reasonable() {
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
                    {"value": 25, "color": "#00FFFF"},
                    {"value": 50, "color": "#00FF00"},
                    {"value": 75, "color": "#FFFF00"},
                    {"value": 100, "color": "#FF0000"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();
    let palette = style.compute_palette().unwrap();

    // Palette should fit in 8-bit PNG (256 colors max)
    assert!(
        palette.colors.len() <= 256,
        "Palette too large for indexed PNG: {} colors",
        palette.colors.len()
    );

    // LUT should be 4096 entries
    assert_eq!(palette.value_to_index.len(), 4096);
}

#[test]
fn test_palette_has_transparent() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "range": {"min": 0.0, "max": 100.0},
                "stops": [
                    {"value": 0, "color": "#FF0000"},
                    {"value": 100, "color": "#0000FF"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();
    let palette = style.compute_palette().unwrap();

    // First color should be transparent (for NaN values)
    assert_eq!(
        palette.colors[0],
        (0, 0, 0, 0),
        "Index 0 should be transparent for NaN values"
    );
}

// ============================================================================
// Edge cases for gradient rendering
// ============================================================================

#[test]
fn test_gradient_values_at_stops() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "range": {"min": 0.0, "max": 100.0},
                "stops": [
                    {"value": 0, "color": "#FF0000"},
                    {"value": 100, "color": "#0000FF"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();

    // Value at first stop should be close to first stop color (red)
    let rgba = apply_style_gradient(&[0.0], 1, 1, style);
    assert!(rgba[0] > 200, "Red channel should be high at stop 0");
    assert!(rgba[2] < 50, "Blue channel should be low at stop 0");

    // Value at last stop should be close to last stop color (blue)
    let rgba = apply_style_gradient(&[100.0], 1, 1, style);
    assert!(rgba[0] < 50, "Red channel should be low at stop 100");
    assert!(rgba[2] > 200, "Blue channel should be high at stop 100");
}

#[test]
fn test_gradient_values_out_of_range() {
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

    // Values below range should clamp
    let rgba = apply_style_gradient(&[-50.0], 1, 1, style);
    // Should be clamped to first stop (black-ish)
    assert!(rgba[0] < 30, "Below-range should clamp to min color");

    // Values above range should clamp
    let rgba = apply_style_gradient(&[150.0], 1, 1, style);
    // Should be clamped to last stop (white-ish)
    assert!(rgba[0] > 220, "Above-range should clamp to max color");
}

#[test]
fn test_gradient_nan_handling() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "range": {"min": 0.0, "max": 100.0},
                "stops": [
                    {"value": 0, "color": "#FF0000"},
                    {"value": 100, "color": "#0000FF"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();

    // NaN values should be rendered as transparent
    // Note: Sentinel values (like -9999) are now converted to NaN during ingestion,
    // so the renderer only needs to handle NaN for transparency
    let rgba = apply_style_gradient(&[f32::NAN], 1, 1, style);
    assert_eq!(rgba[3], 0, "NaN should be transparent");
}

// ============================================================================
// Style definition parsing edge cases
// ============================================================================

#[test]
fn test_style_without_range() {
    // Range should be inferred from stops
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "stops": [
                    {"value": -10, "color": "#0000FF"},
                    {"value": 30, "color": "#FF0000"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();
    let palette = style.compute_palette().unwrap();

    // Range should be inferred from stops
    assert!((palette.min_value - (-10.0)).abs() < 0.01);
    assert!((palette.max_value - 30.0).abs() < 0.01);
}

#[test]
fn test_style_with_labels() {
    let json = r##"{
        "version": "1.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "stops": [
                    {"value": 0, "color": "#0000FF", "label": "Cold"},
                    {"value": 50, "color": "#00FF00", "label": "Mild"},
                    {"value": 100, "color": "#FF0000", "label": "Hot"}
                ]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    let style = config.get_style("test").unwrap();

    // Labels should be parsed
    assert_eq!(style.stops[0].label.as_deref(), Some("Cold"));
    assert_eq!(style.stops[1].label.as_deref(), Some("Mild"));
    assert_eq!(style.stops[2].label.as_deref(), Some("Hot"));
}

#[test]
fn test_style_version_parsing() {
    let json = r##"{
        "version": "2.0",
        "styles": {
            "test": {
                "default": true,
                "name": "Test",
                "type": "gradient",
                "stops": [{"value": 0, "color": "#000"}, {"value": 1, "color": "#FFF"}]
            }
        }
    }"##;

    let config = StyleConfig::from_json(json).unwrap();
    assert_eq!(config.version, "2.0");
}
