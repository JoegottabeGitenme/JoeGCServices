//! GRIB index file (.idx) parser for selective downloads.
//!
//! NOAA provides `.idx` files alongside GRIB2 files that list the byte offset
//! of each parameter/message. This module parses these files to enable
//! downloading only specific parameters via HTTP Range requests.
//!
//! # Index File Format
//!
//! Each line has the format:
//! ```text
//! message_number:byte_offset:d=YYYYMMDDHH:PARAMETER:level:type:
//! ```
//!
//! Example:
//! ```text
//! 1:0:d=2026011500:PRMSL:mean sea level:anl:
//! 580:407331265:d=2026011500:TMP:2 m above ground:anl:
//! ```
//!
//! # Usage
//!
//! ```ignore
//! let index = GribIndex::parse(idx_content, Some(file_size))?;
//! let filters = vec![
//!     ParamFilter::new("TMP", "2 m above ground"),
//!     ParamFilter::new("UGRD", "10 m above ground"),
//! ];
//! let ranges = index.get_byte_ranges(&filters);
//! ```

use anyhow::{anyhow, Context, Result};
use tracing::{debug, warn};

/// A single entry from a GRIB index file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// Message number (1-based line number in index)
    pub message_number: u32,
    /// Byte offset where this message starts in the GRIB file
    pub byte_offset: u64,
    /// Reference date string (e.g., "d=2026011500")
    pub date: String,
    /// Parameter short name (e.g., "TMP", "UGRD")
    pub parameter: String,
    /// Level description (e.g., "2 m above ground", "850 mb")
    pub level: String,
    /// Forecast type (e.g., "anl", "6 hour fcst")
    pub forecast_type: String,
}

impl IndexEntry {
    /// Parse a single line from an index file.
    ///
    /// Format: `message_number:byte_offset:d=YYYYMMDDHH:PARAMETER:level:type:`
    pub fn parse(line: &str) -> Result<Self> {
        let parts: Vec<&str> = line.split(':').collect();

        // Need at least 6 parts (last one may be empty after trailing colon)
        if parts.len() < 6 {
            return Err(anyhow!(
                "Invalid index line format: expected 6+ colon-separated fields, got {}",
                parts.len()
            ));
        }

        let message_number = parts[0]
            .parse::<u32>()
            .context("Failed to parse message number")?;

        let byte_offset = parts[1]
            .parse::<u64>()
            .context("Failed to parse byte offset")?;

        let date = parts[2].to_string();
        let parameter = parts[3].to_string();
        let level = parts[4].to_string();
        let forecast_type = parts[5].to_string();

        Ok(Self {
            message_number,
            byte_offset,
            date,
            parameter,
            level,
            forecast_type,
        })
    }
}

/// Filter for matching parameters in the index.
#[derive(Debug, Clone)]
pub struct ParamFilter {
    /// Parameter short name to match (e.g., "TMP")
    pub parameter: String,
    /// Level string to match (e.g., "2 m above ground")
    pub level: String,
}

impl ParamFilter {
    /// Create a new parameter filter.
    pub fn new(parameter: impl Into<String>, level: impl Into<String>) -> Self {
        Self {
            parameter: parameter.into(),
            level: level.into(),
        }
    }

    /// Check if an index entry matches this filter.
    pub fn matches(&self, entry: &IndexEntry) -> bool {
        entry.parameter == self.parameter && entry.level == self.level
    }
}

/// A byte range to download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// Start byte (inclusive)
    pub start: u64,
    /// End byte (inclusive)
    pub end: u64,
}

impl ByteRange {
    /// Create a new byte range.
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// Size of this range in bytes.
    pub fn size(&self) -> u64 {
        self.end - self.start + 1
    }

    /// Check if this range is adjacent to or overlaps with another.
    /// Two ranges are considered adjacent if they are within `gap_threshold` bytes.
    pub fn can_merge_with(&self, other: &ByteRange, gap_threshold: u64) -> bool {
        if self.end >= other.start {
            // Overlapping or adjacent
            true
        } else {
            // Check if gap is within threshold
            other.start - self.end <= gap_threshold + 1
        }
    }

