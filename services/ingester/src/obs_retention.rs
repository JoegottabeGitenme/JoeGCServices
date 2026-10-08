//! Retention for the `observations` and `taf_forecasts` tables.
//!
//! Until now nothing ever deleted a row: `delete_observations_before` and
//! `delete_tafs_before` existed but had no callers, so the table grew by
//! ~150-250k rows a day to 14.5M (89% of it NDBC), which made even
//! `SELECT count(*) ... WHERE source = 'ndbc'` take ~50 s and, through the EDR
//! `/collections` handler, pushed that listing to 60-90 s.
//!
//! ## Windows
//!
//! Per source, set by environment variable, with a default and a **floor**:
//!
//! | source | env var                          | default | floor | why the floor |
//! |--------|----------------------------------|---------|-------|---------------|
//! | metar  | `OBS_RETENTION_HOURS_METAR`      | 168 (7 d) | 24 h  | API defaults look back <=12 h |
//! | ndbc   | `OBS_RETENTION_HOURS_NDBC`       | 168 (7 d) | 24 h  | same |
//! | dart   | `OBS_RETENTION_HOURS_DART`       | 1440 (60 d) | 1128 h (47 d) | **its downloader re-fetches 45 days** |
//! | taf    | `TAF_RETENTION_HOURS`            | 168 (7 d, after `valid_to`) | 24 h | |
//!
//! The DART floor is the important one. Inserts are `ON CONFLICT DO NOTHING`, so a
//! row deleted while still inside the downloader's lookback is simply fetched and
//! re-inserted on the next poll -- the table would churn the same 45 days forever,
//! looking fine but doing pointless work. A value below the floor is therefore
//! **raised to the floor with a warning**, never obeyed. A value that is not a
//! positive integer falls back to the default, also with a warning (it must never
//! mean "delete everything").
//!
//! These deliberately do NOT reuse `retention.hours` from the model YAMLs: for
//! observation models that field also drives the downloader's lookback window.

use std::time::Duration;

use chrono::{DateTime, Utc};
use storage::observations::ObservationCatalog;
use tracing::{info, warn};

/// Rows deleted per statement. Small enough that no single transaction is long.
pub const DELETE_BATCH_SIZE: i64 = 50_000;
/// Pause between batches so the ingester's own inserts are never starved.
const BATCH_PAUSE: Duration = Duration::from_millis(250);
/// How often a full sweep runs.
const SWEEP_INTERVAL: Duration = Duration::from_secs(600);

/// One observation source's retention rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRule {
    pub source: &'static str,
    pub env: &'static str,
    pub default_hours: u64,
    pub floor_hours: u64,
}

pub const SOURCES: [SourceRule; 3] = [
    SourceRule {
        source: "metar",
        env: "OBS_RETENTION_HOURS_METAR",
        default_hours: 168,
        floor_hours: 24,
    },
    SourceRule {
        source: "ndbc",
        env: "OBS_RETENTION_HOURS_NDBC",
        default_hours: 168,
        floor_hours: 24,
    },
    // DART's downloader re-fetches 45 days (config/models/dart.yaml lookback_minutes
    // 64800); 47 days leaves a 2-day margin.
    SourceRule {
        source: "dart",
        env: "OBS_RETENTION_HOURS_DART",
        default_hours: 1440,
        floor_hours: 1128,
    },
];

pub const TAF_ENV: &str = "TAF_RETENTION_HOURS";
pub const TAF_DEFAULT_HOURS: u64 = 168;
pub const TAF_FLOOR_HOURS: u64 = 24;

/// Resolve one window from a raw env value. Pure, so the rules are testable
/// without touching process environment. Returns `(hours, warning)`.
pub fn resolve_hours(
    raw: Option<&str>,
    default_hours: u64,
    floor_hours: u64,
) -> (u64, Option<String>) {
    let Some(raw) = raw else {
        return (default_hours, None);
    };
    match raw.trim().parse::<u64>() {
        Ok(h) if h >= floor_hours => (h, None),
        Ok(h) if h > 0 => (
            floor_hours,
            Some(format!(
                "{h} h is below the safe minimum of {floor_hours} h; using {floor_hours} h"
            )),
        ),
        _ => (
            default_hours,
            Some(format!("{raw:?} is not a positive integer number of hours; using the default {default_hours} h")),
        ),
    }
}

