//! Freshness metrics for the trail-conditions data.
//!
//! The trail-physics worker runs unattended, so "has this quietly stopped
//! producing data?" has to be answerable from monitoring, not by someone
//! noticing stale colors on a map. Two failure modes look identical to a
//! user (`?conditions=latest` stops advancing) but have different owners, so
//! both are exported:
//!
//! - `trail_conditions_latest_valid_timestamp_seconds` -- the `valid_time` a
//!   `latest` query currently reports (what users see).
//! - `trail_physics_last_processed_timestamp_seconds` -- when the worker last
//!   finished an hour (is the *worker* alive).
//!
//! **Exported as timestamps, not ages, on purpose.** An age gauge computed
//! here would FREEZE at its last value if this updater task ever failed --
//! the age would stop growing and a broken monitor would look healthy. A
//! timestamp can't lie that way: Prometheus computes
//! `time() - <timestamp>`, so a stalled updater makes the age grow, which is
//! exactly when an alert should fire. The query-failure path likewise leaves
//! the previous timestamp in place rather than writing a fake value.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use metrics::gauge;

use crate::state::AppState;

pub const LATEST_VALID_GAUGE: &str = "trail_conditions_latest_valid_timestamp_seconds";
pub const WORKER_PROGRESS_GAUGE: &str = "trail_physics_last_processed_timestamp_seconds";

/// How often the gauges are refreshed. Both queries are index-backed / tiny
/// (see `SegmentConditionsCatalog::latest_valid_time`), so this can be short;
/// it only needs to be well under the alert thresholds (hours).
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// Unix seconds for a timestamp, as the f64 a Prometheus gauge needs.
pub fn to_gauge_value(ts: DateTime<Utc>) -> f64 {
    ts.timestamp() as f64
}

/// Set a timestamp gauge, but only when there is a real value. `None` (no
/// rows yet, or the query failed) deliberately writes NOTHING: a fabricated
/// 0 would read as "stale since 1970" and a stale previous value is the
/// honest one (see the module docs).
pub fn record_timestamp(name: &'static str, ts: Option<DateTime<Utc>>) -> Option<f64> {
    let value = to_gauge_value(ts?);
    gauge!(name).set(value);
    Some(value)
}

/// Spawn the background refresher. Safe to call once at startup.
pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(REFRESH_INTERVAL);
        loop {
            interval.tick().await;

            match state.segment_conditions_catalog.latest_valid_time().await {
                Ok(ts) => {
                    record_timestamp(LATEST_VALID_GAUGE, ts);
                }
                Err(e) => tracing::warn!("freshness: latest_valid_time failed: {}", e),
            }
            match state
                .segment_conditions_catalog
                .last_worker_progress()
                .await
            {
                Ok(ts) => {
                    record_timestamp(WORKER_PROGRESS_GAUGE, ts);
                }
                Err(e) => tracing::warn!("freshness: last_worker_progress failed: {}", e),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn gauge_value_is_unix_seconds() {
        let ts = Utc.with_ymd_and_hms(2026, 10, 5, 21, 0, 0).unwrap();
        assert_eq!(to_gauge_value(ts), 1_791_234_000.0);
    }

    #[test]
    fn missing_timestamp_records_nothing() {
        // No rows yet / query failed: must not write a fake value (a 0
        // would look like 'stale since 1970').
        assert_eq!(record_timestamp(LATEST_VALID_GAUGE, None), None);
    }

    #[test]
    fn present_timestamp_is_recorded_and_returned() {
        let ts = Utc.with_ymd_and_hms(2026, 10, 5, 21, 0, 0).unwrap();
        assert_eq!(
            record_timestamp(WORKER_PROGRESS_GAUGE, Some(ts)),
            Some(1_791_234_000.0)
        );
    }

    #[test]
    fn metric_names_match_the_alert_rules() {
        // deploy/prometheus/alerts.yml references these literally; a rename
        // here without a matching rules edit would silently disable the alert.
        let rules = include_str!("../../../deploy/prometheus/alerts.yml");
        assert!(rules.contains(LATEST_VALID_GAUGE));
        assert!(rules.contains(WORKER_PROGRESS_GAUGE));
    }
}
