//! Tests for wind barb rendering module.

use renderer::barbs::{uv_to_speed_direction, BarbConfig};
use std::f64::consts::PI;

// ============================================================================
// uv_to_speed_direction tests
//
// Note: The direction output uses mathematical angle convention where
// angles are measured counterclockwise from the positive X axis.
// However, the U/V components follow meteorological convention where:
// - U > 0 means wind blowing eastward (from west)
// - V > 0 means wind blowing northward (from south)
// ============================================================================

#[test]
fn test_uv_calm_wind() {
    let (speed, _direction) = uv_to_speed_direction(0.0, 0.0);
    assert!(speed < 0.001);
}

#[test]
fn test_uv_speed_calculation_3_4_5() {
    // Test Pythagorean calculation with 3-4-5 triangle
    let (speed, _) = uv_to_speed_direction(3.0, 4.0);
    assert!((speed - 5.0).abs() < 0.01);

    let (speed, _) = uv_to_speed_direction(3.0, -4.0);
    assert!((speed - 5.0).abs() < 0.01);

    let (speed, _) = uv_to_speed_direction(-3.0, 4.0);
    assert!((speed - 5.0).abs() < 0.01);

    let (speed, _) = uv_to_speed_direction(-3.0, -4.0);
    assert!((speed - 5.0).abs() < 0.01);
}

#[test]
fn test_uv_speed_pure_u() {
    let (speed, _) = uv_to_speed_direction(10.0, 0.0);
    assert!((speed - 10.0).abs() < 0.01);

    let (speed, _) = uv_to_speed_direction(-10.0, 0.0);
    assert!((speed - 10.0).abs() < 0.01);
}

#[test]
fn test_uv_speed_pure_v() {
    let (speed, _) = uv_to_speed_direction(0.0, 10.0);
    assert!((speed - 10.0).abs() < 0.01);

    let (speed, _) = uv_to_speed_direction(0.0, -10.0);
    assert!((speed - 10.0).abs() < 0.01);
}

#[test]
fn test_uv_direction_normalized() {
    // Direction should always be in [0, 2π)
    let test_cases = [
        (10.0, 0.0),
        (-10.0, 0.0),
        (0.0, 10.0),
        (0.0, -10.0),
        (5.0, 5.0),
        (-5.0, 5.0),
        (5.0, -5.0),
        (-5.0, -5.0),
    ];

    for (u, v) in test_cases {
        let (_, direction) = uv_to_speed_direction(u, v);
        assert!(
            direction >= 0.0 && direction < 2.0 * PI,
            "Direction {} out of range for u={}, v={}",
            direction,
            u,
            v
        );
    }
}

#[test]
fn test_uv_direction_consistency() {
    // Same wind components should always give same direction
    for _ in 0..10 {
        let (_, d1) = uv_to_speed_direction(5.0, 5.0);
        let (_, d2) = uv_to_speed_direction(5.0, 5.0);
        assert!((d1 - d2).abs() < 0.0001);
    }
}

#[test]
fn test_uv_direction_opposite_winds() {
    // Opposite wind vectors should give directions ~π apart
    let (_, d1) = uv_to_speed_direction(10.0, 0.0);
    let (_, d2) = uv_to_speed_direction(-10.0, 0.0);

    // The difference should be π (180 degrees)
    let diff = (d1 - d2).abs();
    assert!(
        (diff - PI).abs() < 0.01 || (diff - 3.0 * PI).abs() < 0.01,
        "Expected π difference, got {} (d1={}, d2={})",
        diff,
        d1,
        d2
    );
}

#[test]
fn test_uv_direction_perpendicular_winds() {
    // Perpendicular wind vectors should give directions ~π/2 apart
    let (_, d_u_positive) = uv_to_speed_direction(10.0, 0.0);
    let (_, d_v_positive) = uv_to_speed_direction(0.0, 10.0);

    let diff = (d_u_positive - d_v_positive).abs();
    // Should be π/2 or 3π/2
    assert!(
        (diff - PI / 2.0).abs() < 0.01 || (diff - 3.0 * PI / 2.0).abs() < 0.01,
        "Expected π/2 difference, got {}",
        diff
    );
}

#[test]
fn test_uv_large_values() {
    // Test with hurricane-force winds
    let (speed, direction) = uv_to_speed_direction(50.0, 50.0);

    let expected_speed = (5000.0_f64).sqrt();
    assert!((speed - expected_speed).abs() < 0.1);
    assert!(direction >= 0.0 && direction < 2.0 * PI);
}

#[test]
fn test_uv_small_values() {
    // Test with very light winds
    let (speed, direction) = uv_to_speed_direction(0.1, 0.1);

    let expected_speed = (0.02_f64).sqrt();
    assert!((speed - expected_speed).abs() < 0.001);
    assert!(direction >= 0.0 && direction < 2.0 * PI);
}

