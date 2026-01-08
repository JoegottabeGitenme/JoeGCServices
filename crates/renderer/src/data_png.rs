//! Data PNG encoding for GPU shader consumption.
//!
//! Encodes grid data into PNG format optimized for WebGL texture upload.
//! This format is designed for weather data visualization using GPU shaders,
//! similar to the approach used by Windy.com.
//!
//! ## Encoding Formats
//!
//! ### 16-bit mode (default, `depth=16`)
//!
//! Uses native PNG 16-bit grayscale with 16-bit alpha:
//! - **Gray channel**: 16-bit normalized value (0-65535)
//! - **Alpha channel**: 16-bit validity mask (65535 = valid, 0 = no data)
//!
//! This format displays correctly in image viewers as grayscale.
//!
//! ### 8-bit mode (`depth=8`)
//!
//! Uses PNG 8-bit grayscale with 8-bit alpha:
//! - **Gray channel**: 8-bit normalized value (0-255)
//! - **Alpha channel**: 8-bit validity mask (255 = valid, 0 = no data)
//!
//! Smaller files (~50%) but only 256 distinct values.
//!
//! ## Normalization
//!
//! Physical values are normalized to the 0-65535 (16-bit) or 0-255 (8-bit) range:
//! ```text
//! normalized = (value - min) / (max - min)
//! uint16 = normalized * 65535  // for 16-bit
//! uint8 = normalized * 255     // for 8-bit
//! ```
//!
//! ## GLSL Decoding
//!
//! For both 8-bit and 16-bit modes, WebGL normalizes values to 0-1:
//! ```glsl
//! vec4 texel = texture2D(uDataTexture, vTexCoord);
//! float normalized = texel.r;  // Gray channel, already 0-1 in GLSL
//! float physical_value = normalized * (uMaxValue - uMinValue) + uMinValue;
//! bool valid = texel.a > 0.5;
//! ```
//!
//! ## Metadata
//!
//! Encoding parameters are embedded in PNG tEXt chunks for self-describing files:
//! - `EDR:parameter` - Parameter name
//! - `EDR:units` - Unit symbol
//! - `EDR:min` - Minimum value used for normalization
//! - `EDR:max` - Maximum value used for normalization
//! - `EDR:bbox` - Bounding box as "west,south,east,north"
//! - `EDR:encoding` - Encoding type ("uint16" or "uint8")

use std::io::Write;

/// Maximum PNG dimensions to prevent memory issues
pub const MAX_PNG_DIMENSION: usize = 4096;

/// Metadata for a data-encoded PNG
#[derive(Debug, Clone)]
pub struct DataPngMetadata {
    /// Parameter name (e.g., "wind_u", "temperature_2m")
    pub parameter_name: String,
    /// Unit symbol (e.g., "m/s", "K")
    pub units: String,
    /// Minimum value used for normalization
    pub min_value: f32,
    /// Maximum value used for normalization
    pub max_value: f32,
    /// Bounding box as [west, south, east, north] (minLon, minLat, maxLon, maxLat)
    pub bbox: [f64; 4],
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
}

impl DataPngMetadata {
    /// Create metadata with just min/max, other fields can be set later
    pub fn new(min_value: f32, max_value: f32) -> Self {
        Self {
            parameter_name: String::new(),
            units: String::new(),
            min_value,
            max_value,
            bbox: [0.0; 4],
            width: 0,
            height: 0,
        }
    }

    /// Set parameter name
    pub fn with_parameter(mut self, name: &str) -> Self {
        self.parameter_name = name.to_string();
        self
    }

    /// Set units
    pub fn with_units(mut self, units: &str) -> Self {
        self.units = units.to_string();
        self
    }

    /// Set bounding box
    pub fn with_bbox(mut self, west: f64, south: f64, east: f64, north: f64) -> Self {
        self.bbox = [west, south, east, north];
        self
    }

    /// Set dimensions
    pub fn with_dimensions(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }
}

/// Result of encoding data to PNG
#[derive(Debug)]
pub struct EncodedDataPng {
    /// PNG file bytes
    pub png_bytes: Vec<u8>,
    /// Metadata about the encoding
    pub metadata: DataPngMetadata,
}

/// Encoder for converting grid data to 16-bit PNG
pub struct DataPngEncoder {
    /// Minimum value for normalization
    pub min_value: f32,
    /// Maximum value for normalization
    pub max_value: f32,
}

