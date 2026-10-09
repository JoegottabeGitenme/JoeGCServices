//! Read access to `segment_conditions` -- per-segment, per-hour trail
//! condition output written by the `trail-physics` Python service.
//!
//! This crate never inserts into this table (the Python service writes directly
//! via psycopg); this module serves the EDR reads (`?conditions=latest`, the
//! per-trail and batch time series) and admin/ops queries. The one write is the
//! retention delete (`delete_valid_before_batch`), run by the ingester. No
//! migration methods here -- see `Catalog::migrate_segment_conditions` in
//! catalog.rs for schema ownership.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use wms_common::{WmsError, WmsResult};

/// How many hours of recent past `get_timeseries_for_feature` includes
/// before "now". Enough context for a chart to show the recent trend
/// without shipping days of history.
pub const TIMESERIES_HISTORY_HOURS: i32 = 6;

/// A single segment's condition at one valid time.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SegmentCondition {
    pub feature_id: i64,
    pub run_time: DateTime<Utc>,
    pub valid_time: DateTime<Utc>,
    pub forecast_hour: i32,
    pub soil_moisture: Option<f32>,
    /// Degree of saturation (soil_moisture / theta_s, clipped to 0-1); NULL
    /// outside static-stack coverage. See `SEGMENT_CONDITIONS_SCHEMA_SQL`.
    pub saturation: Option<f32>,
    pub frozen_fraction: Option<f32>,
    pub frost_depth_m: Option<f32>,
    pub swe_mm: Option<f32>,
    pub softness_index: Option<f32>,
    pub confidence: Option<f32>,
    pub model_version: String,
}

/// Read-only catalog for segment conditions, backed by the shared pool.
#[derive(Clone)]
pub struct SegmentConditionsCatalog {
    pool: PgPool,
}

