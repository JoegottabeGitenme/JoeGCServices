//! Storage and retrieval for SPC / NOAA Storm Events severe-convective reports.
//!
//! This module backs the EDR `hail`, `wind`, and `tornado` feature collections.
//! Unlike point observations, storm events carry mixed geometry: hail and wind
//! reports are points (`geom_point`), while tornadoes also carry a track
//! (`geom_track`, a LineString derived from begin/end coordinates).
//!
//! All spatial work happens in PostGIS. Geometry is returned to callers as raw
//! GeoJSON strings (via `ST_AsGeoJSON`) since the workspace intentionally avoids
//! Rust geometry crates.
//!
//! ## County stamping
//!
//! At ingest, each event's `county_fips` is derived by a spatial join against
//! the `tiger_counties` table (TIGER/Line county polygons) using the begin
//! point. This is the source of truth — the CSV's own CZ_FIPS is kept raw for
//! reference but not used for aggregation.

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use wms_common::{WmsError, WmsResult};

/// A single storm event report (hail, thunderstorm wind, or tornado).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StormEvent {
    /// NOAA Storm Events `EVENT_ID` (primary key / dedup key).
    pub event_id: i64,
    /// Episode this event belongs to (`EPISODE_ID`).
    pub episode_id: Option<i64>,
    /// Normalized event type: `"hail"`, `"wind"`, or `"tornado"`.
    pub event_type: String,
    /// Event begin time (UTC).
    pub begin_time: DateTime<Utc>,
    /// Event end time (UTC), if known.
    pub end_time: Option<DateTime<Utc>>,
    /// Begin latitude (WGS84).
    pub begin_lat: Option<f64>,
    /// Begin longitude (WGS84).
    pub begin_lon: Option<f64>,
    /// End latitude (WGS84) — used for tornado tracks.
    pub end_lat: Option<f64>,
    /// End longitude (WGS84) — used for tornado tracks.
    pub end_lon: Option<f64>,
    /// Canonical magnitude (hail inches, wind knots).
    pub magnitude: Option<f64>,
    /// Unit of `magnitude` (e.g. `"in"`, `"kt"`).
    pub magnitude_unit: Option<String>,
    /// Tornado EF/F scale (0..5), tornadoes only.
    pub tor_f_scale: Option<i16>,
    /// State name from the CSV.
    pub state: Option<String>,
    /// County/zone name from the CSV (`CZ_NAME`).
    pub cz_name: Option<String>,
    /// County/zone FIPS from the CSV (`CZ_FIPS`) — raw, not authoritative.
    pub cz_fips: Option<String>,
    /// `CZ_TYPE`: `'C'` (county) or `'Z'` (zone).
    pub cz_type: Option<String>,
    /// Per-event narrative comment from the CSV (`EVENT_NARRATIVE`).
    pub event_narrative: Option<String>,
    /// Storm-system-level narrative comment (`EPISODE_NARRATIVE`).
    pub episode_narrative: Option<String>,
    /// Who reported the event (`SOURCE`, e.g. "Trained Spotter", "ASOS").
    pub report_source: Option<String>,
    /// Raw property damage string (`DAMAGE_PROPERTY`, e.g. "120.00K").
    pub damage_property: Option<String>,
    /// Raw crop damage string (`DAMAGE_CROPS`, e.g. "0.00K").
    pub damage_crops: Option<String>,
    /// Direct injuries (`INJURIES_DIRECT`).
    pub injuries_direct: Option<i32>,
    /// Direct deaths (`DEATHS_DIRECT`).
    pub deaths_direct: Option<i32>,
    /// Human-readable begin location, composed from RANGE + AZIMUTH + LOCATION.
    pub begin_location: Option<String>,
    /// Tornado path length in miles (`TOR_LENGTH`).
    pub tor_length_mi: Option<f64>,
    /// Tornado path width in yards (`TOR_WIDTH`).
    pub tor_width_yd: Option<f64>,
    /// Additional CSV fields preserved as JSON.
    #[serde(default)]
    pub raw: serde_json::Value,
}