/// The complete retention configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObsRetentionConfig {
    /// `(source, window hours)`.
    pub sources: Vec<(&'static str, u64)>,
    pub taf_hours: u64,
}

impl ObsRetentionConfig {
    /// Read from a lookup function (`std::env::var` in production), logging every
    /// adjustment it makes.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let mut sources = Vec::new();
        for rule in SOURCES {
            let (hours, warning) = resolve_hours(
                lookup(rule.env).as_deref(),
                rule.default_hours,
                rule.floor_hours,
            );
            if let Some(w) = warning {
                warn!(env = rule.env, "{w}");
            }
            sources.push((rule.source, hours));
        }
        let (taf_hours, warning) = resolve_hours(
            lookup(TAF_ENV).as_deref(),
            TAF_DEFAULT_HOURS,
            TAF_FLOOR_HOURS,
        );
        if let Some(w) = warning {
            warn!(env = TAF_ENV, "{w}");
        }
        Self { sources, taf_hours }
    }

    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }
}

/// Delete every expired row of one source, in batches. Returns the total deleted.
/// Stops early (returning what it did) if a batch fails, so one bad statement
/// cannot spin; the next sweep resumes.
pub async fn sweep_source(
    catalog: &ObservationCatalog,
    source: &str,
    cutoff: DateTime<Utc>,
    batch_size: i64,
    pause: Duration,
) -> u64 {
    let mut total = 0u64;
    loop {
        match catalog
            .delete_observations_before_batch(source, cutoff, batch_size)
            .await
        {
            Ok(n) => {
                total += n;
                if (n as i64) < batch_size {
                    return total;
                }
                tokio::time::sleep(pause).await;
            }
            Err(e) => {
                warn!(source, error = %e, deleted_so_far = total, "Observation retention batch failed; will resume next sweep");
                return total;
            }
        }
    }
}

/// One full sweep over every source plus TAFs.
pub async fn sweep_once(
    catalog: &ObservationCatalog,
    config: &ObsRetentionConfig,
    now: DateTime<Utc>,
) {
    for (source, hours) in &config.sources {
        let cutoff = now - chrono::Duration::hours(*hours as i64);
        let deleted = sweep_source(catalog, source, cutoff, DELETE_BATCH_SIZE, BATCH_PAUSE).await;
        if deleted > 0 {
            info!(source, deleted, %cutoff, retention_hours = hours, "Observation retention sweep");
        }
    }
    let taf_cutoff = now - chrono::Duration::hours(config.taf_hours as i64);
    match catalog.delete_tafs_before(taf_cutoff).await {
        Ok(0) => {}
        Ok(n) => info!(deleted = n, %taf_cutoff, "TAF retention sweep"),
        Err(e) => warn!(error = %e, "TAF retention sweep failed"),
    }
}

