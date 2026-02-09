//! Time handling utilities for meteorological data.

use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Represents a valid time for meteorological data.
///
/// Combines reference time (model run time) and forecast offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValidTime {
    /// Model run/reference time
    pub reference_time: DateTime<Utc>,
    /// Forecast hour offset from reference time
    pub forecast_hour: u32,
}

impl ValidTime {
    pub fn new(reference_time: DateTime<Utc>, forecast_hour: u32) -> Self {
        Self {
            reference_time,
            forecast_hour,
        }
    }

    /// Create from analysis time (forecast_hour = 0)
    pub fn analysis(reference_time: DateTime<Utc>) -> Self {
        Self {
            reference_time,
            forecast_hour: 0,
        }
    }

    /// Calculate the actual valid time (reference + forecast offset)
    pub fn valid_datetime(&self) -> DateTime<Utc> {
        self.reference_time + Duration::hours(self.forecast_hour as i64)
    }

    /// Parse from ISO 8601 string (returns valid_datetime interpretation)
    pub fn from_iso8601(s: &str) -> Result<DateTime<Utc>, TimeParseError> {
        // Try full datetime with timezone
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Ok(dt.with_timezone(&Utc));
        }

        // Try without timezone (assume UTC)
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
            return Ok(Utc.from_utc_datetime(&ndt));
        }

        // Try date only
        if let Ok(ndt) =
            NaiveDateTime::parse_from_str(&format!("{}T00:00:00", s), "%Y-%m-%dT%H:%M:%S")
        {
            return Ok(Utc.from_utc_datetime(&ndt));
        }

        Err(TimeParseError::InvalidFormat(s.to_string()))
    }

    /// Generate storage path component for this time
    pub fn storage_path(&self) -> String {
        format!(
            "{}/{:03}",
            self.reference_time.format("%Y/%m/%d/%Hz"),
            self.forecast_hour
        )
    }
}

/// A time range for queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeRange {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Self { start, end }
    }

    /// Parse WMS TIME parameter.
    ///
    /// Supports:
    /// - Single time: "2024-01-15T12:00:00Z"
    /// - Time range: "2024-01-15T00:00:00Z/2024-01-16T00:00:00Z"
    /// - Time list: "2024-01-15T00:00:00Z,2024-01-15T06:00:00Z,2024-01-15T12:00:00Z"
    pub fn from_wms_time(s: &str) -> Result<TimeSpec, TimeParseError> {
        if s.eq_ignore_ascii_case("current") {
            return Ok(TimeSpec::Current);
        }

        // Check for range (contains /)
        if let Some((start, end)) = s.split_once('/') {
            let start_dt = ValidTime::from_iso8601(start)?;
            let end_dt = ValidTime::from_iso8601(end)?;
            return Ok(TimeSpec::Range(TimeRange::new(start_dt, end_dt)));
        }

        // Check for list (contains ,)
        if s.contains(',') {
            let times: Result<Vec<_>, _> = s
                .split(',')
                .map(|t| ValidTime::from_iso8601(t.trim()))
                .collect();
            return Ok(TimeSpec::List(times?));
        }

        // Single time
        let dt = ValidTime::from_iso8601(s)?;
        Ok(TimeSpec::Single(dt))
    }

    pub fn contains(&self, dt: &DateTime<Utc>) -> bool {
        dt >= &self.start && dt <= &self.end
    }
}

/// Parsed TIME parameter specification.
#[derive(Debug, Clone)]
pub enum TimeSpec {
    /// Use current/latest available time
    Current,
    /// Single specific time
    Single(DateTime<Utc>),
    /// Time range (start/end)
    Range(TimeRange),
    /// Explicit list of times
    List(Vec<DateTime<Utc>>),
}

#[derive(Debug, thiserror::Error)]
pub enum TimeParseError {
    #[error("Invalid time format: {0}")]
    InvalidFormat(String),
}

/// Model run cycles (common for NWP models).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCycle {
    /// 00Z run
    Z00,
    /// 06Z run
    Z06,
    /// 12Z run
    Z12,
    /// 18Z run
    Z18,
}

impl ModelCycle {
    pub fn from_hour(hour: u32) -> Option<Self> {
        match hour {
            0 => Some(ModelCycle::Z00),
            6 => Some(ModelCycle::Z06),
            12 => Some(ModelCycle::Z12),
            18 => Some(ModelCycle::Z18),
            _ => None,
        }
    }

    pub fn hour(&self) -> u32 {
        match self {
            ModelCycle::Z00 => 0,
            ModelCycle::Z06 => 6,
            ModelCycle::Z12 => 12,
            ModelCycle::Z18 => 18,
        }
    }

    /// Get all cycles for models that run 4x daily
    pub fn all_4x_daily() -> &'static [ModelCycle] {
        &[
            ModelCycle::Z00,
            ModelCycle::Z06,
            ModelCycle::Z12,
            ModelCycle::Z18,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_parse_iso8601() {
        let dt = ValidTime::from_iso8601("2024-01-15T12:00:00Z").unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 12);
    }

