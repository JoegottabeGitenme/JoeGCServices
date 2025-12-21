/// Integration test for parsing MRMS GRIB2 files
mod common;

use bytes::Bytes;
use common::create_test_tables;
use grib2_parser::Grib2Reader;
use std::fs;

#[test]
fn test_mrms_data_distribution() {
    let path = require_test_file!("mrms_refl.grib2");

    let data = fs::read(&path).expect("Failed to read test file");
    let tables = create_test_tables();
    let mut reader = Grib2Reader::new(Bytes::from(data), tables);

    match reader.next_message() {
        Ok(Some(msg)) => {
            let (nj, ni) = msg.grid_dims();
            let values = msg.unpack_data().expect("Failed to unpack");

            println!("Grid: {}x{} = {} points", ni, nj, values.len());

            // Count value categories
            let mut missing_99 = 0;
            let mut missing_999 = 0;
            let mut negative = 0;
            let mut zero_to_10 = 0;
            let mut ten_to_30 = 0;
            let mut above_30 = 0;

            for &v in &values {
                if v <= -99.0 {
                    if v <= -900.0 {
                        missing_999 += 1;
                    } else {
                        missing_99 += 1;
                    }
                } else if v < 0.0 {
                    negative += 1;
                } else if v < 10.0 {
                    zero_to_10 += 1;
                } else if v < 30.0 {
                    ten_to_30 += 1;
                } else {
                    above_30 += 1;
                }
            }

            println!("\nValue distribution:");
            println!("  Missing (-999): {}", missing_999);
            println!("  Missing (-99): {}", missing_99);
            println!("  Negative (< 0 dBZ): {}", negative);
            println!("  0-10 dBZ: {}", zero_to_10);
            println!("  10-30 dBZ: {}", ten_to_30);
            println!("  30+ dBZ: {}", above_30);

            // Check a specific row for data pattern
            let mid_row = (nj / 2) as usize;
            let row_start = mid_row * ni as usize;
            let row_end = row_start + ni as usize;
            let row_data = &values[row_start..row_end];

            // Find runs of non-missing data
            let mut in_data = false;
            let mut run_start = 0;
            let mut runs = Vec::new();

            for (i, &v) in row_data.iter().enumerate() {
                let is_missing = v <= -90.0;
                if !is_missing && !in_data {
                    in_data = true;
                    run_start = i;
                } else if is_missing && in_data {
                    runs.push((run_start, i - 1));
                    in_data = false;
                }
            }
            if in_data {
                runs.push((run_start, row_data.len() - 1));
            }

            println!(
                "\nMiddle row ({}) data runs (non-missing regions):",
                mid_row
            );
            for (start, end) in &runs {
                let span = end - start + 1;
                // Convert to lon
                let lon_start = -129.995 + (*start as f64) * 0.01;
                let lon_end = -129.995 + (*end as f64) * 0.01;
                println!(
                    "  cols {}-{} ({} pts) = lon {:.2}° to {:.2}°",
                    start, end, span, lon_start, lon_end
                );
            }
        }
        Ok(None) => println!("No messages"),
        Err(e) => panic!("Error parsing MRMS file: {}", e),
    }
}

#[test]
fn test_parse_mrms_file() {
    let path = require_test_file!("mrms_refl.grib2");

    let data = fs::read(&path).expect("Failed to read test file");
    let data = Bytes::from(data);
    let tables = create_test_tables();

    let mut reader = Grib2Reader::new(data, tables);

    match reader.next_message() {
        Ok(Some(msg)) => {
            println!("=== MRMS GRIB2 Analysis ===");
            println!("Parameter: {}", msg.parameter());
            println!("Level: {}", msg.level());

            let gd = &msg.grid_definition;
            println!("\n=== Grid Definition (raw millidegrees) ===");
            println!("Grid shape: {}", gd.grid_shape);
            println!("Ni (columns): {}", gd.num_points_longitude);
            println!("Nj (rows): {}", gd.num_points_latitude);
            println!("First lat (mdeg): {}", gd.first_latitude_millidegrees);
            println!("First lon (mdeg): {}", gd.first_longitude_millidegrees);
            println!("Last lat (mdeg): {}", gd.last_latitude_millidegrees);
            println!("Last lon (mdeg): {}", gd.last_longitude_millidegrees);
            println!(
                "Lat increment (mdeg): {}",
                gd.latitude_increment_millidegrees
            );
            println!(
                "Lon increment (mdeg): {}",
                gd.longitude_increment_millidegrees
            );
            println!("Scan mode: {}", gd.scanning_mode);

            // Millidegrees to degrees: divide by 1000
            let first_lat = gd.first_latitude_millidegrees as f64 / 1000.0;
            let first_lon = gd.first_longitude_millidegrees as f64 / 1000.0;
            let last_lat = gd.last_latitude_millidegrees as f64 / 1000.0;
            let last_lon = gd.last_longitude_millidegrees as f64 / 1000.0;
            let lat_inc = gd.latitude_increment_millidegrees as f64 / 1000.0;
            let lon_inc = gd.longitude_increment_millidegrees as f64 / 1000.0;

            println!("\n=== In Degrees ===");
            println!("First lat: {:.6}°", first_lat);
            println!(
                "First lon: {:.6}° ({:.6}° when converted to -180/180)",
                first_lon,
                if first_lon > 180.0 {
                    first_lon - 360.0
                } else {
                    first_lon
                }
            );
            println!("Last lat: {:.6}°", last_lat);
            println!(
                "Last lon: {:.6}° ({:.6}° when converted to -180/180)",
                last_lon,
                if last_lon > 180.0 {
                    last_lon - 360.0
                } else {
                    last_lon
                }
            );
            println!("Lat step: {:.6}°", lat_inc);
            println!("Lon step: {:.6}°", lon_inc);

            // Data analysis
            let unpacked = msg.unpack_data().expect("Cannot unpack");
            let missing: Vec<_> = unpacked
                .iter()
                .filter(|&&v| v <= -90.0 || v > 90.0)
                .collect();
            let valid: Vec<_> = unpacked
                .iter()
                .filter(|&&v| v > -90.0 && v <= 90.0)
                .collect();

            println!("\n=== Data Analysis ===");
            println!("Total values: {}", unpacked.len());
            println!(
                "Missing (no-data): {} ({:.1}%)",
                missing.len(),
                100.0 * missing.len() as f64 / unpacked.len() as f64
            );
            println!(
                "Valid: {} ({:.1}%)",
                valid.len(),
                100.0 * valid.len() as f64 / unpacked.len() as f64
            );

            if !valid.is_empty() {
                let vmin = valid.iter().cloned().fold(f32::INFINITY, |a, &b| a.min(b));
                let vmax = valid
                    .iter()
                    .cloned()
                    .fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                println!("Valid range: {:.2} to {:.2} dBZ", vmin, vmax);
            }

            if !missing.is_empty() {
                use std::collections::HashSet;
                let unique_missing: HashSet<i32> = missing.iter().map(|&&v| v as i32).collect();
                println!(
                    "Missing markers: {:?}",
                    unique_missing.iter().take(5).collect::<Vec<_>>()
                );
            }
        }
        Ok(None) => println!("No messages"),
        Err(e) => panic!("Error parsing MRMS file: {}", e),
    }
}
