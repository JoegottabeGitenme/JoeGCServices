//! Test data generation utilities.
//!
//! This module provides utilities for creating test Zarr files with known values
//! for use in unit and integration tests. The test files are small (10x10 to 100x100)
//! to keep the repository size manageable.
//!
//! # Test Data Files
//!
//! Pre-generated test files are stored in `testdata/` and committed to the repository:
//!
//! | File | Description | Size |
//! |------|-------------|------|
//! | `simple_10x10.zarr/` | Basic 10x10 grid, values = col*1000 + row | ~1KB |
//! | `gfs_style_360.zarr/` | 36x18 grid with 0-360 longitude | ~2KB |
//! | `multiscale_3level.zarr/` | 64x64 grid with 3 pyramid levels | ~10KB |
//! | `chunked_100x100.zarr/` | 100x100 grid with 32x32 chunks | ~50KB |

use std::path::Path;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use zarrs::array::{ArrayBuilder, DataType, FillValue};
use zarrs::array_subset::ArraySubset;
use zarrs_filesystem::FilesystemStore;

use crate::config::{GridProcessorConfig, PyramidConfig};
use crate::downsample::DownsampleMethod;
use crate::types::BoundingBox;
use crate::writer::{ZarrMetadata, ZarrWriter};

/// Create test grid data where value at (col, row) = col * 1000 + row.
/// This pattern makes it easy to verify data integrity after reads.
pub fn create_test_grid(width: usize, height: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    for row in 0..height {
        for col in 0..width {
            data.push((col * 1000 + row) as f32);
        }
    }
    data
}

/// Create test grid data with a temperature-like pattern.
/// Values vary smoothly from 250K (cold) to 310K (hot) based on position.
pub fn create_temperature_grid(width: usize, height: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    for row in 0..height {
        // Simulate latitude-based temperature (colder at edges, warmer in middle)
        let lat_factor = 1.0 - (2.0 * row as f32 / height as f32 - 1.0).abs();
        let temp = 250.0 + lat_factor * 60.0; // 250K to 310K
        for _col in 0..width {
            data.push(temp);
        }
    }
    data
}

/// Create a simple Zarr array with test data (no pyramids, no sharding).
///
/// # Arguments
/// * `path` - Directory to create the Zarr array in
/// * `width` - Grid width
/// * `height` - Grid height  
/// * `chunk_size` - Chunk dimensions (both x and y)
/// * `bbox` - Geographic bounding box
///
/// # Returns
/// The data that was written (for verification in tests)
pub fn write_simple_zarr(
    path: &Path,
    width: usize,
    height: usize,
    chunk_size: usize,
    bbox: &BoundingBox,
) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let data = create_test_grid(width, height);
    write_zarr_with_data(path, &data, width, height, chunk_size, bbox)?;
    Ok(data)
}

