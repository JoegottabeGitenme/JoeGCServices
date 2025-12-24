//! Unit tests for GRIB2 section parsing functions.
//!
//! These tests don't require test data files and focus on individual functions.

use grib2_parser::sections::decode_grib2_signed;

// ============================================================================
// decode_grib2_signed tests
// ============================================================================

#[test]
fn test_decode_grib2_signed_zero() {
    // Zero should decode to zero
    let bytes = [0x00, 0x00, 0x00, 0x00];
    assert_eq!(decode_grib2_signed(&bytes), 0);
}

#[test]
fn test_decode_grib2_signed_positive() {
    // Positive value: 1
    let bytes = [0x00, 0x00, 0x00, 0x01];
    assert_eq!(decode_grib2_signed(&bytes), 1);

    // Positive value: 1000
    let bytes = [0x00, 0x00, 0x03, 0xE8];
    assert_eq!(decode_grib2_signed(&bytes), 1000);

    // Positive value: 90_000_000 (90 degrees in microdegrees)
    let bytes = 90_000_000_u32.to_be_bytes();
    assert_eq!(decode_grib2_signed(&bytes), 90_000_000);
}

#[test]
fn test_decode_grib2_signed_negative() {
    // Sign-magnitude: MSB=1 means negative
    // -1 is represented as 0x80000001
    let bytes = [0x80, 0x00, 0x00, 0x01];
    assert_eq!(decode_grib2_signed(&bytes), -1);

    // -1000 is represented as 0x800003E8
    let bytes = [0x80, 0x00, 0x03, 0xE8];
    assert_eq!(decode_grib2_signed(&bytes), -1000);

    // -90_000_000 (South Pole in microdegrees)
    // 90_000_000 = 0x055D4A80, with sign bit = 0x855D4A80
    let magnitude = 90_000_000_u32;
    let with_sign = magnitude | 0x80000000;
    let bytes = with_sign.to_be_bytes();
    assert_eq!(decode_grib2_signed(&bytes), -90_000_000);
}

#[test]
fn test_decode_grib2_signed_max_positive() {
    // Maximum positive value: 0x7FFFFFFF = 2147483647
    let bytes = [0x7F, 0xFF, 0xFF, 0xFF];
    assert_eq!(decode_grib2_signed(&bytes), 2147483647);
}

#[test]
fn test_decode_grib2_signed_max_negative() {
    // Maximum negative magnitude with sign bit
    // -2147483647 = 0xFFFFFFFF in sign-magnitude
    let bytes = [0xFF, 0xFF, 0xFF, 0xFF];
    assert_eq!(decode_grib2_signed(&bytes), -2147483647);
}

#[test]
fn test_decode_grib2_signed_negative_zero() {
    // -0 (sign bit set but magnitude is 0)
    // In sign-magnitude, this is technically valid but equals 0
    let bytes = [0x80, 0x00, 0x00, 0x00];
    assert_eq!(decode_grib2_signed(&bytes), 0);
}

#[test]
fn test_decode_grib2_signed_wrong_length() {
    // Too short
    assert_eq!(decode_grib2_signed(&[0x00, 0x00, 0x00]), 0);
    assert_eq!(decode_grib2_signed(&[0x00, 0x00]), 0);
    assert_eq!(decode_grib2_signed(&[0x00]), 0);
    assert_eq!(decode_grib2_signed(&[]), 0);

    // Too long
    assert_eq!(decode_grib2_signed(&[0x00, 0x00, 0x00, 0x00, 0x00]), 0);
}