    /// Merge this range with another, returning the combined range.
    pub fn merge(&self, other: &ByteRange) -> ByteRange {
        ByteRange {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Format as HTTP Range header value.
    pub fn to_http_range(&self) -> String {
        format!("bytes={}-{}", self.start, self.end)
    }
}

/// Parsed GRIB index file with utilities for selective downloading.
#[derive(Debug, Clone)]
pub struct GribIndex {
    /// All entries from the index file
    entries: Vec<IndexEntry>,
    /// Total file size (used to calculate last message's end byte)
    file_size: Option<u64>,
}

impl GribIndex {
    /// Parse index file content.
    ///
    /// # Arguments
    /// * `content` - Raw text content of the .idx file
    /// * `file_size` - Optional total size of the GRIB file (for last message range)
    pub fn parse(content: &str, file_size: Option<u64>) -> Result<Self> {
        let mut entries = Vec::new();
        let mut parse_errors = 0;

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            match IndexEntry::parse(line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    parse_errors += 1;
                    debug!(
                        line_num = line_num + 1,
                        line = line,
                        error = %e,
                        "Failed to parse index line, skipping"
                    );
                }
            }
        }

        if entries.is_empty() {
            return Err(anyhow!("No valid entries found in index file"));
        }

        if parse_errors > 0 {
            warn!(
                parse_errors = parse_errors,
                total_lines = entries.len() + parse_errors,
                "Some index lines could not be parsed"
            );
        }

        // Sort by byte offset to ensure correct order
        entries.sort_by_key(|e| e.byte_offset);

        Ok(Self { entries, file_size })
    }

    /// Get number of entries in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if index is empty.
    #[allow(dead_code)] // Used in tests and potentially useful for callers
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Find all entries matching the given filters.
    pub fn find_matching_entries(&self, filters: &[ParamFilter]) -> Vec<&IndexEntry> {
        self.entries
            .iter()
            .filter(|entry| filters.iter().any(|f| f.matches(entry)))
            .collect()
    }

    /// Calculate byte ranges for the given parameter filters.
    ///
    /// Returns a list of byte ranges that cover all matching parameters.
    /// Each range corresponds to one GRIB message.
    pub fn get_byte_ranges(&self, filters: &[ParamFilter]) -> Vec<ByteRange> {
        let matching = self.find_matching_entries(filters);
        if matching.is_empty() {
            return Vec::new();
        }

        let mut ranges = Vec::new();

        for entry in &matching {
            // Find the end byte by looking at the next entry's start
            let end_byte = self.get_message_end_byte(entry);
            ranges.push(ByteRange::new(entry.byte_offset, end_byte));
        }

        // Sort ranges by start byte
        ranges.sort_by_key(|r| r.start);

        ranges
    }

    /// Get byte ranges with adjacent ranges merged.
    ///
    /// # Arguments
    /// * `filters` - Parameter filters to match
    /// * `gap_threshold` - Maximum gap (bytes) between ranges to merge (0 = only merge adjacent)
    #[allow(dead_code)] // Useful utility method for future optimizations
    pub fn get_merged_byte_ranges(
        &self,
        filters: &[ParamFilter],
        gap_threshold: u64,
    ) -> Vec<ByteRange> {
        let ranges = self.get_byte_ranges(filters);
        Self::merge_ranges(ranges, gap_threshold)
    }

    /// Merge adjacent or nearby ranges.
    pub fn merge_ranges(mut ranges: Vec<ByteRange>, gap_threshold: u64) -> Vec<ByteRange> {
        if ranges.len() <= 1 {
            return ranges;
        }

        // Sort by start byte
        ranges.sort_by_key(|r| r.start);

        let mut merged = Vec::new();
        let mut current = ranges[0];

        for range in ranges.into_iter().skip(1) {
            if current.can_merge_with(&range, gap_threshold) {
                current = current.merge(&range);
            } else {
                merged.push(current);
                current = range;
            }
        }
        merged.push(current);

        merged
    }

    /// Get the end byte for a message.
    ///
    /// This is calculated as the byte before the next message starts,
    /// or the last byte of the file for the final message.
    ///
    /// Uses binary search for O(log n) performance since entries are sorted by byte_offset.
    fn get_message_end_byte(&self, entry: &IndexEntry) -> u64 {
        // Binary search for the entry's position
        let search_result = self
            .entries
            .binary_search_by_key(&entry.byte_offset, |e| e.byte_offset);

        let current_idx = match search_result {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1), // Entry not found exactly, use nearest
        };

        // If there's a next entry, use its offset - 1
        if current_idx + 1 < self.entries.len() {
            return self.entries[current_idx + 1].byte_offset.saturating_sub(1);
        }

