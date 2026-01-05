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

    // Print scanning mode info for debugging
    println!(
        "\nScanning mode: 0b{:08b} ({})",
        grid.scanning_mode, grid.scanning_mode
    );
    println!(
        "  Bit 1 (i direction): {}",
        if grid.scanning_mode & 0b10000000 != 0 {
            "-i (east to west)"
        } else {
            "+i (west to east)"
        }
    );
    println!(
        "  Bit 2 (j direction): {}",
        if grid.scanning_mode & 0b01000000 != 0 {
            "+j (south to north)"
        } else {
            "-j (north to south)"
        }
    );
    println!(
        "  Bit 3 (consecutive): {}",
        if grid.scanning_mode & 0b00100000 != 0 {
            "j consecutive (columns)"
        } else {
            "i consecutive (rows)"
        }
    );

    println!(
        "\nFirst grid point: lat={} mdeg, lon={} mdeg",
        grid.first_latitude_millidegrees, grid.first_longitude_millidegrees
    );
    println!(
        "Last grid point:  lat={} mdeg, lon={} mdeg",
        grid.last_latitude_millidegrees, grid.last_longitude_millidegrees
    );

    println!("\nNDFD parsing test passed!");
}

#[test]
fn test_ndfd_grid_template() {
    let path =
        std::env::var("NDFD_TEST_FILE").unwrap_or_else(|_| "data/ndfd/ds.temp.bin".to_string());

    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(_) => {
            eprintln!("Skipping test: {} not found", path);
            return;
        }
    };

    let stripped = grib2_parser::strip_wmo_headers(&data);

    // Find Section 3 manually to see the grid template
    // Section 3 starts after the GRIB header (16 bytes) + Section 1 + Section 2(optional)
    // Look for section 3 by finding the section number byte
    let mut offset = 16; // Skip indicator section

    while offset < stripped.len() - 5 {
        let section_len = u32::from_be_bytes([
            stripped[offset],
            stripped[offset + 1],
            stripped[offset + 2],
            stripped[offset + 3],
        ]) as usize;
        let section_num = stripped[offset + 4];

        if section_num == 3 {
            // Found Section 3
            let gd = &stripped[offset + 5..offset + section_len];
            println!("Section 3 length: {} bytes", section_len);
            println!("Grid definition template data length: {} bytes", gd.len());

            // Bytes 9-10 are grid definition template number (after 5-byte header)
            // GDT bytes start at offset 5 in section
            // Template number is at bytes 13-14 (after section header)
            if section_len > 14 {
                let template = u16::from_be_bytes([stripped[offset + 13], stripped[offset + 14]]);
                println!("Grid Definition Template: {} (3.{})", template, template);

                // For template 30 (Lambert Conformal):
                // https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/grib2_temp3-30.shtml
                if template == 30 {
                    println!("Template 30: Lambert Conformal");
                    // Parse key fields...
                    let nx = u32::from_be_bytes([gd[16], gd[17], gd[18], gd[19]]);
                    let ny = u32::from_be_bytes([gd[20], gd[21], gd[22], gd[23]]);
                    println!("  Nx={}, Ny={}", nx, ny);

                    // La1 at bytes 39-42 (microdegrees, signed)
                    let la1_raw = u32::from_be_bytes([gd[34], gd[35], gd[36], gd[37]]);
                    let la1 = if la1_raw & 0x80000000 != 0 {
                        -((la1_raw & 0x7FFFFFFF) as i32)
                    } else {
                        la1_raw as i32
                    };

                    // Lo1 at bytes 43-46
                    let lo1_raw = u32::from_be_bytes([gd[38], gd[39], gd[40], gd[41]]);
                    let lo1 = if lo1_raw & 0x80000000 != 0 {
                        -((lo1_raw & 0x7FFFFFFF) as i32)
                    } else {
                        lo1_raw as i32
                    };
                    println!(
                        "  La1={} udeg ({:.6}°), Lo1={} udeg ({:.6}°)",
                        la1,
                        la1 as f64 / 1e6,
                        lo1,
                        lo1 as f64 / 1e6
                    );

                    // LaD (Latin) at bytes 48-51
                    let lad_raw = u32::from_be_bytes([gd[43], gd[44], gd[45], gd[46]]);
                    let lad = if lad_raw & 0x80000000 != 0 {
                        -((lad_raw & 0x7FFFFFFF) as i32)
                    } else {
                        lad_raw as i32
                    };

                    // LoV at bytes 52-55
                    let lov_raw = u32::from_be_bytes([gd[47], gd[48], gd[49], gd[50]]);
                    let lov = if lov_raw & 0x80000000 != 0 {
                        -((lov_raw & 0x7FFFFFFF) as i32)
                    } else {
                        lov_raw as i32
                    };
                    println!(
                        "  LaD={} udeg ({:.6}°), LoV={} udeg ({:.6}°)",
                        lad,
                        lad as f64 / 1e6,
                        lov,
                        lov as f64 / 1e6
                    );

                    // Dx, Dy at bytes 56-59, 60-63
                    let dx = u32::from_be_bytes([gd[51], gd[52], gd[53], gd[54]]);
                    let dy = u32::from_be_bytes([gd[55], gd[56], gd[57], gd[58]]);
                    println!(
                        "  Dx={} mm ({:.3} m), Dy={} mm ({:.3} m)",
                        dx,
                        dx as f64 / 1000.0,
                        dy,
                        dy as f64 / 1000.0
                    );

                    // Scanning mode at byte 64
                    let scan_mode = gd[64];
                    println!("  Scanning mode: 0b{:08b} ({})", scan_mode, scan_mode);

                    // Latin1, Latin2 at bytes 65-68, 69-72
                    let latin1_raw = u32::from_be_bytes([gd[60], gd[61], gd[62], gd[63]]);
                    let latin1 = if latin1_raw & 0x80000000 != 0 {
                        -((latin1_raw & 0x7FFFFFFF) as i32)
                    } else {
                        latin1_raw as i32
                    };
                    let latin2_raw = u32::from_be_bytes([gd[64], gd[65], gd[66], gd[67]]);
                    let latin2 = if latin2_raw & 0x80000000 != 0 {
                        -((latin2_raw & 0x7FFFFFFF) as i32)
                    } else {
                        latin2_raw as i32
                    };
                    println!(
                        "  Latin1={} udeg ({:.6}°), Latin2={} udeg ({:.6}°)",
                        latin1,
                        latin1 as f64 / 1e6,
                        latin2,
                        latin2 as f64 / 1e6
                    );
                }
            }
            break;
        }

        if section_num == 8 || section_len == 0 {
            break; // End marker
        }
        offset += section_len;
    }
}
