//! Storage and retrieval for GOES GLM lightning flashes.
//!
//! Backs the EDR `glm-lightning` feature collection (and, in phase 2, its SSE
//! stream). One row per flash, written by the ingester after it parses a
//! 20-second GLM L2 LCFA granule, clipped to CONUS. Rows are short-lived:
//! `delete_before` is called on a timer by the ingester (the table is a live
//! map feed, not an archive).
//!
//! ## Why `geometry`, not `geography`
//!
//! A map viewport is a lon/lat rectangle. A `geography` envelope has
//! great-circle edges that bulge off constant-latitude lines, so a bbox query
//! against it would not match the rectangle on the user's screen. A planar
//! `geometry(Point,4326)` bbox is exactly that rectangle. Radius queries need
//! true geodesic distance, so they cast to `geography` for the exact test and
//! use a deliberately *over-sized* planar box as a GiST prefilter (see
//! [`radius_prefilter_box`]).
//!
//! ## Cursor semantics (`id`)
//!
//! `id` is a `BIGSERIAL` and doubles as a change-feed cursor: a client that has
//! seen `id = N` asks for `after = N`. That is only gap-free if ids become
//! visible in increasing order. A plain sequence does not guarantee that
//! (a transaction holding lower ids can commit after one holding higher ids),
//! so [`LightningCatalog::insert_flashes`] serializes writers with a
//! transaction-scoped advisory lock taken *before* any id is drawn and released
//! at commit.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Postgres, QueryBuilder, Row};

use wms_common::{WmsError, WmsResult};

/// Advisory-lock key serializing writers (see module docs). Arbitrary but fixed;
/// derived from the ASCII of "GLM1".
const INSERT_LOCK_KEY: i64 = 0x474C_4D31;

/// A flash ready to be inserted.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFlash {
    /// `"goes-east"` or `"goes-west"`.
    pub satellite: String,
    pub flash_time: DateTime<Utc>,
    /// Rolling per-satellite `u16` counter (see `netcdf_parser::glm`).
    pub flash_id: i32,
    pub lon: f64,
    pub lat: f64,
    pub energy_j: Option<f32>,
    pub quality: i16,
}

/// A stored flash.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredFlash {
    /// Monotonic change-feed cursor.
    pub id: i64,
    pub satellite: String,
    pub flash_time: DateTime<Utc>,
    pub flash_id: i32,
    pub lon: f64,
    pub lat: f64,
    pub energy_j: Option<f32>,
    pub quality: i16,
}

/// Spatial constraint of a query.
#[derive(Debug, Clone, PartialEq)]
pub enum FlashArea {
    Anywhere,
    /// `[min_lon, max_lon] x [min_lat, max_lat]`, a planar lon/lat rectangle.
    Bbox {
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
    },
    /// Geodesic radius in metres around a point.
    Radius {
        lon: f64,
        lat: f64,
        meters: f64,
    },
}

/// A flash query. `since` is required so no caller can scan the whole table by
/// forgetting a time bound.
#[derive(Debug, Clone, PartialEq)]
pub struct FlashQuery {
    /// Empty = both satellites.
    pub satellites: Vec<String>,
    pub since: DateTime<Utc>,
    pub until: Option<DateTime<Utc>>,
    /// Change-feed cursor: only flashes with `id > after_id`.
    pub after_id: Option<i64>,
    pub area: FlashArea,
    pub limit: i64,
}

/// The planar `[min_lon, min_lat, max_lon, max_lat]` box used to prefilter a
/// radius query through the GiST index. It must contain every point within
/// `meters` of `(lon, lat)`, so it is sized from the *smallest* metres-per-degree
/// that can occur (110 km/deg latitude; longitude shrinks by `cos(lat)`),
/// i.e. it errs large. The exact `ST_DWithin` test then trims it.
pub fn radius_prefilter_box(lon: f64, lat: f64, meters: f64) -> [f64; 4] {
    const M_PER_DEG_LAT_MIN: f64 = 110_000.0;
    let dlat = meters / M_PER_DEG_LAT_MIN;
    // Use the latitude of the box edge farthest from the equator, where a degree
    // of longitude is shortest, so the box is wide enough everywhere inside it.
    let worst_lat = (lat.abs() + dlat).min(89.9);
    let cos = worst_lat.to_radians().cos().max(1e-3);
    let dlon = (meters / (M_PER_DEG_LAT_MIN * cos)).min(180.0);
    [lon - dlon, lat - dlat, lon + dlon, lat + dlat]
}

