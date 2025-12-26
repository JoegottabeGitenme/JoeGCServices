//! Comprehensive tests for BoundingBox operations.

use wms_common::bbox::{BboxParseError, BoundingBox};

// ============================================================================
// Constructor tests
// ============================================================================

#[test]
fn test_bbox_new() {
    let bbox = BoundingBox::new(-180.0, -90.0, 180.0, 90.0);
    assert_eq!(bbox.min_x, -180.0);
    assert_eq!(bbox.min_y, -90.0);
    assert_eq!(bbox.max_x, 180.0);
    assert_eq!(bbox.max_y, 90.0);
}

#[test]
fn test_bbox_clone() {
    let bbox1 = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let bbox2 = bbox1;
    assert_eq!(bbox1, bbox2);
}

// ============================================================================
// from_wms_string tests
// ============================================================================

#[test]
fn test_parse_wms_bbox_integer() {
    let bbox = BoundingBox::from_wms_string("0,0,100,100").unwrap();
    assert_eq!(bbox.min_x, 0.0);
    assert_eq!(bbox.min_y, 0.0);
    assert_eq!(bbox.max_x, 100.0);
    assert_eq!(bbox.max_y, 100.0);
}

#[test]
fn test_parse_wms_bbox_floating() {
    let bbox = BoundingBox::from_wms_string("-125.5,24.75,-66.25,50.125").unwrap();
    assert!((bbox.min_x - (-125.5)).abs() < 0.001);
    assert!((bbox.min_y - 24.75).abs() < 0.001);
    assert!((bbox.max_x - (-66.25)).abs() < 0.001);
    assert!((bbox.max_y - 50.125).abs() < 0.001);
}

#[test]
fn test_parse_wms_bbox_scientific_notation() {
    let bbox = BoundingBox::from_wms_string("1e-6,2e-6,1e6,2e6").unwrap();
    assert!((bbox.min_x - 1e-6).abs() < 1e-10);
    assert!((bbox.max_x - 1e6).abs() < 0.001);
}

#[test]
fn test_parse_wms_bbox_negative() {
    let bbox = BoundingBox::from_wms_string("-180,-90,180,90").unwrap();
    assert_eq!(bbox.min_x, -180.0);
    assert_eq!(bbox.min_y, -90.0);
    assert_eq!(bbox.max_x, 180.0);
    assert_eq!(bbox.max_y, 90.0);
}

#[test]
fn test_parse_wms_bbox_web_mercator() {
    // EPSG:3857 coordinates (meters)
    let bbox =
        BoundingBox::from_wms_string("-20037508.34,-20037508.34,20037508.34,20037508.34").unwrap();
    assert!((bbox.min_x - (-20037508.34)).abs() < 0.01);
    assert!((bbox.max_x - 20037508.34).abs() < 0.01);
}

#[test]
fn test_parse_wms_bbox_invalid_format_too_few() {
    let result = BoundingBox::from_wms_string("0,0,100");
    assert!(matches!(result, Err(BboxParseError::InvalidFormat(_))));
}

#[test]
fn test_parse_wms_bbox_invalid_format_too_many() {
    let result = BoundingBox::from_wms_string("0,0,100,100,200");
    assert!(matches!(result, Err(BboxParseError::InvalidFormat(_))));
}

#[test]
fn test_parse_wms_bbox_invalid_number() {
    let result = BoundingBox::from_wms_string("abc,0,100,100");
    assert!(matches!(result, Err(BboxParseError::InvalidNumber(_))));
}

#[test]
fn test_parse_wms_bbox_empty_string() {
    let result = BoundingBox::from_wms_string("");
    assert!(matches!(result, Err(BboxParseError::InvalidFormat(_))));
}

#[test]
fn test_parse_wms_bbox_whitespace() {
    // WMS strings shouldn't have spaces, but let's handle gracefully
    let result = BoundingBox::from_wms_string(" 0, 0, 100, 100 ");
    // Should fail because of leading spaces in numbers
    assert!(result.is_err());
}

// ============================================================================
// Dimension tests (width/height)
// ============================================================================

#[test]
fn test_bbox_width() {
    let bbox = BoundingBox::new(10.0, 0.0, 30.0, 10.0);
    assert_eq!(bbox.width(), 20.0);
}

#[test]
fn test_bbox_height() {
    let bbox = BoundingBox::new(0.0, 5.0, 10.0, 25.0);
    assert_eq!(bbox.height(), 20.0);
}

#[test]
fn test_bbox_width_negative_coords() {
    let bbox = BoundingBox::new(-100.0, 0.0, -50.0, 10.0);
    assert_eq!(bbox.width(), 50.0);
}

#[test]
fn test_bbox_width_crossing_zero() {
    let bbox = BoundingBox::new(-10.0, 0.0, 10.0, 10.0);
    assert_eq!(bbox.width(), 20.0);
}

#[test]
fn test_bbox_zero_dimensions() {
    let bbox = BoundingBox::new(5.0, 5.0, 5.0, 5.0);
    assert_eq!(bbox.width(), 0.0);
    assert_eq!(bbox.height(), 0.0);
}

// ============================================================================
// Intersection tests
// ============================================================================

#[test]
fn test_bbox_intersects_overlap() {
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(5.0, 5.0, 15.0, 15.0);
    assert!(a.intersects(&b));
    assert!(b.intersects(&a)); // Symmetric
}

#[test]
fn test_bbox_intersects_no_overlap() {
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(20.0, 20.0, 30.0, 30.0);
    assert!(!a.intersects(&b));
    assert!(!b.intersects(&a));
}