impl DataPngEncoder {
    /// Create a new encoder with the given value range
    pub fn new(min_value: f32, max_value: f32) -> Self {
        Self {
            min_value,
            max_value,
        }
    }

    /// Create an encoder by computing min/max from the data
    pub fn from_data(data: &[Option<f32>]) -> Self {
        let (min_val, max_val) = compute_data_range(data);
        Self::new(min_val, max_val)
    }

    /// Encode grid data to a 16-bit PNG
    ///
    /// # Arguments
    /// * `data` - Grid data as `Option<f32>`, None represents no-data/masked pixels
    /// * `width` - Grid width in pixels
    /// * `height` - Grid height in pixels
    ///
    /// # Returns
    /// Encoded PNG with embedded metadata
    pub fn encode(
        &self,
        data: &[Option<f32>],
        width: usize,
        height: usize,
    ) -> Result<EncodedDataPng, String> {
        // Validate dimensions
        if width > MAX_PNG_DIMENSION || height > MAX_PNG_DIMENSION {
            return Err(format!(
                "PNG dimensions {}x{} exceed maximum {}x{}",
                width, height, MAX_PNG_DIMENSION, MAX_PNG_DIMENSION
            ));
        }

        if data.len() != width * height {
            return Err(format!(
                "Data length {} does not match dimensions {}x{}={}",
                data.len(),
                width,
                height,
                width * height
            ));
        }

        // Encode data to 16-bit grayscale+alpha pixels
        let pixels = self.encode_to_gray_alpha_16bit(data);

        // Build PNG with metadata
        let metadata = DataPngMetadata::new(self.min_value, self.max_value)
            .with_dimensions(width as u32, height as u32);

        let png_bytes = self.create_png_with_metadata(&pixels, width, height, &metadata)?;

        Ok(EncodedDataPng {
            png_bytes,
            metadata,
        })
    }

    /// Encode grid data to a 16-bit PNG with full metadata
    pub fn encode_with_metadata(
        &self,
        data: &[Option<f32>],
        width: usize,
        height: usize,
        parameter_name: &str,
        units: &str,
        bbox: [f64; 4],
    ) -> Result<EncodedDataPng, String> {
        // Validate dimensions
        if width > MAX_PNG_DIMENSION || height > MAX_PNG_DIMENSION {
            return Err(format!(
                "PNG dimensions {}x{} exceed maximum {}x{}",
                width, height, MAX_PNG_DIMENSION, MAX_PNG_DIMENSION
            ));
        }

        if data.len() != width * height {
            return Err(format!(
                "Data length {} does not match dimensions {}x{}={}",
                data.len(),
                width,
                height,
                width * height
            ));
        }

        // Encode data to 16-bit grayscale+alpha pixels
        let pixels = self.encode_to_gray_alpha_16bit(data);

        // Build metadata
        let metadata = DataPngMetadata::new(self.min_value, self.max_value)
            .with_parameter(parameter_name)
            .with_units(units)
            .with_bbox(bbox[0], bbox[1], bbox[2], bbox[3])
            .with_dimensions(width as u32, height as u32);

        let png_bytes = self.create_png_with_metadata(&pixels, width, height, &metadata)?;

        Ok(EncodedDataPng {
            png_bytes,
            metadata,
        })
    }

    /// Encode data values to 16-bit Grayscale+Alpha pixel buffer
    ///
    /// Uses native PNG 16-bit depth for both gray and alpha channels.
    /// This produces images that display correctly in viewers while
    /// maintaining full 16-bit precision.
    fn encode_to_gray_alpha_16bit(&self, data: &[Option<f32>]) -> Vec<u8> {
        // 16-bit grayscale + 16-bit alpha = 4 bytes per pixel
        let mut pixels = Vec::with_capacity(data.len() * 4);
        let range = self.max_value - self.min_value;

        for value in data {
            match value {
                Some(v) if !v.is_nan() => {
                    // Normalize to 0.0-1.0
                    let normalized = if range > 0.0 {
                        ((v - self.min_value) / range).clamp(0.0, 1.0)
                    } else {
                        0.5 // If min == max, use middle value
                    };

                    // Convert to 16-bit
                    let uint16_value = (normalized * 65535.0) as u16;

                    // Gray channel (16-bit big-endian)
                    pixels.push((uint16_value >> 8) as u8);
                    pixels.push((uint16_value & 0xFF) as u8);
                    // Alpha channel (16-bit big-endian) - fully opaque
                    pixels.push(0xFF);
                    pixels.push(0xFF);
                }
                _ => {
                    // No data - transparent pixel
                    pixels.push(0);
                    pixels.push(0);
                    pixels.push(0);
                    pixels.push(0);
                }
            }
        }

        pixels
    }

