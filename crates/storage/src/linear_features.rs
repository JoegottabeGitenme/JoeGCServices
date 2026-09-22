//! Storage and retrieval for OSM-sourced linear features (trails, tracks,
//! bridleways) backing the EDR `trails` feature collection.
//!
//! Mirrors `storm_events.rs`'s approach deliberately: all spatial work happens
//! in PostGIS, geometry is returned to callers as raw GeoJSON (`ST_AsGeoJSON`)
//! since the workspace intentionally avoids Rust geometry crates, and rows are
//! upserted with `ON CONFLICT` for idempotent re-sync.
//!
//! ## Soft-delete semantics
//!
//! Unlike storm events (append-only historical archive), this table is
//! periodically re-synced from a live source (OSM/Overpass). OSM way ids can
//! be reused after a mapper splits or redraws a way, and ways can legitimately
//! disappear (trail closed, retagged, geometry redrawn under a new id). A sync
//! pass never hard-deletes: features missing from the latest pass for their
//! region are marked `active = false` via [`LinearFeatureCatalog::mark_inactive_except`]
//! rather than removed, so a client mid-request never has geometry pulled out
//! from under it without at least a soft-delete signal it can filter on.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use wms_common::{WmsError, WmsResult};

/// A linear feature to upsert (write-side).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearFeature {
    /// OSM way id (stable within a sync cycle; primary key).
    pub feature_id: i64,
    /// `'mtb_trail'` | `'hiking_trail'` | `'track'` | `'bridleway'` | ... —
    /// derived from OSM tags at sync time, not restructured storage-side.
    pub feature_class: String,
    /// Trail/way name, if tagged.
    pub name: Option<String>,
    /// Trail-system / area grouping, best-effort from OSM tags.
    pub system: Option<String>,
    /// Ordered (lon, lat) vertices. Must have at least 2 points.
    pub coordinates: Vec<(f64, f64)>,
    /// Raw OSM tags, preserved for future use.
    #[serde(default)]
    pub tags: serde_json::Value,
    /// Sync-region key (from `config/trail-sync.yaml`) that produced this row.
    pub region: String,
}

/// A linear feature with its geometry serialized as GeoJSON, ready for an EDR
/// feature response.
#[derive(Debug, Clone)]
pub struct LinearFeatureItem {
    pub feature_id: i64,
    pub feature_class: String,
    pub name: Option<String>,
    pub system: Option<String>,
    /// Geometry as a raw GeoJSON object string (LineString).
    pub geometry_geojson: String,
    pub tags: serde_json::Value,
    pub region: String,
    pub active: bool,
    pub updated_at: DateTime<Utc>,
}

/// Catalog for linear features, backed by the shared PostGIS pool.
#[derive(Clone)]
pub struct LinearFeatureCatalog {
    pool: PgPool,
}

