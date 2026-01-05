//! NDFD (National Digital Forecast Database) file handling.
//!
//! NDFD files from the NWS Telecommunications Gateway wrap GRIB2 messages
//! with WMO bulletin headers. This module provides functions to strip these
//! headers and extract the raw GRIB2 data.
//!
//! # NDFD File Format
//!
//! NDFD files have the following structure:
//!
//! ```text
//! ****NNNNNNNNNN****\n           <- File flag field separator (19 bytes)
//!                                   NNNNNNNNNN = file size, right-justified with leading zeros
//! TTAAII CCCC DDHHMM\r\r\n       <- WMO super header (21 bytes)
//!                                   TTAAII = WMO header, CCCC = origin, DDHHMM = date/time
//! ****NNNNNNNNNN****\n           <- Bulletin 1 flag field separator (19 bytes)
//! TTAAII CCCC DDHHMM\r\r\n       <- Bulletin 1 WMO header (21 bytes)
//! GRIB...7777                    <- GRIB2 message
//! ****NNNNNNNNNN****\n           <- Bulletin 2 flag field separator (19 bytes)
//! TTAAII CCCC DDHHMM\r\r\n       <- Bulletin 2 WMO header (21 bytes)
//! GRIB...7777                    <- GRIB2 message
//! ...
//! ```
//!
//! # Example
//!
//! ```no_run
//! use grib2_parser::ndfd::{strip_wmo_headers, NdfdReader};
//! use bytes::Bytes;
//!
//! // Method 1: Strip all headers and get concatenated GRIB2 data
//! let ndfd_data = std::fs::read("ds.temp.bin").unwrap();
//! let grib2_data = strip_wmo_headers(&ndfd_data);
//!
//! // Method 2: Iterate over individual GRIB2 messages
//! let ndfd_data = Bytes::from(std::fs::read("ds.temp.bin").unwrap());
//! let reader = NdfdReader::new(ndfd_data);
//! for grib2_message in reader {
//!     println!("Found GRIB2 message of {} bytes", grib2_message.len());
//! }
//! ```

use bytes::Bytes;

/// Magic bytes that identify the start of a GRIB2 message.
const GRIB_MAGIC: &[u8; 4] = b"GRIB";

/// End marker for GRIB2 messages.
const GRIB_END: &[u8; 4] = b"7777";

/// Flag field separator prefix used in NDFD files.
const FLAG_FIELD_PREFIX: &[u8; 4] = b"****";

/// Strip WMO bulletin headers from NDFD data and return concatenated GRIB2 messages.
///
/// This function handles NDFD files from the NWS Telecommunications Gateway
/// which wrap GRIB2 messages with WMO bulletin headers and flag field separators.
///
/// # Arguments
///
/// * `data` - Raw NDFD file data
///
/// # Returns
///
/// A vector containing all GRIB2 messages concatenated together, with all
/// WMO headers and flag field separators removed. This can be passed directly
/// to `Grib2Reader::new()`.
///
/// # Example
///
/// ```no_run
/// use grib2_parser::{strip_wmo_headers, Grib2Reader, Grib2Tables};
/// use bytes::Bytes;
/// use std::sync::Arc;
///
/// let ndfd_data = std::fs::read("ds.temp.bin").unwrap();
/// let grib2_data = strip_wmo_headers(&ndfd_data);
///
/// let tables = Arc::new(Grib2Tables::new());
/// let mut reader = Grib2Reader::new(Bytes::from(grib2_data), tables);
/// ```
pub fn strip_wmo_headers(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        // Find next GRIB magic
        if let Some(grib_start) = find_grib_magic(&data[offset..]) {
            let abs_start = offset + grib_start;

            // Parse the GRIB2 message length from section 0
            // Bytes 8-15 contain the 64-bit message length, but we use bytes 12-15 (lower 32 bits)
            if abs_start + 16 <= data.len() {
                let msg_len = u32::from_be_bytes([
                    data[abs_start + 12],
                    data[abs_start + 13],
                    data[abs_start + 14],
                    data[abs_start + 15],
                ]) as usize;

                if abs_start + msg_len <= data.len() {
                    // Verify this message ends with "7777"
                    let msg_end = abs_start + msg_len;
                    if msg_end >= 4 && &data[msg_end - 4..msg_end] == GRIB_END {
                        // Valid GRIB2 message - copy it
                        result.extend_from_slice(&data[abs_start..msg_end]);
                        offset = msg_end;
                        continue;
                    }
                }
            }

            // If we couldn't parse length, try to find the end marker
            if let Some(end_pos) = find_grib_end(&data[abs_start..]) {
                let msg_end = abs_start + end_pos + 4;
                result.extend_from_slice(&data[abs_start..msg_end]);
                offset = msg_end;
                continue;
            }
        }

        // Move past the current byte and try again
        offset += 1;
    }

    result
}