/// Builds the query. Separate from execution so the generated SQL is unit-testable.
pub fn build_query(q: &FlashQuery) -> QueryBuilder<'_, Postgres> {
    const COLS: &str = "id, satellite, flash_time, flash_id, \
                        ST_X(location) AS lon, ST_Y(location) AS lat, energy_j, quality";

    let mut b: QueryBuilder<Postgres> = QueryBuilder::new("");
    // Without a cursor: the NEWEST `limit` flashes, then re-sorted oldest-first.
    // With a cursor: the OLDEST `limit` after it, so a truncated page can be
    // continued from its last id without skipping anything.
    let newest_first = q.after_id.is_none();
    if newest_first {
        b.push(format!(
            "SELECT * FROM (SELECT {COLS} FROM lightning_flashes WHERE "
        ));
    } else {
        b.push(format!("SELECT {COLS} FROM lightning_flashes WHERE "));
    }

    b.push("flash_time >= ").push_bind(q.since);
    if let Some(until) = q.until {
        b.push(" AND flash_time <= ").push_bind(until);
    }
    if let Some(after) = q.after_id {
        b.push(" AND id > ").push_bind(after);
    }
    if !q.satellites.is_empty() {
        b.push(" AND satellite = ANY(")
            .push_bind(q.satellites.clone())
            .push(")");
    }
    match &q.area {
        FlashArea::Anywhere => {}
        FlashArea::Bbox {
            min_lon,
            min_lat,
            max_lon,
            max_lat,
        } => {
            b.push(" AND location && ST_MakeEnvelope(")
                .push_bind(*min_lon)
                .push(", ")
                .push_bind(*min_lat)
                .push(", ")
                .push_bind(*max_lon)
                .push(", ")
                .push_bind(*max_lat)
                .push(", 4326)");
        }
        FlashArea::Radius { lon, lat, meters } => {
            let [x0, y0, x1, y1] = radius_prefilter_box(*lon, *lat, *meters);
            b.push(" AND location && ST_MakeEnvelope(")
                .push_bind(x0)
                .push(", ")
                .push_bind(y0)
                .push(", ")
                .push_bind(x1)
                .push(", ")
                .push_bind(y1)
                .push(", 4326) AND ST_DWithin(location::geography, ST_SetSRID(ST_MakePoint(")
                .push_bind(*lon)
                .push(", ")
                .push_bind(*lat)
                .push("), 4326)::geography, ")
                .push_bind(*meters)
                .push(")");
        }
    }

    if newest_first {
        b.push(" ORDER BY id DESC LIMIT ")
            .push_bind(q.limit)
            .push(") newest ORDER BY id ASC");
    } else {
        b.push(" ORDER BY id ASC LIMIT ").push_bind(q.limit);
    }
    b
}

/// Read/write access to `lightning_flashes`.
#[derive(Clone)]
pub struct LightningCatalog {
    pool: PgPool,
}