    /// Create 16-bit Grayscale+Alpha PNG with embedded tEXt metadata chunks
    fn create_png_with_metadata(
        &self,
        pixels: &[u8],
        width: usize,
        height: usize,
        metadata: &DataPngMetadata,
    ) -> Result<Vec<u8>, String> {
        let mut png = Vec::new();

        // PNG signature
        png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

        // IHDR chunk
        let mut ihdr_data = Vec::with_capacity(13);
        ihdr_data.extend_from_slice(&(width as u32).to_be_bytes());
        ihdr_data.extend_from_slice(&(height as u32).to_be_bytes());
        ihdr_data.push(16); // bit depth (16-bit)
        ihdr_data.push(4); // color type (Grayscale+Alpha)
        ihdr_data.push(0); // compression method
        ihdr_data.push(0); // filter method
        ihdr_data.push(0); // interlace method
        write_chunk(&mut png, b"IHDR", &ihdr_data);

        // tEXt chunks with metadata (must come before IDAT)
        write_text_chunk(&mut png, "EDR:encoding", "uint16");

        if !metadata.parameter_name.is_empty() {
            write_text_chunk(&mut png, "EDR:parameter", &metadata.parameter_name);
        }

        if !metadata.units.is_empty() {
            write_text_chunk(&mut png, "EDR:units", &metadata.units);
        }

        write_text_chunk(&mut png, "EDR:min", &format!("{}", metadata.min_value));
        write_text_chunk(&mut png, "EDR:max", &format!("{}", metadata.max_value));

        // Only write bbox if it's been set (not all zeros)
        if metadata.bbox != [0.0, 0.0, 0.0, 0.0] {
            write_text_chunk(
                &mut png,
                "EDR:bbox",
                &format!(
                    "{},{},{},{}",
                    metadata.bbox[0], metadata.bbox[1], metadata.bbox[2], metadata.bbox[3]
                ),
            );
        }

        write_text_chunk(&mut png, "EDR:width", &format!("{}", metadata.width));
        write_text_chunk(&mut png, "EDR:height", &format!("{}", metadata.height));

        // IDAT chunk (compressed image data)
        // 16-bit grayscale+alpha = 4 bytes per pixel
        let idat_data = deflate_idat_gray_alpha_16bit(pixels, width, height)
            .map_err(|e| format!("IDAT compression failed: {}", e))?;
        write_chunk(&mut png, b"IDAT", &idat_data);

        // IEND chunk
        write_chunk(&mut png, b"IEND", &[]);

        Ok(png)
    }
}

/// Compute min and max from data, ignoring None values.
///
/// Returns (min, max) tuple. If all values are None/NaN, returns (0.0, 1.0).
/// If all values are the same, returns a range of +/- 0.5 around that value.
pub fn compute_data_range(data: &[Option<f32>]) -> (f32, f32) {
    let mut min_val = f32::INFINITY;
    let mut max_val = f32::NEG_INFINITY;

    for value in data.iter().flatten() {
        if !value.is_nan() {
            min_val = min_val.min(*value);
            max_val = max_val.max(*value);
        }
    }

    // Handle case where all values are None or NaN
    if min_val.is_infinite() || max_val.is_infinite() {
        (0.0, 1.0)
    } else if (max_val - min_val).abs() < f32::EPSILON {
        // All values are the same - create a small range around it
        (min_val - 0.5, max_val + 0.5)
    } else {
        (min_val, max_val)
    }
}

/// Write a PNG chunk
fn write_chunk(png: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    // Write length
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());

    // Write chunk type
    png.extend_from_slice(chunk_type);

    // Write data
    png.extend_from_slice(data);

    // Write CRC
    let crc_data = [chunk_type.as_slice(), data].concat();
    let crc = crc32fast::hash(&crc_data);
    png.extend_from_slice(&crc.to_be_bytes());
}