/// Check if data appears to be in NDFD format (has WMO headers).
///
/// Returns `true` if the data starts with the NDFD flag field separator pattern.
pub fn is_ndfd_format(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == FLAG_FIELD_PREFIX
}

/// Check if data is raw GRIB2 format (starts with "GRIB").
pub fn is_grib2_format(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == GRIB_MAGIC
}

/// Find the offset of the GRIB magic bytes in the data.
fn find_grib_magic(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == GRIB_MAGIC)
}

/// Find the offset of the GRIB end marker ("7777") in the data.
fn find_grib_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == GRIB_END)
}

/// Reader for NDFD files that iterates over individual GRIB2 messages.
///
/// Unlike `strip_wmo_headers` which concatenates all messages, this reader
/// yields each GRIB2 message separately as `Bytes`, which is useful when
/// you need to process messages individually.
///
/// # Example
///
/// ```no_run
/// use grib2_parser::NdfdReader;
/// use bytes::Bytes;
///
/// let data = Bytes::from(std::fs::read("ds.temp.bin").unwrap());
/// let reader = NdfdReader::new(data);
///
/// for (index, grib2_msg) in reader.enumerate() {
///     println!("Message {}: {} bytes", index, grib2_msg.len());
/// }
/// ```
pub struct NdfdReader {
    data: Bytes,
    offset: usize,
}

impl NdfdReader {
    /// Create a new NDFD reader from raw file data.
    ///
    /// The data can be either NDFD format (with WMO headers) or raw GRIB2 format.
    pub fn new(data: Bytes) -> Self {
        Self { data, offset: 0 }
    }

    /// Get the total size of the data.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Get the current reading position.
    pub fn position(&self) -> usize {
        self.offset
    }

    /// Check if there's more data to read.
    pub fn has_more(&self) -> bool {
        self.offset < self.data.len()
    }

    /// Extract the next GRIB2 message from the data.
    fn next_grib_message(&mut self) -> Option<Bytes> {
        if self.offset >= self.data.len() {
            return None;
        }

        let remaining = &self.data[self.offset..];

        // Find the next GRIB magic
        let grib_offset = find_grib_magic(remaining)?;
        let abs_start = self.offset + grib_offset;

        // Parse message length from section 0
        if abs_start + 16 > self.data.len() {
            self.offset = self.data.len();
            return None;
        }

        let msg_len = u32::from_be_bytes([
            self.data[abs_start + 12],
            self.data[abs_start + 13],
            self.data[abs_start + 14],
            self.data[abs_start + 15],
        ]) as usize;

        let msg_end = abs_start + msg_len;
        if msg_end > self.data.len() {
            self.offset = self.data.len();
            return None;
        }

        // Verify end marker
        if msg_end < 4 || &self.data[msg_end - 4..msg_end] != GRIB_END {
            // Try to find end marker manually
            if let Some(end_offset) = find_grib_end(&self.data[abs_start..]) {
                let actual_end = abs_start + end_offset + 4;
                if actual_end <= self.data.len() {
                    self.offset = actual_end;
                    return Some(self.data.slice(abs_start..actual_end));
                }
            }
            self.offset = self.data.len();
            return None;
        }

        self.offset = msg_end;
        Some(self.data.slice(abs_start..msg_end))
    }
}

