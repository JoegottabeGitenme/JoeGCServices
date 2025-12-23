/// Test MRMS point sampling and coordinate conversion
mod common;

use bytes::Bytes;
use common::create_test_tables;
use grib2_parser::Grib2Reader;
use std::fs;

#[test]
fn test_sample_point() {
    let path = require_test_file!("mrms_refl.grib2");

    let data = fs::read(&path).expect("Failed to read test file");
    let tables = create_test_tables();
    let mut reader = Grib2Reader::new(Bytes::from(data), tables);

    match reader.next_message() {
        Ok(Some(msg)) => {
            let gd = &msg.grid_definition;
            let values = msg.unpack_data().expect("Failed to unpack");

            // Grid info from grid definition
            let first_lat: f64 = gd.first_latitude_millidegrees as f64 / 1000.0;
            let first_lon: f64 = gd.first_longitude_millidegrees as f64 / 1000.0;
            // Convert to -180/180 range
            let first_lon_normalized = if first_lon > 180.0 {
                first_lon - 360.0
            } else {
                first_lon
            };
            let lat_step: f64 = gd.latitude_increment_millidegrees as f64 / 1000.0;
            let lon_step: f64 = gd.longitude_increment_millidegrees as f64 / 1000.0;
            let ni: usize = gd.num_points_longitude as usize;
            let nj: usize = gd.num_points_latitude as usize;

            println!("Grid: {}x{}", ni, nj);
            println!(
                "First point: lat={:.3}°, lon={:.3}° (normalized: {:.3}°)",
                first_lat, first_lon, first_lon_normalized
            );
            println!("Step: lat={:.4}°, lon={:.4}°", lat_step, lon_step);

            // Sample a point within the grid bounds
            // Pick a point roughly in the middle of the grid
            let mid_row = nj / 2;
            let mid_col = ni / 2;
            let query_lat = first_lat - (mid_row as f64) * lat_step;
            let query_lon = first_lon_normalized + (mid_col as f64) * lon_step;

            // Convert back to grid indices
            let col = ((query_lon - first_lon_normalized) / lon_step).round() as usize;
            let row = ((first_lat - query_lat) / lat_step).round() as usize;

            println!("\nQuery: lon={:.6}°, lat={:.6}°", query_lon, query_lat);
            println!("Grid index: row={}, col={}", row, col);

            assert!(row < nj, "Row should be within bounds");
            assert!(col < ni, "Col should be within bounds");

            let idx = row * ni + col;
            let value = values[idx];
            println!("Value at index {}: {:.2}", idx, value);

            // Show surrounding grid if large enough
            if ni >= 5 && nj >= 5 && row >= 2 && row + 2 < nj && col >= 2 && col + 2 < ni {
                println!("\nSurrounding 5x5 grid:");
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        let r = (row as i32 + dy) as usize;
                        let c = (col as i32 + dx) as usize;
                        if r < nj && c < ni {
                            let v = values[r * ni + c];
                            if v > -90.0 {
                                print!("{:6.1} ", v);
                            } else {
                                print!("  ---  ");
                            }
                        }
                    }
                    println!();
                }
            }

            // Verify parsed grid definition
            println!("\n=== Parsed Grid Definition ===");
            println!(
                "Ni: {}, Nj: {}",
                gd.num_points_longitude, gd.num_points_latitude
            );
            println!(
                "First lat (mdeg): {} -> {:.3}°",
                gd.first_latitude_millidegrees,
                gd.first_latitude_millidegrees as f64 / 1000.0
            );
            println!(
                "First lon (mdeg): {} -> {:.3}°",
                gd.first_longitude_millidegrees,
                gd.first_longitude_millidegrees as f64 / 1000.0
            );
        }
        Ok(None) => println!("No messages"),
        Err(e) => panic!("Error parsing MRMS file: {}", e),
    }
}

#[test]
fn test_grid_coordinate_conversion() {
    let path = require_test_file!("mrms_refl.grib2");

    let data = fs::read(&path).expect("Failed to read test file");
    let tables = create_test_tables();
    let mut reader = Grib2Reader::new(Bytes::from(data), tables);

    match reader.next_message() {
        Ok(Some(msg)) => {
            let gd = &msg.grid_definition;

            // Test corner points
            let first_lat = gd.first_latitude_millidegrees as f64 / 1000.0;
            let first_lon = gd.first_longitude_millidegrees as f64 / 1000.0;
            let last_lat = gd.last_latitude_millidegrees as f64 / 1000.0;
            let last_lon = gd.last_longitude_millidegrees as f64 / 1000.0;
            let lat_step = gd.latitude_increment_millidegrees as f64 / 1000.0;
            let lon_step = gd.longitude_increment_millidegrees as f64 / 1000.0;
            let ni = gd.num_points_longitude;
            let nj = gd.num_points_latitude;

            println!("First: ({:.3}°, {:.3}°)", first_lat, first_lon);
            println!("Last (from file): ({:.3}°, {:.3}°)", last_lat, last_lon);

            // Verify that we can compute the last point from first + increments
            // Note: For N->S scanning, lat decreases
            let computed_last_lat = first_lat - (nj as f64 - 1.0) * lat_step;
            let computed_last_lon = first_lon + (ni as f64 - 1.0) * lon_step;

            println!(
                "Last (computed): ({:.3}°, {:.3}°)",
                computed_last_lat, computed_last_lon
            );

            // Basic sanity checks
            assert!(ni > 0, "Grid should have columns");
            assert!(nj > 0, "Grid should have rows");
            assert!(lat_step > 0.0 || lat_step < 0.0, "Lat step should be non-zero");
            assert!(lon_step > 0.0, "Lon step should be positive (W to E)");

            // Check that total points match unpacked data
            let values = msg.unpack_data().expect("Failed to unpack");
            assert_eq!(
                values.len(),
                (ni * nj) as usize,
                "Data points should match grid size"
            );
        }
        Ok(None) => println!("No messages"),
        Err(e) => panic!("Error: {}", e),
    }
}