        // This is the last message - use file size if available
        if let Some(size) = self.file_size {
            // Use saturating_sub to handle edge case where size is 0
            size.saturating_sub(1)
        } else {
            // If we don't know file size, we must estimate. Log a warning since
            // this could download too much or too little data.
            warn!(
                byte_offset = entry.byte_offset,
                "No file size available for last message, using 10MB estimate"
            );
            entry.byte_offset + 10_000_000 // 10MB estimate
        }
    }

    /// Get all unique parameters in the index.
    #[allow(dead_code)] // Useful for debugging and introspection
    pub fn parameters(&self) -> Vec<String> {
        let mut params: Vec<String> = self.entries.iter().map(|e| e.parameter.clone()).collect();
        params.sort();
        params.dedup();
        params
    }

    /// Get all unique levels for a given parameter.
    #[allow(dead_code)] // Useful for debugging and introspection
    pub fn levels_for_parameter(&self, parameter: &str) -> Vec<String> {
        let mut levels: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.parameter == parameter)
            .map(|e| e.level.clone())
            .collect();
        levels.sort();
        levels.dedup();
        levels
    }

    /// Get entries as a slice.
    #[allow(dead_code)] // Useful for debugging and introspection
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }
}

/// Convert GRIB2 level code and value to index file level string.
///
/// This maps from our YAML config format to the string format used in .idx files.
pub fn level_to_idx_string(level_code: u8, level_value: Option<u32>) -> Option<String> {
    match level_code {
        // Surface
        1 => Some("surface".to_string()),

        // Isobaric (pressure level in Pa, display as mb)
        100 => {
            let mb = level_value? / 100;
            Some(format!("{} mb", mb))
        }

        // Mean sea level
        101 => Some("mean sea level".to_string()),

        // Height above ground
        103 => {
            let height = level_value?;
            Some(format!("{} m above ground", height))
        }

        // Entire atmosphere
        200 => Some("entire atmosphere".to_string()),

        // Cloud layers
        214 => Some("low cloud layer".to_string()),
        224 => Some("middle cloud layer".to_string()),
        234 => Some("high cloud layer".to_string()),

        // Planetary boundary layer
        220 => Some("planetary boundary layer".to_string()),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_INDEX: &str = r#"
1:0:d=2026011500:PRMSL:mean sea level:anl:
9:3325868:d=2026011500:REFC:entire atmosphere:anl:
14:6377780:d=2026011500:GUST:surface:anl:
580:407331265:d=2026011500:TMP:2 m above ground:anl:
582:409395882:d=2026011500:DPT:2 m above ground:anl:
585:411277095:d=2026011500:UGRD:10 m above ground:anl:
586:412239265:d=2026011500:VGRD:10 m above ground:anl:
"#;

    #[test]
    fn test_parse_entry() {
        let entry =
            IndexEntry::parse("580:407331265:d=2026011500:TMP:2 m above ground:anl:").unwrap();

        assert_eq!(entry.message_number, 580);
        assert_eq!(entry.byte_offset, 407331265);
        assert_eq!(entry.date, "d=2026011500");
        assert_eq!(entry.parameter, "TMP");
        assert_eq!(entry.level, "2 m above ground");
        assert_eq!(entry.forecast_type, "anl");
    }

    #[test]
    fn test_parse_entry_invalid() {
        assert!(IndexEntry::parse("invalid").is_err());
        assert!(IndexEntry::parse("1:2:3").is_err());
        assert!(IndexEntry::parse("not_a_number:0:d=2026:TMP:surface:anl:").is_err());
    }

    #[test]
    fn test_parse_index() {
        let index = GribIndex::parse(SAMPLE_INDEX, Some(500_000_000)).unwrap();

        assert_eq!(index.len(), 7);
        assert!(!index.is_empty());
    }

    #[test]
    fn test_find_matching_entries() {
        let index = GribIndex::parse(SAMPLE_INDEX, Some(500_000_000)).unwrap();

        let filters = vec![
            ParamFilter::new("TMP", "2 m above ground"),
            ParamFilter::new("UGRD", "10 m above ground"),
        ];

        let matches = index.find_matching_entries(&filters);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].parameter, "TMP");
        assert_eq!(matches[1].parameter, "UGRD");
    }

    #[test]
    fn test_get_byte_ranges() {
        let index = GribIndex::parse(SAMPLE_INDEX, Some(500_000_000)).unwrap();

        let filters = vec![ParamFilter::new("TMP", "2 m above ground")];

        let ranges = index.get_byte_ranges(&filters);
        assert_eq!(ranges.len(), 1);

        // TMP starts at 407331265, DPT (next) starts at 409395882
        assert_eq!(ranges[0].start, 407331265);
        assert_eq!(ranges[0].end, 409395881); // One before next message
    }

    #[test]
    fn test_byte_range_size() {
        let range = ByteRange::new(100, 199);
        assert_eq!(range.size(), 100);
    }

    #[test]
    fn test_merge_adjacent_ranges() {
        let ranges = vec![
            ByteRange::new(0, 99),
            ByteRange::new(100, 199), // Adjacent to previous
            ByteRange::new(500, 599), // Gap
        ];

        let merged = GribIndex::merge_ranges(ranges, 0);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].start, 0);
        assert_eq!(merged[0].end, 199);
        assert_eq!(merged[1].start, 500);
        assert_eq!(merged[1].end, 599);
    }

    #[test]
    fn test_merge_with_gap_threshold() {
        let ranges = vec![
            ByteRange::new(0, 99),
            ByteRange::new(150, 199), // 50 byte gap
        ];

        // Without threshold - not merged
        let merged = GribIndex::merge_ranges(ranges.clone(), 0);
        assert_eq!(merged.len(), 2);

        // With threshold - merged
        let merged = GribIndex::merge_ranges(ranges, 100);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].start, 0);
        assert_eq!(merged[0].end, 199);
    }

    #[test]
    fn test_http_range_format() {
        let range = ByteRange::new(407331265, 409395881);
        assert_eq!(range.to_http_range(), "bytes=407331265-409395881");
    }

    #[test]
    fn test_level_to_idx_string() {
        assert_eq!(level_to_idx_string(1, None), Some("surface".to_string()));
        assert_eq!(
            level_to_idx_string(100, Some(85000)),
            Some("850 mb".to_string())
        );
        assert_eq!(
            level_to_idx_string(100, Some(50000)),
            Some("500 mb".to_string())
        );
        assert_eq!(
            level_to_idx_string(101, None),
            Some("mean sea level".to_string())
        );
        assert_eq!(
            level_to_idx_string(103, Some(2)),
            Some("2 m above ground".to_string())
        );
        assert_eq!(
            level_to_idx_string(103, Some(10)),
            Some("10 m above ground".to_string())
        );
        assert_eq!(
            level_to_idx_string(200, None),
            Some("entire atmosphere".to_string())
        );
    }

    #[test]
    fn test_parameters() {
        let index = GribIndex::parse(SAMPLE_INDEX, None).unwrap();
        let params = index.parameters();

        assert!(params.contains(&"TMP".to_string()));
        assert!(params.contains(&"UGRD".to_string()));
        assert!(params.contains(&"PRMSL".to_string()));
    }

    #[test]
    fn test_levels_for_parameter() {
        let index = GribIndex::parse(
            "1:0:d=2026:TMP:2 m above ground:anl:\n2:100:d=2026:TMP:850 mb:anl:\n3:200:d=2026:TMP:500 mb:anl:",
            None,
        )
        .unwrap();

        let levels = index.levels_for_parameter("TMP");
        assert_eq!(levels.len(), 3);
        assert!(levels.contains(&"2 m above ground".to_string()));
        assert!(levels.contains(&"850 mb".to_string()));
        assert!(levels.contains(&"500 mb".to_string()));
    }

    #[test]
    fn test_last_message_uses_file_size() {
        let index = GribIndex::parse("1:0:d=2026:TMP:surface:anl:", Some(1000)).unwrap();

        let ranges = index.get_byte_ranges(&[ParamFilter::new("TMP", "surface")]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, 999); // file_size - 1
    }

    #[test]
    fn test_param_filter_matches() {
        let filter = ParamFilter::new("TMP", "2 m above ground");
        let entry = IndexEntry::parse("1:0:d=2026:TMP:2 m above ground:anl:").unwrap();

        assert!(filter.matches(&entry));

        let non_match = IndexEntry::parse("1:0:d=2026:TMP:850 mb:anl:").unwrap();
        assert!(!filter.matches(&non_match));
    }

    // =========================================================================
    // Edge Case Tests
    // =========================================================================

    #[test]
    fn test_empty_index_file() {
        // Empty content should fail
        let result = GribIndex::parse("", None);
        assert!(result.is_err());

        // Whitespace only should fail
        let result = GribIndex::parse("   \n\n  ", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_single_entry_index() {
        // Single entry with file size
        let index = GribIndex::parse("1:0:d=2026:TMP:surface:anl:", Some(5000)).unwrap();

        assert_eq!(index.len(), 1);
        assert!(!index.is_empty());

        let ranges = index.get_byte_ranges(&[ParamFilter::new("TMP", "surface")]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, 4999); // file_size - 1
    }

    #[test]
    fn test_single_entry_no_file_size() {
        // Single entry without file size - should use fallback estimate
        let index = GribIndex::parse("1:1000:d=2026:TMP:surface:anl:", None).unwrap();

        let ranges = index.get_byte_ranges(&[ParamFilter::new("TMP", "surface")]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 1000);
        // Should use the 10MB fallback estimate
        assert_eq!(ranges[0].end, 1000 + 10_000_000);
    }

    #[test]
    fn test_zero_byte_offset_entry() {
        // Entry with byte_offset = 0 (first message in file)
        let index = GribIndex::parse(
            "1:0:d=2026:TMP:surface:anl:\n2:1000:d=2026:RH:surface:anl:",
            Some(5000),
        )
        .unwrap();

        let ranges = index.get_byte_ranges(&[ParamFilter::new("TMP", "surface")]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, 999); // Next message starts at 1000, so end is 999
    }

    #[test]
    fn test_adjacent_messages() {
        // Two adjacent messages (byte offsets are consecutive)
        let index = GribIndex::parse(
            "1:0:d=2026:TMP:surface:anl:\n2:100:d=2026:RH:surface:anl:",
            Some(200),
        )
        .unwrap();

        // Get both
        let filters = vec![
            ParamFilter::new("TMP", "surface"),
            ParamFilter::new("RH", "surface"),
        ];
        let ranges = index.get_byte_ranges(&filters);
        assert_eq!(ranges.len(), 2);

        // First: 0-99
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, 99);

        // Second: 100-199
        assert_eq!(ranges[1].start, 100);
        assert_eq!(ranges[1].end, 199);

        // When merged, should become single range
        let merged = GribIndex::merge_ranges(ranges, 0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].start, 0);
        assert_eq!(merged[0].end, 199);
    }

    #[test]
    fn test_no_matching_filters() {
        let index = GribIndex::parse(SAMPLE_INDEX, Some(500_000_000)).unwrap();

        let filters = vec![ParamFilter::new("NONEXISTENT", "surface")];

        let matches = index.find_matching_entries(&filters);
        assert!(matches.is_empty());

        let ranges = index.get_byte_ranges(&filters);
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_malformed_lines_skipped() {
        // Mix of valid and invalid lines - invalid should be skipped
        let content = r#"
1:0:d=2026:TMP:surface:anl:
invalid line here
2:1000:d=2026:RH:surface:anl:
also:bad
3:2000:d=2026:PRES:surface:anl:
"#;
        let index = GribIndex::parse(content, Some(5000)).unwrap();

        // Should have parsed 3 valid entries
        assert_eq!(index.len(), 3);
    }

    #[test]
    fn test_saturating_sub_on_zero() {
        // Verify ByteRange handles edge cases with saturating operations
        let range = ByteRange::new(0, 0);
        assert_eq!(range.size(), 1); // 0-0 inclusive is 1 byte
        assert_eq!(range.to_http_range(), "bytes=0-0");
    }

    #[test]
    fn test_file_size_zero() {
        // Edge case: file_size is 0 (should use saturating_sub)
        let index = GribIndex::parse("1:0:d=2026:TMP:surface:anl:", Some(0)).unwrap();

        let ranges = index.get_byte_ranges(&[ParamFilter::new("TMP", "surface")]);
        assert_eq!(ranges.len(), 1);
        // saturating_sub(1) on 0 should give 0, not underflow
        assert_eq!(ranges[0].end, 0);
    }

    #[test]
    fn test_binary_search_correctness() {
        // Verify binary search finds correct end bytes for various positions
        let content = r#"
1:0:d=2026:A:surface:anl:
2:1000:d=2026:B:surface:anl:
3:2000:d=2026:C:surface:anl:
4:3000:d=2026:D:surface:anl:
5:4000:d=2026:E:surface:anl:
"#;
        let index = GribIndex::parse(content, Some(5000)).unwrap();

        // First entry
        let ranges = index.get_byte_ranges(&[ParamFilter::new("A", "surface")]);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, 999);

        // Middle entry
        let ranges = index.get_byte_ranges(&[ParamFilter::new("C", "surface")]);
        assert_eq!(ranges[0].start, 2000);
        assert_eq!(ranges[0].end, 2999);

        // Last entry
        let ranges = index.get_byte_ranges(&[ParamFilter::new("E", "surface")]);
        assert_eq!(ranges[0].start, 4000);
        assert_eq!(ranges[0].end, 4999); // Uses file_size
    }
}
