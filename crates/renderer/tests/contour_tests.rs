//! Tests for contour line (isoline) rendering module.

use renderer::contour::{
    connect_segments, generate_all_contours, generate_contour_levels, march_squares,
    render_contours, smooth_contour, Contour, ContourConfig, Point, Segment, SpecialLevelConfig,
};

// ============================================================================
// generate_contour_levels tests
// ============================================================================

#[test]
fn test_generate_contour_levels_basic() {
    let levels = generate_contour_levels(0.0, 100.0, 10.0);
    assert_eq!(
        levels,
        vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0]
    );
}

#[test]
fn test_generate_contour_levels_offset_start() {
    // Range doesn't start on interval multiple
    let levels = generate_contour_levels(3.0, 27.0, 5.0);
    assert_eq!(levels, vec![5.0, 10.0, 15.0, 20.0, 25.0]);
}

#[test]
fn test_generate_contour_levels_negative_range() {
    let levels = generate_contour_levels(-20.0, 20.0, 10.0);
    assert_eq!(levels, vec![-20.0, -10.0, 0.0, 10.0, 20.0]);
}

#[test]
fn test_generate_contour_levels_fractional_interval() {
    let levels = generate_contour_levels(0.0, 1.0, 0.25);
    assert_eq!(levels.len(), 5);
    assert!((levels[0] - 0.0).abs() < 0.001);
    assert!((levels[4] - 1.0).abs() < 0.001);
}

#[test]
fn test_generate_contour_levels_invalid_interval() {
    // Zero or negative interval should return empty
    let levels = generate_contour_levels(0.0, 100.0, 0.0);
    assert!(levels.is_empty());

    let levels = generate_contour_levels(0.0, 100.0, -10.0);
    assert!(levels.is_empty());
}

#[test]
fn test_generate_contour_levels_inverted_range() {
    // max < min should return empty
    let levels = generate_contour_levels(100.0, 0.0, 10.0);
    assert!(levels.is_empty());
}

#[test]
fn test_generate_contour_levels_single_level() {
    // Range only fits one level
    let levels = generate_contour_levels(9.0, 11.0, 10.0);
    assert_eq!(levels, vec![10.0]);
}

// ============================================================================
// march_squares tests
// ============================================================================

#[test]
fn test_march_squares_empty_cases() {
    // Grid too small
    let segments = march_squares(&[1.0], 1, 1, 0.5);
    assert!(segments.is_empty());

    // Empty data
    let segments = march_squares(&[], 0, 0, 0.5);
    assert!(segments.is_empty());

    // Mismatched dimensions
    let segments = march_squares(&[1.0, 2.0], 3, 3, 0.5);
    assert!(segments.is_empty());
}

#[test]
fn test_march_squares_flat_field_below() {
    // All values below threshold - no contour (case 0)
    let data = vec![1.0, 1.0, 1.0, 1.0];
    let segments = march_squares(&data, 2, 2, 5.0);
    assert!(segments.is_empty());
}

#[test]
fn test_march_squares_flat_field_above() {
    // All values above threshold - no contour (case 15)
    let data = vec![10.0, 10.0, 10.0, 10.0];
    let segments = march_squares(&data, 2, 2, 5.0);
    assert!(segments.is_empty());
}

#[test]
fn test_march_squares_flat_field_at_level() {
    // All values exactly at threshold - all >= so case 15
    let data = vec![5.0, 5.0, 5.0, 5.0];
    let segments = march_squares(&data, 2, 2, 5.0);
    assert!(segments.is_empty());
}

#[test]
fn test_march_squares_single_corner_above() {
    // Only top-left above threshold (case 1)
    let data = vec![
        10.0, 0.0, // row 0
        0.0, 0.0, // row 1
    ];
    let segments = march_squares(&data, 2, 2, 5.0);
    assert_eq!(segments.len(), 1);

    // Segment should go from left edge to top edge
    let seg = &segments[0];
    assert!(seg.start.x < 0.5); // On left edge
    assert!(seg.end.y < 0.5); // On top edge
}