#[test]
fn test_uv_asymmetric_components() {
    // Very asymmetric wind components
    let (speed, _) = uv_to_speed_direction(100.0, 1.0);
    // Speed should be dominated by U
    let expected = ((100.0_f64).powi(2) + 1.0).sqrt();
    assert!((speed - expected).abs() < 0.01);
}

// ============================================================================
// BarbConfig tests
// ============================================================================

#[test]
fn test_barb_config_default() {
    let config = BarbConfig::default();
    assert_eq!(config.size, 108);
    assert_eq!(config.spacing, 30);
    assert_eq!(config.color, "#000000");
}

#[test]
fn test_barb_config_custom() {
    let config = BarbConfig {
        size: 64,
        spacing: 50,
        color: "#FF0000".to_string(),
    };

    assert_eq!(config.size, 64);
    assert_eq!(config.spacing, 50);
    assert_eq!(config.color, "#FF0000");
}

#[test]
fn test_barb_config_clone() {
    let config = BarbConfig::default();
    let cloned = config.clone();

    assert_eq!(config.size, cloned.size);
    assert_eq!(config.spacing, cloned.spacing);
    assert_eq!(config.color, cloned.color);
}

// ============================================================================
// Wind speed classification tests (speed only)
// ============================================================================

#[test]
fn test_wind_speed_calm() {
    // Calm winds: 0-1 m/s
    let (speed, _) = uv_to_speed_direction(0.5, 0.5);
    assert!(speed < 1.0);
}

#[test]
fn test_wind_speed_light() {
    // Light winds: ~5 m/s
    let (speed, _) = uv_to_speed_direction(3.0, 4.0);
    assert!((speed - 5.0).abs() < 0.1);
}

#[test]
fn test_wind_speed_moderate() {
    // Moderate winds: ~15 m/s
    let (speed, _) = uv_to_speed_direction(9.0, 12.0);
    assert!((speed - 15.0).abs() < 0.1);
}

#[test]
fn test_wind_speed_strong() {
    // Strong winds: ~25 m/s
    let (speed, _) = uv_to_speed_direction(15.0, 20.0);
    assert!((speed - 25.0).abs() < 0.1);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_uv_negative_zero() {
    // Test -0.0 doesn't cause issues
    let (speed, _direction) = uv_to_speed_direction(-0.0, -0.0);
    assert!(speed < 0.001);
}

#[test]
fn test_uv_symmetry_speed() {
    // Speed should be same regardless of sign combinations
    let combinations = [(5.0, 5.0), (5.0, -5.0), (-5.0, 5.0), (-5.0, -5.0)];

    let expected_speed = (50.0_f64).sqrt();
    for (u, v) in combinations {
        let (speed, _) = uv_to_speed_direction(u, v);
        assert!(
            (speed - expected_speed).abs() < 0.01,
            "Speed mismatch for u={}, v={}: got {}",
            u,
            v,
            speed
        );
    }
}

#[test]
fn test_uv_quadrant_coverage() {
    // Ensure we get different directions in different quadrants
    let q1 = uv_to_speed_direction(5.0, 5.0).1; // u+, v+
    let q2 = uv_to_speed_direction(-5.0, 5.0).1; // u-, v+
    let q3 = uv_to_speed_direction(-5.0, -5.0).1; // u-, v-
    let q4 = uv_to_speed_direction(5.0, -5.0).1; // u+, v-

    // All four should be different
    let dirs = [q1, q2, q3, q4];
    for i in 0..4 {
        for j in (i + 1)..4 {
            assert!(
                (dirs[i] - dirs[j]).abs() > 0.1,
                "Directions in quadrants {} and {} are too similar: {} vs {}",
                i + 1,
                j + 1,
                dirs[i],
                dirs[j]
            );
        }
    }
}

#[test]
fn test_uv_continuous_direction_change() {
    // Direction should change smoothly as we rotate the wind vector
    let mut prev_dir = 0.0;
    let mut first = true;

    for i in 0..360 {
        let angle = (i as f64) * PI / 180.0;
        let u = 10.0 * angle.cos();
        let v = 10.0 * angle.sin();

        let (_, dir) = uv_to_speed_direction(u as f32, v as f32);

        if !first {
            // Direction change should be smooth (< 90 degrees usually)
            let change = (dir - prev_dir).abs();
            // Allow for wrapping at 2π
            let change = if change > PI {
                2.0 * PI - change
            } else {
                change
            };
            assert!(
                change < 0.1,
                "Large direction jump at angle {}: {} to {}",
                i,
                prev_dir,
                dir
            );
        }

        prev_dir = dir;
        first = false;
    }
}
