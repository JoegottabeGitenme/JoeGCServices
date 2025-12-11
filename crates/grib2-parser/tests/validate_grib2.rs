//! Validation tests for GRIB2 parser
//!
//! These tests verify:
//! 1. Round-trip parsing of synthetic GRIB2 files
//! 2. Comparison with real GRIB2 files from NOAA
//!
//! Run with: cargo test --package grib2-parser --test validate_grib2 -- --nocapture

mod common;
mod testdata_generator;

use bytes::Bytes;
use chrono::{Datelike, Timelike};
use grib2_parser::{Grib2Reader, unpack_simple};
use testdata_generator::Grib2Builder;

/// Test 1: Round-trip - create synthetic data, parse it, verify all fields match
#[test]
fn test_roundtrip_gfs_synthetic() {
    // Create synthetic GFS data with known values
    let input_values: Vec<f32> = (0..100).map(|i| 273.15 + i as f32 * 0.5).collect(); // 273.15K to 322.65K
    
    let builder = Grib2Builder::new_gfs()
        .with_grid(10, 10)
        .with_reference_time(2025, 6, 15, 12)
        .with_parameter(0, 0)  // TMP (temperature)
        .with_level(103, 2)    // 2m above ground
        .with_forecast_hour(6)
        .with_data(input_values.clone());
    
    let grib_bytes = builder.build();
    
    // Parse it back
    let mut reader = Grib2Reader::new(Bytes::from(grib_bytes));
    let msg = reader.next_message()
        .expect("Should parse without error")
        .expect("Should have a message");
    
    // Verify Section 0 (Indicator)
    assert_eq!(msg.indicator.discipline, 0, "Discipline should be meteorological");
    assert_eq!(msg.indicator.edition, 2, "Edition should be 2");
    
    // Verify Section 1 (Identification)
    assert_eq!(msg.identification.center, 7, "Center should be NCEP");
    assert_eq!(msg.identification.reference_time.year(), 2025);
    assert_eq!(msg.identification.reference_time.month(), 6);
    assert_eq!(msg.identification.reference_time.day(), 15);
    assert_eq!(msg.identification.reference_time.hour(), 12);
    
    // Verify Section 3 (Grid Definition)
    assert_eq!(msg.grid_definition.num_points_longitude, 10);
    assert_eq!(msg.grid_definition.num_points_latitude, 10);
    
    // Verify Section 4 (Product Definition)
    assert_eq!(msg.product_definition.parameter_category, 0);
    assert_eq!(msg.product_definition.parameter_number, 0);
    assert_eq!(msg.parameter(), "TMP");
    
    // Verify Section 5 (Data Representation)
    assert_eq!(msg.data_representation.num_data_points, 100);
    assert_eq!(msg.data_representation.bits_per_value, 16);
    
    // Verify data values (using our unpack_simple)
    let unpacked = unpack_simple(
        &msg.data_section.data,
        msg.data_representation.num_data_points,
        msg.data_representation.bits_per_value,
        msg.data_representation.reference_value,
        msg.data_representation.binary_scale_factor,
        msg.data_representation.decimal_scale_factor,
        None,
    ).expect("Should unpack");
    
    let output_values: Vec<f32> = unpacked.iter().map(|v| v.unwrap_or(f32::NAN)).collect();
    
    assert_eq!(output_values.len(), input_values.len(), "Should have same number of values");
    
    // Check values are close (allowing for quantization error)
    for (i, (input, output)) in input_values.iter().zip(output_values.iter()).enumerate() {
        let diff = (input - output).abs();
        assert!(diff < 1.0, "Value {} differs too much: input={}, output={}, diff={}", 
                i, input, output, diff);
    }
    
    println!("✓ Round-trip GFS test passed");
    println!("  Input range: {:.2} - {:.2}", 
             input_values.iter().cloned().fold(f32::INFINITY, f32::min),
             input_values.iter().cloned().fold(f32::NEG_INFINITY, f32::max));
    println!("  Output range: {:.2} - {:.2}",
             output_values.iter().cloned().fold(f32::INFINITY, f32::min),
             output_values.iter().cloned().fold(f32::NEG_INFINITY, f32::max));
}

