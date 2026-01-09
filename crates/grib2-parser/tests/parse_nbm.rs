//! Test parsing NBM (National Blend of Models) GRIB2 files
//!
//! NBM uses different grid projections per region:
//! - CONUS: Lambert Conformal (Template 30)
//! - Alaska: Polar Stereographic (Template 20)
//! - Hawaii/PR/Guam: Mercator (Template 10)

mod common;

use grib2_parser::{Grib2Reader, Grib2Tables};
use std::path::Path;
use std::sync::Arc;

/// Test parsing an NBM Guam file (Mercator projection, Template 10)
#[test]
fn test_parse_nbm_guam_mercator() {
    let test_file = Path::new("/tmp/nbm-test/blend.t00z.core.f001.gu.grib2");

    if !test_file.exists() {
        eprintln!("Skipping NBM Guam test - file not found at {:?}", test_file);
        eprintln!("Download with: aws s3 cp s3://noaa-nbm-grib2-pds/blend.20260108/00/core/blend.t00z.core.f001.gu.grib2 /tmp/nbm-test/ --no-sign-request");
        return;
    }

    let data = std::fs::read(test_file).expect("Failed to read NBM file");
    let tables = Arc::new(Grib2Tables::default());
    let mut reader = Grib2Reader::new(data.into(), tables);

    let mut message_count = 0;
    let mut parameters_found = Vec::new();

    while let Ok(Some(message)) = reader.next_message() {
        message_count += 1;

        // Check grid dimensions (Guam is 193x193)
        let grid = &message.grid_definition;
        assert_eq!(
            grid.num_points_longitude, 193,
            "Expected 193 points in X direction for Guam"
        );
        assert_eq!(
            grid.num_points_latitude, 193,
            "Expected 193 points in Y direction for Guam"
        );

        // Track parameters found
        let param_name = message.product_definition.parameter_short_name.clone();
        if !parameters_found.contains(&param_name) {
            parameters_found.push(param_name.clone());
            println!(
                "Found parameter: {} - {} at {}",
                param_name,
                message.product_definition.parameter_category,
                message.product_definition.level_description
            );
        }

        // Verify we can unpack data
        let values = message.unpack_data().expect("Failed to unpack data");
        assert!(!values.is_empty(), "Unpacked data should not be empty");
        assert_eq!(
            values.len(),
            (193 * 193) as usize,
            "Expected {} values for 193x193 grid",
            193 * 193
        );
    }

    println!("\nNBM Guam file summary:");
    println!("  Total messages: {}", message_count);
    println!("  Unique parameters: {}", parameters_found.len());
    println!("  Parameters: {:?}", parameters_found);

    assert!(
        message_count > 0,
        "Should have parsed at least one message from NBM file"
    );
}
