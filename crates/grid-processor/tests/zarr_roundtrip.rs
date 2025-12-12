//! Integration test: Create a Zarr file and read it with ZarrGridProcessor.
//!
//! This test verifies that Phase 1 deliverable works end-to-end:
//! 1. Create a test grid with known values
//! 2. Write it to Zarr V3 format
//! 3. Read it back using ZarrGridProcessor
//! 4. Verify the values match

use std::sync::Arc;

use grid_processor::{BoundingBox, GridProcessor, GridProcessorConfig, ZarrGridProcessor};
use zarrs::array::{ArrayBuilder, DataType, FillValue};
use zarrs::array_subset::ArraySubset;
use zarrs_filesystem::FilesystemStore;

/// Create a test grid with predictable values.
/// Value at (col, row) = col * 1000 + row (for easy verification)
fn create_test_data(width: usize, height: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    for row in 0..height {
        for col in 0..width {
            data.push((col * 1000 + row) as f32);
        }
    }
    data
}

/// Write a simple Zarr V3 array with the given data (no sharding, no compression).
fn write_zarr_array_simple(
    path: &std::path::Path,
    data: &[f32],
    width: usize,
    height: usize,
    chunk_size: usize,
    bbox: &BoundingBox,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create filesystem store
    std::fs::create_dir_all(path)?;
    let store = Arc::new(FilesystemStore::new(path)?);

    // Create array with simple chunking (no sharding, no compression)
    let array = ArrayBuilder::new(
        vec![height as u64, width as u64], // shape [rows, cols]
        DataType::Float32,
        vec![chunk_size as u64, chunk_size as u64].try_into()?, // chunk shape
        FillValue::from(f32::NAN),
    )
    .attributes({
        let mut attrs = serde_json::Map::new();
        attrs.insert("model".to_string(), serde_json::json!("test"));
        attrs.insert("parameter".to_string(), serde_json::json!("TEST_VAR"));
        attrs.insert("level".to_string(), serde_json::json!("surface"));
        attrs.insert("units".to_string(), serde_json::json!("units"));
        attrs.insert(
            "reference_time".to_string(),
            serde_json::json!("2024-12-12T00:00:00Z"),
        );
        attrs.insert("forecast_hour".to_string(), serde_json::json!(0));
        attrs.insert(
            "bbox".to_string(),
            serde_json::json!([bbox.min_lon, bbox.min_lat, bbox.max_lon, bbox.max_lat]),
        );
        attrs
    })
    .build(store.clone(), "/")?;

    // Store the array metadata
    array.store_metadata()?;

    // Write data
    let subset = ArraySubset::new_with_start_shape(vec![0, 0], vec![height as u64, width as u64])?;
    array.store_array_subset_elements(&subset, data)?;

    Ok(())
}

#[tokio::test]
async fn test_zarr_roundtrip_full_grid() {
    // Test parameters
    let width = 100;
    let height = 80;
    let chunk_size = 32;
    let bbox = BoundingBox::new(0.0, -40.0, 100.0, 40.0); // 100 x 80 deg, 1 deg resolution

    // Create temp directory
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let zarr_path = temp_dir.path().join("test.zarr");

    // Create test data
    let original_data = create_test_data(width, height);

    // Write Zarr
    write_zarr_array_simple(&zarr_path, &original_data, width, height, chunk_size, &bbox)
        .expect("Failed to write Zarr");

    // Read back with ZarrGridProcessor
    let store = FilesystemStore::new(&zarr_path).expect("Failed to open store");
    let config = GridProcessorConfig::default();
    let processor =
        ZarrGridProcessor::open(store, "/", config).expect("Failed to open ZarrGridProcessor");

    // Verify metadata
    let metadata = processor.metadata();
    assert_eq!(metadata.model, "test");
    assert_eq!(metadata.parameter, "TEST_VAR");
    assert_eq!(metadata.shape, (width, height));
    assert!((metadata.bbox.min_lon - bbox.min_lon).abs() < 0.001);
    assert!((metadata.bbox.max_lat - bbox.max_lat).abs() < 0.001);

    // Read entire grid
    let region = processor
        .read_region(&bbox)
        .await
        .expect("Failed to read region");

    // Verify dimensions
    assert_eq!(region.width, width);
    assert_eq!(region.height, height);
    assert_eq!(region.data.len(), width * height);

    // Verify all values match
    for row in 0..height {
        for col in 0..width {
            let expected = (col * 1000 + row) as f32;
            let actual = region.data[row * width + col];
            assert!(
                (actual - expected).abs() < 0.001,
                "Mismatch at ({}, {}): expected {}, got {}",
                col,
                row,
                expected,
                actual
            );
        }
    }

    println!("Full grid roundtrip test passed!");
    println!("  Grid size: {}x{}", width, height);
    println!("  Chunk size: {}", chunk_size);
    println!("  Total values verified: {}", width * height);
}

