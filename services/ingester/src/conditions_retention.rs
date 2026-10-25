//! Retention for `segment_conditions` (per-trail-segment condition rows written
//! hourly by the `trail-physics` worker).
//!
//! Nothing ever deleted a row, so the table grew ~1.6M rows a day to 11.4M rows /
//! 4.5 GB. Everything the API serves is recent: `?conditions=latest` wants the
//! newest row at or before now, and the per-trail series wants the last
//! `TIMESERIES_HISTORY_HOURS` (6) plus the forecast horizon. Rows older than that
//! only cost I/O -- measured on production, a 711-trail `conditions=latest` request
//! sorted 233,550 historical rows to return 711.
//!
//! ## Window
//!
//! `TRAIL_CONDITIONS_RETENTION_HOURS`, default 12, with a **floor** of
//! `TIMESERIES_HISTORY_HOURS + 2` (8 h): a smaller value would cut into the history
//! the series endpoint promises. A value below the floor is raised to it with a
//! warning, and a value that is not a positive integer falls back to the default
//! (it must never mean "delete everything"), exactly as in `obs_retention`.
//!
//! The cutoff is on `valid_time`, so future forecast hours are never touched.
//!
//! If the worker stops for longer than the window, `latest` has no row left for
//! the affected trails and they render uncolored, rather than showing conditions
//! that are many hours old as if they were current.
//!
//! The worker's own ledger (`trail_physics_progress`) is a separate table, so
//! deleting old rows does not make it reprocess old forecast hours.

use std::time::Duration;

use chrono::{DateTime, Utc};
use storage::segment_conditions::{SegmentConditionsCatalog, TIMESERIES_HISTORY_HOURS};
use tracing::{info, warn};

use crate::obs_retention::{resolve_hours, DELETE_BATCH_SIZE};

pub const ENV: &str = "TRAIL_CONDITIONS_RETENTION_HOURS";
pub const DEFAULT_HOURS: u64 = 12;
/// Never keep less than the series endpoint's history plus a 2 h margin.
pub const FLOOR_HOURS: u64 = TIMESERIES_HISTORY_HOURS as u64 + 2;

/// Pause between batches so the worker's own upserts are never starved.
const BATCH_PAUSE: Duration = Duration::from_millis(250);
/// How often a sweep runs (the first runs at startup).
const SWEEP_INTERVAL: Duration = Duration::from_secs(600);

/// Retention window from a lookup function (`std::env::var` in production),
/// logging any adjustment.
pub fn retention_hours_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> u64 {
    let (hours, warning) = resolve_hours(lookup(ENV).as_deref(), DEFAULT_HOURS, FLOOR_HOURS);
    if let Some(w) = warning {
        warn!(env = ENV, "{w}");
    }
    hours
}

pub fn retention_hours_from_env() -> u64 {
    retention_hours_from_lookup(|k| std::env::var(k).ok())
}

/// Delete every expired row, in batches. Returns the total deleted. Stops early
/// (returning what it did) if a batch fails, so one bad statement cannot spin;
/// the next sweep resumes.
pub async fn sweep(
    catalog: &SegmentConditionsCatalog,
    cutoff: DateTime<Utc>,
    batch_size: i64,
    pause: Duration,
) -> u64 {
    let mut total = 0u64;
    loop {
        match catalog.delete_valid_before_batch(cutoff, batch_size).await {
            Ok(n) => {
                total += n;
                if (n as i64) < batch_size {
                    return total;
                }
                tokio::time::sleep(pause).await;
            }
            Err(e) => {
                warn!(error = %e, deleted_so_far = total, "Trail conditions retention batch failed; will resume next sweep");
                return total;
            }
        }
    }
}