#[test]
fn test_bbox_intersects_adjacent_edge() {
    // Touching at edge - not intersecting (open interval)
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(10.0, 0.0, 20.0, 10.0);
    assert!(!a.intersects(&b));
}

#[test]
fn test_bbox_intersects_contains() {
    let outer = BoundingBox::new(0.0, 0.0, 100.0, 100.0);
    let inner = BoundingBox::new(25.0, 25.0, 75.0, 75.0);
    assert!(outer.intersects(&inner));
    assert!(inner.intersects(&outer));
}

#[test]
fn test_bbox_intersects_partial_horizontal() {
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(5.0, 0.0, 15.0, 10.0);
    assert!(a.intersects(&b));
}

#[test]
fn test_bbox_intersects_partial_vertical() {
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(0.0, 5.0, 10.0, 15.0);
    assert!(a.intersects(&b));
}

#[test]
fn test_bbox_intersection_result() {
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(5.0, 5.0, 15.0, 15.0);
    let intersection = a.intersection(&b).unwrap();

    assert_eq!(intersection.min_x, 5.0);
    assert_eq!(intersection.min_y, 5.0);
    assert_eq!(intersection.max_x, 10.0);
    assert_eq!(intersection.max_y, 10.0);
}

#[test]
fn test_bbox_intersection_none() {
    let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let b = BoundingBox::new(20.0, 20.0, 30.0, 30.0);
    assert!(a.intersection(&b).is_none());
}

#[test]
fn test_bbox_intersection_with_self() {
    let bbox = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let intersection = bbox.intersection(&bbox).unwrap();
    assert_eq!(intersection, bbox);
}

// ============================================================================
// Contains point tests
// ============================================================================

#[test]
fn test_bbox_contains_point_inside() {
    let bbox = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    assert!(bbox.contains_point(5.0, 5.0));
}

#[test]
fn test_bbox_contains_point_on_edge() {
    let bbox = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    // Edges should be included
    assert!(bbox.contains_point(0.0, 5.0)); // Left edge
    assert!(bbox.contains_point(10.0, 5.0)); // Right edge
    assert!(bbox.contains_point(5.0, 0.0)); // Bottom edge
    assert!(bbox.contains_point(5.0, 10.0)); // Top edge
}

#[test]
fn test_bbox_contains_point_corner() {
    let bbox = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    // Corners should be included
    assert!(bbox.contains_point(0.0, 0.0));
    assert!(bbox.contains_point(10.0, 0.0));
    assert!(bbox.contains_point(0.0, 10.0));
    assert!(bbox.contains_point(10.0, 10.0));
}

#[test]
fn test_bbox_contains_point_outside() {
    let bbox = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    assert!(!bbox.contains_point(-1.0, 5.0));
    assert!(!bbox.contains_point(11.0, 5.0));
    assert!(!bbox.contains_point(5.0, -1.0));
    assert!(!bbox.contains_point(5.0, 11.0));
}

#[test]
fn test_bbox_contains_point_negative_coords() {
    let bbox = BoundingBox::new(-10.0, -10.0, 10.0, 10.0);
    assert!(bbox.contains_point(0.0, 0.0));
    assert!(bbox.contains_point(-5.0, -5.0));
    assert!(!bbox.contains_point(-15.0, 0.0));
}

// ============================================================================
// Cache key tests
// ============================================================================

#[test]
fn test_bbox_cache_key_format() {
    let bbox = BoundingBox::new(-125.0, 24.0, -66.0, 50.0);
    let key = bbox.cache_key();
    assert!(key.contains("-125"));
    assert!(key.contains("24"));
    assert!(key.contains("-66"));
    assert!(key.contains("50"));
}

#[test]
fn test_bbox_cache_key_deterministic() {
    let bbox = BoundingBox::new(1.0, 2.0, 3.0, 4.0);
    let key1 = bbox.cache_key();
    let key2 = bbox.cache_key();
    assert_eq!(key1, key2);
}

#[test]
fn test_bbox_cache_key_different_for_different_boxes() {
    let bbox1 = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let bbox2 = BoundingBox::new(0.0, 0.0, 10.0, 10.001);
    assert_ne!(bbox1.cache_key(), bbox2.cache_key());
}

#[test]
fn test_bbox_cache_key_precision() {
    // Cache key should handle floating point precision
    let bbox1 = BoundingBox::new(0.1 + 0.2, 0.0, 1.0, 1.0);
    let bbox2 = BoundingBox::new(0.3, 0.0, 1.0, 1.0);
    // After quantization, these should be the same
    assert_eq!(bbox1.cache_key(), bbox2.cache_key());
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_bbox_very_small() {
    let bbox = BoundingBox::new(0.0, 0.0, 1e-10, 1e-10);
    assert!(bbox.width() > 0.0);
    assert!(bbox.height() > 0.0);
}

#[test]
fn test_bbox_very_large() {
    let bbox = BoundingBox::new(-1e10, -1e10, 1e10, 1e10);
    assert_eq!(bbox.width(), 2e10);
    assert_eq!(bbox.height(), 2e10);
}

#[test]
fn test_bbox_inverted_does_not_panic() {
    // Inverted bbox (min > max) - constructor doesn't validate
    let bbox = BoundingBox::new(10.0, 10.0, 0.0, 0.0);
    // Width/height will be negative
    assert_eq!(bbox.width(), -10.0);
    assert_eq!(bbox.height(), -10.0);
}
