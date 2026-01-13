//! Temporal interpolation utilities for EDR API queries.
//!
//! This module provides functions to interpolate weather data values between
//! available timesteps, enabling users to request data at arbitrary times
//! even when the underlying data is only available at discrete intervals.

use chrono::{DateTime, Duration, Utc};

/// Find the two times that bracket a target time in a sorted list of available times.
///
/// Returns `(before, after)` where:
/// - `before` is the latest time <= target
/// - `after` is the earliest time > target
///
/// Returns `(None, None)` if the available times list is empty.
/// Returns `(Some(time), None)` if target is at or after the last available time.
/// Returns `(None, Some(time))` if target is before the first available time.
pub fn find_bracketing_times(
    target: DateTime<Utc>,
    available_times: &[DateTime<Utc>],
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    if available_times.is_empty() {
        return (None, None);
    }

    // Find the first time that is > target
    let after_idx = available_times.iter().position(|&t| t > target);

    match after_idx {
        None => {
            // Target is at or after all available times
            (available_times.last().copied(), None)
        }
        Some(0) => {
            // Target is before all available times
            (None, Some(available_times[0]))
        }
        Some(idx) => {
            // Target is between two times
            (Some(available_times[idx - 1]), Some(available_times[idx]))
        }
    }
}

/// Calculate the interpolation weight for a target time between two bracketing times.
///
/// Returns a value between 0.0 and 1.0 representing the relative position of
/// the target time within the interval [before_time, after_time].
///
/// - Returns 0.0 if target == before_time
/// - Returns 1.0 if target == after_time
/// - Returns 0.5 if target is exactly halfway between
///
/// Panics if before_time >= after_time.
pub fn calculate_weight(
    before_time: DateTime<Utc>,
    after_time: DateTime<Utc>,
    target_time: DateTime<Utc>,
) -> f64 {
    assert!(before_time < after_time, "before_time must be < after_time");

    let total_duration = (after_time - before_time).num_seconds();
    let target_offset = (target_time - before_time).num_seconds();

    if total_duration == 0 {
        return 0.0;
    }

    (target_offset as f64) / (total_duration as f64)
}

/// Linearly interpolate between two f32 values.
///
/// Returns: `before_value * (1 - weight) + after_value * weight`
///
/// The weight should be between 0.0 and 1.0:
/// - weight = 0.0 returns before_value
/// - weight = 1.0 returns after_value
/// - weight = 0.5 returns the average
///
/// If either value is NaN or infinite, returns NaN.
pub fn linear_interpolate_f32(before_value: f32, after_value: f32, weight: f64) -> f32 {
    // Handle NaN and infinite values
    if !before_value.is_finite() || !after_value.is_finite() {
        return f32::NAN;
    }

    let before_weight = 1.0 - weight;
    (before_value as f64 * before_weight + after_value as f64 * weight) as f32
}

/// Expand a datetime interval into a list of times at regular step intervals.
///
/// Given a start time, end time, and step duration, this generates a list of
/// times starting at `start`, incrementing by `step`, up to and including `end`.
///
/// # Arguments
/// * `start` - The start of the interval
/// * `end` - The end of the interval (inclusive)
/// * `step` - The duration between each generated time
///
/// # Returns
/// A vector of DateTime values at step intervals. If step is zero or negative,
/// returns just the start time. If start > end, returns an empty vector.
///
/// # Example
/// ```
/// use chrono::{Duration, TimeZone, Utc};
/// use edr_api::temporal_interpolation::expand_interval_with_step;
///
/// // Generate times every 10 minutes from 00:00 to 00:30
/// let start = Utc.with_ymd_and_hms(2026, 1, 13, 0, 0, 0).unwrap();
/// let end = Utc.with_ymd_and_hms(2026, 1, 13, 0, 30, 0).unwrap();
/// let step = Duration::minutes(10);
/// let times = expand_interval_with_step(start, end, step);
/// assert_eq!(times.len(), 4);
/// // Result: [00:00, 00:10, 00:20, 00:30]
/// ```
pub fn expand_interval_with_step(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    step: Duration,
) -> Vec<DateTime<Utc>> {
    if start > end {
        return vec![];
    }

    if step <= Duration::zero() {
        return vec![start];
    }

    let mut times = Vec::new();
    let mut current = start;

    while current <= end {
        times.push(current);
        current = current + step;
    }

    times
}

