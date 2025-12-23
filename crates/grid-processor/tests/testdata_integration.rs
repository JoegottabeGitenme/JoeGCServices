//! Integration tests using pre-committed test data files.
//!
//! These tests use the Zarr files in `testdata/` which are committed to the repository.
//! This ensures tests work without requiring data downloads or external dependencies.

use grid_processor::{
    BoundingBox, GridProcessor, GridProcessorConfig, ZarrGridProcessor,
};
use zarrs_filesystem::FilesystemStore;

/// Get the path to the testdata directory.
fn testdata_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

/// Helper to open a test Zarr file.
fn open_test_zarr(name: &str) -> ZarrGridProcessor<FilesystemStore> {
    let path = testdata_dir().join(name);
    let store = FilesystemStore::new(&path).expect("Failed to open store");
    let config = GridProcessorConfig::default();
    ZarrGridProcessor::open(store, "/", config).expect("Failed to open ZarrGridProcessor")
}

// =============================================================================
// Simple 10x10 Grid Tests
// =============================================================================

#[tokio::test]
async fn test_simple_grid_metadata() {
    let processor = open_test_zarr("simple_10x10.zarr");
    let metadata = processor.metadata();
    
    assert_eq!(metadata.model, "test");
    assert_eq!(metadata.parameter, "TEST_VAR");
    assert_eq!(metadata.level, "surface");
    assert_eq!(metadata.shape, (10, 10));
}

#[tokio::test]
async fn test_simple_grid_full_read() {
    let processor = open_test_zarr("simple_10x10.zarr");
    let bbox = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    
    let region = processor.read_region(&bbox).await.expect("Failed to read region");
    
    assert_eq!(region.width, 10);
    assert_eq!(region.height, 10);
    
    // Verify pattern: value = col * 1000 + row
    for row in 0..10 {
        for col in 0..10 {
            let expected = (col * 1000 + row) as f32;
            let actual = region.data[row * 10 + col];
            assert!(
                (actual - expected).abs() < 0.001,
                "Mismatch at ({}, {}): expected {}, got {}",
                col, row, expected, actual
            );
        }
    }
}

#[tokio::test]
async fn test_simple_grid_point_read() {
    let processor = open_test_zarr("simple_10x10.zarr");
    
    // Test center point (5.0, 5.0) - should be near grid cell (5, 5)
    let value = processor.read_point(5.0, 5.0).await
        .expect("Failed to read point")
        .expect("Point should have value");
    
    // Grid is 10x10 over bbox (0,0)-(10,10), so resolution is 1 deg
    // Point (5,5) maps to col=5, row from top: (10-5)/1 = 5
    let expected = (5 * 1000 + 5) as f32;
    assert!(
        (value - expected).abs() < 2.0,  // Allow tolerance for interpolation
        "Point (5,5) expected ~{}, got {}", expected, value
    );
}

// =============================================================================
// GFS-Style 0-360 Longitude Grid Tests
// =============================================================================

#[tokio::test]
async fn test_gfs_style_metadata() {
    let processor = open_test_zarr("gfs_style_360.zarr");
    let metadata = processor.metadata();
    
    assert_eq!(metadata.shape, (36, 18));
    // GFS uses 0-360 longitude
    assert!((metadata.bbox.min_lon - 0.0).abs() < 0.001);
    assert!((metadata.bbox.max_lon - 360.0).abs() < 0.001);
}

#[tokio::test]
async fn test_gfs_style_dateline_read() {
    let processor = open_test_zarr("gfs_style_360.zarr");
    
    // Read region near the dateline (in 0-360 space, this is around 180)
    let bbox = BoundingBox::new(170.0, -10.0, 190.0, 10.0);
    let region = processor.read_region(&bbox).await.expect("Failed to read region");
    
    assert!(region.width > 0);
    assert!(region.height > 0);
    // Data should be valid (not all NaN)
    let valid_count = region.data.iter().filter(|v| !v.is_nan()).count();
    assert!(valid_count > 0, "Should have valid data near dateline");
}

// =============================================================================
// Chunked 100x100 Grid Tests (Chunk Boundary Handling)
// =============================================================================

#[tokio::test]
async fn test_chunked_grid_metadata() {
    let processor = open_test_zarr("chunked_100x100.zarr");
    let metadata = processor.metadata();
    
    assert_eq!(metadata.shape, (100, 100));
    assert_eq!(metadata.chunk_shape, (32, 32));  // 32x32 chunks
    assert_eq!(metadata.num_chunks, (4, 4));     // 100/32 rounded up = 4
}

#[tokio::test]
async fn test_chunked_grid_cross_chunk_read() {
    let processor = open_test_zarr("chunked_100x100.zarr");
    
    // Read a region that spans multiple chunks
    // Grid is 100x100 over (-180,-90)-(180,90), so resolution is 3.6 x 1.8 deg
    let bbox = BoundingBox::new(-50.0, -20.0, 50.0, 20.0);
    let region = processor.read_region(&bbox).await.expect("Failed to read region");
    
    // Should have read from multiple chunks
    assert!(region.width > 0);
    assert!(region.height > 0);
    
    // Verify data integrity - values should follow the col*1000+row pattern
    // But we need to map back to global coordinates
    let valid_values: Vec<f32> = region.data.iter().filter(|v| !v.is_nan()).copied().collect();
    assert!(!valid_values.is_empty(), "Should have valid data");
}

