//! Time handling utilities for meteorological data.

use chrono::{DateTime, Duration, NaiveDateTime, Utc, TimeZone};
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
        Self { reference_time, forecast_hour }
    }

    /// Create from analysis time (forecast_hour = 0)
    pub fn analysis(reference_time: DateTime<Utc>) -> Self {
        Self { reference_time, forecast_hour: 0 }
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
        if let Ok(ndt) = NaiveDateTime::parse_from_str(&format!("{}T00:00:00", s), "%Y-%m-%dT%H:%M:%S") {
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
        &[ModelCycle::Z00, ModelCycle::Z06, ModelCycle::Z12, ModelCycle::Z18]
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
    fn test_parse_wms_time_range() {
        let spec = TimeRange::from_wms_time("2024-01-15T00:00:00Z/2024-01-16T00:00:00Z").unwrap();
        match spec {
            TimeSpec::Range(_r) => {
                // Successfully parsed as range
            }
            _ => panic!("Expected range"),
        }
    }

    #[test]
    fn test_valid_time_storage_path() {
        let vt = ValidTime::new(
            Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap(),
            6
        );
        assert_eq!(vt.storage_path(), "2024/01/15/12z/006");
    }
}