/// Run sweeps forever. The first runs immediately so a restart cleans up at once.
pub async fn run(catalog: ObservationCatalog, config: ObsRetentionConfig) {
    info!(?config, "Observation/TAF retention enabled");
    let mut tick = tokio::time::interval(SWEEP_INTERVAL);
    loop {
        tick.tick().await;
        sweep_once(&catalog, &config, Utc::now()).await;
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
    fn defaults_are_7_days_for_stations_and_60_for_dart() {
        let c = ObsRetentionConfig::from_lookup(env(&[]));
        assert_eq!(
            c.sources,
            vec![("metar", 168), ("ndbc", 168), ("dart", 1440)]
        );
        assert_eq!(c.taf_hours, 168);
    }

    #[test]
    fn valid_overrides_are_honoured() {
        let c = ObsRetentionConfig::from_lookup(env(&[
            ("OBS_RETENTION_HOURS_METAR", "720"),
            ("OBS_RETENTION_HOURS_DART", "2000"),
            ("TAF_RETENTION_HOURS", "48"),
        ]));
        assert_eq!(
            c.sources,
            vec![("metar", 720), ("ndbc", 168), ("dart", 2000)]
        );
        assert_eq!(c.taf_hours, 48);
    }

    #[test]
    fn a_window_below_the_floor_is_raised_never_obeyed() {
        // 24 h of DART would delete rows the downloader re-fetches from 45 days back.
        let (h, w) = resolve_hours(Some("24"), 1440, 1128);
        assert_eq!(h, 1128);
        assert!(w.unwrap().contains("below the safe minimum"));
        // exactly the floor is fine
        assert_eq!(resolve_hours(Some("1128"), 1440, 1128), (1128, None));
        assert_eq!(resolve_hours(Some("1127"), 1440, 1128).0, 1128);
    }

    #[test]
    fn nonsense_never_means_delete_everything() {
        for bad in ["0", "-5", "abc", "", "1.5", " ", "7d"] {
            let (h, w) = resolve_hours(Some(bad), 168, 24);
            assert_eq!(h, 168, "{bad:?} must fall back to the default, got {h}");
            assert!(w.is_some(), "{bad:?} must warn");
        }
    }

    #[test]
    fn whitespace_around_a_valid_number_is_tolerated() {
        assert_eq!(resolve_hours(Some(" 96 "), 168, 24), (96, None));
    }

    #[test]
    fn the_dart_floor_exceeds_the_downloaders_45_day_lookback() {
        // This is the invariant the floor exists for. If someone lengthens dart's
        // lookback_minutes, this test makes them lengthen the floor too.
        let yaml = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/models/dart.yaml"
        ))
        .unwrap();
        let lookback_min: u64 = yaml
            .lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix("lookback_minutes:")
                    .map(|v| v.split('#').next().unwrap().trim().parse().unwrap())
            })
            .expect("dart.yaml has lookback_minutes");
        let dart = SOURCES.iter().find(|s| s.source == "dart").unwrap();
        assert!(
            dart.floor_hours * 60 > lookback_min,
            "DART floor {} h does not cover its downloader lookback of {} min",
            dart.floor_hours,
            lookback_min
        );
        assert!(dart.default_hours >= dart.floor_hours);
    }

    #[test]
    fn every_other_floor_covers_its_downloaders_lookback_too() {
        for (src, file) in [("metar", "metar.yaml"), ("ndbc", "ndbc.yaml")] {
            let yaml = std::fs::read_to_string(format!(
                "{}/../../config/models/{}",
                env!("CARGO_MANIFEST_DIR"),
                file
            ))
            .unwrap();
            let lookback_min: u64 = yaml
                .lines()
                .find_map(|l| {
                    l.trim()
                        .strip_prefix("lookback_minutes:")
                        .map(|v| v.split('#').next().unwrap().trim().parse().unwrap())
                })
                .unwrap();
            let rule = SOURCES.iter().find(|s| s.source == src).unwrap();
            assert!(rule.floor_hours * 60 > lookback_min, "{src}");
        }
        assert!(TAF_FLOOR_HOURS >= 2);
    }

    #[test]
    fn every_retained_source_matches_a_source_the_collections_actually_use() {
        // The EDR handlers query these exact source strings.
        let cfg = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/edr/metar.yaml"
        ))
        .unwrap();
        assert!(cfg.contains("observation_source: metar"));
        for src in ["ndbc", "dart"] {
            let cfg = std::fs::read_to_string(format!(
                "{}/../../config/edr/{}.yaml",
                env!("CARGO_MANIFEST_DIR"),
                src
            ))
            .unwrap();
            assert!(cfg.contains(&format!("observation_source: {src}")), "{src}");
        }
    }
}

