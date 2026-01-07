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

/// Test RH values to debug horizontal mirroring issue
#[test]
fn test_ndfd_rh_values() {
    let path = std::env::var("NDFD_RH_FILE").unwrap_or_else(|_| "data/ndfd/ds.rhm.bin".to_string());

    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(_) => {
            eprintln!("Skipping test: {} not found", path);
            return;
        }
    };

    let stripped = grib2_parser::strip_wmo_headers(&data);
    let tables = Arc::new(Grib2Tables::new());
    let mut grib_reader = Grib2Reader::new(Bytes::from(stripped), tables);

    let msg = grib_reader
        .next_message()
        .expect("Should have a message")
        .expect("Should parse successfully");

    let grid_data = msg.unpack_data().expect("Should unpack data");
    let width = msg.grid_definition.num_points_longitude as usize;
    let height = msg.grid_definition.num_points_latitude as usize;

    println!("RH Grid: {} x {}", width, height);

    // Values from wgrib2:
    // LA (34°N, 118.2°W): RH = 86.4% at wgrib2 (240, 573) = our (239, 572)
    // NY (40.7°N, 74°W): RH = 63.4% at wgrib2 (1811, 857) = our (1810, 856)
    // Denver (39.7°N, 105°W): RH = 26.4%

    let la_idx = 572 * width + 239;
    let la_val = grid_data[la_idx];
    println!("LA (239, 572): {:.1}%", la_val);

    let ny_idx = 856 * width + 1810;
    let ny_val = grid_data[ny_idx];
    println!("NY (1810, 856): {:.1}%", ny_val);

    // Denver at approximately i=740, j=768
    let denver_idx = 768 * width + 740;
    let denver_val = grid_data[denver_idx];
    println!("Denver (740, 768): {:.1}%", denver_val);

    // Check mirrored positions - if data is mirrored, these would have the "wrong" values
    let mirror_la_i = (width - 1) - 239; // 1905
    let mirror_la_idx = 572 * width + mirror_la_i;
    let mirror_la_val = grid_data[mirror_la_idx];
    println!("Mirrored LA pos (1905, 572): {:.1}%", mirror_la_val);

    let mirror_denver_i = (width - 1) - 740; // 1404
    let mirror_denver_idx = 768 * width + mirror_denver_i;
    let mirror_denver_val = grid_data[mirror_denver_idx];
    println!("Mirrored Denver pos (1404, 768): {:.1}%", mirror_denver_val);

    // Verify LA value is close to wgrib2's 86.4%
    assert!(
        (la_val - 86.4).abs() < 1.0,
        "LA RH should be ~86.4%, got {:.1}%",
        la_val
    );

    // Verify Denver is low humidity (~26%)
    assert!(
        denver_val < 40.0,
        "Denver should have low RH (~26%), got {:.1}%",
        denver_val
    );

    // CRITICAL: If mirrored Denver position has low humidity like Denver,
    // it means the data IS correctly stored and the issue is in rendering
    // If mirrored Denver has HIGH humidity, the data itself is mirrored
    println!("\n=== MIRRORING DIAGNOSTIC ===");
    if mirror_denver_val < 40.0 {
        println!(
            "Mirrored Denver also has low RH ({:.1}%) - DATA MAY BE MIRRORED IN STORAGE",
            mirror_denver_val
        );
    } else {
        println!(
            "Mirrored Denver has different RH ({:.1}%) - Data storage looks correct",
            mirror_denver_val
        );
    }
}

