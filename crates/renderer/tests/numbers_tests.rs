//! Tests for numbers rendering module.

use renderer::numbers::{NumbersConfig, UnitTransform};

// ============================================================================
// UnitTransform tests
// ============================================================================

#[test]
fn test_unit_transform_none() {
    let transform = UnitTransform::None;
    assert_eq!(transform.apply(100.0), 100.0);
    assert_eq!(transform.apply(-50.0), -50.0);
    assert_eq!(transform.apply(0.0), 0.0);
}

#[test]
fn test_unit_transform_subtract() {
    // Kelvin to Celsius: subtract 273.15
    let transform = UnitTransform::Subtract(273.15);

    // Freezing point: 273.15 K = 0 C
    let result = transform.apply(273.15);
    assert!((result - 0.0).abs() < 0.01);

    // Boiling point: 373.15 K = 100 C
    let result = transform.apply(373.15);
    assert!((result - 100.0).abs() < 0.01);

    // Below freezing: 263.15 K = -10 C
    let result = transform.apply(263.15);
    assert!((result - (-10.0)).abs() < 0.01);
}

#[test]
fn test_unit_transform_divide() {
    // Pascals to hectopascals: divide by 100
    let transform = UnitTransform::Divide(100.0);

    let result = transform.apply(101325.0);
    assert!((result - 1013.25).abs() < 0.01);

    let result = transform.apply(100.0);
    assert!((result - 1.0).abs() < 0.01);
}

#[test]
fn test_unit_transform_linear() {
    // y = 1.8x + 32 (Celsius to Fahrenheit)
    let transform = UnitTransform::Linear {
        scale: 1.8,
        offset: 32.0,
    };

    // 0 C = 32 F
    let result = transform.apply(0.0);
    assert!((result - 32.0).abs() < 0.01);

    // 100 C = 212 F
    let result = transform.apply(100.0);
    assert!((result - 212.0).abs() < 0.01);

    // -40 C = -40 F (special crossover point)
    let result = transform.apply(-40.0);
    assert!((result - (-40.0)).abs() < 0.01);
}

#[test]
fn test_unit_transform_default() {
    let transform = UnitTransform::default();
    // Default should be None (no transformation)
    assert_eq!(transform.apply(42.0), 42.0);
}

#[test]
fn test_unit_transform_from_legacy_none() {
    let transform = UnitTransform::from_legacy(None);
    assert_eq!(transform.apply(100.0), 100.0);
}

#[test]
fn test_unit_transform_from_legacy_positive() {
    // Positive value = subtraction
    let transform = UnitTransform::from_legacy(Some(273.15));
    let result = transform.apply(273.15);
    assert!((result - 0.0).abs() < 0.01);
}

#[test]
fn test_unit_transform_from_legacy_negative() {
    // Negative value = division by abs value
    let transform = UnitTransform::from_legacy(Some(-100.0));
    let result = transform.apply(1000.0);
    assert!((result - 10.0).abs() < 0.01);
}

// ============================================================================
// NumbersConfig tests
// ============================================================================

#[test]
fn test_numbers_config_default() {
    let config = NumbersConfig::default();
    assert_eq!(config.spacing, 60);
    assert_eq!(config.font_size, 12.0);
    assert!(config.color_stops.is_empty());
    assert!(config.unit_conversion.is_none());
}

#[test]
fn test_numbers_config_custom() {
    let config = NumbersConfig {
        spacing: 100,
        font_size: 16.0,
        color_stops: vec![],
        unit_conversion: Some(273.15),
    };

    assert_eq!(config.spacing, 100);
    assert_eq!(config.font_size, 16.0);
}

// ============================================================================
// Edge case tests
// ============================================================================

#[test]
fn test_unit_transform_divide_by_zero() {
    // Division by zero should return infinity (not panic)
    let transform = UnitTransform::Divide(0.0);
    let result = transform.apply(100.0);
    assert!(result.is_infinite());
}

#[test]
fn test_unit_transform_with_nan() {
    let transform = UnitTransform::Subtract(10.0);
    let result = transform.apply(f32::NAN);
    assert!(result.is_nan());
}

#[test]
fn test_unit_transform_with_infinity() {
    let transform = UnitTransform::Subtract(10.0);

    let result = transform.apply(f32::INFINITY);
    assert!(result.is_infinite());

    let result = transform.apply(f32::NEG_INFINITY);
    assert!(result.is_infinite());
}

#[test]
fn test_unit_transform_chained_operations() {
    // Test that transforms can be applied sequentially
    let k_to_c = UnitTransform::Subtract(273.15);
    let c_to_f = UnitTransform::Linear {
        scale: 1.8,
        offset: 32.0,
    };

    // Convert 300 K to F
    let celsius = k_to_c.apply(300.0); // 26.85 C
    let fahrenheit = c_to_f.apply(celsius); // ~80.33 F

    assert!((celsius - 26.85).abs() < 0.01);
    assert!((fahrenheit - 80.33).abs() < 0.1);
}

// ============================================================================
// Additional UnitTransform coverage
// ============================================================================

#[test]
fn test_unit_transform_linear_identity() {
    // Scale 1, offset 0 should be identity
    let transform = UnitTransform::Linear {
        scale: 1.0,
        offset: 0.0,
    };
    assert_eq!(transform.apply(42.0), 42.0);
}

#[test]
fn test_unit_transform_linear_scale_only() {
    let transform = UnitTransform::Linear {
        scale: 2.0,
        offset: 0.0,
    };
    assert_eq!(transform.apply(5.0), 10.0);
}

#[test]
fn test_unit_transform_linear_offset_only() {
    let transform = UnitTransform::Linear {
        scale: 1.0,
        offset: 100.0,
    };
    assert_eq!(transform.apply(5.0), 105.0);
}

#[test]
fn test_unit_transform_subtract_zero() {
    let transform = UnitTransform::Subtract(0.0);
    assert_eq!(transform.apply(100.0), 100.0);
}

#[test]
fn test_unit_transform_divide_one() {
    let transform = UnitTransform::Divide(1.0);
    assert_eq!(transform.apply(100.0), 100.0);
}
