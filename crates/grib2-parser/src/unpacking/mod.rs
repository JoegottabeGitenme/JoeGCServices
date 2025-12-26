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
}