/// A storm event with its geometry serialized as GeoJSON, ready for an EDR
/// feature response.
#[derive(Debug, Clone)]
pub struct StormEventFeature {
    /// NOAA `EVENT_ID`.
    pub event_id: i64,
    /// Normalized event type.
    pub event_type: String,
    /// Event begin time (UTC).
    pub begin_time: DateTime<Utc>,
    /// Event end time (UTC), if known.
    pub end_time: Option<DateTime<Utc>>,
    /// Canonical magnitude value.
    pub magnitude: Option<f64>,
    /// Unit of `magnitude`.
    pub magnitude_unit: Option<String>,
    /// Tornado EF/F scale.
    pub tor_f_scale: Option<i16>,
    /// State name.
    pub state: Option<String>,
    /// County/zone name.
    pub cz_name: Option<String>,
    /// Stamped county FIPS (from TIGER join).
    pub county_fips: Option<String>,
    /// Geometry as a raw GeoJSON object string (Point for hail/wind,
    /// LineString for tornadoes with a track).
    pub geometry_geojson: String,
    // -- Report / narrative fields --
    /// Per-event narrative comment (mirrors the text shown on the NOAA/SPC site).
    pub event_narrative: Option<String>,
    /// Storm-system-level narrative.
    pub episode_narrative: Option<String>,
    /// Who reported the event (e.g. "Trained Spotter", "ASOS", "Law Enforcement").
    pub report_source: Option<String>,
    /// Raw property damage string (e.g. "120.00K", "2.50M").
    pub damage_property: Option<String>,
    /// Raw crop damage string.
    pub damage_crops: Option<String>,
    /// Direct injuries.
    pub injuries_direct: Option<i32>,
    /// Direct deaths.
    pub deaths_direct: Option<i32>,
    /// Human-readable begin location (e.g. "3.94 ESE BELKNAP").
    pub begin_location: Option<String>,
    /// Tornado path length in miles.
    pub tor_length_mi: Option<f64>,
    /// Tornado path width in yards.
    pub tor_width_yd: Option<f64>,
}

/// The result of a per-county events query — individual event features plus
/// lightweight county metadata and an ETag seed.
#[derive(Debug)]
pub struct CountyEventsResult {
    /// 5-digit county FIPS.
    pub county_fips: String,
    /// County name from `tiger_counties` (None if TIGER not loaded).
    pub county_name: Option<String>,
    /// State abbreviation from `tiger_counties`.
    pub state: Option<String>,
    /// Individual event features for this county.
    pub features: Vec<StormEventFeature>,
    /// Latest `ingested_at` across all returned rows — used to generate an ETag.
    /// Stable until the next ingest cycle touches this county.
    pub max_ingested_at: Option<DateTime<Utc>>,
}

/// A county-level aggregate row, optionally carrying boundary geometry.
#[derive(Debug, Clone, Serialize)]
pub struct CountyAggregate {
    /// 5-digit county FIPS.
    pub county_fips: String,
    /// County name.
    pub name: Option<String>,
    /// State abbreviation.
    pub state: Option<String>,
    /// Event type these counts apply to.
    pub event_type: String,
    /// Total count across the requested year range.
    pub total: i64,
    /// Per-year counts, keyed by year string.
    pub by_year: std::collections::BTreeMap<String, i64>,
    /// Simplified boundary geometry as a GeoJSON object string, when requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry_geojson: Option<String>,
}

/// Catalog for storm event reports, backed by the shared PostGIS pool.
#[derive(Clone)]
pub struct StormEventCatalog {
    pool: PgPool,
}