/// Write a tEXt metadata chunk
///
/// PNG tEXt chunks contain keyword-value pairs.
/// Format: keyword (1-79 bytes) + null separator + text value
pub fn write_text_chunk(png: &mut Vec<u8>, keyword: &str, text: &str) {
    // tEXt chunk format: keyword + null byte + text
    let mut data = Vec::with_capacity(keyword.len() + 1 + text.len());
    data.extend_from_slice(keyword.as_bytes());
    data.push(0); // Null separator
    data.extend_from_slice(text.as_bytes());

    write_chunk(png, b"tEXt", &data);
}

/// Deflate Grayscale+Alpha image data for IDAT chunk (8-bit mode)
fn deflate_idat_gray_alpha(
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Add filter byte (0 = no filter) to each scanline
    // Grayscale+Alpha = 2 bytes per pixel
    let mut uncompressed = Vec::with_capacity(height * (1 + width * 2));
    for y in 0..height {
        uncompressed.push(0); // filter type: none
        let row_start = y * width * 2;
        let row_end = row_start + width * 2;
        uncompressed.extend_from_slice(&pixels[row_start..row_end]);
    }

    // Compress with flate2
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&uncompressed)?;
    let compressed = encoder.finish()?;

    Ok(compressed)
}

/// Deflate 16-bit Grayscale+Alpha image data for IDAT chunk
fn deflate_idat_gray_alpha_16bit(
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Add filter byte (0 = no filter) to each scanline
    // 16-bit Grayscale+Alpha = 4 bytes per pixel (2 for gray, 2 for alpha)
    let mut uncompressed = Vec::with_capacity(height * (1 + width * 4));
    for y in 0..height {
        uncompressed.push(0); // filter type: none
        let row_start = y * width * 4;
        let row_end = row_start + width * 4;
        uncompressed.extend_from_slice(&pixels[row_start..row_end]);
    }

    // Compress with flate2
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&uncompressed)?;
    let compressed = encoder.finish()?;

    Ok(compressed)
}

/// Encoder for 8-bit grayscale PNG output.
///
/// Uses Grayscale+Alpha (2 channels, 8-bit each):
/// - Gray channel: normalized value (0-255)
/// - Alpha channel: validity mask (255 = valid, 0 = no data)
///
/// This format is ~4x smaller than the 16-bit RGBA format but has only
/// 256 distinct values instead of 65536.
///
/// ## GLSL Decoding
///
/// ```glsl
/// vec4 texel = texture2D(uDataTexture, vTexCoord);
/// float normalized = texel.r;  // Already 0-1 in GLSL
/// float physical_value = normalized * (uMaxValue - uMinValue) + uMinValue;
/// bool valid = texel.a > 0.5;
/// ```
pub struct DataPng8BitEncoder {
    /// Minimum value for normalization
    pub min_value: f32,
    /// Maximum value for normalization
    pub max_value: f32,
}

impl DataPng8BitEncoder {
    /// Create a new 8-bit encoder with the given value range
    pub fn new(min_value: f32, max_value: f32) -> Self {
        Self {
            min_value,
            max_value,
        }
    }

    /// Create an encoder by computing min/max from the data
    pub fn from_data(data: &[Option<f32>]) -> Self {
        let (min_val, max_val) = compute_data_range(data);
        Self::new(min_val, max_val)
    }

    /// Encode grid data to an 8-bit Grayscale+Alpha PNG
    pub fn encode_with_metadata(
        &self,
        data: &[Option<f32>],
        width: usize,
        height: usize,
        parameter_name: &str,
        units: &str,
        bbox: [f64; 4],
    ) -> Result<EncodedDataPng, String> {
        // Validate dimensions
        if width > MAX_PNG_DIMENSION || height > MAX_PNG_DIMENSION {
            return Err(format!(
                "PNG dimensions {}x{} exceed maximum {}x{}",
                width, height, MAX_PNG_DIMENSION, MAX_PNG_DIMENSION
            ));
        }

        if data.len() != width * height {
            return Err(format!(
                "Data length {} does not match dimensions {}x{}={}",
                data.len(),
                width,
                height,
                width * height
            ));
        }

        // Encode data to Grayscale+Alpha pixels
        let pixels = self.encode_to_gray_alpha(data);

        // Build metadata
        let metadata = DataPngMetadata::new(self.min_value, self.max_value)
            .with_parameter(parameter_name)
            .with_units(units)
            .with_bbox(bbox[0], bbox[1], bbox[2], bbox[3])
            .with_dimensions(width as u32, height as u32);

        let png_bytes = self.create_png_with_metadata(&pixels, width, height, &metadata)?;

        Ok(EncodedDataPng {
            png_bytes,
            metadata,
        })
    }

