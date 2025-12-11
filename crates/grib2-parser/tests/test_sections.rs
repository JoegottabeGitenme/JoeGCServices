/// Test individual GRIB2 section parsing
mod common;

use chrono::Datelike;
use grib2_parser::sections;
use std::fs;

#[test]
fn test_parse_section0() {
    let path = require_test_file!("gfs_sample.grib2");

    let data = fs::read(&path).expect("Failed to read test file");

    let indicator = sections::parse_indicator(&data).expect("Failed to parse indicator");

    assert_eq!(indicator.magic, [0x47, 0x52, 0x49, 0x42]); // "GRIB"
    assert_eq!(indicator.edition, 2);
    assert!(indicator.message_length > 0);

    println!("Section 0:");
    println!("  Edition: {}", indicator.edition);
    println!("  Discipline: {}", indicator.discipline);
    println!("  Message length: {} bytes", indicator.message_length);
}

#[test]
fn test_parse_section1() {
    let path = require_test_file!("gfs_sample.grib2");

    let data = fs::read(&path).expect("Failed to read test file");

    let id = sections::parse_identification(&data).expect("Failed to parse identification");

    println!("Section 1:");
    println!("  Reference time: {}", id.reference_time);
    println!("  Center: {}", id.center);
    println!("  Sub-center: {}", id.sub_center);

    // Verify center is NCEP
    assert_eq!(id.center, 7, "Center should be NCEP (7)");

    // Reference time should be reasonable (2020-2030 range)
    assert!(
        id.reference_time.year() >= 2020 && id.reference_time.year() <= 2030,
        "Reference time year should be reasonable"
    );
}