#[test]
fn test_march_squares_horizontal_gradient() {
    // Left side low, right side high
    let data = vec![
        0.0, 10.0, // row 0
        0.0, 10.0, // row 1
    ];
    let segments = march_squares(&data, 2, 2, 5.0);

    // Should get a vertical line segment (case 3 or 12)
    assert_eq!(segments.len(), 1);
    let seg = &segments[0];

    // Start and end should have same x (vertical line)
    assert!((seg.start.x - seg.end.x).abs() < 0.01);
}

#[test]
fn test_march_squares_vertical_gradient() {
    // Top low, bottom high
    let data = vec![
        0.0, 0.0, // row 0
        10.0, 10.0, // row 1
    ];
    let segments = march_squares(&data, 2, 2, 5.0);

    // Should get a horizontal line segment (case 6 or 9)
    assert_eq!(segments.len(), 1);
    let seg = &segments[0];

    // Start and end should have same y (horizontal line)
    assert!((seg.start.y - seg.end.y).abs() < 0.01);
}

#[test]
fn test_march_squares_saddle_case_5() {
    // Diagonal pattern: TL and BR high (case 5)
    let data = vec![
        10.0, 0.0, // row 0
        0.0, 10.0, // row 1
    ];
    let segments = march_squares(&data, 2, 2, 5.0);

    // Saddle point should produce 2 segments
    assert_eq!(segments.len(), 2);
}

#[test]
fn test_march_squares_saddle_case_10() {
    // Diagonal pattern: TR and BL high (case 10)
    let data = vec![
        0.0, 10.0, // row 0
        10.0, 0.0, // row 1
    ];
    let segments = march_squares(&data, 2, 2, 5.0);

    // Saddle point should produce 2 segments
    assert_eq!(segments.len(), 2);
}

#[test]
fn test_march_squares_nan_handling() {
    // NaN values should cause cell to be skipped
    let data = vec![
        f32::NAN,
        10.0,
        10.0, // row 0
        0.0,
        10.0,
        10.0, // row 1
        0.0,
        0.0,
        0.0, // row 2
    ];
    let segments = march_squares(&data, 3, 3, 5.0);

    // Should still generate some segments from valid cells
    // but cells touching NaN should be skipped
    assert!(!segments.is_empty());
}

#[test]
fn test_march_squares_interpolation_accuracy() {
    // Test that interpolation places contour at correct position
    let data = vec![
        0.0, 100.0, // row 0
        0.0, 100.0, // row 1
    ];
    // Contour at 50 should be at x=0.5
    let segments = march_squares(&data, 2, 2, 50.0);
    assert_eq!(segments.len(), 1);

    let seg = &segments[0];
    // Both points should be at x=0.5 (halfway)
    assert!((seg.start.x - 0.5).abs() < 0.01);
    assert!((seg.end.x - 0.5).abs() < 0.01);
}

#[test]
fn test_march_squares_circular_peak() {
    // 5x5 grid with peak in center - should generate closed contour
    #[rustfmt::skip]
    let data = vec![
        0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 2.0, 4.0, 2.0, 0.0,
        0.0, 4.0, 8.0, 4.0, 0.0,
        0.0, 2.0, 4.0, 2.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0,
    ];
    let segments = march_squares(&data, 5, 5, 3.0);

    // Should have multiple segments forming a closed contour
    assert!(segments.len() >= 4);
}

#[test]
fn test_march_squares_larger_grid() {
    // Test with larger grid to ensure algorithm scales
    let width = 20;
    let height = 20;
    let mut data = vec![0.0; width * height];

    // Create gradient from left to right
    for y in 0..height {
        for x in 0..width {
            data[y * width + x] = x as f32;
        }
    }

    let segments = march_squares(&data, width, height, 10.0);

    // Should have segments for each row
    assert!(!segments.is_empty());
}

// ============================================================================
// connect_segments tests
// ============================================================================

#[test]
fn test_connect_segments_empty() {
    let contours = connect_segments(vec![]);
    assert!(contours.is_empty());
}

#[test]
fn test_connect_segments_single() {
    let segments = vec![Segment {
        start: Point::new(0.0, 0.0),
        end: Point::new(1.0, 1.0),
    }];
    let contours = connect_segments(segments);

    assert_eq!(contours.len(), 1);
    assert_eq!(contours[0].points.len(), 2);
    assert!(!contours[0].closed);
}