impl Iterator for NdfdReader {
    type Item = Bytes;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_grib_message()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_ndfd_format() {
        assert!(is_ndfd_format(b"****0042656952****\n"));
        assert!(!is_ndfd_format(b"GRIB"));
        assert!(!is_ndfd_format(b""));
    }

    #[test]
    fn test_is_grib2_format() {
        assert!(is_grib2_format(b"GRIB\x00\x00\x00\x02"));
        assert!(!is_grib2_format(b"****0042656952"));
        assert!(!is_grib2_format(b""));
    }

    #[test]
    fn test_find_grib_magic() {
        assert_eq!(find_grib_magic(b"GRIB"), Some(0));
        assert_eq!(find_grib_magic(b"xxxGRIB"), Some(3));
        assert_eq!(find_grib_magic(b"xxx"), None);
    }

    #[test]
    fn test_find_grib_end() {
        assert_eq!(find_grib_end(b"7777"), Some(0));
        assert_eq!(find_grib_end(b"xxx7777"), Some(3));
        assert_eq!(find_grib_end(b"xxx"), None);
    }

    #[test]
    fn test_strip_simple_ndfd() {
        // Create a minimal NDFD-like structure with one GRIB2 message
        // Flag field + WMO header + GRIB2 message
        let mut ndfd_data = Vec::new();

        // File flag field separator: ****0000000040****\n
        ndfd_data.extend_from_slice(b"****0000000040****\n");

        // WMO super header: YEUZ98 KWBN 051447\r\r\n
        ndfd_data.extend_from_slice(b"YEUZ98 KWBN 051447\r\r\n");

        // Bulletin flag field separator
        ndfd_data.extend_from_slice(b"****0000000020****\n");

        // Bulletin WMO header
        ndfd_data.extend_from_slice(b"YEUB16 KWBN 051447\r\r\n");

        // Minimal GRIB2 message (16 bytes + "7777")
        // Section 0: GRIB + 2 bytes reserved + discipline + edition + 8 bytes length
        let mut grib2_msg = vec![0u8; 20];
        grib2_msg[0..4].copy_from_slice(b"GRIB");
        grib2_msg[6] = 0; // discipline
        grib2_msg[7] = 2; // edition
                          // Message length = 20 (in bytes 8-15, but we use 12-15)
        grib2_msg[12] = 0;
        grib2_msg[13] = 0;
        grib2_msg[14] = 0;
        grib2_msg[15] = 20;
        // End marker
        grib2_msg[16..20].copy_from_slice(b"7777");

        ndfd_data.extend_from_slice(&grib2_msg);

        // Strip headers
        let result = strip_wmo_headers(&ndfd_data);

        // Should get just the GRIB2 message
        assert_eq!(result.len(), 20);
        assert_eq!(&result[0..4], b"GRIB");
        assert_eq!(&result[16..20], b"7777");
    }

    #[test]
    fn test_ndfd_reader_iteration() {
        // Create data with two GRIB2 messages
        let mut data = Vec::new();

        // First GRIB2 message
        let mut msg1 = vec![0u8; 20];
        msg1[0..4].copy_from_slice(b"GRIB");
        msg1[7] = 2;
        msg1[15] = 20;
        msg1[16..20].copy_from_slice(b"7777");
        data.extend_from_slice(&msg1);

        // Some padding (simulating WMO headers between messages)
        data.extend_from_slice(b"****0000000020****\nXXXXXX XXXX XXXXXX\r\r\n");

        // Second GRIB2 message
        let mut msg2 = vec![0u8; 24];
        msg2[0..4].copy_from_slice(b"GRIB");
        msg2[7] = 2;
        msg2[15] = 24;
        msg2[20..24].copy_from_slice(b"7777");
        data.extend_from_slice(&msg2);

        let reader = NdfdReader::new(Bytes::from(data));
        let messages: Vec<_> = reader.collect();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].len(), 20);
        assert_eq!(messages[1].len(), 24);
    }
}
