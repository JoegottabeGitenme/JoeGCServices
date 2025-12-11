//! GRIB2 parser implementation (WMO FM 92 GRIB Edition 2).
//!
//! This crate provides a pure Rust implementation for parsing GRIB2 files,
//! the standard format for meteorological data exchange.
//!
//! # Overview
//!
//! GRIB2 files contain one or more messages, each representing a weather field.
//! Each message consists of 8 sections:
//!
//! - Section 0: Indicator (16 bytes) - File identifier and message length
//! - Section 1: Identification (21+ bytes) - Model, reference time, etc.
//! - Section 2: Local Use (optional) - Implementation-specific data
//! - Section 3: Grid Definition (72+ bytes) - Lat/lon grid, dimensions
//! - Section 4: Product Definition (34+ bytes) - Parameter, level, forecast hour
//! - Section 5: Data Representation (21+ bytes) - Packing method, scale factors
//! - Section 6: Bitmap (optional) - Missing value indicators
//! - Section 7: Data (~1.2 MB for GFS) - Compressed grid values
//! - Section 8: End (4 bytes) - "7777" terminator
//!
//! # Example
//!
//! ```ignore
//! use grib2_parser::Grib2Reader;
//! use bytes::Bytes;
//!
//! let data = std::fs::read("gfs.grib2").unwrap();
//! let reader = Grib2Reader::new(Bytes::from(data));
//!
//! for message in reader.iter_messages() {
//!     println!("Parameter: {}", message.product_definition.parameter_short_name);
//!     println!("Level: {}", message.product_definition.level);
//!     println!("Forecast hour: {}", message.product_definition.forecast_hour);
//! }
//! ```

pub mod sections;
pub mod unpacking;

pub use unpacking::unpack_simple;

use bytes::Bytes;
use thiserror::Error;

/// Result type for GRIB2 parser operations.
pub type Grib2Result<T> = Result<T, Grib2Error>;

/// Error types for GRIB2 parsing.
#[derive(Error, Debug)]
pub enum Grib2Error {
    /// Invalid GRIB2 message format
    #[error("Invalid GRIB2 format: {0}")]
    InvalidFormat(String),

    /// Unexpected end of file
    #[error("Unexpected end of data")]
    UnexpectedEnd,

    /// Invalid section number or format
    #[error("Invalid section {section}: {reason}")]
    InvalidSection { section: u8, reason: String },

    /// Unsupported template
    #[error("Unsupported template {template_number}: {reason}")]
    UnsupportedTemplate { template_number: u16, reason: String },

    /// Data unpacking error
    #[error("Data unpacking failed: {0}")]
    UnpackingError(String),

    /// Parsing error with position
    #[error("Parse error at offset {offset}: {reason}")]
    ParseError { offset: usize, reason: String },

    /// Invalid coordinate or grid definition
    #[error("Invalid grid: {0}")]
    InvalidGrid(String),

    /// Other errors
    #[error("{0}")]
    Other(String),
}

/// A single GRIB2 message containing weather data.
#[derive(Debug, Clone)]
pub struct Grib2Message {
    /// Message offset in file (bytes)
    pub offset: usize,

    /// Indicator section (Section 0)
    pub indicator: sections::Indicator,

    /// Identification section (Section 1)
    pub identification: sections::Identification,

    /// Grid definition section (Section 3)
    pub grid_definition: sections::GridDefinition,

    /// Product definition section (Section 4)
    pub product_definition: sections::ProductDefinition,

    /// Data representation section (Section 5)
    pub data_representation: sections::DataRepresentation,

    /// Bitmap section (Section 6) - None if not present
    pub bitmap: Option<sections::Bitmap>,

    /// Raw data section (Section 7)
    pub data_section: sections::DataSection,

    /// The complete message data (for full reconstruction if needed)
    pub raw_data: Bytes,
}

impl Grib2Message {
    /// Get the valid time (reference time + forecast hour)
    pub fn valid_time(&self) -> chrono::DateTime<chrono::Utc> {
        let offset = chrono::Duration::hours(self.product_definition.forecast_hour as i64);
        self.identification.reference_time + offset
    }

    /// Get parameter short name
    pub fn parameter(&self) -> &str {
        &self.product_definition.parameter_short_name
    }

    /// Get level description
    pub fn level(&self) -> &str {
        &self.product_definition.level_description
    }

    /// Get grid dimensions
    pub fn grid_dims(&self) -> (u32, u32) {
        (
            self.grid_definition.num_points_latitude,
            self.grid_definition.num_points_longitude,
        )
    }