    #[test]
    fn test_parse_iso8601_without_timezone() {
        let dt = ValidTime::from_iso8601("2024-01-15T12:00:00").unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.hour(), 12);
    }

    #[test]
    fn test_parse_iso8601_date_only() {
        let dt = ValidTime::from_iso8601("2024-01-15").unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 0);
    }

    #[test]
    fn test_parse_iso8601_invalid() {
        let err = ValidTime::from_iso8601("not-a-date");
        assert!(err.is_err());
        let err = err.unwrap_err();
        assert!(matches!(err, TimeParseError::InvalidFormat(_)));
    }

    #[test]
    fn test_parse_wms_time_range() {
        let spec = TimeRange::from_wms_time("2024-01-15T00:00:00Z/2024-01-16T00:00:00Z").unwrap();
        match spec {
            TimeSpec::Range(r) => {
                assert_eq!(r.start.day(), 15);
                assert_eq!(r.end.day(), 16);
            }
            _ => panic!("Expected range"),
        }
    }

    #[test]
    fn test_parse_wms_time_current() {
        let spec = TimeRange::from_wms_time("current").unwrap();
        assert!(matches!(spec, TimeSpec::Current));

        // Test case-insensitivity
        let spec2 = TimeRange::from_wms_time("CURRENT").unwrap();
        assert!(matches!(spec2, TimeSpec::Current));
    }

    #[test]
    fn test_parse_wms_time_single() {
        let spec = TimeRange::from_wms_time("2024-01-15T12:00:00Z").unwrap();
        match spec {
            TimeSpec::Single(dt) => {
                assert_eq!(dt.hour(), 12);
            }
            _ => panic!("Expected single time"),
        }
    }

    #[test]
    fn test_parse_wms_time_list() {
        let spec = TimeRange::from_wms_time(
            "2024-01-15T00:00:00Z,2024-01-15T06:00:00Z,2024-01-15T12:00:00Z",
        )
        .unwrap();
        match spec {
            TimeSpec::List(times) => {
                assert_eq!(times.len(), 3);
                assert_eq!(times[0].hour(), 0);
                assert_eq!(times[1].hour(), 6);
                assert_eq!(times[2].hour(), 12);
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_valid_time_storage_path() {
        let vt = ValidTime::new(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap(), 6);
        assert_eq!(vt.storage_path(), "2024/01/15/12z/006");
    }

    #[test]
    fn test_valid_time_analysis() {
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let vt = ValidTime::analysis(ref_time);
        assert_eq!(vt.reference_time, ref_time);
        assert_eq!(vt.forecast_hour, 0);
    }

    #[test]
    fn test_valid_time_valid_datetime() {
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let vt = ValidTime::new(ref_time, 6);
        let valid = vt.valid_datetime();
        assert_eq!(valid.hour(), 18); // 12 + 6 = 18
        assert_eq!(valid.day(), 15);
    }

    #[test]
    fn test_valid_time_valid_datetime_next_day() {
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 18, 0, 0).unwrap();
        let vt = ValidTime::new(ref_time, 12);
        let valid = vt.valid_datetime();
        assert_eq!(valid.hour(), 6); // 18 + 12 = 30 = 6 next day
        assert_eq!(valid.day(), 16);
    }

    #[test]
    fn test_time_range_new() {
        let start = Utc.with_ymd_and_hms(2024, 1, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 16, 0, 0, 0).unwrap();
        let range = TimeRange::new(start, end);
        assert_eq!(range.start, start);
        assert_eq!(range.end, end);
    }

    #[test]
    fn test_time_range_contains() {
        let start = Utc.with_ymd_and_hms(2024, 1, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 16, 0, 0, 0).unwrap();
        let range = TimeRange::new(start, end);

        // Test inside range
        let inside = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        assert!(range.contains(&inside));

        // Test at boundaries
        assert!(range.contains(&start));
        assert!(range.contains(&end));

        // Test outside range
        let before = Utc.with_ymd_and_hms(2024, 1, 14, 23, 0, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2024, 1, 16, 1, 0, 0).unwrap();
        assert!(!range.contains(&before));
        assert!(!range.contains(&after));
    }

    #[test]
    fn test_model_cycle_from_hour() {
        assert_eq!(ModelCycle::from_hour(0), Some(ModelCycle::Z00));
        assert_eq!(ModelCycle::from_hour(6), Some(ModelCycle::Z06));
        assert_eq!(ModelCycle::from_hour(12), Some(ModelCycle::Z12));
        assert_eq!(ModelCycle::from_hour(18), Some(ModelCycle::Z18));

        // Invalid hours
        assert_eq!(ModelCycle::from_hour(3), None);
        assert_eq!(ModelCycle::from_hour(9), None);
        assert_eq!(ModelCycle::from_hour(24), None);
    }

    #[test]
    fn test_model_cycle_hour() {
        assert_eq!(ModelCycle::Z00.hour(), 0);
        assert_eq!(ModelCycle::Z06.hour(), 6);
        assert_eq!(ModelCycle::Z12.hour(), 12);
        assert_eq!(ModelCycle::Z18.hour(), 18);
    }

    #[test]
    fn test_model_cycle_all_4x_daily() {
        let cycles = ModelCycle::all_4x_daily();
        assert_eq!(cycles.len(), 4);
        assert_eq!(cycles[0], ModelCycle::Z00);
        assert_eq!(cycles[1], ModelCycle::Z06);
        assert_eq!(cycles[2], ModelCycle::Z12);
        assert_eq!(cycles[3], ModelCycle::Z18);
    }

    #[test]
    fn test_time_parse_error_display() {
        let err = TimeParseError::InvalidFormat("bad-time".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("Invalid time format"));
        assert!(msg.contains("bad-time"));
    }
}