/// Real-PostGIS tests of the sweep: the batching, the per-source windows, the TAF
/// cascade, and that nothing it shouldn't touch is touched. `#[ignore]`d (CI runs
/// them with Docker). Locally:
/// `LIGHTNING_TEST_DATABASE_URL=postgres://postgres:t@localhost:15434/postgres \
///    cargo test -p ingester --bin ingester -- --ignored obs_retention::db_tests`
#[cfg(test)]
mod db_tests {
    use super::*;
    use sqlx::PgPool;
    use storage::Catalog;

    async fn pool() -> (PgPool, Option<test_utils::containers::TestInfrastructure>) {
        let (url, infra) = match std::env::var("LIGHTNING_TEST_DATABASE_URL") {
            Ok(u) => (u, None),
            Err(_) => {
                let infra = test_utils::containers::TestInfrastructure::start().await;
                (infra.postgres_url(), Some(infra))
            }
        };
        let catalog = Catalog::connect(&url).await.unwrap();
        catalog.migrate().await.unwrap();
        catalog.migrate_observations().await.unwrap();
        let pool = catalog.pool_clone();
        sqlx::query("TRUNCATE observations, taf_periods, taf_forecasts, locations CASCADE")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO locations (id, name, location) VALUES ('L1', 'x', ST_GeogFromText('POINT(-105 40)'))")
            .execute(&pool)
            .await
            .unwrap();
        (pool, infra)
    }

