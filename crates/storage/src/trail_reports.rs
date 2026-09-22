//! Storage for crowdsourced/scraped trail condition reports.
//!
//! Phase 0 groundwork from the trail-conditions design doc (Rung 2): this is
//! a raw label archive, not a served EDR collection. Reports accumulate here
//! so a future session can test whether the physics feature stack separates
//! reported condition classes at all, before any physics gets built.
//!
//! Append-mostly: `insert_report` upserts on `(source, source_id)` so
//! re-running a scrape is safe, but nothing here is ever deleted (matches
//! `storm_events`' "historical archive" retention posture -- not wired into
//! `crates/retention` at all).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use wms_common::{WmsError, WmsResult};

/// A single trail condition report to insert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrailReport {
    /// Report source, e.g. `"trailforks"`, `"csv_import"`.
    pub source: String,
    /// The source's own report id, when available -- used for dedup on
    /// re-scrape. `None` for sources without a stable id (e.g. some CSV
    /// exports); such reports may duplicate on repeated import, which is an
    /// accepted v1 limitation for a training-label archive.
    pub source_id: Option<String>,
    /// Free-text trail/system name as reported (not yet resolved to a
    /// `linear_features.feature_id`).
    pub trail_ref: Option<String>,
    /// Resolved `linear_features.feature_id`, when a join has been done.
    pub feature_id: Option<i64>,
    pub reported_at: DateTime<Utc>,
    /// Verbatim reported condition text (e.g. "muddy, avoid").
    pub condition_raw: Option<String>,
    /// Best-effort normalized class (e.g. `dry`, `wet`, `muddy`, `snow`,
    /// `closed`, `unknown`) -- normalization happens at scrape time, not here.
    pub condition_class: Option<String>,
    pub lon: Option<f64>,
    pub lat: Option<f64>,
    /// Additional source fields preserved as JSON.
    #[serde(default)]
    pub raw: serde_json::Value,
}

/// Catalog for trail condition reports, backed by the shared PostGIS pool.
#[derive(Clone)]
pub struct TrailReportCatalog {
    pool: PgPool,
}

impl TrailReportCatalog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Run the trail-reports schema migration (idempotent). Exists so the
    /// storage-layer catalog is self-sufficient for tests; production boot
    /// uses `Catalog::migrate_trail_reports()` instead.
    pub async fn migrate(&self) -> WmsResult<()> {
        use crate::catalog::TRAIL_REPORTS_SCHEMA_SQL;

        for statement in TRAIL_REPORTS_SCHEMA_SQL.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| {
                        WmsError::DatabaseError(format!("Trail reports migration failed: {}", e))
                    })?;
            }
        }
        Ok(())
    }

    /// Insert (or, when `source_id` is present, upsert) a single report.
    pub async fn insert_report(&self, report: &TrailReport) -> WmsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO trail_reports (
                source, source_id, trail_ref, feature_id, reported_at,
                condition_raw, condition_class, geom, raw
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                CASE WHEN $8::float8 IS NOT NULL AND $9::float8 IS NOT NULL
                     THEN ST_SetSRID(ST_MakePoint($8, $9), 4326) END,
                $10
            )
            ON CONFLICT (source, source_id) DO UPDATE SET
                trail_ref = EXCLUDED.trail_ref,
                feature_id = EXCLUDED.feature_id,
                reported_at = EXCLUDED.reported_at,
                condition_raw = EXCLUDED.condition_raw,
                condition_class = EXCLUDED.condition_class,
                geom = EXCLUDED.geom,
                raw = EXCLUDED.raw
            "#,
        )
        .bind(&report.source)
        .bind(&report.source_id)
        .bind(&report.trail_ref)
        .bind(report.feature_id)
        .bind(report.reported_at)
        .bind(&report.condition_raw)
        .bind(&report.condition_class)
        .bind(report.lon)
        .bind(report.lat)
        .bind(&report.raw)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Insert trail report failed: {}", e)))?;

        Ok(())
    }

    /// Batch insert. Returns the number of reports successfully processed.
    pub async fn insert_reports(&self, reports: &[TrailReport]) -> WmsResult<usize> {
        let mut count = 0;
        for report in reports {
            self.insert_report(report).await?;
            count += 1;
        }
        Ok(count)
    }

    /// Count reports, optionally filtered by source (used by the admin/ops
    /// side to confirm a scrape actually landed rows).
    pub async fn count_reports(&self, source: Option<&str>) -> WmsResult<i64> {
        let count: (i64,) = sqlx::query_as(
            r#"SELECT COUNT(*) FROM trail_reports WHERE ($1::text IS NULL OR source = $1)"#,
        )
        .bind(source)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Count trail reports failed: {}", e)))?;
        Ok(count.0)
    }
}