impl StormEventCatalog {
    /// Create a new catalog sharing an existing connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Run the storm-events schema migration (idempotent).
    pub async fn migrate(&self) -> WmsResult<()> {
        use crate::catalog::STORM_EVENTS_SCHEMA_SQL;

        for statement in STORM_EVENTS_SCHEMA_SQL.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| {
                        WmsError::DatabaseError(format!("Storm events migration failed: {}", e))
                    })?;
            }
        }

        Ok(())
    }

    /// Upsert a single storm event.
    ///
    /// Geometry is built server-side. For tornadoes with both begin and end
    /// coordinates a LineString track is created; otherwise `geom_track` is
    /// NULL. `county_fips` is stamped via spatial join against `tiger_counties`
    /// using the begin point (NULL when no county contains the point or when
    /// coordinates are missing).
    pub async fn upsert_event(&self, event: &StormEvent) -> WmsResult<()> {
        let is_tornado = event.event_type == "tornado";
        let has_track = is_tornado
            && event.end_lat.is_some()
            && event.end_lon.is_some()
            && event.begin_lat.is_some()
            && event.begin_lon.is_some();

        sqlx::query(
            r#"
            INSERT INTO storm_events (
                event_id, episode_id, event_type, begin_time, end_time,
                geom_point, geom_track,
                begin_lat, begin_lon, end_lat, end_lon,
                magnitude, magnitude_unit, tor_f_scale,
                state, cz_name, cz_fips, cz_type,
                county_fips, source, raw,
                event_narrative, episode_narrative, report_source,
                damage_property, damage_crops,
                injuries_direct, deaths_direct,
                begin_location, tor_length_mi, tor_width_yd,
                ingested_at
            ) VALUES (
                $1, $2, $3, $4, $5,
                CASE WHEN $7::float8 IS NOT NULL AND $6::float8 IS NOT NULL
                     THEN ST_SetSRID(ST_MakePoint($7, $6), 4326) END,
                CASE WHEN $18 THEN
                     ST_SetSRID(ST_MakeLine(ST_MakePoint($7, $6), ST_MakePoint($9, $8)), 4326)
                     END,
                $6, $7, $8, $9,
                $10, $11, $12,
                $13, $14, $15, $16,
                (SELECT geoid FROM tiger_counties
                   WHERE $7::float8 IS NOT NULL AND $6::float8 IS NOT NULL
                     AND ST_Contains(geom, ST_SetSRID(ST_MakePoint($7, $6), 4326))
                   LIMIT 1),
                'storm_events', $17,
                $19, $20, $21,
                $22, $23,
                $24, $25,
                $26, $27, $28,
                NOW()
            )
            ON CONFLICT (event_id) DO UPDATE SET
                episode_id = EXCLUDED.episode_id,
                event_type = EXCLUDED.event_type,
                begin_time = EXCLUDED.begin_time,
                end_time = EXCLUDED.end_time,
                geom_point = EXCLUDED.geom_point,
                geom_track = EXCLUDED.geom_track,
                begin_lat = EXCLUDED.begin_lat,
                begin_lon = EXCLUDED.begin_lon,
                end_lat = EXCLUDED.end_lat,
                end_lon = EXCLUDED.end_lon,
                magnitude = EXCLUDED.magnitude,
                magnitude_unit = EXCLUDED.magnitude_unit,
                tor_f_scale = EXCLUDED.tor_f_scale,
                state = EXCLUDED.state,
                cz_name = EXCLUDED.cz_name,
                cz_fips = EXCLUDED.cz_fips,
                cz_type = EXCLUDED.cz_type,
                county_fips = EXCLUDED.county_fips,
                raw = EXCLUDED.raw,
                event_narrative = EXCLUDED.event_narrative,
                episode_narrative = EXCLUDED.episode_narrative,
                report_source = EXCLUDED.report_source,
                damage_property = EXCLUDED.damage_property,
                damage_crops = EXCLUDED.damage_crops,
                injuries_direct = EXCLUDED.injuries_direct,
                deaths_direct = EXCLUDED.deaths_direct,
                begin_location = EXCLUDED.begin_location,
                tor_length_mi = EXCLUDED.tor_length_mi,
                tor_width_yd = EXCLUDED.tor_width_yd,
                ingested_at = NOW()
            "#,
        )
        .bind(event.event_id) // $1
        .bind(event.episode_id) // $2
        .bind(&event.event_type) // $3
        .bind(event.begin_time) // $4
        .bind(event.end_time) // $5
        .bind(event.begin_lat) // $6
        .bind(event.begin_lon) // $7
        .bind(event.end_lat) // $8
        .bind(event.end_lon) // $9
        .bind(event.magnitude) // $10
        .bind(&event.magnitude_unit) // $11
        .bind(event.tor_f_scale) // $12
        .bind(&event.state) // $13
        .bind(&event.cz_name) // $14
        .bind(&event.cz_fips) // $15
        .bind(&event.cz_type) // $16
        .bind(&event.raw) // $17
        .bind(has_track) // $18
        .bind(&event.event_narrative) // $19
        .bind(&event.episode_narrative) // $20
        .bind(&event.report_source) // $21
        .bind(&event.damage_property) // $22
        .bind(&event.damage_crops) // $23
        .bind(event.injuries_direct) // $24
        .bind(event.deaths_direct) // $25
        .bind(&event.begin_location) // $26
        .bind(event.tor_length_mi) // $27
        .bind(event.tor_width_yd) // $28
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Upsert storm event failed: {}", e)))?;

        Ok(())
    }

    /// Batch upsert. Returns the number of events processed.
    pub async fn upsert_events(&self, events: &[StormEvent]) -> WmsResult<usize> {
        for event in events {
            self.upsert_event(event).await?;
        }
        Ok(events.len())
    }

    /// Get events of a type within `radius_m` meters of (lon, lat), optionally
    /// constrained to a time range. Returns each event's geometry as GeoJSON.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_events_in_radius(
        &self,
        event_type: &str,
        lon: f64,
        lat: f64,
        radius_m: f64,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: i64,
    ) -> WmsResult<Vec<StormEventFeature>> {
        let rows = sqlx::query_as::<_, StormEventRow>(
            r#"
            SELECT event_id, event_type, begin_time, end_time,
                   magnitude, magnitude_unit, tor_f_scale, state, cz_name, county_fips,
                   ST_AsGeoJSON(COALESCE(geom_track, geom_point)) AS geometry_geojson,
                   event_narrative, episode_narrative, report_source,
                   damage_property, damage_crops,
                   injuries_direct, deaths_direct,
                   begin_location, tor_length_mi, tor_width_yd
            FROM storm_events
            WHERE event_type = $1
              AND geom_point IS NOT NULL
              AND ST_DWithin(geom_point::geography,
                             ST_SetSRID(ST_MakePoint($2, $3), 4326)::geography, $4)
              AND ($5::timestamptz IS NULL OR begin_time >= $5)
              AND ($6::timestamptz IS NULL OR begin_time <= $6)
            ORDER BY begin_time DESC
            LIMIT $7
            "#,
        )
        .bind(event_type)
        .bind(lon)
        .bind(lat)
        .bind(radius_m)
        .bind(start)
        .bind(end)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Storm events radius query failed: {}", e)))?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Get events of a type intersecting a bounding box, optionally constrained
    /// to a time range. Uses the combined geometry so tornado tracks crossing
    /// the box edge are included.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_events_in_bbox(
        &self,
        event_type: &str,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: i64,
        offset: i64,
    ) -> WmsResult<Vec<StormEventFeature>> {
        let rows = sqlx::query_as::<_, StormEventRow>(
            r#"
            SELECT event_id, event_type, begin_time, end_time,
                   magnitude, magnitude_unit, tor_f_scale, state, cz_name, county_fips,
                   ST_AsGeoJSON(COALESCE(geom_track, geom_point)) AS geometry_geojson,
                   event_narrative, episode_narrative, report_source,
                   damage_property, damage_crops,
                   injuries_direct, deaths_direct,
                   begin_location, tor_length_mi, tor_width_yd
            FROM storm_events
            WHERE event_type = $1
              AND COALESCE(geom_track, geom_point) IS NOT NULL
              AND ST_Intersects(COALESCE(geom_track, geom_point),
                                ST_MakeEnvelope($2, $3, $4, $5, 4326))
              AND ($6::timestamptz IS NULL OR begin_time >= $6)
              AND ($7::timestamptz IS NULL OR begin_time <= $7)
            ORDER BY begin_time DESC
            LIMIT $8 OFFSET $9
            "#,
        )
        .bind(event_type)
        .bind(min_lon)
        .bind(min_lat)
        .bind(max_lon)
        .bind(max_lat)
        .bind(start)
        .bind(end)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Storm events bbox query failed: {}", e)))?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Fetch a single event by `EVENT_ID`.
    pub async fn get_event_by_id(&self, event_id: i64) -> WmsResult<Option<StormEventFeature>> {
        let row = sqlx::query_as::<_, StormEventRow>(
            r#"
            SELECT event_id, event_type, begin_time, end_time,
                   magnitude, magnitude_unit, tor_f_scale, state, cz_name, county_fips,
                   ST_AsGeoJSON(COALESCE(geom_track, geom_point)) AS geometry_geojson,
                   event_narrative, episode_narrative, report_source,
                   damage_property, damage_crops,
                   injuries_direct, deaths_direct,
                   begin_location, tor_length_mi, tor_width_yd
            FROM storm_events
            WHERE event_id = $1
            "#,
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get storm event by id failed: {}", e)))?;

        Ok(row.map(Into::into))
    }

    /// Get the temporal extent (min/max begin_time) for an event type.
    pub async fn get_event_time_range(
        &self,
        event_type: &str,
    ) -> WmsResult<Option<(DateTime<Utc>, DateTime<Utc>)>> {
        type MinMaxRow = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);
        let row: Option<MinMaxRow> = sqlx::query_as(
            r#"SELECT MIN(begin_time), MAX(begin_time) FROM storm_events WHERE event_type = $1"#,
        )
        .bind(event_type)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Storm events time range failed: {}", e)))?;

        Ok(match row {
            Some((Some(min), Some(max))) => Some((min, max)),
            _ => None,
        })
    }

    /// Get all events of a type within a specific county (by FIPS), with optional
    /// year filtering.  Returns the events, the county name/state from
    /// `tiger_counties`, and the `max(ingested_at)` for ETag generation.
    ///
    /// This is the per-county cache-optimised endpoint:
    /// - Hits the existing `idx_storm_events_county` index directly.
    /// - Default (no year bounds) returns the full 1995-present history.
    /// - Year-bounded calls produce a small fixed set of cache keys per county.
    pub async fn get_events_by_county(
        &self,
        event_type: &str,
        county_fips: &str,
        start_year: Option<i32>,
        end_year: Option<i32>,
    ) -> WmsResult<CountyEventsResult> {
        // Validate FIPS format early (5 ASCII digits).
        if county_fips.len() != 5 || !county_fips.chars().all(|c| c.is_ascii_digit()) {
            return Err(WmsError::DatabaseError(format!(
                "Invalid county FIPS '{}': must be exactly 5 digits",
                county_fips
            )));
        }

        // Look up county name / state from TIGER (may be absent if TIGER not loaded).
        #[derive(sqlx::FromRow)]
        struct CountyMeta {
            name: Option<String>,
            state_abbr: Option<String>,
        }
        let meta: Option<CountyMeta> =
            sqlx::query_as("SELECT name, state_abbr FROM tiger_counties WHERE geoid = $1")
                .bind(county_fips)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    WmsError::DatabaseError(format!("County metadata lookup failed: {}", e))
                })?;

        // Fetch events and max(ingested_at) in one query.
        #[derive(sqlx::FromRow)]
        struct EventWithMeta {
            event_id: i64,
            event_type: String,
            begin_time: DateTime<Utc>,
            end_time: Option<DateTime<Utc>>,
            magnitude: Option<f64>,
            magnitude_unit: Option<String>,
            tor_f_scale: Option<i16>,
            state: Option<String>,
            cz_name: Option<String>,
            county_fips: Option<String>,
            geometry_geojson: Option<String>,
            max_ingested_at: Option<DateTime<Utc>>,
            event_narrative: Option<String>,
            episode_narrative: Option<String>,
            report_source: Option<String>,
            damage_property: Option<String>,
            damage_crops: Option<String>,
            injuries_direct: Option<i32>,
            deaths_direct: Option<i32>,
            begin_location: Option<String>,
            tor_length_mi: Option<f64>,
            tor_width_yd: Option<f64>,
        }

        let rows = sqlx::query_as::<_, EventWithMeta>(
            r#"
            SELECT event_id, event_type, begin_time, end_time,
                   magnitude, magnitude_unit, tor_f_scale, state, cz_name, county_fips,
                   ST_AsGeoJSON(COALESCE(geom_track, geom_point)) AS geometry_geojson,
                   MAX(ingested_at) OVER () AS max_ingested_at,
                   event_narrative, episode_narrative, report_source,
                   damage_property, damage_crops,
                   injuries_direct, deaths_direct,
                   begin_location, tor_length_mi, tor_width_yd
            FROM storm_events
            WHERE event_type = $1
              AND county_fips  = $2
              AND ($3::int IS NULL OR EXTRACT(YEAR FROM begin_time) >= $3)
              AND ($4::int IS NULL OR EXTRACT(YEAR FROM begin_time) <= $4)
            ORDER BY begin_time DESC
            "#,
        )
        .bind(event_type)
        .bind(county_fips)
        .bind(start_year)
        .bind(end_year)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("County events query failed: {}", e)))?;

        let max_ingested_at = rows.first().and_then(|r| r.max_ingested_at);
        let features: Vec<StormEventFeature> = rows
            .into_iter()
            .map(|r| StormEventFeature {
                event_id: r.event_id,
                event_type: r.event_type,
                begin_time: r.begin_time,
                end_time: r.end_time,
                magnitude: r.magnitude,
                magnitude_unit: r.magnitude_unit,
                tor_f_scale: r.tor_f_scale,
                state: r.state,
                cz_name: r.cz_name,
                county_fips: r.county_fips,
                geometry_geojson: r.geometry_geojson.unwrap_or_else(|| "null".to_string()),
                event_narrative: r.event_narrative,
                episode_narrative: r.episode_narrative,
                report_source: r.report_source,
                damage_property: r.damage_property,
                damage_crops: r.damage_crops,
                injuries_direct: r.injuries_direct,
                deaths_direct: r.deaths_direct,
                begin_location: r.begin_location,
                tor_length_mi: r.tor_length_mi,
                tor_width_yd: r.tor_width_yd,
            })
            .collect();

        Ok(CountyEventsResult {
            county_fips: county_fips.to_string(),
            county_name: meta.as_ref().and_then(|m| m.name.clone()),
            state: meta.as_ref().and_then(|m| m.state_abbr.clone()),
            features,
            max_ingested_at,
        })
    }

    /// Count events of a type (used for collection availability).
    pub async fn count_events(&self, event_type: &str) -> WmsResult<i64> {
        let count: (i64,) =
            sqlx::query_as(r#"SELECT COUNT(*) FROM storm_events WHERE event_type = $1"#)
                .bind(event_type)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    WmsError::DatabaseError(format!("Count storm events failed: {}", e))
                })?;
        Ok(count.0)
    }

    /// County aggregate counts for an event type over a year range, joined to
    /// `tiger_counties` for name + (optionally simplified) boundary geometry.
    ///
    /// `bbox` and `state` are optional filters. `geometry_tolerance` controls
    /// `ST_SimplifyPreserveTopology`; pass `None` to omit geometry entirely.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_county_counts(
        &self,
        event_type: &str,
        start_year: Option<i32>,
        end_year: Option<i32>,
        bbox: Option<(f64, f64, f64, f64)>,
        state: Option<&str>,
        geometry_tolerance: Option<f64>,
    ) -> WmsResult<Vec<CountyAggregate>> {
        let (min_lon, min_lat, max_lon, max_lat) = bbox.unwrap_or((-180.0, -90.0, 180.0, 90.0));
        let has_bbox = bbox.is_some();

        // Geometry expression: either simplified GeoJSON or NULL.
        let geom_select = if geometry_tolerance.is_some() {
            "ST_AsGeoJSON(ST_SimplifyPreserveTopology(c.geom, $8))"
        } else {
            "NULL::text"
        };

        let sql = format!(
            r#"
            SELECT
                m.county_fips,
                c.name AS county_name,
                c.state_abbr,
                m.event_type,
                m.year,
                m.count,
                {geom} AS geometry_geojson
            FROM mv_county_event_counts m
            LEFT JOIN tiger_counties c ON c.geoid = m.county_fips
            WHERE m.event_type = $1
              AND ($2::int IS NULL OR m.year >= $2)
              AND ($3::int IS NULL OR m.year <= $3)
              AND ($4::text IS NULL OR c.state_abbr = $4)
              AND (NOT $5 OR (c.geom IS NOT NULL AND
                   ST_Intersects(c.geom, ST_MakeEnvelope($6, $7, $9, $10, 4326))))
            ORDER BY m.county_fips, m.year
            "#,
            geom = geom_select,
        );

        let rows = sqlx::query_as::<_, CountyCountRow>(&sql)
            .bind(event_type)
            .bind(start_year)
            .bind(end_year)
            .bind(state)
            .bind(has_bbox)
            .bind(min_lon)
            .bind(min_lat)
            .bind(geometry_tolerance.unwrap_or(0.0))
            .bind(max_lon)
            .bind(max_lat)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("County counts query failed: {}", e)))?;

        // Fold per-(county,year) rows into per-county aggregates.
        let mut out: indexmap_lite::OrderedMap = indexmap_lite::OrderedMap::new();
        for r in rows {
            let entry = out.entry(r.county_fips.clone(), || CountyAggregate {
                county_fips: r.county_fips.clone(),
                name: r.county_name.clone(),
                state: r.state_abbr.clone(),
                event_type: r.event_type.clone(),
                total: 0,
                by_year: std::collections::BTreeMap::new(),
                geometry_geojson: r.geometry_geojson.clone(),
            });
            entry.total += r.count;
            *entry.by_year.entry(r.year.to_string()).or_insert(0) += r.count;
            if entry.geometry_geojson.is_none() {
                entry.geometry_geojson = r.geometry_geojson.clone();
            }
        }

        Ok(out.into_values())
    }

    /// Refresh the county-aggregate materialized view (monthly exposure).
    ///
    /// Uses CONCURRENTLY so readers are not blocked; this requires the unique
    /// index defined in the schema. Falls back to a plain refresh if the
    /// concurrent variant fails (e.g. on first populate).
    pub async fn refresh_county_counts(&self) -> WmsResult<()> {
        let concurrent =
            sqlx::query("REFRESH MATERIALIZED VIEW CONCURRENTLY mv_county_event_counts")
                .execute(&self.pool)
                .await;
        if concurrent.is_err() {
            sqlx::query("REFRESH MATERIALIZED VIEW mv_county_event_counts")
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    WmsError::DatabaseError(format!("Refresh county counts failed: {}", e))
                })?;
        }
        Ok(())
    }
}

