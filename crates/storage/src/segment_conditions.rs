//! Read access to `segment_conditions` -- per-segment, per-hour trail
//! condition output written by the `trail-physics` Python service.
//!
//! This crate never writes this table (the Python service writes directly
//! via psycopg); this module exists for the future EDR exposure
//! (`?conditions=latest` on the `trails` collection) and admin/ops queries.
//! No migration/write methods here -- see `Catalog::migrate_segment_conditions`
//! in catalog.rs for schema ownership.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use wms_common::{WmsError, WmsResult};

/// A single segment's condition at one valid time.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SegmentCondition {
    pub feature_id: i64,
    pub run_time: DateTime<Utc>,
    pub valid_time: DateTime<Utc>,
    pub forecast_hour: i32,
    pub soil_moisture: Option<f32>,
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

    /// Latest available condition row for a single feature (across all
    /// model_versions -- the most recently ingested one wins), regardless
    /// of whether it's an analysis or forecast-hour row.
    pub async fn get_latest_for_feature(
        &self,
        feature_id: i64,
    ) -> WmsResult<Option<SegmentCondition>> {
        sqlx::query_as::<_, SegmentCondition>(
            r#"
            SELECT feature_id, run_time, valid_time, forecast_hour,
                   soil_moisture, frozen_fraction, frost_depth_m, swe_mm,
                   softness_index, confidence, model_version
            FROM segment_conditions
            WHERE feature_id = $1
            ORDER BY ingested_at DESC
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
    /// viewport's worth of segments rather than N round trips).
    pub async fn get_latest_for_features(
        &self,
        feature_ids: &[i64],
    ) -> WmsResult<Vec<SegmentCondition>> {
        sqlx::query_as::<_, SegmentCondition>(
            r#"
            SELECT DISTINCT ON (feature_id)
                   feature_id, run_time, valid_time, forecast_hour,
                   soil_moisture, frozen_fraction, frost_depth_m, swe_mm,
                   softness_index, confidence, model_version
            FROM segment_conditions
            WHERE feature_id = ANY($1)
            ORDER BY feature_id, ingested_at DESC
            "#,
        )
        .bind(feature_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            WmsError::DatabaseError(format!("Get latest segment conditions batch failed: {}", e))
        })
    }

    /// Full timeseries (analysis + forecast hours) for one feature from the
    /// most recent model run -- the per-segment detail-view query.
    pub async fn get_timeseries_for_feature(
        &self,
        feature_id: i64,
    ) -> WmsResult<Vec<SegmentCondition>> {
        sqlx::query_as::<_, SegmentCondition>(
            r#"
            WITH latest_run AS (
                SELECT run_time FROM segment_conditions
                WHERE feature_id = $1
                ORDER BY run_time DESC
                LIMIT 1
            )
            SELECT feature_id, run_time, valid_time, forecast_hour,
                   soil_moisture, frozen_fraction, frost_depth_m, swe_mm,
                   softness_index, confidence, model_version
            FROM segment_conditions
            WHERE feature_id = $1
              AND run_time = (SELECT run_time FROM latest_run)
            ORDER BY valid_time ASC
            "#,
        )
        .bind(feature_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            WmsError::DatabaseError(format!("Get segment condition timeseries failed: {}", e))
        })
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