#[test]
fn test_connect_segments_chain() {
    // Three segments that form a chain
    let segments = vec![
        Segment {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 0.0),
        },
        Segment {
            start: Point::new(1.0, 0.0),
            end: Point::new(2.0, 0.0),
        },
        Segment {
            start: Point::new(2.0, 0.0),
            end: Point::new(3.0, 0.0),
        },
    ];
    let contours = connect_segments(segments);

    assert_eq!(contours.len(), 1);
    assert_eq!(contours[0].points.len(), 4);
    assert!(!contours[0].closed);
}

#[test]
fn test_connect_segments_closed_square() {
    // Four segments forming a closed square
    let segments = vec![
        Segment {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 0.0),
        },
        Segment {
            start: Point::new(1.0, 0.0),
            end: Point::new(1.0, 1.0),
        },
        Segment {
            start: Point::new(1.0, 1.0),
            end: Point::new(0.0, 1.0),
        },
        Segment {
            start: Point::new(0.0, 1.0),
            end: Point::new(0.0, 0.0),
        },
    ];
    let contours = connect_segments(segments);

    assert_eq!(contours.len(), 1);
    assert!(contours[0].closed);
}

#[test]
fn test_connect_segments_two_separate() {
    // Two separate line segments
    let segments = vec![
        Segment {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 0.0),
        },
        Segment {
            start: Point::new(10.0, 10.0),
            end: Point::new(11.0, 10.0),
        },
    ];
    let contours = connect_segments(segments);

    assert_eq!(contours.len(), 2);
}

#[test]
fn test_connect_segments_reversed() {
    // Segments connected but with reversed direction
    let segments = vec![
        Segment {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 0.0),
        },
        Segment {
            start: Point::new(2.0, 0.0),
            end: Point::new(1.0, 0.0), // End matches previous end
        },
    ];
    let contours = connect_segments(segments);

    assert_eq!(contours.len(), 1);
    assert_eq!(contours[0].points.len(), 3);
}

// ============================================================================
// smooth_contour tests
// ============================================================================

#[test]
fn test_smooth_contour_no_iterations() {
    let contour = Contour {
        level: 5.0,
        points: vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
        ],
        closed: false,
    };

    let result = smooth_contour(&contour, 0);
    assert_eq!(result.points.len(), contour.points.len());
}

#[test]
fn test_smooth_contour_too_few_points() {
    let contour = Contour {
        level: 5.0,
        points: vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0)],
        closed: false,
    };

    let result = smooth_contour(&contour, 3);
    // Should return unchanged since < 3 points
    assert_eq!(result.points.len(), 2);
}

#[test]
fn test_smooth_contour_increases_points() {
    let contour = Contour {
        level: 5.0,
        points: vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
            Point::new(0.0, 1.0),
        ],
        closed: false,
    };

    let result = smooth_contour(&contour, 1);
    // Chaikin's algorithm typically increases point count
    assert!(result.points.len() > contour.points.len());
}

#[test]
fn test_smooth_contour_preserves_endpoints() {
    let contour = Contour {
        level: 5.0,
        points: vec![
            Point::new(0.0, 0.0),
            Point::new(5.0, 5.0),
            Point::new(10.0, 0.0),
        ],
        closed: false,
    };

    let result = smooth_contour(&contour, 2);

    // For open contours, endpoints should be preserved
    let first = &result.points[0];
    let last = result.points.last().unwrap();

    assert!((first.x - 0.0).abs() < 0.01);
    assert!((first.y - 0.0).abs() < 0.01);
    assert!((last.x - 10.0).abs() < 0.01);
    assert!((last.y - 0.0).abs() < 0.01);
}

#[test]
fn test_smooth_contour_closed() {
    let contour = Contour {
        level: 5.0,
        points: vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
            Point::new(0.0, 1.0),
        ],
        closed: true,
    };

    let result = smooth_contour(&contour, 1);
    assert!(result.closed);
}

#[test]
fn test_smooth_contour_preserves_level() {
    let contour = Contour {
        level: 273.15,
        points: vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
        ],
        closed: false,
    };

    let result = smooth_contour(&contour, 2);
    assert!((result.level - 273.15).abs() < 0.01);
}

// ============================================================================
// ContourConfig tests
// ============================================================================

#[test]
fn test_contour_config_default() {
    let config = ContourConfig::default();
    assert!(config.levels.is_empty());
    assert_eq!(config.line_width, 2.0);
    assert_eq!(config.line_color, [0, 0, 0, 255]);
    assert!(!config.labels_enabled);
}

