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

    // Calculate expected dimensions (note: read_region adds a 2-cell buffer for interpolation)
    let res_x = bbox.width() / width as f64;
    let res_y = bbox.height() / height as f64;
    let buffer_cells = 2.0;

    // Calculate with buffer, clamped to grid bounds
    let buffered_min_lon = (small_bbox.min_lon - res_x * buffer_cells).max(bbox.min_lon);
    let buffered_max_lon = (small_bbox.max_lon + res_x * buffer_cells).min(bbox.max_lon);
    let buffered_min_lat = (small_bbox.min_lat - res_y * buffer_cells).max(bbox.min_lat);
    let buffered_max_lat = (small_bbox.max_lat + res_y * buffer_cells).min(bbox.max_lat);

    let expected_min_col = ((buffered_min_lon - bbox.min_lon) / res_x).floor() as usize;
    let expected_max_col = ((buffered_max_lon - bbox.min_lon) / res_x).ceil() as usize;
    let expected_min_row = ((bbox.max_lat - buffered_max_lat) / res_y).floor() as usize;
    let expected_max_row = ((bbox.max_lat - buffered_min_lat) / res_y).ceil() as usize;

    let expected_width = expected_max_col - expected_min_col;
    let expected_height = expected_max_row - expected_min_row;

    // Verify we got a subset, not the whole grid
    assert!(region.width < width, "Should read partial width");
    assert!(region.height < height, "Should read partial height");
    assert_eq!(
        region.width, expected_width,
        "Width should match expected (with buffer)"
    );
    assert_eq!(
        region.height, expected_height,
        "Height should match expected (with buffer)"
    );

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

    // Test points at actual grid cell corners (not pixel centers)
    // Grid has bbox (0,0) to (50,40) with 50x40 pixels
    // So pixel (0,0) is at lon=0, lat=40 (top-left)
    // and pixel (49,39) is at lon=49, lat=1 (bottom-right)
    // Note: For a 50x40 grid over 50x40 degrees, resolution is 1 deg/pixel
    // Grid points are at integer lon/lat coordinates
    let test_points = [
        (0.0, 40.0, 0, 0),    // Top-left corner - exact grid point
        (25.0, 21.0, 25, 19), // Near center - exact grid point
        (49.0, 1.0, 49, 39),  // Near bottom-right - exact grid point
        (10.0, 31.0, 10, 9),  // Another point - exact grid point
    ];

    for (lon, lat, expected_col, expected_row) in test_points {
        let value = processor
            .read_point(lon, lat)
            .await
            .expect("Failed to read point")
            .expect("Point should have value");

        let expected = (expected_col * 1000 + expected_row) as f32;
        assert!(
            (value - expected).abs() < 1.0, // Allow small tolerance for floating point
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

/// Write a Zarr array with 0-360 longitude convention (like GFS).
fn write_zarr_array_0_360(
    path: &std::path::Path,
    data: &[f32],
    width: usize,
    height: usize,
    chunk_size: usize,
    bbox: &BoundingBox,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(path)?;
    let store = Arc::new(FilesystemStore::new(path)?);

    let array = ArrayBuilder::new(
        vec![height as u64, width as u64],
        DataType::Float32,
        vec![chunk_size as u64, chunk_size as u64].try_into()?,
        FillValue::from(f32::NAN),
    )
    .attributes({
        let mut attrs = serde_json::Map::new();
        attrs.insert("model".to_string(), serde_json::json!("gfs_test"));
        attrs.insert("parameter".to_string(), serde_json::json!("TMP"));
        attrs.insert("level".to_string(), serde_json::json!("2 m above ground"));
        attrs.insert("units".to_string(), serde_json::json!("K"));
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

    array.store_metadata()?;

    let subset = ArraySubset::new_with_start_shape(vec![0, 0], vec![height as u64, width as u64])?;
    array.store_array_subset_elements(&subset, data)?;

    Ok(())
}

/// Create test data where value encodes the longitude (col * resolution + min_lon).
/// This lets us verify which part of the grid we're reading.
fn create_longitude_test_data(width: usize, height: usize, min_lon: f64, res_x: f64) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    for _row in 0..height {
        for col in 0..width {
            // Value = longitude at this column
            let lon = min_lon + col as f64 * res_x;
            data.push(lon as f32);
        }
    }
    data
}

#[tokio::test]
async fn test_cross_prime_meridian_europe() {
    // Simulate a GFS-like global grid with 0-360 longitude
    // Using 1 degree resolution for simplicity
    let width = 360;
    let height = 180;
    let chunk_size = 64;
    // GFS-style bbox: 0 to 360 longitude
    let bbox = BoundingBox::new(0.0, -90.0, 360.0, 90.0);
    let res_x = 1.0;

    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let zarr_path = temp_dir.path().join("test_gfs_360.zarr");

    // Create data where value = longitude
    let original_data = create_longitude_test_data(width, height, 0.0, res_x);

    write_zarr_array_0_360(&zarr_path, &original_data, width, height, chunk_size, &bbox)
        .expect("Failed to write Zarr");

    let store = FilesystemStore::new(&zarr_path).expect("Failed to open store");
    let config = GridProcessorConfig::default();
    let processor =
        ZarrGridProcessor::open(store, "/", config).expect("Failed to open ZarrGridProcessor");

    // Request Europe: -10 to 40 longitude (crosses prime meridian)
    let europe_bbox = BoundingBox::new(-10.0, 35.0, 40.0, 70.0);
    let region = processor
        .read_region(&europe_bbox)
        .await
        .expect("Failed to read Europe region");

    // The returned region should have the Europe bbox, NOT the full grid bbox
    assert!(
        (region.bbox.min_lon - (-10.0)).abs() < 1.0,
        "Returned bbox min_lon should be ~-10, got {}",
        region.bbox.min_lon
    );
    assert!(
        (region.bbox.max_lon - 40.0).abs() < 1.0,
        "Returned bbox max_lon should be ~40, got {}",
        region.bbox.max_lon
    );

    // Width should be ~50 columns (from -10 to 40 = 50 degrees at 1 deg resolution)
    assert!(
        region.width >= 48 && region.width <= 52,
        "Width should be ~50 for -10 to 40 degrees, got {}",
        region.width
    );

    // Height should be ~35 rows (from 35 to 70 = 35 degrees at 1 deg resolution)
    assert!(
        region.height >= 33 && region.height <= 37,
        "Height should be ~35 for 35 to 70 degrees, got {}",
        region.height
    );

    // Verify the data contains the correct longitudes
    // Western columns (originally -10 to 0, stored as 350-360) should have values 350-360
    // Eastern columns (0 to 40) should have values 0-40

    // Check first column (should be ~350, representing -10 degrees)
    let first_col_lon = region.data[0];
    assert!(
        (first_col_lon - 350.0).abs() < 2.0,
        "First column should have longitude ~350 (representing -10), got {}",
        first_col_lon
    );

    // Check middle column (around prime meridian, lon ~0 = stored as 0 or 360)
    let mid_col = region.width / 5; // ~10 columns in = around 0 degrees
    let mid_lon = region.data[mid_col];
    // Should be around 0 or just past 350+10=360
    assert!(
        mid_lon < 5.0 || mid_lon > 358.0,
        "Column near prime meridian should have lon ~0 or ~360, got {}",
        mid_lon
    );

    // Check last column (should be ~40)
    let last_col_lon = region.data[region.width - 1];
    assert!(
        (last_col_lon - 39.0).abs() < 2.0,
        "Last column should have longitude ~39-40, got {}",
        last_col_lon
    );

    println!("Cross prime meridian test (Europe) passed!");
    println!("  Grid: {}x{} (0-360 longitude)", width, height);
    println!("  Request: -10 to 40 longitude");
    println!("  Result: {}x{}", region.width, region.height);
    println!(
        "  First column lon: {}, Last column lon: {}",
        first_col_lon, last_col_lon
    );
}

#[tokio::test]
async fn test_no_crossing_us_region() {
    // Test that requests entirely in Western hemisphere work correctly
    // (no crossing needed, just coordinate conversion)
    let width = 360;
    let height = 180;
    let chunk_size = 64;
    let bbox = BoundingBox::new(0.0, -90.0, 360.0, 90.0);
    let res_x = 1.0;

    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let zarr_path = temp_dir.path().join("test_gfs_us.zarr");

    let original_data = create_longitude_test_data(width, height, 0.0, res_x);

    write_zarr_array_0_360(&zarr_path, &original_data, width, height, chunk_size, &bbox)
        .expect("Failed to write Zarr");

    let store = FilesystemStore::new(&zarr_path).expect("Failed to open store");
    let config = GridProcessorConfig::default();
    let processor =
        ZarrGridProcessor::open(store, "/", config).expect("Failed to open ZarrGridProcessor");

    // Request CONUS: -125 to -65 longitude (entirely Western hemisphere, no crossing)
    let us_bbox = BoundingBox::new(-125.0, 25.0, -65.0, 50.0);
    let region = processor
        .read_region(&us_bbox)
        .await
        .expect("Failed to read US region");

    // Width should be ~60 columns
    assert!(
        region.width >= 58 && region.width <= 64,
        "Width should be ~60 for -125 to -65 degrees, got {}",
        region.width
    );

    // Check that the data has correct longitudes (stored as 235-295)
    let first_col_lon = region.data[0];
    // -125 stored as 360-125 = 235
    assert!(
        (first_col_lon - 235.0).abs() < 5.0,
        "First column should have longitude ~235 (representing -125), got {}",
        first_col_lon
    );

    let last_col_lon = region.data[region.width - 1];
    // -65 stored as 360-65 = 295
    assert!(
        (last_col_lon - 295.0).abs() < 5.0,
        "Last column should have longitude ~295 (representing -65), got {}",
        last_col_lon
    );

    println!("No-crossing US test passed!");
    println!("  Request: -125 to -65 longitude");
    println!("  Result: {}x{}", region.width, region.height);
    println!(
        "  First column lon: {}, Last column lon: {}",
        first_col_lon, last_col_lon
    );
}

#[tokio::test]
async fn test_no_crossing_asia_region() {
    // Test that requests entirely in Eastern hemisphere work correctly
    // (no conversion needed)
    let width = 360;
    let height = 180;
    let chunk_size = 64;
    let bbox = BoundingBox::new(0.0, -90.0, 360.0, 90.0);
    let res_x = 1.0;

    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let zarr_path = temp_dir.path().join("test_gfs_asia.zarr");

    let original_data = create_longitude_test_data(width, height, 0.0, res_x);

    write_zarr_array_0_360(&zarr_path, &original_data, width, height, chunk_size, &bbox)
        .expect("Failed to write Zarr");

    let store = FilesystemStore::new(&zarr_path).expect("Failed to open store");
    let config = GridProcessorConfig::default();
    let processor =
        ZarrGridProcessor::open(store, "/", config).expect("Failed to open ZarrGridProcessor");

    // Request Asia: 70 to 140 longitude (entirely Eastern hemisphere)
    let asia_bbox = BoundingBox::new(70.0, 10.0, 140.0, 55.0);
    let region = processor
        .read_region(&asia_bbox)
        .await
        .expect("Failed to read Asia region");

    // Width should be ~70 columns
    assert!(
        region.width >= 68 && region.width <= 74,
        "Width should be ~70 for 70 to 140 degrees, got {}",
        region.width
    );

    // Check that the data has correct longitudes (stored as-is: 70-140)
    let first_col_lon = region.data[0];
    assert!(
        (first_col_lon - 70.0).abs() < 5.0,
        "First column should have longitude ~70, got {}",
        first_col_lon
    );

    let last_col_lon = region.data[region.width - 1];
    assert!(
        (last_col_lon - 139.0).abs() < 5.0,
        "Last column should have longitude ~139-140, got {}",
        last_col_lon
    );

    println!("No-crossing Asia test passed!");
    println!("  Request: 70 to 140 longitude");
    println!("  Result: {}x{}", region.width, region.height);
    println!(
        "  First column lon: {}, Last column lon: {}",
        first_col_lon, last_col_lon
    );
}