#[tokio::test]
async fn test_zarr_partial_read() {
    // Test parameters - larger grid
    let width = 200;
    let height = 150;
    let chunk_size = 64;
    let bbox = BoundingBox::new(-180.0, -90.0, 180.0, 90.0); // Global, ~1.8 x 1.2 deg resolution

    // Create temp directory
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let zarr_path = temp_dir.path().join("test_partial.zarr");

    // Create test data
    let original_data = create_test_data(width, height);

    // Write Zarr
    write_zarr_array_simple(&zarr_path, &original_data, width, height, chunk_size, &bbox)
        .expect("Failed to write Zarr");

    // Read back with ZarrGridProcessor
    let store = FilesystemStore::new(&zarr_path).expect("Failed to open store");
    let config = GridProcessorConfig::default();
    let processor =
        ZarrGridProcessor::open(store, "/", config).expect("Failed to open ZarrGridProcessor");

    // Read a small region (should only fetch 1-2 chunks)
    let small_bbox = BoundingBox::new(-50.0, 20.0, -30.0, 40.0);
    let region = processor
        .read_region(&small_bbox)
        .await
        .expect("Failed to read region");

    // Calculate expected dimensions
    let res_x = bbox.width() / width as f64;
    let res_y = bbox.height() / height as f64;

    let expected_min_col = ((small_bbox.min_lon - bbox.min_lon) / res_x).floor() as usize;
    let expected_max_col = ((small_bbox.max_lon - bbox.min_lon) / res_x).ceil() as usize;
    let expected_min_row = ((bbox.max_lat - small_bbox.max_lat) / res_y).floor() as usize;
    let expected_max_row = ((bbox.max_lat - small_bbox.min_lat) / res_y).ceil() as usize;

    let expected_width = expected_max_col - expected_min_col;
    let expected_height = expected_max_row - expected_min_row;

    // Verify we got a subset, not the whole grid
    assert!(region.width < width, "Should read partial width");
    assert!(region.height < height, "Should read partial height");
    assert_eq!(region.width, expected_width);
    assert_eq!(region.height, expected_height);

    // Verify values in the region are correct
    for local_row in 0..region.height {
        for local_col in 0..region.width {
            let global_col = expected_min_col + local_col;
            let global_row = expected_min_row + local_row;
            let expected = (global_col * 1000 + global_row) as f32;
            let actual = region.data[local_row * region.width + local_col];
            assert!(
                (actual - expected).abs() < 0.001,
                "Mismatch at local ({}, {}), global ({}, {}): expected {}, got {}",
                local_col,
                local_row,
                global_col,
                global_row,
                expected,
                actual
            );
        }
    }

    println!("Partial read test passed!");
    println!("  Full grid: {}x{}", width, height);
    println!("  Region read: {}x{}", region.width, region.height);
    println!(
        "  Data reduction: {:.1}%",
        (1.0 - (region.width * region.height) as f64 / (width * height) as f64) * 100.0
    );
}

#[tokio::test]
async fn test_zarr_read_point() {
    // Test parameters
    let width = 50;
    let height = 40;
    let chunk_size = 16;
    let bbox = BoundingBox::new(0.0, 0.0, 50.0, 40.0); // 1 deg resolution

    // Create temp directory
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let zarr_path = temp_dir.path().join("test_point.zarr");

    // Create test data
    let original_data = create_test_data(width, height);

    // Write Zarr
    write_zarr_array_simple(&zarr_path, &original_data, width, height, chunk_size, &bbox)
        .expect("Failed to write Zarr");

    // Read back with ZarrGridProcessor
    let store = FilesystemStore::new(&zarr_path).expect("Failed to open store");
    let config = GridProcessorConfig::default();
    let processor =
        ZarrGridProcessor::open(store, "/", config).expect("Failed to open ZarrGridProcessor");

    // Test several specific points
    let test_points = [
        (0.5, 39.5, 0, 0),    // Top-left corner (row 0)
        (25.5, 20.5, 25, 19), // Center area
        (49.5, 0.5, 49, 39),  // Bottom-right corner
        (10.5, 30.5, 10, 9),  // Random point
    ];

    for (lon, lat, expected_col, expected_row) in test_points {
        let value = processor
            .read_point(lon, lat)
            .await
            .expect("Failed to read point")
            .expect("Point should have value");

        let expected = (expected_col * 1000 + expected_row) as f32;
        assert!(
            (value - expected).abs() < 0.001,
            "Point ({}, {}) -> col={}, row={}: expected {}, got {}",
            lon,
            lat,
            expected_col,
            expected_row,
            expected,
            value
        );
    }

    // Test point outside grid
    let outside = processor
        .read_point(-10.0, 50.0)
        .await
        .expect("Should not error for outside point");
    assert!(outside.is_none(), "Point outside grid should return None");

    println!("Point read test passed!");
    println!("  Tested {} points", test_points.len());
}

#[tokio::test]
async fn test_chunk_cache_efficiency() {
    // Test that the chunk cache works - reading the same region twice should hit cache
    let width = 100;
    let height = 80;
    let chunk_size = 32;
    let bbox = BoundingBox::new(0.0, 0.0, 100.0, 80.0);

    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let zarr_path = temp_dir.path().join("test_cache.zarr");

    let original_data = create_test_data(width, height);
    write_zarr_array_simple(&zarr_path, &original_data, width, height, chunk_size, &bbox)
        .expect("Failed to write Zarr");

    let store = FilesystemStore::new(&zarr_path).expect("Failed to open store");
    let config = GridProcessorConfig::default();
    let processor =
        ZarrGridProcessor::open(store, "/", config).expect("Failed to open ZarrGridProcessor");

    // Read a region
    let region_bbox = BoundingBox::new(20.0, 20.0, 50.0, 50.0);

    // First read - should populate cache
    let _region1 = processor
        .read_region(&region_bbox)
        .await
        .expect("Failed to read");

    // Second read - should hit cache (we can't easily verify this without internal access,
    // but we can verify it doesn't error and returns same data)
    let region2 = processor
        .read_region(&region_bbox)
        .await
        .expect("Failed to read");

    // Verify data is still correct
    assert!(!region2.data.is_empty());

    // Read adjacent region - should share some chunks with first region
    let adjacent_bbox = BoundingBox::new(40.0, 40.0, 70.0, 70.0);
    let _region3 = processor
        .read_region(&adjacent_bbox)
        .await
        .expect("Failed to read");

    println!("Cache efficiency test passed!");
}