impl SegmentConditionsCatalog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // Tie-break note (all four read queries below): `UNIQUE(feature_id, valid_time,
    // model_version)` allows two model versions on one valid hour, and they can share
    // a `run_time`. `ORDER BY ... run_time DESC` alone leaves that tie to the
    // planner, so the single-trail and batch endpoints could disagree. Every query
    // therefore ends with `model_version DESC`; they all pick the same row.

    /// Latest available condition row for a single feature: the row with
    /// the greatest `valid_time` that is not in the future (i.e. the most
    /// recent hour we actually have a computed value for -- a genuine
    /// "current conditions" nowcast), tie-broken by the newest `run_time`
    /// (a fresher model initialization for the same valid hour supersedes
    /// an older one, standard NWP practice).
    ///
    /// **Session 14 fix**: this previously ordered by `ingested_at DESC`
    /// -- "whichever row this service happened to write most recently",
    /// which is right in steady state (processing is strictly
    /// chronological) but silently wrong during any backlog/reprocessing
    /// window: on trail-physics' very first production run, this served
    /// hours-to-a-day-old data as "latest" for the entire ~21h it took to
    /// work through its initial backlog, even though it was writing rows
    /// in valid_time order the whole time. Ordering by `valid_time`
    /// directly makes "latest" mean what it says regardless of processing
    /// order, ingest lag, service restarts, or future backfill jobs.
    pub async fn get_latest_for_feature(
        &self,
        feature_id: i64,
    ) -> WmsResult<Option<SegmentCondition>> {
        sqlx::query_as::<_, SegmentCondition>(
            r#"
            SELECT feature_id, run_time, valid_time, forecast_hour,
                   soil_moisture, saturation, frozen_fraction, frost_depth_m, swe_mm,
                   softness_index, confidence, model_version
            FROM segment_conditions
            WHERE feature_id = $1 AND valid_time <= NOW()
            ORDER BY valid_time DESC, run_time DESC, model_version DESC
            LIMIT 1
            "#,
        )
        .bind(feature_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get latest segment condition failed: {}", e)))
    }

    /// Latest condition row for each of a batch of features (the `trails`
    /// `/items?conditions=latest` use case -- one query for a whole
    /// viewport's worth of segments rather than N round trips). Same
    /// valid-time-nearest-now semantics as `get_latest_for_feature` --
    /// see that method's own docstring for the bug this fixes.
    ///
    /// **Query shape matters here.** The obvious form,
    /// `SELECT DISTINCT ON (feature_id) ... ORDER BY feature_id, valid_time DESC,
    /// run_time DESC`, has to read and sort *every* historical row of every
    /// requested feature to keep one each: measured on production for a 711-trail
    /// viewport, 233,550 rows sorted (spilling to disk) and ~63,000 buffers read to
    /// return 711 rows, so the request cost followed the table's history depth, not
    /// the viewport (1.7-3 s typical, 8-34 s under load). This form probes the
    /// `(feature_id, valid_time)` index once per feature and takes the top row
    /// (~4,200 buffers, the same 711 rows). Duplicate ids in the input are collapsed, as
    /// `DISTINCT ON` did. Rows come back ordered by `feature_id`.
    pub async fn get_latest_for_features(
        &self,
        feature_ids: &[i64],
    ) -> WmsResult<Vec<SegmentCondition>> {
        sqlx::query_as::<_, SegmentCondition>(
            r#"
            SELECT c.feature_id, c.run_time, c.valid_time, c.forecast_hour,
                   c.soil_moisture, c.saturation, c.frozen_fraction, c.frost_depth_m,
                   c.swe_mm, c.softness_index, c.confidence, c.model_version
            FROM (SELECT DISTINCT u AS feature_id FROM unnest($1::bigint[]) AS u) AS f
            CROSS JOIN LATERAL (
                SELECT s.feature_id, s.run_time, s.valid_time, s.forecast_hour,
                       s.soil_moisture, s.saturation, s.frozen_fraction, s.frost_depth_m,
                       s.swe_mm, s.softness_index, s.confidence, s.model_version
                FROM segment_conditions s
                WHERE s.feature_id = f.feature_id AND s.valid_time <= NOW()
                ORDER BY s.valid_time DESC, s.run_time DESC, s.model_version DESC
                LIMIT 1
            ) c
            ORDER BY c.feature_id
            "#,
        )
        .bind(feature_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            WmsError::DatabaseError(format!("Get latest segment conditions batch failed: {}", e))
        })
    }

    /// Best-available time series for one feature: for every `valid_time`
    /// from `HISTORY_HOURS` ago onward, the value from the **newest model
    /// run that has one** -- the per-hour freshest estimate, stitched across
    /// runs. The per-trail forecast-chart query.
    ///
    /// **Why not "all rows of the latest run"** (this method's first
    /// version, Session 14): the horizon then depends on which run happens
    /// to be newest *at that moment*. Measured on production: recent runs
    /// hold 5-41 forecast hours (the newest run is partial while it ingests;
    /// hourly HRRR runs are shorter than the 6-hourly synoptic ones), so a
    /// "firm until 10am" chart would have a 5-hour horizon one minute and
    /// 18 the next. Stitching by valid_time gives a stable horizon (the
    /// furthest hour ANY recent run reaches) and the freshest value for
    /// each hour. Each point carries its own `run_time`.
    ///
    /// Includes a few hours of recent past so a chart can show "what
    /// happened" as well as "what's coming".
    pub async fn get_timeseries_for_feature(
        &self,
        feature_id: i64,
    ) -> WmsResult<Vec<SegmentCondition>> {
        sqlx::query_as::<_, SegmentCondition>(
            r#"
            SELECT DISTINCT ON (valid_time)
                   feature_id, run_time, valid_time, forecast_hour,
                   soil_moisture, saturation, frozen_fraction, frost_depth_m, swe_mm,
                   softness_index, confidence, model_version
            FROM segment_conditions
            WHERE feature_id = $1
              AND valid_time >= NOW() - make_interval(hours => $2)
            ORDER BY valid_time ASC, run_time DESC, model_version DESC
            "#,
        )
        .bind(feature_id)
        .bind(TIMESERIES_HISTORY_HOURS)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            WmsError::DatabaseError(format!("Get segment condition timeseries failed: {}", e))
        })
    }

    /// `get_timeseries_for_feature` for many features in one query (the batch
    /// `/collections/trails/conditions?ids=` endpoint). Same stitching rule and
    /// history window per feature; rows are ordered by `feature_id`, then
    /// `valid_time` ascending, so grouping them is a single pass. A feature with
    /// no rows simply contributes none.
    pub async fn get_timeseries_for_features(
        &self,
        feature_ids: &[i64],
    ) -> WmsResult<Vec<SegmentCondition>> {
        sqlx::query_as::<_, SegmentCondition>(
            r#"
            SELECT DISTINCT ON (feature_id, valid_time)
                   feature_id, run_time, valid_time, forecast_hour,
                   soil_moisture, saturation, frozen_fraction, frost_depth_m, swe_mm,
                   softness_index, confidence, model_version
            FROM segment_conditions
            WHERE feature_id = ANY($1)
              AND valid_time >= NOW() - make_interval(hours => $2)
            ORDER BY feature_id, valid_time ASC, run_time DESC, model_version DESC
            "#,
        )
        .bind(feature_ids)
        .bind(TIMESERIES_HISTORY_HOURS)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            WmsError::DatabaseError(format!(
                "Get segment condition timeseries batch failed: {}",
                e
            ))
        })
    }

    /// Delete up to `batch_size` rows whose `valid_time` is older than `before`
    /// and return how many went. The retention sweep (ingester) calls this in a
    /// loop. Deleting by `ctid` found through `idx_segment_conditions_valid_time`
    /// keeps each statement short instead of one multi-million-row transaction.
    pub async fn delete_valid_before_batch(
        &self,
        before: DateTime<Utc>,
        batch_size: i64,
    ) -> WmsResult<u64> {
        let result = sqlx::query(
            r#"
            DELETE FROM segment_conditions
            WHERE ctid = ANY(ARRAY(
                SELECT ctid FROM segment_conditions
                WHERE valid_time < $1
                LIMIT $2
            ))
            "#,
        )
        .bind(before)
        .bind(batch_size.max(1))
        .execute(&self.pool)
        .await
        .map_err(|e| {
            WmsError::DatabaseError(format!("Delete segment conditions batch failed: {}", e))
        })?;
        Ok(result.rows_affected())
    }

    /// Count of rows (used for collection-availability / staleness checks --
    /// e.g. an ops check that the trail-physics service is actually running).
    pub async fn count_rows(&self) -> WmsResult<i64> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM segment_conditions")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                WmsError::DatabaseError(format!("Count segment conditions failed: {}", e))
            })?;
        Ok(count.0)
    }

    /// The most recent `valid_time` that is not in the future -- i.e. the
    /// hour a `?conditions=latest` query would currently report. The
    /// freshness a USER sees: if this stops advancing, `latest` is stale
    /// regardless of why (worker down, HRRR ingest stalled, DB trouble).
    /// Uses `idx_segment_conditions_valid_time`, so it's cheap enough to
    /// poll every few seconds, unlike `MAX(ingested_at)` (no index -> a
    /// scan of a table that grows without bound by design).
    pub async fn latest_valid_time(&self) -> WmsResult<Option<DateTime<Utc>>> {
        let row: (Option<DateTime<Utc>>,) = sqlx::query_as(
            "SELECT MAX(valid_time) FROM segment_conditions WHERE valid_time <= NOW()",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get latest valid time failed: {}", e)))?;
        Ok(row.0)
    }

    /// When the `trail-physics` worker last finished a forecast hour, from
    /// its own progress ledger (a small table, so a cheap query). Distinguishes
    /// "the worker is dead" from "the worker is fine but upstream HRRR ingest
    /// stopped": both make `latest_valid_time` go stale, but only one is
    /// the worker's fault.
    pub async fn last_worker_progress(&self) -> WmsResult<Option<DateTime<Utc>>> {
        let row: (Option<DateTime<Utc>>,) =
            sqlx::query_as("SELECT MAX(processed_at) FROM trail_physics_progress")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    WmsError::DatabaseError(format!("Get last worker progress failed: {}", e))
                })?;
        Ok(row.0)
    }

    /// Most recent `ingested_at` across the whole table -- the staleness
    /// signal ("has trail-physics run in the last N hours?").
    pub async fn most_recent_ingest(&self) -> WmsResult<Option<DateTime<Utc>>> {
        let row: (Option<DateTime<Utc>>,) =
            sqlx::query_as("SELECT MAX(ingested_at) FROM segment_conditions")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    WmsError::DatabaseError(format!(
                        "Get most recent segment condition ingest failed: {}",
                        e
                    ))
                })?;
        Ok(row.0)
    }
}