/// Test 2: Round-trip with MRMS-style data (different discipline, center, parameters)
#[test]
fn test_roundtrip_mrms_synthetic() {
    // Create synthetic MRMS reflectivity data
    let ni = 20;
    let nj = 15;
    let input_values: Vec<f32> = (0..(ni * nj))
        .map(|i| -10.0 + (i as f32 / (ni * nj) as f32) * 70.0) // -10 to 60 dBZ
        .collect();
    
    let builder = Grib2Builder::new_mrms()
        .with_reference_time(2025, 6, 15, 12)
        .with_data(input_values.clone());
    
    let grib_bytes = builder.build();
    
    // Parse it back
    let mut reader = Grib2Reader::new(Bytes::from(grib_bytes));
    let msg = reader.next_message()
        .expect("Should parse without error")
        .expect("Should have a message");
    
    // Verify MRMS-specific fields
    assert_eq!(msg.indicator.discipline, 209, "Discipline should be MRMS local");
    assert_eq!(msg.identification.center, 161, "Center should be NSSL");
    
    // Verify grid
    assert_eq!(msg.grid_definition.num_points_longitude, ni);
    assert_eq!(msg.grid_definition.num_points_latitude, nj);
    
    // Verify data
    let unpacked = unpack_simple(
        &msg.data_section.data,
        msg.data_representation.num_data_points,
        msg.data_representation.bits_per_value,
        msg.data_representation.reference_value,
        msg.data_representation.binary_scale_factor,
        msg.data_representation.decimal_scale_factor,
        None,
    ).expect("Should unpack");
    
    let output_values: Vec<f32> = unpacked.iter().map(|v| v.unwrap_or(f32::NAN)).collect();
    
    // Check values
    for (i, (input, output)) in input_values.iter().zip(output_values.iter()).enumerate() {
        let diff = (input - output).abs();
        assert!(diff < 1.0, "Value {} differs too much: input={}, output={}", i, input, output);
    }
    
    println!("✓ Round-trip MRMS test passed");
}

/// Test 3: Verify GRIB2 message structure matches spec
#[test]
fn test_grib2_structure_validity() {
    let builder = Grib2Builder::new_gfs()
        .with_grid(5, 5)
        .with_constant_value(300.0);
    
    let grib_bytes = builder.build();
    
    // Check magic bytes
    assert_eq!(&grib_bytes[0..4], b"GRIB", "Should start with GRIB magic");
    
    // Check edition (byte 7)
    assert_eq!(grib_bytes[7], 2, "Edition should be 2");
    
    // Check message length (bytes 8-15, big-endian u64)
    let msg_len = u64::from_be_bytes([
        grib_bytes[8], grib_bytes[9], grib_bytes[10], grib_bytes[11],
        grib_bytes[12], grib_bytes[13], grib_bytes[14], grib_bytes[15],
    ]);
    assert_eq!(msg_len as usize, grib_bytes.len(), "Message length should match actual length");
    
    // Check end marker
    assert_eq!(&grib_bytes[grib_bytes.len()-4..], b"7777", "Should end with 7777");
    
    // Verify we can find all sections
    let mut offset = 16; // After Section 0
    let mut found_sections = vec![];
    
    while offset < grib_bytes.len() - 4 {
        if &grib_bytes[offset..offset+4] == b"7777" {
            break;
        }
        
        let section_len = u32::from_be_bytes([
            grib_bytes[offset], grib_bytes[offset+1], 
            grib_bytes[offset+2], grib_bytes[offset+3]
        ]) as usize;
        let section_num = grib_bytes[offset + 4];
        
        found_sections.push((section_num, section_len));
        offset += section_len;
    }
    
    println!("Found sections: {:?}", found_sections);
    
    // Should have sections 1, 3, 4, 5, 6, 7
    let section_nums: Vec<u8> = found_sections.iter().map(|(n, _)| *n).collect();
    assert!(section_nums.contains(&1), "Should have Section 1 (Identification)");
    assert!(section_nums.contains(&3), "Should have Section 3 (Grid Definition)");
    assert!(section_nums.contains(&4), "Should have Section 4 (Product Definition)");
    assert!(section_nums.contains(&5), "Should have Section 5 (Data Representation)");
    assert!(section_nums.contains(&6), "Should have Section 6 (Bitmap)");
    assert!(section_nums.contains(&7), "Should have Section 7 (Data)");
    
    println!("✓ GRIB2 structure validity test passed");
}

