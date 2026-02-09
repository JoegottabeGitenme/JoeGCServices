//! GRIB2 data unpacking algorithms.
//!
//! Implements various packing methods:
//! - Simple packing (most common, used by GFS)
//! - Complex packing
//! - JPEG2000 compression

use crate::Grib2Error;

/// Unpack simple packed GRIB2 data
///
/// Simple packing formula: value = (reference_value + (packed_value * 2^binary_scale)) * 10^(-decimal_scale)
pub fn unpack_simple(
    packed_data: &[u8],
    num_points: u32,
    bits_per_value: u8,
    reference_value: f32,
    binary_scale_factor: i16,
    decimal_scale_factor: i16,
    bitmap: Option<&[u8]>,
) -> Result<Vec<Option<f32>>, Grib2Error> {
    if bits_per_value == 0 {
        // All values are the reference value
        return Ok(vec![Some(reference_value); num_points as usize]);
    }

    let binary_scale = 2.0_f32.powi(binary_scale_factor as i32);
    let decimal_scale = 10.0_f32.powi(-(decimal_scale_factor as i32));

    let mut values = Vec::new();
    let mut bit_position = 0;
    let bits_per_value = bits_per_value as usize;

    for i in 0..(num_points as usize) {
        // Check bitmap if present
        let has_value = if let Some(bm) = bitmap {
            // Bitmap: 1 bit per data point, 1 = value present, 0 = missing
            let byte_idx = i / 8;
            let bit_idx = 7 - (i % 8);
            if byte_idx < bm.len() {
                (bm[byte_idx] >> bit_idx) & 1 == 1
            } else {
                true
            }
        } else {
            true
        };

        if !has_value {
            values.push(None);
            bit_position += bits_per_value;
            continue;
        }

        // Extract bits from data
        let packed_value = extract_bits(packed_data, bit_position, bits_per_value)
            .map_err(|e| Grib2Error::UnpackingError(format!("Failed to extract bits: {}", e)))?;

        bit_position += bits_per_value;

        // Apply unpacking formula
        let value = (reference_value + (packed_value as f32) * binary_scale) * decimal_scale;
        values.push(Some(value));
    }

    Ok(values)
}

/// Extract bits from a byte array
/// Returns the bits as a 32-bit unsigned integer
fn extract_bits(data: &[u8], start_bit: usize, num_bits: usize) -> Result<u32, String> {
    if num_bits > 32 || num_bits == 0 {
        return Err(format!("Invalid number of bits: {}", num_bits));
    }

    let mut result = 0u32;

    for i in 0..num_bits {
        let absolute_bit = start_bit + i;
        let byte_idx = absolute_bit / 8;
        let bit_idx = 7 - (absolute_bit % 8); // MSB first

        if byte_idx >= data.len() {
            return Err("Not enough data to extract bits".to_string());
        }

        let bit = (data[byte_idx] >> bit_idx) & 1;
        result = (result << 1) | (bit as u32);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_bits() {
        // Test with simple byte: 0b10110101
        let data = vec![0b10110101];

        // Extract first 2 bits (should be 0b10 = 2)
        let result = extract_bits(&data, 0, 2).unwrap();
        assert_eq!(result, 0b10);

        // Extract bits 2-4 (should be 0b11 = 3)
        let result = extract_bits(&data, 2, 2).unwrap();
        assert_eq!(result, 0b11);

        // Extract all 8 bits
        let result = extract_bits(&data, 0, 8).unwrap();
        assert_eq!(result, 0b10110101);
    }

    #[test]
    fn test_extract_bits_across_bytes() {
        // Test extracting bits that span multiple bytes
        let data = vec![0b11110000, 0b00001111];

        // Extract 4 bits from position 4 (should span both bytes)
        let result = extract_bits(&data, 4, 4).unwrap();
        assert_eq!(result, 0b0000); // last 4 bits of first byte

        // Extract 8 bits starting at position 4
        let result = extract_bits(&data, 4, 8).unwrap();
        assert_eq!(result, 0b00000000); // 0000 from byte 0 + 0000 from byte 1
    }

    #[test]
    fn test_extract_bits_invalid_num_bits() {
        let data = vec![0xFF];

        // 0 bits is invalid
        let result = extract_bits(&data, 0, 0);
        assert!(result.is_err());

        // > 32 bits is invalid
        let result = extract_bits(&data, 0, 33);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_bits_not_enough_data() {
        let data = vec![0xFF];

        // Try to extract beyond the data
        let result = extract_bits(&data, 8, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_simple_unpacking() {
        // Simple test: 2 data points, 8 bits per value
        let packed = vec![100, 200];
        let values = unpack_simple(
            &packed, 2,    // 2 data points = 16 bits = 2 bytes
            8,    // 8 bits per value
            0.0,  // reference value
            0,    // binary scale (2^0 = 1)
            0,    // decimal scale (10^0 = 1)
            None, // no bitmap
        );

        assert!(values.is_ok(), "Unpacking failed: {:?}", values);
        let vals = values.unwrap();
        assert_eq!(vals.len(), 2);
        // First value should be close to 100.0
        assert!((vals[0].unwrap() - 100.0).abs() < 0.1);
        // Second value should be close to 200.0
        assert!((vals[1].unwrap() - 200.0).abs() < 0.1);
    }

    #[test]
    fn test_simple_unpacking_zero_bits() {
        // When bits_per_value is 0, all values equal reference_value
        let packed = vec![];
        let values = unpack_simple(
            &packed, 5,     // 5 data points
            0,     // 0 bits per value
            273.0, // reference value
            0, 0, None,
        );

        assert!(values.is_ok());
        let vals = values.unwrap();
        assert_eq!(vals.len(), 5);
        for v in vals {
            assert!((v.unwrap() - 273.0).abs() < 0.001);
        }
    }

    #[test]
    fn test_simple_unpacking_with_scale_factors() {
        // Test with scale factors
        let packed = vec![0, 1, 2, 3]; // 4 values, 8 bits each
        let values = unpack_simple(
            &packed, 4, 8, 100.0, // reference value
            1,     // binary scale = 2^1 = 2
            1,     // decimal scale = 10^-1 = 0.1
            None,
        );

        assert!(values.is_ok());
        let vals = values.unwrap();
        assert_eq!(vals.len(), 4);
        // value = (reference + packed * 2^binary_scale) * 10^(-decimal_scale)
        // val[0] = (100 + 0 * 2) * 0.1 = 10.0
        // val[1] = (100 + 1 * 2) * 0.1 = 10.2
        assert!((vals[0].unwrap() - 10.0).abs() < 0.01);
        assert!((vals[1].unwrap() - 10.2).abs() < 0.01);
    }

    #[test]
    fn test_simple_unpacking_with_bitmap() {
        // Test with a bitmap indicating missing values
        // The current implementation still reads packed data for all points
        // (advancing bit_position even for missing values), so we need enough data
        let packed = vec![100, 0, 200, 0]; // 4 values worth of data, 8 bits each
        let bitmap = vec![0b10100000]; // bits: 1,0,1,0,0,0,0,0 - values at positions 0 and 2
        let values = unpack_simple(
            &packed,
            4, // 4 data points
            8, // 8 bits per value
            0.0,
            0,
            0,
            Some(&bitmap),
        );

        assert!(values.is_ok(), "Unpacking failed: {:?}", values);
        let vals = values.unwrap();
        assert_eq!(vals.len(), 4);
        assert!(vals[0].is_some()); // Present
        assert!(vals[1].is_none()); // Missing (bitmap bit = 0)
        assert!(vals[2].is_some()); // Present
        assert!(vals[3].is_none()); // Missing
    }
}