/// Row type for storm-event feature queries.
#[derive(sqlx::FromRow)]
struct StormEventRow {
    event_id: i64,
    event_type: String,
    begin_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    magnitude: Option<f64>,
    magnitude_unit: Option<String>,
    tor_f_scale: Option<i16>,
    state: Option<String>,
    cz_name: Option<String>,
    county_fips: Option<String>,
    geometry_geojson: Option<String>,
    event_narrative: Option<String>,
    episode_narrative: Option<String>,
    report_source: Option<String>,
    damage_property: Option<String>,
    damage_crops: Option<String>,
    injuries_direct: Option<i32>,
    deaths_direct: Option<i32>,
    begin_location: Option<String>,
    tor_length_mi: Option<f64>,
    tor_width_yd: Option<f64>,
}

impl From<StormEventRow> for StormEventFeature {
    fn from(r: StormEventRow) -> Self {
        StormEventFeature {
            event_id: r.event_id,
            event_type: r.event_type,
            begin_time: r.begin_time,
            end_time: r.end_time,
            magnitude: r.magnitude,
            magnitude_unit: r.magnitude_unit,
            tor_f_scale: r.tor_f_scale,
            state: r.state,
            cz_name: r.cz_name,
            county_fips: r.county_fips,
            geometry_geojson: r.geometry_geojson.unwrap_or_else(|| "null".to_string()),
            event_narrative: r.event_narrative,
            episode_narrative: r.episode_narrative,
            report_source: r.report_source,
            damage_property: r.damage_property,
            damage_crops: r.damage_crops,
            injuries_direct: r.injuries_direct,
            deaths_direct: r.deaths_direct,
            begin_location: r.begin_location,
            tor_length_mi: r.tor_length_mi,
            tor_width_yd: r.tor_width_yd,
        }
    }
}