/// Write a Zarr array with provided data.
pub fn write_zarr_with_data(
    path: &Path,
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
        attrs.insert("model".to_string(), serde_json::json!("test"));
        attrs.insert("parameter".to_string(), serde_json::json!("TEST_VAR"));
        attrs.insert("level".to_string(), serde_json::json!("surface"));
        attrs.insert("units".to_string(), serde_json::json!("units"));
        attrs.insert(
            "reference_time".to_string(),
            serde_json::json!("2024-12-22T00:00:00Z"),
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

/// Write a Zarr array with multi-resolution pyramids.
pub fn write_multiscale_zarr(
    path: &Path,
    width: usize,
    height: usize,
    chunk_size: usize,
    bbox: &BoundingBox,
    min_dimension: usize,
) -> Result<ZarrMetadata, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(path)?;
    let data = create_test_grid(width, height);

    let config = GridProcessorConfig {
        zarr_chunk_size: chunk_size,
        ..Default::default()
    };

    let pyramid_config = PyramidConfig {
        enabled: true,
        min_dimension,
        downscale_factor: 2,
        default_method: DownsampleMethod::Mean,
    };

    let store = Arc::new(FilesystemStore::new(path)?);
    let writer = ZarrWriter::new(config);

    let reference_time = Utc.with_ymd_and_hms(2024, 12, 22, 0, 0, 0).unwrap();

    let result = writer.write_multiscale(
        store,
        "/",
        &data,
        width,
        height,
        bbox,
        "test",
        "TEST_VAR",
        "surface",
        "units",
        reference_time,
        0,
        &pyramid_config,
        DownsampleMethod::Mean,
    )?;

    Ok(result.zarr_metadata)
}

/// Get the path to the testdata directory.
pub fn testdata_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

/// Get the path to a specific test Zarr file.
pub fn testdata_path(name: &str) -> std::path::PathBuf {
    testdata_dir().join(name)
}

/// Generate all test data files (run manually to regenerate).
///
/// This is called by `cargo test -- --ignored generate_testdata` or via
/// the script `scripts/generate_testdata.sh`.
#[cfg(test)]
pub fn generate_all_testdata() -> Result<(), Box<dyn std::error::Error>> {
    let testdata = testdata_dir();
    std::fs::create_dir_all(&testdata)?;

    // 1. Simple 10x10 grid
    let simple_path = testdata.join("simple_10x10.zarr");
    if simple_path.exists() {
        std::fs::remove_dir_all(&simple_path)?;
    }
    write_simple_zarr(
        &simple_path,
        10,
        10,
        8,
        &BoundingBox::new(0.0, 0.0, 10.0, 10.0),
    )?;
    println!("Created: simple_10x10.zarr");

    // 2. GFS-style grid with 0-360 longitude
    let gfs_path = testdata.join("gfs_style_360.zarr");
    if gfs_path.exists() {
        std::fs::remove_dir_all(&gfs_path)?;
    }
    write_simple_zarr(
        &gfs_path,
        36,
        18,
        16,
        &BoundingBox::new(0.0, -90.0, 360.0, 90.0),
    )?;
    println!("Created: gfs_style_360.zarr");

    // 3. Multiscale pyramid (multiple levels based on min_dimension=16)
    let multiscale_path = testdata.join("multiscale_3level.zarr");
    if multiscale_path.exists() {
        std::fs::remove_dir_all(&multiscale_path)?;
    }
    write_multiscale_zarr(
        &multiscale_path,
        64,
        64,
        32,
        &BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
        16, // min_dimension - will generate levels until dimension < 16
    )?;
    println!("Created: multiscale_3level.zarr");

    // 4. Chunked 100x100 grid (tests chunk boundary handling)
    let chunked_path = testdata.join("chunked_100x100.zarr");
    if chunked_path.exists() {
        std::fs::remove_dir_all(&chunked_path)?;
    }
    write_simple_zarr(
        &chunked_path,
        100,
        100,
        32,
        &BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
    )?;
    println!("Created: chunked_100x100.zarr");

    println!("\nAll test data files generated in: {:?}", testdata);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_test_grid() {
        let grid = create_test_grid(5, 4);
        assert_eq!(grid.len(), 20);

        // Check pattern: value = col * 1000 + row
        assert_eq!(grid[0], 0.0); // (0, 0)
        assert_eq!(grid[1], 1000.0); // (1, 0)
        assert_eq!(grid[5], 1.0); // (0, 1)
        assert_eq!(grid[6], 1001.0); // (1, 1)
    }

    #[test]
    fn test_create_temperature_grid() {
        let grid = create_temperature_grid(10, 10);
        assert_eq!(grid.len(), 100);

        // All values should be between 250K and 310K
        for val in &grid {
            assert!(
                *val >= 250.0 && *val <= 310.0,
                "Temperature out of range: {}",
                val
            );
        }
    }

    /// This test generates all test data files.
    /// Run with: cargo test --package grid-processor generate_testdata -- --ignored
    #[test]
    #[ignore]
    fn generate_testdata() {
        generate_all_testdata().expect("Failed to generate test data");
    }
}