impl LightningCatalog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert flashes, ignoring ones already stored (`UNIQUE(satellite,
    /// flash_time, flash_id)`), so re-ingesting a granule is a no-op. Returns
    /// `(inserted, ids_of_inserted)` -- the ids, in ascending order, are what the
    /// stream layer publishes.
    ///
    /// Writers are serialized by an advisory lock so ids commit in order (module
    /// docs). The critical section is one statement on a few dozen rows.
    pub async fn insert_flashes(&self, flashes: &[NewFlash]) -> WmsResult<Vec<i64>> {
        if flashes.is_empty() {
            return Ok(Vec::new());
        }
        let satellites: Vec<&str> = flashes.iter().map(|f| f.satellite.as_str()).collect();
        let times: Vec<DateTime<Utc>> = flashes.iter().map(|f| f.flash_time).collect();
        let flash_ids: Vec<i32> = flashes.iter().map(|f| f.flash_id).collect();
        let lons: Vec<f64> = flashes.iter().map(|f| f.lon).collect();
        let lats: Vec<f64> = flashes.iter().map(|f| f.lat).collect();
        let energies: Vec<Option<f32>> = flashes.iter().map(|f| f.energy_j).collect();
        let qualities: Vec<i16> = flashes.iter().map(|f| f.quality).collect();

        let err =
            |e: sqlx::Error| WmsError::DatabaseError(format!("Lightning insert failed: {}", e));

        let mut tx = self.pool.begin().await.map_err(err)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(INSERT_LOCK_KEY)
            .execute(&mut *tx)
            .await
            .map_err(err)?;

        let rows = sqlx::query(
            r#"
            INSERT INTO lightning_flashes
                (satellite, flash_time, flash_id, location, energy_j, quality)
            SELECT satellite, flash_time, flash_id,
                   ST_SetSRID(ST_MakePoint(lon, lat), 4326), energy_j, quality
            FROM UNNEST($1::text[], $2::timestamptz[], $3::int4[], $4::float8[],
                        $5::float8[], $6::float4[], $7::int2[])
                 WITH ORDINALITY AS t(satellite, flash_time, flash_id, lon, lat,
                                      energy_j, quality, ord)
            ORDER BY ord
            ON CONFLICT (satellite, flash_time, flash_id) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(&satellites)
        .bind(&times)
        .bind(&flash_ids)
        .bind(&lons)
        .bind(&lats)
        .bind(&energies)
        .bind(&qualities)
        .fetch_all(&mut *tx)
        .await
        .map_err(err)?;

        tx.commit().await.map_err(err)?;
        let mut ids: Vec<i64> = rows.iter().map(|r| r.get::<i64, _>("id")).collect();
        ids.sort_unstable();
        Ok(ids)
    }

    pub async fn query_flashes(&self, q: &FlashQuery) -> WmsResult<Vec<StoredFlash>> {
        let mut builder = build_query(q);
        let rows = builder
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Lightning query failed: {}", e)))?;
        Ok(rows
            .into_iter()
            .map(|r| StoredFlash {
                id: r.get("id"),
                satellite: r.get("satellite"),
                flash_time: r.get("flash_time"),
                flash_id: r.get("flash_id"),
                lon: r.get("lon"),
                lat: r.get("lat"),
                energy_j: r.get("energy_j"),
                quality: r.get("quality"),
            })
            .collect())
    }

    /// Flashes with `id` in `ids` (stream replay by explicit id list), ascending.
    pub async fn get_by_ids(&self, ids: &[i64]) -> WmsResult<Vec<StoredFlash>> {
        let rows = sqlx::query(
            "SELECT id, satellite, flash_time, flash_id, ST_X(location) AS lon, \
             ST_Y(location) AS lat, energy_j, quality \
             FROM lightning_flashes WHERE id = ANY($1) ORDER BY id ASC",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Lightning get_by_ids failed: {}", e)))?;
        Ok(rows
            .into_iter()
            .map(|r| StoredFlash {
                id: r.get("id"),
                satellite: r.get("satellite"),
                flash_time: r.get("flash_time"),
                flash_id: r.get("flash_id"),
                lon: r.get("lon"),
                lat: r.get("lat"),
                energy_j: r.get("energy_j"),
                quality: r.get("quality"),
            })
            .collect())
    }

    /// Delete flashes older than `cutoff`. Returns rows deleted.
    pub async fn delete_before(&self, cutoff: DateTime<Utc>) -> WmsResult<u64> {
        let r = sqlx::query("DELETE FROM lightning_flashes WHERE flash_time < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Lightning delete failed: {}", e)))?;
        Ok(r.rows_affected())
    }

    /// The newest stored flash time per satellite (freshness / monitoring).
    pub async fn latest_flash_times(&self) -> WmsResult<Vec<(String, DateTime<Utc>)>> {
        let rows = sqlx::query(
            "SELECT satellite, MAX(flash_time) AS t FROM lightning_flashes GROUP BY satellite ORDER BY satellite",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Lightning latest time failed: {}", e)))?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("satellite"), r.get("t")))
            .collect())
    }

    pub async fn count(&self) -> WmsResult<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM lightning_flashes")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Lightning count failed: {}", e)))?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn q(area: FlashArea) -> FlashQuery {
        FlashQuery {
            satellites: vec![],
            since: Utc.with_ymd_and_hms(2026, 10, 7, 20, 0, 0).unwrap(),
            until: None,
            after_id: None,
            area,
            limit: 100,
        }
    }

    #[test]
    fn without_a_cursor_takes_the_newest_n_then_sorts_ascending() {
        let sql = build_query(&q(FlashArea::Anywhere)).sql().to_string();
        assert!(sql.starts_with("SELECT * FROM (SELECT id,"));
        assert!(sql.contains("ORDER BY id DESC LIMIT"));
        assert!(sql.ends_with(") newest ORDER BY id ASC"));
    }

    #[test]
    fn with_a_cursor_takes_the_oldest_n_after_it_so_paging_never_skips() {
        let mut query = q(FlashArea::Anywhere);
        query.after_id = Some(41);
        let sql = build_query(&query).sql().to_string();
        assert!(sql.contains("id > $2"));
        assert!(sql.contains("ORDER BY id ASC LIMIT"));
        assert!(
            !sql.contains("DESC"),
            "a cursor page must be oldest-first: {sql}"
        );
    }

    #[test]
    fn a_time_bound_is_always_present() {
        for area in [
            FlashArea::Anywhere,
            FlashArea::Bbox {
                min_lon: -106.0,
                min_lat: 39.0,
                max_lon: -105.0,
                max_lat: 40.0,
            },
            FlashArea::Radius {
                lon: -105.2,
                lat: 39.7,
                meters: 5000.0,
            },
        ] {
            assert!(build_query(&q(area)).sql().contains("flash_time >= $1"));
        }
    }

    #[test]
    fn clauses_are_only_emitted_for_constraints_that_are_set() {
        let plain = build_query(&q(FlashArea::Anywhere)).sql().to_string();
        assert!(!plain.contains("satellite = ANY"));
        assert!(!plain.contains("flash_time <="));
        assert!(!plain.contains("ST_MakeEnvelope"));

        let mut full = q(FlashArea::Bbox {
            min_lon: -106.0,
            min_lat: 39.0,
            max_lon: -105.0,
            max_lat: 40.0,
        });
        full.satellites = vec!["goes-east".into()];
        full.until = Some(Utc.with_ymd_and_hms(2026, 10, 7, 21, 0, 0).unwrap());
        let sql = build_query(&full).sql().to_string();
        assert!(sql.contains("satellite = ANY"));
        assert!(sql.contains("flash_time <="));
        assert!(sql.contains("location && ST_MakeEnvelope"));
        assert!(
            !sql.contains("ST_DWithin"),
            "bbox must not use the radius test"
        );
    }

    #[test]
    fn radius_uses_a_planar_prefilter_and_an_exact_geodesic_test() {
        let sql = build_query(&q(FlashArea::Radius {
            lon: -105.2,
            lat: 39.7,
            meters: 5000.0,
        }))
        .sql()
        .to_string();
        assert!(
            sql.contains("location && ST_MakeEnvelope"),
            "needs the index prefilter"
        );
        assert!(
            sql.contains("ST_DWithin(location::geography"),
            "needs the exact distance test"
        );
    }

    #[test]
    fn user_values_are_always_bound_never_interpolated() {
        let mut query = q(FlashArea::Anywhere);
        query.satellites = vec!["x'; DROP TABLE lightning_flashes; --".into()];
        let sql = build_query(&query).sql().to_string();
        assert!(!sql.contains("DROP"), "{sql}");
    }

    /// The prefilter must contain every point inside the circle, or the query
    /// would silently drop true matches. Checked by brute force over a grid of
    /// latitudes (including near the pole) and radii.
    #[test]
    fn radius_prefilter_box_contains_the_whole_circle() {
        fn haversine_m(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
            let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
            let (dp, dl) = ((lat2 - lat1).to_radians(), (lon2 - lon1).to_radians());
            let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
            2.0 * 6_371_008.8 * a.sqrt().asin()
        }
        for &lat in &[0.0, 24.0, 39.7, 50.0, 70.0, 85.0] {
            for &meters in &[1_000.0, 25_000.0, 100_000.0, 500_000.0] {
                let [x0, y0, x1, y1] = radius_prefilter_box(-105.0, lat, meters);
                // sweep the circle's boundary and a bit inside it
                for deg in (0..360).step_by(5) {
                    for frac in [0.5, 0.99] {
                        let bearing = (deg as f64).to_radians();
                        let d = meters * frac / 6_371_008.8;
                        let (la1, lo1) = (lat.to_radians(), (-105.0f64).to_radians());
                        let la2 =
                            (la1.sin() * d.cos() + la1.cos() * d.sin() * bearing.cos()).asin();
                        let lo2 = lo1
                            + (bearing.sin() * d.sin() * la1.cos())
                                .atan2(d.cos() - la1.sin() * la2.sin());
                        let (plat, plon) = (la2.to_degrees(), lo2.to_degrees());
                        assert!(haversine_m(-105.0, lat, plon, plat) <= meters * 1.001);
                        assert!(
                            plon >= x0 && plon <= x1 && plat >= y0 && plat <= y1,
                            "point ({plon},{plat}) at bearing {deg} frac {frac} escapes box [{x0},{y0},{x1},{y1}] (lat {lat}, r {meters})"
                        );
                    }
                }
            }
        }
    }
}