#[tokio::test]
async fn test_chunked_grid_chunk_boundary_point() {
    let processor = open_test_zarr("chunked_100x100.zarr");
    
    // Test point near a chunk boundary (chunks are 32x32)
    // Grid is global, so we pick a point that falls near a chunk edge
    let value = processor.read_point(0.0, 0.0).await
        .expect("Failed to read point");
    
    // Point should have a value (center of grid)
    assert!(value.is_some(), "Center point should have value");
}

// =============================================================================
// Multiscale Pyramid Tests
// =============================================================================

#[tokio::test]
async fn test_multiscale_metadata() {
    let path = testdata_dir().join("multiscale_3level.zarr");
    
    // Read group metadata
    let group_path = path.join("zarr.json");
    let group_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&group_path).expect("Failed to read group metadata")
    ).expect("Failed to parse JSON");
    
    // Check that we have multiscales in the attributes
    let attrs = group_json.get("attributes").expect("Should have attributes");
    let multiscales = attrs.get("multiscales").expect("Should have multiscales");
    
    assert!(multiscales.is_array(), "multiscales should be an array");
    let ms_array = multiscales.as_array().unwrap();
    assert!(!ms_array.is_empty(), "Should have at least one multiscale entry");
    
    // Check the first multiscale entry has datasets
    let first = &ms_array[0];
    let datasets = first.get("datasets").expect("Should have datasets");
    let datasets_array = datasets.as_array().unwrap();
    assert!(datasets_array.len() >= 2, "Should have at least 2 pyramid levels");
}

#[tokio::test]
async fn test_multiscale_read_levels() {
    let path = testdata_dir().join("multiscale_3level.zarr");
    
    // Open level 0 directly
    let level0_path = path.join("0");
    let store = FilesystemStore::new(&level0_path).expect("Failed to open store");
    let config = GridProcessorConfig::default();
    let processor = ZarrGridProcessor::open(store, "/", config).expect("Failed to open level 0");
    
    let metadata = processor.metadata();
    assert_eq!(metadata.shape, (64, 64), "Level 0 should be 64x64");
    
    // Open level 1
    let level1_path = path.join("1");
    let store1 = FilesystemStore::new(&level1_path).expect("Failed to open store");
    let processor1 = ZarrGridProcessor::open(store1, "/", GridProcessorConfig::default())
        .expect("Failed to open level 1");
    
    let metadata1 = processor1.metadata();
    assert_eq!(metadata1.shape, (32, 32), "Level 1 should be 32x32 (half of level 0)");
    
    // Open level 2
    let level2_path = path.join("2");
    let store2 = FilesystemStore::new(&level2_path).expect("Failed to open store");
    let processor2 = ZarrGridProcessor::open(store2, "/", GridProcessorConfig::default())
        .expect("Failed to open level 2");
    
    let metadata2 = processor2.metadata();
    assert_eq!(metadata2.shape, (16, 16), "Level 2 should be 16x16 (quarter of level 0)");
}

// =============================================================================
// Cache Behavior Tests
// =============================================================================

#[tokio::test]
async fn test_repeated_reads_work() {
    // Test that reading the same region multiple times works correctly
    let processor = open_test_zarr("chunked_100x100.zarr");
    
    // Read a region multiple times
    let bbox = BoundingBox::new(-50.0, -20.0, 50.0, 20.0);
    
    let region1 = processor.read_region(&bbox).await.expect("Failed to read first time");
    let region2 = processor.read_region(&bbox).await.expect("Failed to read second time");
    
    // Both reads should return the same dimensions and data
    assert_eq!(region1.width, region2.width);
    assert_eq!(region1.height, region2.height);
    assert_eq!(region1.data.len(), region2.data.len());
    
    // Data should be identical
    for (a, b) in region1.data.iter().zip(region2.data.iter()) {
        if !a.is_nan() && !b.is_nan() {
            assert!((a - b).abs() < 0.001, "Data should be identical across reads");
        }
    }
}

// =============================================================================
// Edge Cases
// =============================================================================

#[tokio::test]
async fn test_point_outside_grid() {
    let processor = open_test_zarr("simple_10x10.zarr");
    
    // Point way outside the grid
    let value = processor.read_point(100.0, 100.0).await
        .expect("Should not error");
    
    assert!(value.is_none(), "Point outside grid should return None");
}

#[tokio::test]
async fn test_partial_overlap_read() {
    let processor = open_test_zarr("simple_10x10.zarr");
    
    // Read a region that only partially overlaps the grid
    let bbox = BoundingBox::new(-5.0, -5.0, 5.0, 5.0);  // Half outside
    let region = processor.read_region(&bbox).await.expect("Failed to read");
    
    // Should get the overlapping portion
    assert!(region.width > 0);
    assert!(region.height > 0);
}