    /// `n` observations of `source`, one per hour, ending `newest_age_h` hours ago.
    /// Each needs a distinct (location, source, obs_time) -- the dedup key.
    async fn seed(pool: &PgPool, source: &str, n: i64, newest_age_h: i64) {
        sqlx::query(
            "INSERT INTO observations (location_id, source, obs_time) \
             SELECT 'L1', $1, now() - make_interval(hours => $2::int + g) FROM generate_series(0, $3::int - 1) g",
        )
        .bind(source)
        .bind(newest_age_h as i32)
        .bind(n as i32)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn count(pool: &PgPool, source: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM observations WHERE source = $1")
            .bind(source)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn cfg() -> ObsRetentionConfig {
        ObsRetentionConfig::from_lookup(|_| None) // defaults: metar/ndbc 168 h, dart 1440 h, taf 168 h
    }

    #[tokio::test]
    #[ignore]
    async fn each_source_is_cut_at_its_own_window_and_nothing_else_is_touched() {
        let (pool, _infra) = pool().await;
        // 400 hourly rows each, newest = now. Ages span 0..399 h (~16.6 days).
        for src in ["metar", "ndbc", "dart", "some_future_source"] {
            seed(&pool, src, 400, 0).await;
        }
        sweep_once(&ObservationCatalog::new(pool.clone()), &cfg(), Utc::now()).await;

        // metar/ndbc keep ages 0..167 (168 h window) -> 168 rows
        assert_eq!(count(&pool, "metar").await, 168);
        assert_eq!(count(&pool, "ndbc").await, 168);
        // dart keeps 60 days (1440 h): all 400 hours are younger -> untouched
        assert_eq!(count(&pool, "dart").await, 400);
        // a source with no rule is NEVER deleted from
        assert_eq!(
            count(&pool, "some_future_source").await,
            400,
            "an unconfigured source must be left alone"
        );
        // and what remains is exactly the newest rows
        let oldest_kept: f64 = sqlx::query_scalar("SELECT (extract(epoch FROM now() - min(obs_time))/3600)::float8 FROM observations WHERE source='metar'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!((166.9..168.1).contains(&oldest_kept), "{oldest_kept}");
    }

    #[tokio::test]
    #[ignore]
    async fn batching_removes_everything_expired_across_many_batches() {
        let (pool, _infra) = pool().await;
        seed(&pool, "ndbc", 1000, 200).await; // all 1000 rows older than the 168 h window
        seed(&pool, "ndbc", 50, 0).await; // 50 recent rows that must survive
        let before = count(&pool, "ndbc").await;
        let cutoff = Utc::now() - chrono::Duration::hours(168);
        // batch of 70 forces ~15 round trips for the 1000 expired rows
        let deleted = sweep_source(
            &ObservationCatalog::new(pool.clone()),
            "ndbc",
            cutoff,
            70,
            Duration::from_millis(0),
        )
        .await;
        assert_eq!(deleted, 1000);
        assert_eq!(count(&pool, "ndbc").await, before - 1000);
    }

    #[tokio::test]
    #[ignore]
    async fn an_exact_multiple_of_the_batch_size_still_terminates() {
        // 3 full batches then one empty one: the loop must stop on the empty batch,
        // not spin forever on "n == batch_size".
        let (pool, _infra) = pool().await;
        seed(&pool, "metar", 300, 200).await;
        let cutoff = Utc::now() - chrono::Duration::hours(168);
        let deleted = tokio::time::timeout(
            Duration::from_secs(20),
            sweep_source(
                &ObservationCatalog::new(pool.clone()),
                "metar",
                cutoff,
                100,
                Duration::from_millis(0),
            ),
        )
        .await
        .expect("sweep did not terminate");
        assert_eq!(deleted, 300);
    }

    #[tokio::test]
    #[ignore]
    async fn a_sweep_with_nothing_expired_deletes_nothing_and_is_idempotent() {
        let (pool, _infra) = pool().await;
        seed(&pool, "metar", 100, 0).await; // all within 168 h
        let c = ObservationCatalog::new(pool.clone());
        sweep_once(&c, &cfg(), Utc::now()).await;
        sweep_once(&c, &cfg(), Utc::now()).await;
        assert_eq!(count(&pool, "metar").await, 100);
    }

    #[tokio::test]
    #[ignore]
    async fn tafs_expire_after_valid_to_and_their_periods_cascade() {
        let (pool, _infra) = pool().await;
        // old: valid_to 10 days ago; fresh: valid_to in the future
        for (loc_issue_h, valid_to_h) in [(250, -240), (6, 18)] {
            // valid_to_h negative = in the past by that many hours
            let id: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO taf_forecasts (location_id, issue_time, valid_from, valid_to) \
                 VALUES ('L1', now() - make_interval(hours => $1), now() - make_interval(hours => $1), now() + make_interval(hours => $2)) RETURNING id",
            )
            .bind(loc_issue_h)
            .bind(valid_to_h)
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO taf_periods (taf_id, period_from, period_to, change_indicator) VALUES ($1, now(), now() + interval '1 hour', 'FM')")
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
        }
        sweep_once(&ObservationCatalog::new(pool.clone()), &cfg(), Utc::now()).await;
        let tafs: i64 = sqlx::query_scalar("SELECT count(*) FROM taf_forecasts")
            .fetch_one(&pool)
            .await
            .unwrap();
        let periods: i64 = sqlx::query_scalar("SELECT count(*) FROM taf_periods")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            (tafs, periods),
            (1, 1),
            "the expired TAF and ITS period are gone, the live one is untouched"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn a_failing_batch_stops_the_sweep_instead_of_spinning() {
        // Point the sweep at a table that does not exist by dropping it: the loop
        // must return (having deleted nothing), not retry forever.
        let (pool, _infra) = pool().await;
        sqlx::query("ALTER TABLE observations RENAME TO observations_gone")
            .execute(&pool)
            .await
            .unwrap();
        let deleted = tokio::time::timeout(
            Duration::from_secs(10),
            sweep_source(
                &ObservationCatalog::new(pool.clone()),
                "metar",
                Utc::now(),
                100,
                Duration::from_millis(0),
            ),
        )
        .await
        .expect("a failing sweep must still return");
        assert_eq!(deleted, 0);
        sqlx::query("ALTER TABLE observations_gone RENAME TO observations")
            .execute(&pool)
            .await
            .unwrap();
    }
}
