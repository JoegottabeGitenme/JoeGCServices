//! Integration test for parsing NDFD files.

use bytes::Bytes;
use grib2_parser::{strip_wmo_headers, Grib2Reader, Grib2Tables, NdfdReader};
use std::sync::Arc;

/// Test parsing a real NDFD file if it exists.
#[test]
fn test_parse_ndfd_file() {
    let path =
        std::env::var("NDFD_TEST_FILE").unwrap_or_else(|_| "data/ndfd/ds.temp.bin".to_string());

    // Skip if file doesn't exist
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(_) => {
            eprintln!("Skipping test: {} not found", path);
            return;
        }
    };

    println!("File size: {} bytes", data.len());

    // Check if it's NDFD format
    assert!(data.len() >= 4, "File too small");
    let is_ndfd = &data[0..4] == b"****";
    println!("NDFD format detected: {}", is_ndfd);

    // Strip WMO headers
    let stripped = strip_wmo_headers(&data);
    println!("After stripping: {} bytes", stripped.len());
    assert!(!stripped.is_empty(), "Stripped data should not be empty");

    // Count messages using NdfdReader
    let reader = NdfdReader::new(Bytes::from(data.clone()));
    let msg_count: usize = reader.count();
    println!("GRIB2 messages in file: {}", msg_count);
    assert!(msg_count > 0, "Should have at least one GRIB2 message");

    // Parse with Grib2Reader
    let tables = Arc::new(Grib2Tables::new());
    let mut grib_reader = Grib2Reader::new(Bytes::from(stripped), tables);

    // Parse first message
    let msg = grib_reader
        .next_message()
        .expect("Should have a message")
        .expect("Should parse successfully");

    println!("First message parameter: {}", msg.parameter());
    println!("First message level: {}", msg.level());

    // Check grid definition - NDFD is 2145 x 1377 (Lambert Conformal)
    let grid = &msg.grid_definition;
    println!(
        "Grid: {} x {} points",
        grid.num_points_longitude, grid.num_points_latitude
    );

    // NDFD CONUS is 2145 x 1377
    // Note: nx = num_points_longitude, ny = num_points_latitude
    assert_eq!(
        grid.num_points_longitude, 2145,
        "NDFD CONUS should be 2145 x-points"
    );
    assert_eq!(
        grid.num_points_latitude, 1377,
        "NDFD CONUS should be 1377 y-points"
    );

    println!("NDFD parsing test passed!");
}