/// Row type for county-aggregate queries.
#[derive(sqlx::FromRow)]
struct CountyCountRow {
    county_fips: String,
    county_name: Option<String>,
    state_abbr: Option<String>,
    event_type: String,
    year: i32,
    count: i64,
    geometry_geojson: Option<String>,
}

/// Parse a year out of a datetime for aggregate filtering.
pub fn year_of(dt: DateTime<Utc>) -> i32 {
    dt.year()
}

/// Minimal insertion-ordered map so county aggregates keep a stable order
/// without pulling in a new dependency.
mod indexmap_lite {
    use super::CountyAggregate;
    use std::collections::HashMap;

    pub struct OrderedMap {
        order: Vec<String>,
        map: HashMap<String, CountyAggregate>,
    }

    impl OrderedMap {
        pub fn new() -> Self {
            Self {
                order: Vec::new(),
                map: HashMap::new(),
            }
        }

        pub fn entry(
            &mut self,
            key: String,
            default: impl FnOnce() -> CountyAggregate,
        ) -> &mut CountyAggregate {
            if !self.map.contains_key(&key) {
                self.order.push(key.clone());
                self.map.insert(key.clone(), default());
            }
            self.map.get_mut(&key).unwrap()
        }

        pub fn into_values(mut self) -> Vec<CountyAggregate> {
            self.order
                .into_iter()
                .filter_map(|k| self.map.remove(&k))
                .collect()
        }
    }
}