    /// Encode data values to Grayscale+Alpha pixel buffer
    fn encode_to_gray_alpha(&self, data: &[Option<f32>]) -> Vec<u8> {
        let mut pixels = Vec::with_capacity(data.len() * 2);
        let range = self.max_value - self.min_value;

        for value in data {
            match value {
                Some(v) if !v.is_nan() => {
                    // Normalize to 0.0-1.0
                    let normalized = if range > 0.0 {
                        ((v - self.min_value) / range).clamp(0.0, 1.0)
                    } else {
                        0.5 // If min == max, use middle value
                    };

                    // Convert to 8-bit
                    let gray = (normalized * 255.0) as u8;
                    let alpha = 255u8; // Valid data

                    pixels.push(gray);
                    pixels.push(alpha);
                }
                _ => {
                    // No data - transparent pixel
                    pixels.push(0);
                    pixels.push(0);
                }
            }
        }

        pixels
    }

    /// Create 8-bit Grayscale+Alpha PNG with embedded tEXt metadata chunks
    fn create_png_with_metadata(
        &self,
        pixels: &[u8],
        width: usize,
        height: usize,
        metadata: &DataPngMetadata,
    ) -> Result<Vec<u8>, String> {
        let mut png = Vec::new();

        // PNG signature
        png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

        // IHDR chunk
        let mut ihdr_data = Vec::with_capacity(13);
        ihdr_data.extend_from_slice(&(width as u32).to_be_bytes());
        ihdr_data.extend_from_slice(&(height as u32).to_be_bytes());
        ihdr_data.push(8); // bit depth (8-bit)
        ihdr_data.push(4); // color type (Grayscale+Alpha)
        ihdr_data.push(0); // compression method
        ihdr_data.push(0); // filter method
        ihdr_data.push(0); // interlace method
        write_chunk(&mut png, b"IHDR", &ihdr_data);

        // tEXt chunks with metadata (must come before IDAT)
        write_text_chunk(&mut png, "EDR:encoding", "uint8");

        if !metadata.parameter_name.is_empty() {
            write_text_chunk(&mut png, "EDR:parameter", &metadata.parameter_name);
        }

        if !metadata.units.is_empty() {
            write_text_chunk(&mut png, "EDR:units", &metadata.units);
        }

        write_text_chunk(&mut png, "EDR:min", &format!("{}", metadata.min_value));
        write_text_chunk(&mut png, "EDR:max", &format!("{}", metadata.max_value));

        // Only write bbox if it's been set (not all zeros)
        if metadata.bbox != [0.0, 0.0, 0.0, 0.0] {
            write_text_chunk(
                &mut png,
                "EDR:bbox",
                &format!(
                    "{},{},{},{}",
                    metadata.bbox[0], metadata.bbox[1], metadata.bbox[2], metadata.bbox[3]
                ),
            );
        }

        write_text_chunk(&mut png, "EDR:width", &format!("{}", metadata.width));
        write_text_chunk(&mut png, "EDR:height", &format!("{}", metadata.height));

        // IDAT chunk (compressed image data)
        let idat_data = deflate_idat_gray_alpha(pixels, width, height)
            .map_err(|e| format!("IDAT compression failed: {}", e))?;
        write_chunk(&mut png, b"IDAT", &idat_data);

        // IEND chunk
        write_chunk(&mut png, b"IEND", &[]);

        Ok(png)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let encoder = DataPngEncoder::new(-50.0, 50.0);
        assert_eq!(encoder.min_value, -50.0);
        assert_eq!(encoder.max_value, 50.0);
    }

    #[test]
    fn test_encoder_from_data() {
        let data = vec![Some(10.0), Some(20.0), None, Some(15.0)];
        let encoder = DataPngEncoder::from_data(&data);
        assert_eq!(encoder.min_value, 10.0);
        assert_eq!(encoder.max_value, 20.0);
    }

    #[test]
    fn test_compute_data_range() {
        let data = vec![Some(-5.0), Some(10.0), None, Some(5.0)];
        let (min, max) = compute_data_range(&data);
        assert_eq!(min, -5.0);
        assert_eq!(max, 10.0);
    }

    #[test]
    fn test_compute_data_range_all_none() {
        let data: Vec<Option<f32>> = vec![None, None, None];
        let (min, max) = compute_data_range(&data);
        assert_eq!(min, 0.0);
        assert_eq!(max, 1.0);
    }