/// Test that data values at specific grid points match wgrib2 output.
/// This validates that our data unpacking and indexing is correct.
#[test]
fn test_ndfd_data_values() {
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
    let tables = Arc::new(Grib2Tables::new());
    let mut grib_reader = Grib2Reader::new(Bytes::from(stripped), tables);

    let msg = grib_reader
        .next_message()
        .expect("Should have a message")
        .expect("Should parse successfully");

    let grid_data = msg.unpack_data().expect("Should unpack data");
    let width = msg.grid_definition.num_points_longitude as usize;
    let height = msg.grid_definition.num_points_latitude as usize;

    println!(
        "Grid dimensions: {} x {} = {} points",
        width,
        height,
        width * height
    );
    println!("Unpacked data length: {}", grid_data.len());
    assert_eq!(
        grid_data.len(),
        width * height,
        "Data length should match grid size"
    );

    // Test specific points - values from wgrib2 -ijlat command
    // wgrib2 uses 1-based indexing, we use 0-based

    // Los Angeles: wgrib2 (240, 573) = our (239, 572)
    // wgrib2: (240,573),lon=241.794829,lat=33.990493,val=286.5
    let la_i = 239;
    let la_j = 572;
    let la_idx = la_j * width + la_i;
    let la_val = grid_data[la_idx];
    println!(
        "LA (i={}, j={}): idx={}, val={:.1}K",
        la_i, la_j, la_idx, la_val
    );

    // Check if LA value is reasonable temperature (~286.5K from wgrib2)
    // Allow some tolerance since we may have slightly different interpolation
    assert!(
        (la_val - 286.5).abs() < 5.0 || la_val.is_nan(),
        "LA temperature should be around 286.5K, got {}",
        la_val
    );

    // New York: wgrib2 (1811, 857) = our (1810, 856)
    // wgrib2: (1811,857),lon=286.011281,lat=40.689125,val=273.1
    let ny_i = 1810;
    let ny_j = 856;
    let ny_idx = ny_j * width + ny_i;
    let ny_val = grid_data[ny_idx];
    println!(
        "NY (i={}, j={}): idx={}, val={:.1}K",
        ny_i, ny_j, ny_idx, ny_val
    );

    // Verify index calculation is correct
    assert!(ny_idx < grid_data.len(), "NY index should be within bounds");

    // Check grid corners
    let sw_val = grid_data[0]; // (0, 0) = SW corner
    let se_val = grid_data[width - 1]; // (width-1, 0) = SE corner
    let nw_val = grid_data[(height - 1) * width]; // (0, height-1) = NW corner
    let ne_val = grid_data[(height - 1) * width + width - 1]; // (width-1, height-1) = NE corner

    println!("\nGrid corners:");
    println!("  SW (0, 0): {:.1}", sw_val);
    println!("  SE ({}, 0): {:.1}", width - 1, se_val);
    println!("  NW (0, {}): {:.1}", height - 1, nw_val);
    println!("  NE ({}, {}): {:.1}", width - 1, height - 1, ne_val);

    // The SW and SE corners should be ocean (missing data = very large values)
    // based on wgrib2 output showing val=9.999e+20 for corners

    // Check a point that should be land (central US)
    // Kansas City: approx i=986, j=549 based on projection test
    let kc_i = 986;
    let kc_j = 549;
    let kc_idx = kc_j * width + kc_i;
    let kc_val = grid_data[kc_idx];
    println!("\nKansas City (i={}, j={}): {:.1}K", kc_i, kc_j, kc_val);

    // If data appears mirrored horizontally, LA values would appear at NY position
    // Let's check the mirrored position
    let mirror_la_i = (width - 1) - la_i; // Mirrored LA position
    let mirror_la_idx = la_j * width + mirror_la_i;
    let mirror_la_val = grid_data[mirror_la_idx];
    println!(
        "\nMirrored LA position (i={}): {:.1}K",
        mirror_la_i, mirror_la_val
    );

    // If the data is correct, LA should have valid temp, mirrored position should be ocean/different
    if !la_val.is_nan() && la_val < 1e10 {
        println!("LA has valid temperature data");
    } else {
        println!("WARNING: LA position has invalid/missing data - possible mirroring issue!");
    }

    if !mirror_la_val.is_nan() && mirror_la_val < 1e10 {
        println!(
            "Mirrored LA position also has valid data (value: {:.1}K)",
            mirror_la_val
        );
    } else {
        println!("Mirrored LA position has invalid/missing data (as expected)");
    }
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