/// Run sweeps forever. The first runs immediately so a restart cleans up at once.
pub async fn run(catalog: SegmentConditionsCatalog, retention_hours: u64) {
    info!(retention_hours, "Trail conditions retention enabled");
    let mut tick = tokio::time::interval(SWEEP_INTERVAL);
    loop {
        tick.tick().await;
        let cutoff = Utc::now() - chrono::Duration::hours(retention_hours as i64);
        let deleted = sweep(&catalog, cutoff, DELETE_BATCH_SIZE, BATCH_PAUSE).await;
        if deleted > 0 {
            info!(deleted, %cutoff, retention_hours, "Trail conditions retention sweep");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn default_is_twelve_hours() {
        assert_eq!(retention_hours_from_lookup(env(&[])), 12);
    }

    #[test]
    fn the_floor_always_covers_the_history_the_series_endpoint_serves() {
        // If TIMESERIES_HISTORY_HOURS is ever raised, the floor follows it.
        const { assert!(FLOOR_HOURS > TIMESERIES_HISTORY_HOURS as u64) };
        assert_eq!(FLOOR_HOURS, 8);
        const { assert!(DEFAULT_HOURS >= FLOOR_HOURS) };
    }

    #[test]
    fn explicit_values_at_or_above_the_floor_are_honoured() {
        assert_eq!(retention_hours_from_lookup(env(&[(ENV, "24")])), 24);
        assert_eq!(retention_hours_from_lookup(env(&[(ENV, "8")])), 8);
        assert_eq!(retention_hours_from_lookup(env(&[(ENV, " 48 ")])), 48);
    }

    #[test]
    fn a_window_below_the_floor_is_raised_never_obeyed() {
        for v in ["1", "6", "7"] {
            assert_eq!(
                retention_hours_from_lookup(env(&[(ENV, v)])),
                FLOOR_HOURS,
                "{v}"
            );
        }
    }

    #[test]
    fn garbage_means_the_default_never_delete_everything() {
        for v in ["0", "-5", "abc", "", "1.5"] {
            assert_eq!(
                retention_hours_from_lookup(env(&[(ENV, v)])),
                DEFAULT_HOURS,
                "{v:?}"
            );
        }
    }
}

/// Real-PostGIS tests of the sweep. `#[ignore]`d (CI runs them with Docker).
#[cfg(test)]
mod db_tests {
    use super::*;
    use sqlx::PgPool;
    use storage::Catalog;

    async fn pool() -> (PgPool, test_utils::containers::TestInfrastructure) {
        let infra = test_utils::containers::TestInfrastructure::start().await;
        let catalog = Catalog::connect(&infra.postgres_url()).await.unwrap();
        catalog.migrate().await.unwrap();
        catalog.migrate_segment_conditions().await.unwrap();
        (catalog.pool_clone(), infra)
    }

    /// `n` hourly rows for `feature_id`, starting `first_age_h` hours from now
    /// (negative = past) and moving forward one hour per row.
    async fn seed(pool: &PgPool, feature_id: i64, first_age_h: i32, n: i32) {
        sqlx::query(
            "INSERT INTO segment_conditions (feature_id, run_time, valid_time, forecast_hour, model_version) \
             SELECT $1, now() - interval '40 hours', now() + make_interval(hours => $2 + g), 0, 'v' \
             FROM generate_series(0, $3 - 1) g",
        )
        .bind(feature_id)
        .bind(first_age_h)
        .bind(n)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn count(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM segment_conditions")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    #[ignore]
    async fn the_sweep_keeps_the_window_and_the_future_and_deletes_the_rest_in_batches() {
        let (pool, _infra) = pool().await;
        // Hours -30..=+10 relative to now, two features: 41 rows each.
        seed(&pool, 1, -30, 41).await;
        seed(&pool, 2, -30, 41).await;
        let catalog = SegmentConditionsCatalog::new(pool.clone());

        // 12.5 h ago, so the hourly row at -12 h is clearly inside the window
        // (the cutoff is computed a little after the rows were seeded)
        let cutoff = Utc::now() - chrono::Duration::minutes(12 * 60 + 30);
        // tiny batches force several rounds
        let deleted = sweep(&catalog, cutoff, 7, Duration::from_millis(1)).await;

        // per feature: hours -30..=-13 are older than the cutoff -> 18 rows
        assert_eq!(deleted, 36);
        assert_eq!(count(&pool).await, 82 - 36);
        let oldest: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT min(valid_time) FROM segment_conditions")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(oldest >= cutoff, "{oldest} < {cutoff}");
        let newest: chrono::DateTime<Utc> =
            sqlx::query_scalar("SELECT max(valid_time) FROM segment_conditions")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(newest > Utc::now(), "forecast hours are never deleted");

        // idempotent
        assert_eq!(
            sweep(&catalog, cutoff, 7, Duration::from_millis(1)).await,
            0
        );
    }

    #[tokio::test]
    #[ignore]
    async fn the_default_window_leaves_everything_the_api_serves() {
        let (pool, _infra) = pool().await;
        seed(&pool, 1, -30, 41).await;
        let catalog = SegmentConditionsCatalog::new(pool.clone());
        let cutoff = Utc::now() - chrono::Duration::hours(DEFAULT_HOURS as i64);
        sweep(&catalog, cutoff, 1000, Duration::ZERO).await;

        // everything the series endpoint returns (last 6 h + future) is intact,
        // and so is the row `latest` resolves to
        let series = catalog.get_timeseries_for_feature(1).await.unwrap();
        assert!(series.len() >= 6 + 10, "series {}", series.len());
        assert!(!catalog
            .get_latest_for_features(&[1])
            .await
            .unwrap()
            .is_empty());
        // and even at the floor, the history window is untouched
        let floor_cutoff = Utc::now() - chrono::Duration::hours(FLOOR_HOURS as i64);
        sweep(&catalog, floor_cutoff, 1000, Duration::ZERO).await;
        assert_eq!(
            catalog.get_timeseries_for_feature(1).await.unwrap().len(),
            series.len(),
            "the floor must not eat into the served history"
        );
    }
}