    /// Unpack the grid data values using the external `grib` crate.
    /// 
    /// This method now uses the mature `grib` crate which supports:
    /// - Template 5.0: Simple packing
    /// - Template 5.2: Complex packing
    /// - Template 5.3: Complex packing with spatial differencing
    /// - Template 5.40/5.41: JPEG 2000 compression (if enabled)
    /// - Template 5.15: PNG compression (enabled by default)
    pub fn unpack_data(&self) -> Grib2Result<Vec<f32>> {
        use std::io::Cursor;
        
        // Use the grib crate to parse and decode the message
        let cursor = Cursor::new(self.raw_data.as_ref());
        let grib_file = grib::from_reader(cursor)
            .map_err(|e| Grib2Error::UnpackingError(format!("Failed to parse with grib crate: {}", e)))?;
        
        // Get the first (and should be only) submessage and decode immediately
        if let Some((_index, submessage)) = grib_file.iter().next() {
            // Create decoder and dispatch
            let decoder = grib::Grib2SubmessageDecoder::from(submessage)
                .map_err(|e| Grib2Error::UnpackingError(format!("Failed to create decoder: {}", e)))?;
            
            let values: Vec<f32> = decoder.dispatch()
                .map_err(|e| Grib2Error::UnpackingError(format!("Failed to decode values: {}", e)))?
                .collect();
            
            return Ok(values);
        }
        
        Err(Grib2Error::UnpackingError("No submessage found in GRIB data".to_string()))
    }
}

/// GRIB2 file reader that iterates over messages.
pub struct Grib2Reader {
    data: Bytes,
    current_offset: usize,
}

impl Grib2Reader {
    /// Create a new GRIB2 reader from raw bytes.
    pub fn new(data: Bytes) -> Self {
        Self {
            data,
            current_offset: 0,
        }
    }

    /// Get the total file size in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Get current reading position.
    pub fn position(&self) -> usize {
        self.current_offset
    }

    /// Check if there's more data to read.
    pub fn has_more(&self) -> bool {
        self.current_offset < self.data.len()
    }

    /// Read and parse the next GRIB2 message.
    pub fn next_message(&mut self) -> Grib2Result<Option<Grib2Message>> {
        // Check if we're at the end
        if self.current_offset >= self.data.len() {
            return Ok(None);
        }

        let message_offset = self.current_offset;
        let remaining = &self.data[self.current_offset..];

        // Parse Section 0 (Indicator)
        if remaining.len() < 16 {
            return Err(Grib2Error::InvalidFormat(
                "Not enough data for Section 0".to_string(),
            ));
        }

        let indicator = sections::parse_indicator(remaining)
            .map_err(|e| Grib2Error::ParseError {
                offset: message_offset,
                reason: format!("Failed to parse indicator: {}", e),
            })?;

        let message_length = indicator.message_length as usize;
        if message_length < 16 {
            return Err(Grib2Error::InvalidFormat(
                "Message length too short".to_string(),
            ));
        }

        if message_offset + message_length > self.data.len() {
            return Err(Grib2Error::UnexpectedEnd);
        }

        let message_data = &self.data[message_offset..message_offset + message_length];

        // Parse all sections - use the full message data for the parsers
        let identification =
            sections::parse_identification(message_data).map_err(|e| Grib2Error::ParseError {
                offset: message_offset + 16,
                reason: format!("Failed to parse identification: {}", e),
            })?;

        let grid_definition =
            sections::parse_grid_definition(message_data).map_err(|e| {
                Grib2Error::ParseError {
                    offset: message_offset + 16,
                    reason: format!("Failed to parse grid definition: {}", e),
                }
            })?;

        let product_definition = sections::parse_product_definition(message_data, indicator.discipline)
            .map_err(|e| Grib2Error::ParseError {
                offset: message_offset + 16,
                reason: format!("Failed to parse product definition: {}", e),
            })?;

        let data_representation =
            sections::parse_data_representation(message_data).map_err(|e| {
                Grib2Error::ParseError {
                    offset: message_offset + 16,
                    reason: format!("Failed to parse data representation: {}", e),
                }
            })?;

        let bitmap = sections::parse_bitmap(message_data).ok();

        let data_section = sections::parse_data_section(message_data).map_err(|e| {
            Grib2Error::ParseError {
                offset: message_offset + 16,
                reason: format!("Failed to parse data section: {}", e),
            }
        })?;

        // Verify end section
        if message_data.len() < 4
            || &message_data[message_data.len() - 4..] != b"7777"
        {
            return Err(Grib2Error::InvalidFormat(
                "Message does not end with '7777'".to_string(),
            ));
        }

        let message = Grib2Message {
            offset: message_offset,
            indicator,
            identification,
            grid_definition,
            product_definition,
            data_representation,
            bitmap,
            data_section,
            raw_data: Bytes::copy_from_slice(message_data),
        };

        self.current_offset += message_length;
        Ok(Some(message))
    }

    /// Create an iterator over all messages in the file.
    pub fn iter_messages(&mut self) -> MessageIterator<'_> {
        MessageIterator { reader: self }
    }
}

/// Iterator over GRIB2 messages.
pub struct MessageIterator<'a> {
    reader: &'a mut Grib2Reader,
}

impl<'a> Iterator for MessageIterator<'a> {
    type Item = Grib2Result<Grib2Message>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.reader.next_message() {
            Ok(Some(msg)) => Some(Ok(msg)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grib2_magic() {
        let data = Bytes::from(&b"GRIB\x00\x00\x00\x02hello7777"[..]);
        let reader = Grib2Reader::new(data);
        assert_eq!(reader.size(), 17);
    }

    #[test]
    fn test_reader_position() {
        let data = Bytes::from(vec![0u8; 100]);
        let reader = Grib2Reader::new(data);
        assert_eq!(reader.position(), 0);
        assert!(reader.has_more());
    }
}