#[test]
fn test_decode_grib2_signed_common_coordinates() {
    // Test common latitude/longitude values used in GRIB2

    // 45.0 degrees = 45,000,000 microdegrees
    let bytes = 45_000_000_u32.to_be_bytes();
    assert_eq!(decode_grib2_signed(&bytes), 45_000_000);

    // -45.0 degrees (South)
    let magnitude = 45_000_000_u32;
    let with_sign = magnitude | 0x80000000;
    let bytes = with_sign.to_be_bytes();
    assert_eq!(decode_grib2_signed(&bytes), -45_000_000);

    // 180.0 degrees
    let bytes = 180_000_000_u32.to_be_bytes();
    assert_eq!(decode_grib2_signed(&bytes), 180_000_000);

    // -180.0 degrees
    let magnitude = 180_000_000_u32;
    let with_sign = magnitude | 0x80000000;
    let bytes = with_sign.to_be_bytes();
    assert_eq!(decode_grib2_signed(&bytes), -180_000_000);
}

#[test]
fn test_decode_grib2_signed_small_values() {
    // Small positive values
    for i in 1..100 {
        let bytes = (i as u32).to_be_bytes();
        assert_eq!(decode_grib2_signed(&bytes), i);
    }
}

#[test]
fn test_decode_grib2_signed_small_negative_values() {
    // Small negative values
    for i in 1..100 {
        let with_sign = (i as u32) | 0x80000000;
        let bytes = with_sign.to_be_bytes();
        assert_eq!(decode_grib2_signed(&bytes), -(i as i32));
    }
}

// ============================================================================
// Difference from two's complement tests
// ============================================================================

#[test]
fn test_sign_magnitude_vs_twos_complement() {
    // This test demonstrates that GRIB2 uses sign-magnitude, NOT two's complement

    // In two's complement, -1 would be 0xFFFFFFFF
    // In sign-magnitude, -1 is 0x80000001

    // Two's complement -1 (would incorrectly decode as large negative if treated as such)
    let twos_complement_minus_one = [0xFF, 0xFF, 0xFF, 0xFF];
    // In sign-magnitude this is -(0x7FFFFFFF) = -2147483647
    assert_eq!(decode_grib2_signed(&twos_complement_minus_one), -2147483647);

    // Correct sign-magnitude -1
    let sign_magnitude_minus_one = [0x80, 0x00, 0x00, 0x01];
    assert_eq!(decode_grib2_signed(&sign_magnitude_minus_one), -1);
}

#[test]
fn test_sign_bit_only() {
    // Just the sign bit set, no magnitude bits
    let bytes = [0x80, 0x00, 0x00, 0x00];
    // This is negative zero, which should be 0
    assert_eq!(decode_grib2_signed(&bytes), 0);
}

// ============================================================================
// Real-world coordinate tests (from actual GRIB2 files)
// ============================================================================

#[test]
fn test_gfs_typical_coordinates() {
    // GFS global grid typically uses millidegrees (1e-6 degrees)

    // First latitude of 90 N = 90,000,000
    let first_lat = 90_000_000_u32.to_be_bytes();
    assert_eq!(decode_grib2_signed(&first_lat), 90_000_000);

    // Last latitude of 90 S = -90,000,000
    let last_lat = (90_000_000_u32 | 0x80000000).to_be_bytes();
    assert_eq!(decode_grib2_signed(&last_lat), -90_000_000);

    // Longitude 0
    let lon_0 = 0_u32.to_be_bytes();
    assert_eq!(decode_grib2_signed(&lon_0), 0);

    // Longitude 359.75 = 359,750,000 millidegrees
    let lon_359 = 359_750_000_u32.to_be_bytes();
    assert_eq!(decode_grib2_signed(&lon_359), 359_750_000);
}

#[test]
fn test_hrrr_typical_coordinates() {
    // HRRR uses Lambert conformal projection with specific corner coordinates

    // Example: 21.138 N = 21,138,000 microdegrees
    let lat = 21_138_000_u32.to_be_bytes();
    assert_eq!(decode_grib2_signed(&lat), 21_138_000);

    // Example: -134.095 W = -134,095,000 microdegrees (western longitude)
    let lon = (134_095_000_u32 | 0x80000000).to_be_bytes();
    assert_eq!(decode_grib2_signed(&lon), -134_095_000);
}
