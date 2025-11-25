use grib2_parser::sections;
use chrono::Datelike;
use std::fs;

#[test]
fn test_parse_section0() {
    let data = fs::read("../../testdata/gfs_sample.grib2").expect("Failed to read test file");
    
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
    let data = fs::read("../../testdata/gfs_sample.grib2").expect("Failed to read test file");
    
    let id = sections::parse_identification(&data).expect("Failed to parse identification");
    
    println!("Section 1:");
    println!("  Reference time: {}", id.reference_time);
    println!("  Center: {}", id.center);
    println!("  Sub-center: {}", id.sub_center);
    
    assert_eq!(id.reference_time.year(), 2025);
    assert_eq!(id.reference_time.month(), 11);
    assert_eq!(id.reference_time.day(), 25);
    assert_eq!(id.center, 7); // NCEP
}