    #[test]
    fn test_compute_data_range_same_values() {
        let data = vec![Some(5.0), Some(5.0), Some(5.0)];
        let (min, max) = compute_data_range(&data);
        assert_eq!(min, 4.5);
        assert_eq!(max, 5.5);
    }

    #[test]
    fn test_16bit_encoding_values() {
        let encoder = DataPngEncoder::new(0.0, 100.0);

        // Test minimum value (0.0) -> should encode to 0x0000
        // 16-bit grayscale+alpha: [gray_hi, gray_lo, alpha_hi, alpha_lo]
        let data = vec![Some(0.0)];
        let pixels = encoder.encode_to_gray_alpha_16bit(&data);
        assert_eq!(pixels[0], 0); // Gray high byte = 0
        assert_eq!(pixels[1], 0); // Gray low byte = 0
        assert_eq!(pixels[2], 0xFF); // Alpha high byte = 0xFF (valid)
        assert_eq!(pixels[3], 0xFF); // Alpha low byte = 0xFF (valid)

        // Test maximum value (100.0) -> should encode to 0xFFFF
        let data = vec![Some(100.0)];
        let pixels = encoder.encode_to_gray_alpha_16bit(&data);
        assert_eq!(pixels[0], 255); // Gray high byte = 0xFF
        assert_eq!(pixels[1], 255); // Gray low byte = 0xFF
        assert_eq!(pixels[2], 0xFF); // Alpha high byte = 0xFF (valid)
        assert_eq!(pixels[3], 0xFF); // Alpha low byte = 0xFF (valid)

        // Test middle value (50.0) -> should encode to ~0x7FFF (32767)
        let data = vec![Some(50.0)];
        let pixels = encoder.encode_to_gray_alpha_16bit(&data);
        // 50/100 * 65535 = 32767.5 -> 32767 = 0x7FFF
        assert_eq!(pixels[0], 127); // Gray high byte = 0x7F
        assert_eq!(pixels[1], 255); // Gray low byte = 0xFF
        assert_eq!(pixels[2], 0xFF); // Alpha high byte
        assert_eq!(pixels[3], 0xFF); // Alpha low byte
    }

    #[test]
    fn test_null_values_transparent() {
        let encoder = DataPngEncoder::new(0.0, 100.0);
        let data = vec![None];
        let pixels = encoder.encode_to_gray_alpha_16bit(&data);

        assert_eq!(pixels[0], 0); // Gray high
        assert_eq!(pixels[1], 0); // Gray low
        assert_eq!(pixels[2], 0); // Alpha high = transparent
        assert_eq!(pixels[3], 0); // Alpha low = transparent
    }

    #[test]
    fn test_nan_values_transparent() {
        let encoder = DataPngEncoder::new(0.0, 100.0);
        let data = vec![Some(f32::NAN)];
        let pixels = encoder.encode_to_gray_alpha_16bit(&data);

        assert_eq!(pixels[2], 0); // Alpha high = transparent
        assert_eq!(pixels[3], 0); // Alpha low = transparent
    }

    #[test]
    fn test_encode_basic() {
        let encoder = DataPngEncoder::new(0.0, 100.0);
        let data = vec![Some(50.0), Some(25.0), None, Some(75.0)];

        let result = encoder.encode(&data, 2, 2);
        assert!(result.is_ok());

        let encoded = result.unwrap();
        assert!(!encoded.png_bytes.is_empty());
        assert_eq!(encoded.metadata.width, 2);
        assert_eq!(encoded.metadata.height, 2);
        assert_eq!(encoded.metadata.min_value, 0.0);
        assert_eq!(encoded.metadata.max_value, 100.0);
    }

    #[test]
    fn test_encode_with_metadata() {
        let encoder = DataPngEncoder::new(-50.0, 50.0);
        let data = vec![Some(0.0); 4];

        let result =
            encoder.encode_with_metadata(&data, 2, 2, "wind_u", "m/s", [-100.0, 35.0, -98.0, 37.0]);
        assert!(result.is_ok());

        let encoded = result.unwrap();
        assert_eq!(encoded.metadata.parameter_name, "wind_u");
        assert_eq!(encoded.metadata.units, "m/s");
        assert_eq!(encoded.metadata.bbox, [-100.0, 35.0, -98.0, 37.0]);
    }