/// Parse an ISO 8601 duration string into a chrono::Duration.
///
/// Supports formats like:
/// - PT10M (10 minutes)
/// - PT30M (30 minutes)
/// - PT1H (1 hour)
/// - PT1H30M (1 hour 30 minutes)
/// - P1D (1 day)
/// - P1DT12H (1 day 12 hours)
///
/// # Returns
/// Returns `Some(Duration)` if parsing succeeds, `None` otherwise.
pub fn parse_iso8601_duration(duration_str: &str) -> Option<Duration> {
    let s = duration_str.trim();

    if !s.starts_with('P') {
        return None;
    }

    let s = &s[1..]; // Remove 'P' prefix

    let (date_part, time_part) = if let Some(t_pos) = s.find('T') {
        (&s[..t_pos], Some(&s[t_pos + 1..]))
    } else {
        (s, None)
    };

    let mut total_seconds = 0i64;

    // Parse date part (days)
    if !date_part.is_empty() {
        if let Some(d_pos) = date_part.find('D') {
            if let Ok(days) = date_part[..d_pos].parse::<i64>() {
                total_seconds += days * 86400; // 24 * 60 * 60
            } else {
                return None;
            }
        }
    }

    // Parse time part (hours, minutes, seconds)
    if let Some(time_str) = time_part {
        let mut remainder = time_str;

        // Hours
        if let Some(h_pos) = remainder.find('H') {
            if let Ok(hours) = remainder[..h_pos].parse::<i64>() {
                total_seconds += hours * 3600;
                remainder = &remainder[h_pos + 1..];
            } else {
                return None;
            }
        }

        // Minutes
        if let Some(m_pos) = remainder.find('M') {
            if let Ok(minutes) = remainder[..m_pos].parse::<i64>() {
                total_seconds += minutes * 60;
                remainder = &remainder[m_pos + 1..];
            } else {
                return None;
            }
        }

        // Seconds
        if let Some(s_pos) = remainder.find('S') {
            if let Ok(seconds) = remainder[..s_pos].parse::<i64>() {
                total_seconds += seconds;
            } else {
                return None;
            }
        }
    }

    Some(Duration::seconds(total_seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_find_bracketing_times() {
        let times = vec![
            Utc.with_ymd_and_hms(2026, 1, 13, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 13, 3, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 13, 6, 0, 0).unwrap(),
        ];

        // Target in the middle
        let target = Utc.with_ymd_and_hms(2026, 1, 13, 4, 0, 0).unwrap();
        let (before, after) = find_bracketing_times(target, &times);
        assert_eq!(before, Some(times[1]));
        assert_eq!(after, Some(times[2]));

        // Target before all
        let target = Utc.with_ymd_and_hms(2026, 1, 12, 23, 0, 0).unwrap();
        let (before, after) = find_bracketing_times(target, &times);
        assert_eq!(before, None);
        assert_eq!(after, Some(times[0]));

        // Target after all
        let target = Utc.with_ymd_and_hms(2026, 1, 13, 12, 0, 0).unwrap();
        let (before, after) = find_bracketing_times(target, &times);
        assert_eq!(before, Some(times[2]));
        assert_eq!(after, None);

        // Target exactly on a time (should return that time as before, next as after)
        let target = times[1];
        let (before, after) = find_bracketing_times(target, &times);
        assert_eq!(before, Some(times[1])); // Latest time <= target is the time itself
        assert_eq!(after, Some(times[2]));
    }

    #[test]
    fn test_calculate_weight() {
        let before = Utc.with_ymd_and_hms(2026, 1, 13, 0, 0, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 1, 13, 3, 0, 0).unwrap();

        // Target at start
        let target = before;
        assert!((calculate_weight(before, after, target) - 0.0).abs() < 1e-6);

        // Target at end
        let target = after;
        assert!((calculate_weight(before, after, target) - 1.0).abs() < 1e-6);

        // Target in middle
        let target = Utc.with_ymd_and_hms(2026, 1, 13, 1, 30, 0).unwrap();
        assert!((calculate_weight(before, after, target) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_linear_interpolate_f32() {
        // Basic interpolation
        assert!((linear_interpolate_f32(0.0, 10.0, 0.0) - 0.0).abs() < 1e-6);
        assert!((linear_interpolate_f32(0.0, 10.0, 1.0) - 10.0).abs() < 1e-6);
        assert!((linear_interpolate_f32(0.0, 10.0, 0.5) - 5.0).abs() < 1e-6);
        assert!((linear_interpolate_f32(0.0, 10.0, 0.25) - 2.5).abs() < 1e-6);

        // NaN handling
        assert!(linear_interpolate_f32(f32::NAN, 10.0, 0.5).is_nan());
        assert!(linear_interpolate_f32(0.0, f32::NAN, 0.5).is_nan());
    }

    #[test]
    fn test_expand_interval_with_step() {
        let start = Utc.with_ymd_and_hms(2026, 1, 13, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 13, 0, 30, 0).unwrap();
        let step = Duration::minutes(10);

        let times = expand_interval_with_step(start, end, step);
        assert_eq!(times.len(), 4);
        assert_eq!(
            times[0],
            Utc.with_ymd_and_hms(2026, 1, 13, 0, 0, 0).unwrap()
        );
        assert_eq!(
            times[1],
            Utc.with_ymd_and_hms(2026, 1, 13, 0, 10, 0).unwrap()
        );
        assert_eq!(
            times[2],
            Utc.with_ymd_and_hms(2026, 1, 13, 0, 20, 0).unwrap()
        );
        assert_eq!(
            times[3],
            Utc.with_ymd_and_hms(2026, 1, 13, 0, 30, 0).unwrap()
        );
    }

    #[test]
    fn test_parse_iso8601_duration() {
        assert_eq!(parse_iso8601_duration("PT10M"), Some(Duration::minutes(10)));
        assert_eq!(parse_iso8601_duration("PT30M"), Some(Duration::minutes(30)));
        assert_eq!(parse_iso8601_duration("PT1H"), Some(Duration::hours(1)));
        assert_eq!(
            parse_iso8601_duration("PT1H30M"),
            Some(Duration::minutes(90))
        );
        assert_eq!(parse_iso8601_duration("P1D"), Some(Duration::days(1)));
        assert_eq!(parse_iso8601_duration("P1DT12H"), Some(Duration::hours(36)));
        assert_eq!(parse_iso8601_duration("invalid"), None);
    }
}