#[test]
fn test_contour_config_get_level_color() {
    let mut config = ContourConfig::default();
    config.line_color = [255, 0, 0, 255]; // Default red
    config.special_levels.push(SpecialLevelConfig {
        level: 0.0,
        line_color: Some([0, 0, 255, 255]), // Blue for freezing
        line_width: None,
        label: None,
    });

    // Special level should return special color
    let color = config.get_level_color(0.0);
    assert_eq!(color, [0, 0, 255, 255]);

    // Other level should return default
    let color = config.get_level_color(10.0);
    assert_eq!(color, [255, 0, 0, 255]);
}

#[test]
fn test_contour_config_get_level_width() {
    let mut config = ContourConfig::default();
    config.line_width = 1.0;
    config.special_levels.push(SpecialLevelConfig {
        level: 0.0,
        line_color: None,
        line_width: Some(3.0), // Thicker for freezing
        label: None,
    });

    assert_eq!(config.get_level_width(0.0), 3.0);
    assert_eq!(config.get_level_width(10.0), 1.0);
}

#[test]
fn test_contour_config_get_level_label() {
    let mut config = ContourConfig::default();
    config.label_unit_offset = -273.15; // Kelvin to Celsius
    config.special_levels.push(SpecialLevelConfig {
        level: 273.15,
        line_color: None,
        line_width: None,
        label: Some("0°C".to_string()),
    });

    // Special level should return custom label
    assert_eq!(config.get_level_label(273.15), "0°C");

    // Other level should return calculated value with offset
    let label = config.get_level_label(283.15);
    assert!(label.contains("10")); // Should be approximately 10°C
}

// ============================================================================
// Integration tests
// ============================================================================

#[test]
fn test_generate_all_contours() {
    #[rustfmt::skip]
    let data = vec![
        0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 5.0, 5.0, 5.0, 0.0,
        0.0, 5.0, 10.0, 5.0, 0.0,
        0.0, 5.0, 5.0, 5.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0,
    ];

    let mut config = ContourConfig::default();
    config.levels = vec![2.5, 7.5];
    config.smoothing_passes = 0;

    let contours = generate_all_contours(&data, 5, 5, &config);

    // Should have contours for both levels
    assert!(!contours.is_empty());

    // Check that levels are set correctly
    let has_25 = contours.iter().any(|c| (c.level - 2.5).abs() < 0.01);
    let has_75 = contours.iter().any(|c| (c.level - 7.5).abs() < 0.01);
    assert!(has_25);
    assert!(has_75);
}

#[test]
fn test_render_contours_output_size() {
    let data = vec![0.0, 10.0, 0.0, 10.0, 20.0, 10.0, 0.0, 10.0, 0.0];
    let width = 3;
    let height = 3;

    let mut config = ContourConfig::default();
    config.levels = vec![5.0, 15.0];

    let pixels = render_contours(&data, width, height, &config);

    assert_eq!(pixels.len(), width * height * 4);
}

#[test]
fn test_render_contours_with_labels() {
    // Larger grid for labels
    let width = 100;
    let height = 100;
    let mut data = vec![0.0; width * height];

    // Create diagonal gradient
    for y in 0..height {
        for x in 0..width {
            data[y * width + x] = (x + y) as f32;
        }
    }

    let mut config = ContourConfig::default();
    config.levels = vec![50.0, 100.0, 150.0];
    config.labels_enabled = true;
    config.label_spacing = 50.0;

    let pixels = render_contours(&data, width, height, &config);
    assert_eq!(pixels.len(), width * height * 4);
}

#[test]
fn test_render_contours_small_dimensions() {
    // Test with minimal dimensions (1x1 is too small for contours)
    let data: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
    let config = ContourConfig::default();

    // Should not panic with small but valid dimensions
    let pixels = render_contours(&data, 2, 2, &config);
    assert_eq!(pixels.len(), 2 * 2 * 4);
}

#[test]
fn test_render_contours_no_levels() {
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let config = ContourConfig::default(); // Empty levels

    // Should return valid image with no contours drawn
    let pixels = render_contours(&data, 2, 2, &config);
    assert_eq!(pixels.len(), 16);
}