    #[test]
    fn test_dimension_validation() {
        let encoder = DataPngEncoder::new(0.0, 100.0);
        let data = vec![Some(50.0); 4];

        // Valid dimensions
        assert!(encoder.encode(&data, 2, 2).is_ok());

        // Invalid: exceeds max dimension
        let result = encoder.encode(&data, MAX_PNG_DIMENSION + 1, 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceed maximum"));
    }

    #[test]
    fn test_data_length_validation() {
        let encoder = DataPngEncoder::new(0.0, 100.0);
        let data = vec![Some(50.0); 4];

        // Mismatched length
        let result = encoder.encode(&data, 3, 2);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not match"));
    }

    #[test]
    fn test_png_signature() {
        let encoder = DataPngEncoder::new(0.0, 100.0);
        let data = vec![Some(50.0); 4];

        let result = encoder.encode(&data, 2, 2).unwrap();
        let bytes = &result.png_bytes;

        // Check PNG signature
        assert_eq!(&bytes[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn test_contains_text_chunks() {
        let encoder = DataPngEncoder::new(-10.0, 10.0);
        let data = vec![Some(0.0); 4];

        let result = encoder
            .encode_with_metadata(&data, 2, 2, "temperature", "K", [-100.0, 35.0, -98.0, 37.0])
            .unwrap();

        let png_str = String::from_utf8_lossy(&result.png_bytes);

        // Check that tEXt chunks are present
        assert!(png_str.contains("EDR:encoding"));
        assert!(png_str.contains("EDR:parameter"));
        assert!(png_str.contains("EDR:min"));
        assert!(png_str.contains("EDR:max"));
    }

    #[test]
    fn test_value_clamping() {
        let encoder = DataPngEncoder::new(0.0, 100.0);

        // Value below minimum should clamp to 0
        let data = vec![Some(-50.0)];
        let pixels = encoder.encode_to_gray_alpha_16bit(&data);
        assert_eq!(pixels[0], 0); // Gray high
        assert_eq!(pixels[1], 0); // Gray low
        assert_eq!(pixels[2], 0xFF); // Alpha high = valid (not masked)
        assert_eq!(pixels[3], 0xFF); // Alpha low = valid (not masked)

        // Value above maximum should clamp to max
        let data = vec![Some(150.0)];
        let pixels = encoder.encode_to_gray_alpha_16bit(&data);
        assert_eq!(pixels[0], 255); // Gray high
        assert_eq!(pixels[1], 255); // Gray low
    }

    #[test]
    fn test_empty_transparent_png() {
        let encoder = DataPngEncoder::new(0.0, 100.0);
        let data: Vec<Option<f32>> = vec![None; 4];

        let result = encoder.encode(&data, 2, 2);
        assert!(result.is_ok());

        // All pixels should be transparent
        let encoded = result.unwrap();
        assert!(!encoded.png_bytes.is_empty());
    }

    #[test]
    fn test_metadata_builder() {
        let metadata = DataPngMetadata::new(-50.0, 50.0)
            .with_parameter("wind_u")
            .with_units("m/s")
            .with_bbox(-100.0, 35.0, -98.0, 37.0)
            .with_dimensions(256, 256);

        assert_eq!(metadata.parameter_name, "wind_u");
        assert_eq!(metadata.units, "m/s");
        assert_eq!(metadata.bbox, [-100.0, 35.0, -98.0, 37.0]);
        assert_eq!(metadata.width, 256);
        assert_eq!(metadata.height, 256);
        assert_eq!(metadata.min_value, -50.0);
        assert_eq!(metadata.max_value, 50.0);
    }

    // ============================================
    // 8-bit encoder tests
    // ============================================

    #[test]
    fn test_8bit_encoder_creation() {
        let encoder = DataPng8BitEncoder::new(-50.0, 50.0);
        assert_eq!(encoder.min_value, -50.0);
        assert_eq!(encoder.max_value, 50.0);
    }

    #[test]
    fn test_8bit_encoder_from_data() {
        let data = vec![Some(10.0), Some(20.0), None, Some(15.0)];
        let encoder = DataPng8BitEncoder::from_data(&data);
        assert_eq!(encoder.min_value, 10.0);
        assert_eq!(encoder.max_value, 20.0);
    }

    #[test]
    fn test_8bit_encoding_values() {
        let encoder = DataPng8BitEncoder::new(0.0, 100.0);

        // Test minimum value (0.0) -> should encode to 0
        let data = vec![Some(0.0)];
        let pixels = encoder.encode_to_gray_alpha(&data);
        assert_eq!(pixels[0], 0); // Gray = 0
        assert_eq!(pixels[1], 255); // Alpha = valid

        // Test maximum value (100.0) -> should encode to 255
        let data = vec![Some(100.0)];
        let pixels = encoder.encode_to_gray_alpha(&data);
        assert_eq!(pixels[0], 255); // Gray = 255
        assert_eq!(pixels[1], 255); // Alpha = valid

        // Test middle value (50.0) -> should encode to ~127
        let data = vec![Some(50.0)];
        let pixels = encoder.encode_to_gray_alpha(&data);
        assert_eq!(pixels[0], 127); // Gray = 127 (50/100 * 255 = 127.5)
        assert_eq!(pixels[1], 255); // Alpha = valid
    }

    #[test]
    fn test_8bit_null_values_transparent() {
        let encoder = DataPng8BitEncoder::new(0.0, 100.0);
        let data = vec![None];
        let pixels = encoder.encode_to_gray_alpha(&data);

        assert_eq!(pixels[0], 0); // Gray
        assert_eq!(pixels[1], 0); // Alpha = transparent
    }

    #[test]
    fn test_8bit_nan_values_transparent() {
        let encoder = DataPng8BitEncoder::new(0.0, 100.0);
        let data = vec![Some(f32::NAN)];
        let pixels = encoder.encode_to_gray_alpha(&data);

        assert_eq!(pixels[1], 0); // Alpha = transparent
    }

    #[test]
    fn test_8bit_encode_with_metadata() {
        let encoder = DataPng8BitEncoder::new(-50.0, 50.0);
        let data = vec![Some(0.0); 4];

        let result =
            encoder.encode_with_metadata(&data, 2, 2, "wind_u", "m/s", [-100.0, 35.0, -98.0, 37.0]);
        assert!(result.is_ok());

        let encoded = result.unwrap();
        assert!(!encoded.png_bytes.is_empty());
        assert_eq!(encoded.metadata.parameter_name, "wind_u");
        assert_eq!(encoded.metadata.units, "m/s");
        assert_eq!(encoded.metadata.bbox, [-100.0, 35.0, -98.0, 37.0]);
        assert_eq!(encoded.metadata.width, 2);
        assert_eq!(encoded.metadata.height, 2);
    }

    #[test]
    fn test_8bit_png_signature() {
        let encoder = DataPng8BitEncoder::new(0.0, 100.0);
        let data = vec![Some(50.0); 4];

        let result = encoder
            .encode_with_metadata(&data, 2, 2, "test", "", [0.0, 0.0, 0.0, 0.0])
            .unwrap();
        let bytes = &result.png_bytes;

        // Check PNG signature
        assert_eq!(&bytes[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn test_8bit_contains_uint8_encoding() {
        let encoder = DataPng8BitEncoder::new(-10.0, 10.0);
        let data = vec![Some(0.0); 4];

        let result = encoder
            .encode_with_metadata(&data, 2, 2, "temperature", "K", [-100.0, 35.0, -98.0, 37.0])
            .unwrap();

        let png_str = String::from_utf8_lossy(&result.png_bytes);

        // Check that encoding is marked as uint8
        assert!(png_str.contains("EDR:encoding"));
        assert!(png_str.contains("uint8"));
    }

    #[test]
    fn test_8bit_smaller_than_16bit() {
        let data: Vec<Option<f32>> = (0..256 * 256).map(|i| Some(i as f32 / 65536.0)).collect();

        let encoder_16 = DataPngEncoder::new(0.0, 1.0);
        let encoder_8 = DataPng8BitEncoder::new(0.0, 1.0);

        let result_16 = encoder_16
            .encode_with_metadata(&data, 256, 256, "test", "", [0.0, 0.0, 1.0, 1.0])
            .unwrap();
        let result_8 = encoder_8
            .encode_with_metadata(&data, 256, 256, "test", "", [0.0, 0.0, 1.0, 1.0])
            .unwrap();

        // 8-bit should be significantly smaller (roughly half due to 2 channels vs 4)
        assert!(
            result_8.png_bytes.len() < result_16.png_bytes.len(),
            "8-bit PNG ({} bytes) should be smaller than 16-bit PNG ({} bytes)",
            result_8.png_bytes.len(),
            result_16.png_bytes.len()
        );
    }
}