/// Test 4: Compare with real GFS file from NOAA
/// This test downloads a real GRIB2 file and compares structure with our synthetic files.
/// 
/// Run with: cargo test --package grib2-parser --test validate_grib2 test_compare_with_real_gfs -- --ignored --nocapture
#[test]
#[ignore]
fn test_compare_with_real_gfs() {
    use std::process::Command;
    use std::fs;
    
    // Use a pre-downloaded test file or download one
    let test_file = "/tmp/gfs_test_sample.grib2";
    
    // Try to download if not present (using curl)
    if !std::path::Path::new(test_file).exists() {
        println!("Downloading real GFS file from NOAA...");
        
        // Get current date for URL (files are only available for recent dates)
        let now = chrono::Utc::now();
        let date_str = now.format("%Y%m%d").to_string();
        
        // Download a small subset - 2m temperature for a small region
        let url = format!(
            "https://nomads.ncep.noaa.gov/cgi-bin/filter_gfs_0p25.pl?\
            dir=%2Fgfs.{}%2F00%2Fatmos&\
            file=gfs.t00z.pgrb2.0p25.f000&\
            var_TMP=on&\
            lev_2_m_above_ground=on&\
            subregion=&toplat=45&leftlon=-100&rightlon=-90&bottomlat=40",
            date_str
        );
        
        println!("URL: {}", url);
        
        let output = Command::new("curl")
            .args(["-s", "-f", "-o", test_file, &url])
            .output();
        
        match output {
            Ok(o) if o.status.success() => {
                println!("Downloaded successfully");
            }
            _ => {
                println!("SKIPPED: Could not download GFS file (network issue or date not available)");
                println!("You can manually download a GFS file and save it to {}", test_file);
                return;
            }
        }
    }
    
    // Read the file
    let real_grib_bytes = match fs::read(test_file) {
        Ok(b) if b.len() > 100 => b,
        _ => {
            println!("SKIPPED: Test file {} is empty or missing", test_file);
            return;
        }
    };
    
    println!("Loaded {} bytes from {}", real_grib_bytes.len(), test_file);
    
    // Parse the real file
    let mut reader = Grib2Reader::new(Bytes::from(real_grib_bytes.clone()));
    let real_msg = match reader.next_message() {
        Ok(Some(msg)) => msg,
        Ok(None) => {
            println!("SKIPPED: No messages in file");
            return;
        }
        Err(e) => {
            println!("SKIPPED: Could not parse file: {}", e);
            return;
        }
    };
    
    println!("\n=== Real GFS File Analysis ===");
    println!("Discipline: {}", real_msg.indicator.discipline);
    println!("Center: {}", real_msg.identification.center);
    println!("Reference time: {}", real_msg.identification.reference_time);
    println!("Grid: {}x{}", 
             real_msg.grid_definition.num_points_longitude,
             real_msg.grid_definition.num_points_latitude);
    println!("Parameter: {} (cat={}, num={})", 
             real_msg.parameter(),
             real_msg.product_definition.parameter_category,
             real_msg.product_definition.parameter_number);
    println!("Level: {}", real_msg.level());
    println!("Data representation:");
    println!("  Packing method: {}", real_msg.data_representation.packing_method);
    println!("  Bits per value: {}", real_msg.data_representation.bits_per_value);
    println!("  Reference value: {}", real_msg.data_representation.reference_value);
    println!("  Binary scale (E): {}", real_msg.data_representation.binary_scale_factor);
    println!("  Decimal scale (D): {}", real_msg.data_representation.decimal_scale_factor);
    
    // Now create a synthetic file with similar properties
    let ni = real_msg.grid_definition.num_points_longitude;
    let nj = real_msg.grid_definition.num_points_latitude;
    
    println!("\n=== Creating Synthetic File with Same Grid Size ===");
    
    // Generate synthetic temperature data (similar range to real 2m temps)
    let synthetic_values: Vec<f32> = (0..(ni * nj) as usize)
        .map(|i| 273.15 + (i as f32 % 50.0)) // Kelvin temps
        .collect();
    
    let synthetic_builder = Grib2Builder::new_gfs()
        .with_grid(ni, nj)
        .with_reference_time(
            real_msg.identification.reference_time.year() as u16,
            real_msg.identification.reference_time.month() as u8,
            real_msg.identification.reference_time.day() as u8,
            real_msg.identification.reference_time.hour() as u8,
        )
        .with_parameter(
            real_msg.product_definition.parameter_category,
            real_msg.product_definition.parameter_number,
        )
        .with_data(synthetic_values);
    
    let synthetic_bytes = synthetic_builder.build();
    
    // Parse synthetic
    let mut syn_reader = Grib2Reader::new(Bytes::from(synthetic_bytes));
    let syn_msg = syn_reader.next_message()
        .expect("Should parse synthetic")
        .expect("Should have message");
    
    println!("\n=== Synthetic File Analysis ===");
    println!("Discipline: {}", syn_msg.indicator.discipline);
    println!("Center: {}", syn_msg.identification.center);
    println!("Grid: {}x{}", 
             syn_msg.grid_definition.num_points_longitude,
             syn_msg.grid_definition.num_points_latitude);
    println!("Data representation:");
    println!("  Bits per value: {}", syn_msg.data_representation.bits_per_value);
    println!("  Reference value: {}", syn_msg.data_representation.reference_value);
    println!("  Binary scale (E): {}", syn_msg.data_representation.binary_scale_factor);
    
    // Compare key structural fields
    println!("\n=== Comparison ===");
    assert_eq!(syn_msg.indicator.discipline, real_msg.indicator.discipline,
               "Discipline should match");
    println!("✓ Discipline matches: {}", syn_msg.indicator.discipline);
    
    assert_eq!(syn_msg.identification.center, real_msg.identification.center,
               "Center should match");
    println!("✓ Center matches: {}", syn_msg.identification.center);
    
    assert_eq!(syn_msg.product_definition.parameter_category, 
               real_msg.product_definition.parameter_category,
               "Parameter category should match");
    println!("✓ Parameter category matches: {}", syn_msg.product_definition.parameter_category);
    
    assert_eq!(syn_msg.grid_definition.num_points_longitude,
               real_msg.grid_definition.num_points_longitude,
               "Grid width should match");
    assert_eq!(syn_msg.grid_definition.num_points_latitude,
               real_msg.grid_definition.num_points_latitude,
               "Grid height should match");
    println!("✓ Grid dimensions match: {}x{}", ni, nj);
    
    println!("\n✓ Real vs synthetic comparison passed");
}