impl LinearFeatureCatalog {
    /// Create a new catalog sharing an existing connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Run the linear-features schema migration (idempotent). Exists so the
    /// storage-layer catalog is self-sufficient for tests; production boot
    /// uses `Catalog::migrate_linear_features()` instead.
    pub async fn migrate(&self) -> WmsResult<()> {
        use crate::catalog::LINEAR_FEATURES_SCHEMA_SQL;

        for statement in LINEAR_FEATURES_SCHEMA_SQL.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| {
                        WmsError::DatabaseError(format!("Linear features migration failed: {}", e))
                    })?;
            }
        }

        Ok(())
    }

    /// Upsert a single linear feature. Geometry is built server-side from a
    /// WKT LineString text built in Rust (no PostGIS geometry construction
    /// functions needed since we already have the full ordered vertex list,
    /// unlike storm events which only have two endpoints).
    ///
    /// A feature reappearing after being soft-deleted is revived (`active`
    /// reset to true) automatically since this is called only for features
    /// actually present in the current sync pass.
    pub async fn upsert_feature(&self, feature: &LinearFeature) -> WmsResult<()> {
        if feature.coordinates.len() < 2 {
            return Err(WmsError::DatabaseError(format!(
                "Linear feature {} has fewer than 2 coordinates, skipping",
                feature.feature_id
            )));
        }

        let wkt = linestring_wkt(&feature.coordinates);

        sqlx::query(
            r#"
            INSERT INTO linear_features (
                feature_id, feature_class, name, system, geom, tags, region,
                active, source, first_seen_at, last_seen_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, ST_GeomFromText($5, 4326), $6, $7,
                TRUE, 'osm', NOW(), NOW(), NOW()
            )
            ON CONFLICT (feature_id) DO UPDATE SET
                feature_class = EXCLUDED.feature_class,
                name = EXCLUDED.name,
                system = EXCLUDED.system,
                geom = EXCLUDED.geom,
                tags = EXCLUDED.tags,
                region = EXCLUDED.region,
                active = TRUE,
                last_seen_at = NOW(),
                updated_at = NOW()
            "#,
        )
        .bind(feature.feature_id)
        .bind(&feature.feature_class)
        .bind(&feature.name)
        .bind(&feature.system)
        .bind(&wkt)
        .bind(&feature.tags)
        .bind(&feature.region)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Upsert linear feature failed: {}", e)))?;

        Ok(())
    }

    /// Batch upsert. Returns the number of features successfully processed;
    /// individual coordinate-validation failures are skipped, not fatal.
    pub async fn upsert_features(&self, features: &[LinearFeature]) -> WmsResult<usize> {
        let mut count = 0;
        for feature in features {
            match self.upsert_feature(feature).await {
                Ok(()) => count += 1,
                Err(e) => {
                    tracing::warn!(
                        feature_id = feature.feature_id,
                        error = %e,
                        "Skipping linear feature"
                    );
                }
            }
        }
        Ok(count)
    }

    /// Soft-delete: mark features in `region` inactive if their id is not in
    /// `seen_ids` (i.e. absent from the latest sync pass). Returns the number
    /// of rows newly marked inactive.
    pub async fn mark_inactive_except(&self, region: &str, seen_ids: &[i64]) -> WmsResult<u64> {
        let result = sqlx::query(
            r#"
            UPDATE linear_features
            SET active = FALSE, updated_at = NOW()
            WHERE region = $1
              AND active = TRUE
              AND NOT (feature_id = ANY($2))
            "#,
        )
        .bind(region)
        .bind(seen_ids)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Mark inactive failed: {}", e)))?;

        Ok(result.rows_affected())
    }

    /// Get active features intersecting a bounding box, optionally filtered by
    /// feature class.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_features_in_bbox(
        &self,
        feature_class: Option<&str>,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
        limit: i64,
        offset: i64,
    ) -> WmsResult<Vec<LinearFeatureItem>> {
        let rows = sqlx::query_as::<_, LinearFeatureRow>(
            r#"
            SELECT feature_id, feature_class, name, system,
                   ST_AsGeoJSON(geom) AS geometry_geojson,
                   tags, region, active, updated_at
            FROM linear_features
            WHERE active = TRUE
              AND ($1::text IS NULL OR feature_class = $1)
              AND ST_Intersects(geom, ST_MakeEnvelope($2, $3, $4, $5, 4326))
            ORDER BY feature_id
            LIMIT $6 OFFSET $7
            "#,
        )
        .bind(feature_class)
        .bind(min_lon)
        .bind(min_lat)
        .bind(max_lon)
        .bind(max_lat)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            WmsError::DatabaseError(format!("Linear features bbox query failed: {}", e))
        })?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Get active features within `radius_m` meters of (lon, lat).
    pub async fn get_features_in_radius(
        &self,
        feature_class: Option<&str>,
        lon: f64,
        lat: f64,
        radius_m: f64,
        limit: i64,
    ) -> WmsResult<Vec<LinearFeatureItem>> {
        let rows = sqlx::query_as::<_, LinearFeatureRow>(
            r#"
            SELECT feature_id, feature_class, name, system,
                   ST_AsGeoJSON(geom) AS geometry_geojson,
                   tags, region, active, updated_at
            FROM linear_features
            WHERE active = TRUE
              AND ($1::text IS NULL OR feature_class = $1)
              AND ST_DWithin(geom::geography,
                             ST_SetSRID(ST_MakePoint($2, $3), 4326)::geography, $4)
            ORDER BY feature_id
            LIMIT $5
            "#,
        )
        .bind(feature_class)
        .bind(lon)
        .bind(lat)
        .bind(radius_m)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            WmsError::DatabaseError(format!("Linear features radius query failed: {}", e))
        })?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Name search (`?q=` on the trails collection). Same normalization
    /// strategy as `search_populated_places`: unaccent + punctuation-stripped
    /// lowercase compared with `LIKE '%term%'`, ranked exact > prefix > substring.
    pub async fn search_features(
        &self,
        query: &str,
        feature_class: Option<&str>,
        limit: i64,
    ) -> WmsResult<Vec<LinearFeatureItem>> {
        let rows = sqlx::query_as::<_, LinearFeatureRow>(
            r#"
            WITH q AS (
                SELECT regexp_replace(unaccent(lower($1)), '[^a-z0-9]', '', 'g') AS norm
            )
            SELECT f.feature_id, f.feature_class, f.name, f.system,
                   ST_AsGeoJSON(f.geom) AS geometry_geojson,
                   f.tags, f.region, f.active, f.updated_at
            FROM linear_features f, q
            WHERE f.active = TRUE
              AND q.norm <> ''
              AND ($2::text IS NULL OR f.feature_class = $2)
              AND regexp_replace(unaccent(lower(coalesce(f.name, ''))), '[^a-z0-9]', '', 'g')
                  LIKE '%' || q.norm || '%'
            ORDER BY
              CASE
                WHEN regexp_replace(unaccent(lower(coalesce(f.name, ''))), '[^a-z0-9]', '', 'g') = q.norm THEN 0
                WHEN regexp_replace(unaccent(lower(coalesce(f.name, ''))), '[^a-z0-9]', '', 'g') LIKE q.norm || '%' THEN 1
                ELSE 2
              END ASC,
              f.name ASC
            LIMIT $3
            "#,
        )
        .bind(query)
        .bind(feature_class)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Linear features search failed: {}", e)))?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Fetch a single feature by OSM way id.
    pub async fn get_feature_by_id(&self, feature_id: i64) -> WmsResult<Option<LinearFeatureItem>> {
        let row = sqlx::query_as::<_, LinearFeatureRow>(
            r#"
            SELECT feature_id, feature_class, name, system,
                   ST_AsGeoJSON(geom) AS geometry_geojson,
                   tags, region, active, updated_at
            FROM linear_features
            WHERE feature_id = $1
            "#,
        )
        .bind(feature_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get linear feature by id failed: {}", e)))?;

        Ok(row.map(Into::into))
    }

    /// Count active features, optionally filtered by class (used for
    /// collection availability checks).
    pub async fn count_features(&self, feature_class: Option<&str>) -> WmsResult<i64> {
        let count: (i64,) = sqlx::query_as(
            r#"SELECT COUNT(*) FROM linear_features WHERE active = TRUE
               AND ($1::text IS NULL OR feature_class = $1)"#,
        )
        .bind(feature_class)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Count linear features failed: {}", e)))?;
        Ok(count.0)
    }
}

/// Build a WKT `LINESTRING(lon lat, lon lat, ...)` string from ordered vertices.
fn linestring_wkt(coords: &[(f64, f64)]) -> String {
    let points: Vec<String> = coords
        .iter()
        .map(|(lon, lat)| format!("{} {}", lon, lat))
        .collect();
    format!("LINESTRING({})", points.join(", "))
}

/// Row type for linear-feature queries.
#[derive(sqlx::FromRow)]
struct LinearFeatureRow {
    feature_id: i64,
    feature_class: String,
    name: Option<String>,
    system: Option<String>,
    geometry_geojson: Option<String>,
    tags: serde_json::Value,
    region: String,
    active: bool,
    updated_at: DateTime<Utc>,
}

impl From<LinearFeatureRow> for LinearFeatureItem {
    fn from(r: LinearFeatureRow) -> Self {
        LinearFeatureItem {
            feature_id: r.feature_id,
            feature_class: r.feature_class,
            name: r.name,
            system: r.system,
            geometry_geojson: r.geometry_geojson.unwrap_or_else(|| "null".to_string()),
            tags: r.tags,
            region: r.region,
            active: r.active,
            updated_at: r.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linestring_wkt_basic() {
        let coords = vec![(-105.2, 39.7), (-105.21, 39.71), (-105.22, 39.72)];
        assert_eq!(
            linestring_wkt(&coords),
            "LINESTRING(-105.2 39.7, -105.21 39.71, -105.22 39.72)"
        );
    }

    #[test]
    fn test_linestring_wkt_two_points() {
        let coords = vec![(-105.0, 39.0), (-105.1, 39.1)];
        assert_eq!(linestring_wkt(&coords), "LINESTRING(-105 39, -105.1 39.1)");
    }
}