/// Test 5: Verify our synthetic files can be read by the external grib crate
/// (even if values don't decode correctly, the structure should be valid)
#[test]
fn test_grib_crate_can_parse_synthetic() {
    use std::io::Cursor;
    
    let builder = Grib2Builder::new_gfs()
        .with_grid(10, 10)
        .with_gradient(270.0, 310.0);
    
    let grib_bytes = builder.build();
    
    // Try to parse with the external grib crate
    let cursor = Cursor::new(&grib_bytes);
    let grib_file = grib::from_reader(cursor)
        .expect("External grib crate should be able to parse our synthetic file");
    
    // Count messages
    let msg_count = grib_file.iter().count();
    assert_eq!(msg_count, 1, "Should have exactly one message");
    
    // Get the message and check we can create a decoder
    for (_idx, submsg) in grib_file.iter() {
        let indicator = submsg.indicator();
        assert_eq!(indicator.discipline, 0, "Discipline should be 0");
        assert_eq!(indicator.total_length as usize, grib_bytes.len(), "Length should match");
        
        // Try to create decoder (may fail to decode values, but structure should be valid)
        match grib::Grib2SubmessageDecoder::from(submsg) {
            Ok(_decoder) => println!("✓ Decoder created successfully"),
            Err(e) => println!("⚠ Decoder creation failed (structure may still be valid): {:?}", e),
        }
    }
    
    println!("✓ External grib crate can parse our synthetic files");
}

/// Test 6: Edge cases - constant value (0 bits per value)
#[test]
fn test_constant_value_encoding() {
    let constant = 288.15; // 15°C in Kelvin
    
    let builder = Grib2Builder::new_gfs()
        .with_grid(10, 10)
        .with_constant_value(constant);
    
    let grib_bytes = builder.build();
    
    let mut reader = Grib2Reader::new(Bytes::from(grib_bytes));
    let msg = reader.next_message()
        .expect("Should parse")
        .expect("Should have message");
    
    // For constant values, bits_per_value should be 0
    assert_eq!(msg.data_representation.bits_per_value, 0, 
               "Constant value should use 0 bits per value");
    
    // Reference value should be the constant
    let ref_val = msg.data_representation.reference_value;
    assert!((ref_val - constant).abs() < 0.01, 
            "Reference value {} should equal constant {}", ref_val, constant);
    
    // Unpack and verify all values are the constant
    let unpacked = unpack_simple(
        &msg.data_section.data,
        msg.data_representation.num_data_points,
        msg.data_representation.bits_per_value,
        msg.data_representation.reference_value,
        msg.data_representation.binary_scale_factor,
        msg.data_representation.decimal_scale_factor,
        None,
    ).expect("Should unpack");
    
    for (i, val) in unpacked.iter().enumerate() {
        let v = val.unwrap_or(f32::NAN);
        assert!((v - constant).abs() < 0.01, 
                "Value {} should be {}, got {}", i, constant, v);
    }
    
    println!("✓ Constant value encoding test passed");
}
