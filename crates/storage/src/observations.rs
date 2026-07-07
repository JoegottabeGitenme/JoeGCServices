//! Point observation storage for METARs, TAFs, and MADIS data.
//!
//! This module provides storage and retrieval for point observation data,
//! separate from the gridded data stored in MinIO. All observation data
//! is stored directly in PostgreSQL with PostGIS for spatial queries.
//!
//! ## Data Model
//!
//! - `locations`: Canonical registry of observation stations (airports, weather stations)
//! - `observations`: Time-series of surface observations linked to locations
//!
//! ## Units
//!
//! All values are stored in SI units:
//! - Temperature: Kelvin (K)
//! - Wind speed: meters/second (m/s)
//! - Pressure: Pascals (Pa)
//! - Visibility: meters (m)
//! - Precipitation: millimeters (mm)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use wms_common::{WmsError, WmsResult};

/// A geographic location (airport, weather station, city, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    /// Unique identifier (e.g., "KJFK", "KDEN", "NYC").
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Longitude in degrees (WGS84).
    pub lon: f64,
    /// Latitude in degrees (WGS84).
    pub lat: f64,
    /// Elevation in meters above sea level.
    pub elevation_m: Option<f32>,
    /// Location type (e.g., "airport", "mesonet", "city").
    pub location_type: Option<String>,
    /// Country code (e.g., "US", "CA").
    pub country: Option<String>,
    /// Region/state (e.g., "NY", "CA").
    pub region: Option<String>,
    /// Additional properties as JSON.
    #[serde(default)]
    pub properties: serde_json::Value,
}

impl Location {
    /// Create a new location with minimal fields.
    pub fn new(id: impl Into<String>, name: impl Into<String>, lon: f64, lat: f64) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            lon,
            lat,
            elevation_m: None,
            location_type: None,
            country: None,
            region: None,
            properties: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    /// Set the location type.
    pub fn with_type(mut self, location_type: impl Into<String>) -> Self {
        self.location_type = Some(location_type.into());
        self
    }

    /// Set the country code.
    pub fn with_country(mut self, country: impl Into<String>) -> Self {
        self.country = Some(country.into());
        self
    }

    /// Set the region/state.
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set the elevation.
    pub fn with_elevation(mut self, elevation_m: f32) -> Self {
        self.elevation_m = Some(elevation_m);
        self
    }

    /// Merge a key/value into the JSON properties object.
    pub fn with_property(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        if let serde_json::Value::Object(ref mut map) = self.properties {
            map.insert(key.into(), value);
        } else {
            let mut map = serde_json::Map::new();
            map.insert(key.into(), value);
            self.properties = serde_json::Value::Object(map);
        }
        self
    }
}

/// Row shape for populated-place queries (adds a computed `population` column
/// used only for ordering/filtering; not stored on `Location`).
#[derive(sqlx::FromRow)]
struct PopulatedPlaceRow {
    id: String,
    name: String,
    description: Option<String>,
    lon: f64,
    lat: f64,
    elevation_m: Option<f32>,
    location_type: Option<String>,
    country: Option<String>,
    region: Option<String>,
    properties: serde_json::Value,
    #[allow(dead_code)]
    population: i64,
}

impl From<PopulatedPlaceRow> for Location {
    fn from(r: PopulatedPlaceRow) -> Self {
        Location {
            id: r.id,
            name: r.name,
            description: r.description,
            lon: r.lon,
            lat: r.lat,
            elevation_m: r.elevation_m,
            location_type: r.location_type,
            country: r.country,
            region: r.region,
            properties: r.properties,
        }
    }
}

/// A surface weather observation.
///
/// All values are in SI units. Fields are optional because not all
/// observation sources provide all parameters.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Observation {
    /// Unique identifier (auto-generated if not provided).
    pub id: Option<Uuid>,
    /// Location ID (foreign key to locations table).
    pub location_id: String,
    /// Data source (e.g., "metar", "madis_mesonet", "madis_maritime").
    pub source: String,
    /// Observation time (when the observation was made).
    pub obs_time: DateTime<Utc>,
    /// Receipt time (when the observation was received).
    pub receipt_time: Option<DateTime<Utc>>,

    // Core meteorological parameters (SI units)
    /// Air temperature in Kelvin.
    pub temperature_k: Option<f32>,
    /// Dewpoint temperature in Kelvin.
    pub dewpoint_k: Option<f32>,
    /// Wind direction in degrees (0-360, from which wind is blowing).
    pub wind_direction_deg: Option<i16>,
    /// Wind speed in meters per second.
    pub wind_speed_ms: Option<f32>,
    /// Wind gust speed in meters per second.
    pub wind_gust_ms: Option<f32>,
    /// Altimeter setting in Pascals.
    pub altimeter_pa: Option<f32>,
    /// Sea level pressure in Pascals.
    pub sea_level_pressure_pa: Option<f32>,
    /// Visibility in meters.
    pub visibility_m: Option<f32>,
    /// 1-hour precipitation accumulation in millimeters.
    pub precip_1hr_mm: Option<f32>,
    /// Relative humidity in percent (0-100).
    pub relative_humidity_pct: Option<f32>,

    // Ocean/marine parameters (SI units)
    /// Significant wave height in meters.
    pub wave_height_m: Option<f32>,
    /// Dominant wave period in seconds.
    pub dominant_wave_period_s: Option<f32>,
    /// Average wave period in seconds.
    pub average_wave_period_s: Option<f32>,
    /// Mean wave direction in degrees (0-360, from which waves are coming).
    pub mean_wave_direction_deg: Option<i16>,
    /// Sea surface / water temperature in Kelvin.
    pub water_temp_k: Option<f32>,
    /// Water level (tide) above/below MLLW in meters.
    pub tide_m: Option<f32>,
    /// Water column height in meters (DART tsunami buoys).
    pub water_column_height_m: Option<f32>,

    // Aviation-specific fields
    /// Raw observation text (e.g., METAR string).
    pub raw_text: Option<String>,
    /// Flight category (VFR, MVFR, IFR, LIFR).
    pub flight_category: Option<String>,
    /// Weather phenomena string (e.g., "RA BR" for rain and mist).
    pub wx_string: Option<String>,
    /// Cloud layers as JSON array.
    pub cloud_layers: Option<serde_json::Value>,

    // QC flags
    /// Temperature QC flag.
    pub temperature_qc: Option<char>,
    /// Dewpoint QC flag.
    pub dewpoint_qc: Option<char>,
    /// Wind QC flag.
    pub wind_qc: Option<char>,
    /// Pressure QC flag.
    pub pressure_qc: Option<char>,
}

/// Result of a batch observation insert operation.
#[derive(Debug, Clone, Default)]
pub struct ObservationInsertResult {
    /// Number of observations inserted.
    pub inserted: usize,
    /// Number of duplicates skipped.
    pub duplicates: usize,
    /// Number of locations created/updated.
    pub locations_upserted: usize,
}

/// Query parameters for fetching observations.
#[derive(Debug, Clone, Default)]
pub struct ObservationQuery {
    /// Filter by location ID.
    pub location_id: Option<String>,
    /// Filter by source.
    pub source: Option<String>,
    /// Filter by multiple sources.
    pub sources: Option<Vec<String>>,
    /// Start of time range (inclusive).
    pub start_time: Option<DateTime<Utc>>,
    /// End of time range (inclusive).
    pub end_time: Option<DateTime<Utc>>,
    /// Maximum number of results.
    pub limit: Option<i64>,
}

/// Query parameters for spatial observation queries.
#[derive(Debug, Clone)]
pub struct SpatialObservationQuery {
    /// Center longitude for radius queries.
    pub lon: f64,
    /// Center latitude for radius queries.
    pub lat: f64,
    /// Radius in meters for radius queries.
    pub radius_m: Option<f64>,
    /// Bounding box for area queries: (min_lon, min_lat, max_lon, max_lat).
    pub bbox: Option<(f64, f64, f64, f64)>,
    /// Filter by source.
    pub source: Option<String>,
    /// Start of time range.
    pub start_time: Option<DateTime<Utc>>,
    /// End of time range.
    pub end_time: Option<DateTime<Utc>>,
    /// Maximum number of results.
    pub limit: Option<i64>,
}

/// Location with the latest observation data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationWithLatestObs {
    pub location: Location,
    pub latest_obs: Option<Observation>,
    pub obs_count: i64,
}

/// Observation catalog for point observation data.
///
/// Provides methods for storing and querying observation data using PostGIS
/// for spatial operations.
pub struct ObservationCatalog {
    pool: PgPool,
}

impl ObservationCatalog {
    /// Create a new ObservationCatalog using an existing connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Run the observations schema migration.
    ///
    /// This should be called after the main catalog migration.
    /// Requires PostGIS extension to be available.
    pub async fn migrate(&self) -> WmsResult<()> {
        use crate::catalog::OBSERVATIONS_SCHEMA_SQL;

        for statement in OBSERVATIONS_SCHEMA_SQL.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| {
                        WmsError::DatabaseError(format!("Observations migration failed: {}", e))
                    })?;
            }
        }

        Ok(())
    }

    // ========== Location Methods ==========

    /// Upsert a location (insert or update if exists).
    pub async fn upsert_location(&self, location: &Location) -> WmsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO locations (id, name, description, location, elevation_m,
                                   location_type, country, region, properties, updated_at)
            VALUES ($1, $2, $3, ST_SetSRID(ST_MakePoint($4, $5), 4326)::geography,
                    $6, $7, $8, $9, $10, NOW())
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                description = COALESCE(EXCLUDED.description, locations.description),
                location = EXCLUDED.location,
                elevation_m = COALESCE(EXCLUDED.elevation_m, locations.elevation_m),
                location_type = COALESCE(EXCLUDED.location_type, locations.location_type),
                country = COALESCE(EXCLUDED.country, locations.country),
                region = COALESCE(EXCLUDED.region, locations.region),
                properties = locations.properties || EXCLUDED.properties,
                updated_at = NOW()
            "#,
        )
        .bind(&location.id)
        .bind(&location.name)
        .bind(&location.description)
        .bind(location.lon)
        .bind(location.lat)
        .bind(location.elevation_m)
        .bind(&location.location_type)
        .bind(&location.country)
        .bind(&location.region)
        .bind(&location.properties)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Upsert location failed: {}", e)))?;

        Ok(())
    }

    /// Batch upsert multiple locations.
    pub async fn upsert_locations(&self, locations: &[Location]) -> WmsResult<usize> {
        let mut count = 0;
        for location in locations {
            self.upsert_location(location).await?;
            count += 1;
        }
        Ok(count)
    }

    /// Get a location by ID.
    pub async fn get_location(&self, id: &str) -> WmsResult<Option<Location>> {
        #[derive(sqlx::FromRow)]
        struct LocationRow {
            id: String,
            name: String,
            description: Option<String>,
            lon: f64,
            lat: f64,
            elevation_m: Option<f32>,
            location_type: Option<String>,
            country: Option<String>,
            region: Option<String>,
            properties: serde_json::Value,
        }

        let row = sqlx::query_as::<_, LocationRow>(
            r#"
            SELECT id, name, description,
                   ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
                   elevation_m, location_type, country, region, properties
            FROM locations
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get location failed: {}", e)))?;

        Ok(row.map(|r| Location {
            id: r.id,
            name: r.name,
            description: r.description,
            lon: r.lon,
            lat: r.lat,
            elevation_m: r.elevation_m,
            location_type: r.location_type,
            country: r.country,
            region: r.region,
            properties: r.properties,
        }))
    }

    /// Get all locations, optionally filtered by type.
    pub async fn get_locations(&self, location_type: Option<&str>) -> WmsResult<Vec<Location>> {
        #[derive(sqlx::FromRow)]
        struct LocationRow {
            id: String,
            name: String,
            description: Option<String>,
            lon: f64,
            lat: f64,
            elevation_m: Option<f32>,
            location_type: Option<String>,
            country: Option<String>,
            region: Option<String>,
            properties: serde_json::Value,
        }

        let rows = if let Some(loc_type) = location_type {
            sqlx::query_as::<_, LocationRow>(
                r#"
                SELECT id, name, description,
                       ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
                       elevation_m, location_type, country, region, properties
                FROM locations
                WHERE location_type = $1
                ORDER BY id
                "#,
            )
            .bind(loc_type)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, LocationRow>(
                r#"
                SELECT id, name, description,
                       ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
                       elevation_m, location_type, country, region, properties
                FROM locations
                ORDER BY id
                "#,
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| WmsError::DatabaseError(format!("Get locations failed: {}", e)))?;

        Ok(rows
            .into_iter()
            .map(|r| Location {
                id: r.id,
                name: r.name,
                description: r.description,
                lon: r.lon,
                lat: r.lat,
                elevation_m: r.elevation_m,
                location_type: r.location_type,
                country: r.country,
                region: r.region,
                properties: r.properties,
            })
            .collect())
    }

    /// Get locations within a radius of a point.
    pub async fn get_locations_in_radius(
        &self,
        lon: f64,
        lat: f64,
        radius_m: f64,
    ) -> WmsResult<Vec<Location>> {
        #[derive(sqlx::FromRow)]
        struct LocationRow {
            id: String,
            name: String,
            description: Option<String>,
            lon: f64,
            lat: f64,
            elevation_m: Option<f32>,
            location_type: Option<String>,
            country: Option<String>,
            region: Option<String>,
            properties: serde_json::Value,
            #[allow(dead_code)]
            distance_m: f64,
        }

        let rows = sqlx::query_as::<_, LocationRow>(
            r#"
            SELECT id, name, description,
                   ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
                   elevation_m, location_type, country, region, properties,
                   ST_Distance(location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography) as distance_m
            FROM locations
            WHERE ST_DWithin(location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography, $3)
            ORDER BY distance_m
            "#,
        )
        .bind(lon)
        .bind(lat)
        .bind(radius_m)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get locations in radius failed: {}", e)))?;

        Ok(rows
            .into_iter()
            .map(|r| Location {
                id: r.id,
                name: r.name,
                description: r.description,
                lon: r.lon,
                lat: r.lat,
                elevation_m: r.elevation_m,
                location_type: r.location_type,
                country: r.country,
                region: r.region,
                properties: r.properties,
            })
            .collect())
    }

    /// Get locations within a bounding box.
    pub async fn get_locations_in_bbox(
        &self,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
    ) -> WmsResult<Vec<Location>> {
        #[derive(sqlx::FromRow)]
        struct LocationRow {
            id: String,
            name: String,
            description: Option<String>,
            lon: f64,
            lat: f64,
            elevation_m: Option<f32>,
            location_type: Option<String>,
            country: Option<String>,
            region: Option<String>,
            properties: serde_json::Value,
        }

        let rows = sqlx::query_as::<_, LocationRow>(
            r#"
            SELECT id, name, description,
                   ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
                   elevation_m, location_type, country, region, properties
            FROM locations
            WHERE ST_Within(
                location::geometry,
                ST_MakeEnvelope($1, $2, $3, $4, 4326)
            )
            ORDER BY id
            "#,
        )
        .bind(min_lon)
        .bind(min_lat)
        .bind(max_lon)
        .bind(max_lat)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get locations in bbox failed: {}", e)))?;

        Ok(rows
            .into_iter()
            .map(|r| Location {
                id: r.id,
                name: r.name,
                description: r.description,
                lon: r.lon,
                lat: r.lat,
                elevation_m: r.elevation_m,
                location_type: r.location_type,
                country: r.country,
                region: r.region,
                properties: r.properties,
            })
            .collect())
    }

    /// Count total locations.
    pub async fn count_locations(&self) -> WmsResult<i64> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM locations")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Count locations failed: {}", e)))?;

        Ok(count)
    }

    /// Count locations of a given type (e.g. "populated_place", "airport").
    pub async fn count_locations_by_type(&self, location_type: &str) -> WmsResult<i64> {
        let count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM locations WHERE location_type = $1")
                .bind(location_type)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    WmsError::DatabaseError(format!("Count locations by type failed: {}", e))
                })?;

        Ok(count)
    }

    // ========== Populated Places Methods ==========
    //
    // Populated places are stored in the shared `locations` table with
    // location_type = 'populated_place' and population in properties->>'population'.
    // These power the EDR `populated` collection (a coordinate registry the
    // frontend uses to fetch point forecasts without external geocoding).

    /// List populated places, optionally filtered by minimum population and bbox.
    ///
    /// Ordered by population descending (biggest cities first). `limit` caps
    /// the result count.
    pub async fn get_populated_places(
        &self,
        min_population: i64,
        bbox: Option<(f64, f64, f64, f64)>,
        limit: i64,
    ) -> WmsResult<Vec<Location>> {
        // bbox is (min_lon, min_lat, max_lon, max_lat); NULL sentinels when absent
        let (min_lon, min_lat, max_lon, max_lat, has_bbox) = match bbox {
            Some((a, b, c, d)) => (a, b, c, d, true),
            None => (0.0, 0.0, 0.0, 0.0, false),
        };

        let rows = sqlx::query_as::<_, PopulatedPlaceRow>(
            r#"
            SELECT id, name, description,
                   ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
                   elevation_m, location_type, country, region, properties,
                   COALESCE((properties->>'population')::bigint, 0) as population
            FROM locations
            WHERE location_type = 'populated_place'
              AND COALESCE((properties->>'population')::bigint, 0) >= $1
              AND (NOT $6 OR ST_Within(
                  location::geometry,
                  ST_MakeEnvelope($2, $3, $4, $5, 4326)
              ))
            ORDER BY population DESC
            LIMIT $7
            "#,
        )
        .bind(min_population)
        .bind(min_lon)
        .bind(min_lat)
        .bind(max_lon)
        .bind(max_lat)
        .bind(has_bbox)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get populated places failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get populated places within a radius of a point, filtered by population.
    ///
    /// Uses the GiST-indexed `ST_DWithin`; ordered by distance ascending.
    pub async fn get_populated_places_in_radius(
        &self,
        lon: f64,
        lat: f64,
        radius_m: f64,
        min_population: i64,
        limit: i64,
    ) -> WmsResult<Vec<Location>> {
        let rows = sqlx::query_as::<_, PopulatedPlaceRow>(
            r#"
            SELECT id, name, description,
                   ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
                   elevation_m, location_type, country, region, properties,
                   COALESCE((properties->>'population')::bigint, 0) as population
            FROM locations
            WHERE location_type = 'populated_place'
              AND COALESCE((properties->>'population')::bigint, 0) >= $4
              AND ST_DWithin(location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography, $3)
            ORDER BY ST_Distance(location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography)
            LIMIT $5
            "#,
        )
        .bind(lon)
        .bind(lat)
        .bind(radius_m)
        .bind(min_population)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            WmsError::DatabaseError(format!("Get populated places in radius failed: {}", e))
        })?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get a single populated place by ID (only returns type='populated_place').
    pub async fn get_populated_place(&self, id: &str) -> WmsResult<Option<Location>> {
        let row = sqlx::query_as::<_, PopulatedPlaceRow>(
            r#"
            SELECT id, name, description,
                   ST_X(location::geometry) as lon, ST_Y(location::geometry) as lat,
                   elevation_m, location_type, country, region, properties,
                   COALESCE((properties->>'population')::bigint, 0) as population
            FROM locations
            WHERE id = $1 AND location_type = 'populated_place'
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get populated place failed: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    /// Free-text search over populated place names.
    ///
    /// Matching is accent- and punctuation-insensitive: both the query and the
    /// stored name are normalized via `regexp_replace(unaccent(lower(x)),
    /// '[^a-z0-9]', '', 'g')` (so "st louis" matches "St. Louis", "cañon"
    /// matches "Canon"). Results are ranked exact > prefix > substring, then by
    /// population descending.
    ///
    /// - `query`: the city name text (already stripped of any "City, ST" state part).
    /// - `state`: optional 2-letter USPS code to constrain results (matched against `region`).
    /// - `min_population`: optional floor (`0` = no floor).
    /// - `bbox`: optional (min_lon, min_lat, max_lon, max_lat) region bias/filter.
    /// - `limit`: max rows.
    pub async fn search_populated_places(
        &self,
        query: &str,
        state: Option<&str>,
        min_population: i64,
        bbox: Option<(f64, f64, f64, f64)>,
        limit: i64,
    ) -> WmsResult<Vec<Location>> {
        let (min_lon, min_lat, max_lon, max_lat, has_bbox) = match bbox {
            Some((a, b, c, d)) => (a, b, c, d, true),
            None => (0.0, 0.0, 0.0, 0.0, false),
        };

        // Normalized column expression, reused for match + rank.
        // NOTE: keep this in sync with the trigram index in OBSERVATIONS_SCHEMA_SQL.
        let rows = sqlx::query_as::<_, PopulatedPlaceRow>(
            r#"
            WITH q AS (
                SELECT regexp_replace(unaccent(lower($1)), '[^a-z0-9]', '', 'g') AS norm
            )
            SELECT l.id, l.name, l.description,
                   ST_X(l.location::geometry) as lon, ST_Y(l.location::geometry) as lat,
                   l.elevation_m, l.location_type, l.country, l.region, l.properties,
                   COALESCE((l.properties->>'population')::bigint, 0) as population
            FROM locations l, q
            WHERE l.location_type = 'populated_place'
              AND q.norm <> ''
              AND regexp_replace(unaccent(lower(l.name)), '[^a-z0-9]', '', 'g')
                  LIKE '%' || q.norm || '%'
              AND COALESCE((l.properties->>'population')::bigint, 0) >= $2
              AND ($3::text IS NULL OR l.region = $3)
              AND (NOT $8 OR ST_Within(
                  l.location::geometry,
                  ST_MakeEnvelope($4, $5, $6, $7, 4326)
              ))
            ORDER BY
              CASE
                WHEN regexp_replace(unaccent(lower(l.name)), '[^a-z0-9]', '', 'g') = q.norm THEN 0
                WHEN regexp_replace(unaccent(lower(l.name)), '[^a-z0-9]', '', 'g') LIKE q.norm || '%' THEN 1
                ELSE 2
              END ASC,
              COALESCE((l.properties->>'population')::bigint, 0) DESC
            LIMIT $9
            "#,
        )
        .bind(query)
        .bind(min_population)
        .bind(state)
        .bind(min_lon)
        .bind(min_lat)
        .bind(max_lon)
        .bind(max_lat)
        .bind(has_bbox)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Search populated places failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    // ========== ZIP Code Methods ==========

    /// Look up a ZIP code (exact 5-digit match) and enrich it with the nearest
    /// populated place (for a "City, ST" display label).
    ///
    /// `zip` may be given with or without the "ZIP" prefix ("80202" or
    /// "ZIP80202"). Returns None if the ZIP is unknown. The returned
    /// `Location`'s `properties` includes:
    ///   - `zip`: the 5-digit code
    ///   - `nearest_place`: nearest city name (if any within ~40 km)
    ///   - `nearest_state`: that city's state
    /// `name` is set to the nearest place name when available (else the ZIP),
    /// and `region` to the nearest place's state, so ZIP features render like
    /// populated places.
    pub async fn get_zip_code(&self, zip: &str) -> WmsResult<Option<Location>> {
        // Accept "80202", "ZIP80202", or ZIP+4 "80202-1234" -> normalize to the
        // 5-digit code.
        let stripped = zip.trim().trim_start_matches("ZIP");
        let code = stripped.split('-').next().unwrap_or(stripped).trim();
        let id = format!("ZIP{}", code);

        #[derive(sqlx::FromRow)]
        struct ZipRow {
            id: String,
            zip: String,
            lon: f64,
            lat: f64,
            near_name: Option<String>,
            near_state: Option<String>,
        }

        // Find the ZIP, then LATERAL-join a representative populated place for
        // the label. We prefer the most recognizable nearby city rather than
        // the strictly-nearest centroid: any city within 8 km is treated as
        // "here", and among those (or, failing that, all cities within 40 km)
        // we pick the most populous, tie-broken by distance. This makes a
        // Denver ZIP say "Denver" rather than an adjacent enclave.
        let row = sqlx::query_as::<_, ZipRow>(
            r#"
            SELECT z.id,
                   z.properties->>'zip' AS zip,
                   ST_X(z.location::geometry) AS lon,
                   ST_Y(z.location::geometry) AS lat,
                   p.name AS near_name,
                   p.region AS near_state
            FROM locations z
            LEFT JOIN LATERAL (
                SELECT name, region
                FROM locations pp
                WHERE pp.location_type = 'populated_place'
                  AND ST_DWithin(pp.location, z.location, 40000)
                ORDER BY
                    (ST_DWithin(pp.location, z.location, 8000)) DESC,
                    COALESCE((pp.properties->>'population')::bigint, 0) DESC,
                    ST_Distance(pp.location, z.location) ASC
                LIMIT 1
            ) p ON true
            WHERE z.id = $1 AND z.location_type = 'zip'
            "#,
        )
        .bind(&id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get ZIP code failed: {}", e)))?;

        Ok(row.map(|r| {
            let display_name = r.near_name.clone().unwrap_or_else(|| r.zip.clone());
            let mut loc = Location::new(r.id, display_name, r.lon, r.lat)
                .with_type("zip")
                .with_country("US")
                .with_property("zip", serde_json::Value::String(r.zip.clone()));
            if let Some(ref name) = r.near_name {
                loc = loc.with_property("nearest_place", serde_json::Value::String(name.clone()));
            }
            if let Some(ref st) = r.near_state {
                loc = loc
                    .with_region(st.clone())
                    .with_property("nearest_state", serde_json::Value::String(st.clone()))
                    .with_property("state", serde_json::Value::String(st.clone()));
            }
            loc
        }))
    }

    // ========== Observation Methods ==========

    /// Insert a single observation.
    ///
    /// Returns true if inserted, false if duplicate (already exists).
    pub async fn insert_observation(&self, obs: &Observation) -> WmsResult<bool> {
        let result = sqlx::query(
            r#"
            INSERT INTO observations (
                location_id, source, obs_time, receipt_time,
                temperature_k, dewpoint_k, wind_direction_deg, wind_speed_ms, wind_gust_ms,
                altimeter_pa, sea_level_pressure_pa, visibility_m, precip_1hr_mm, relative_humidity_pct,
                wave_height_m, dominant_wave_period_s, average_wave_period_s,
                mean_wave_direction_deg, water_temp_k, tide_m, water_column_height_m,
                raw_text, flight_category, wx_string, cloud_layers,
                temperature_qc, dewpoint_qc, wind_qc, pressure_qc
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                $15, $16, $17, $18, $19, $20, $21,
                $22, $23, $24, $25, $26, $27, $28, $29
            )
            ON CONFLICT (location_id, source, obs_time) DO NOTHING
            "#,
        )
        .bind(&obs.location_id)
        .bind(&obs.source)
        .bind(obs.obs_time)
        .bind(obs.receipt_time)
        .bind(obs.temperature_k)
        .bind(obs.dewpoint_k)
        .bind(obs.wind_direction_deg)
        .bind(obs.wind_speed_ms)
        .bind(obs.wind_gust_ms)
        .bind(obs.altimeter_pa)
        .bind(obs.sea_level_pressure_pa)
        .bind(obs.visibility_m)
        .bind(obs.precip_1hr_mm)
        .bind(obs.relative_humidity_pct)
        .bind(obs.wave_height_m)
        .bind(obs.dominant_wave_period_s)
        .bind(obs.average_wave_period_s)
        .bind(obs.mean_wave_direction_deg)
        .bind(obs.water_temp_k)
        .bind(obs.tide_m)
        .bind(obs.water_column_height_m)
        .bind(&obs.raw_text)
        .bind(&obs.flight_category)
        .bind(&obs.wx_string)
        .bind(&obs.cloud_layers)
        .bind(obs.temperature_qc.map(|c| c.to_string()))
        .bind(obs.dewpoint_qc.map(|c| c.to_string()))
        .bind(obs.wind_qc.map(|c| c.to_string()))
        .bind(obs.pressure_qc.map(|c| c.to_string()))
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Insert observation failed: {}", e)))?;

        Ok(result.rows_affected() > 0)
    }

    /// Batch insert observations.
    ///
    /// Returns statistics about the insert operation.
    pub async fn insert_observations(
        &self,
        observations: &[Observation],
    ) -> WmsResult<ObservationInsertResult> {
        let mut result = ObservationInsertResult::default();

        for obs in observations {
            let inserted = self.insert_observation(obs).await?;
            if inserted {
                result.inserted += 1;
            } else {
                result.duplicates += 1;
            }
        }

        Ok(result)
    }

    /// Get observations for a location within a time range.
    ///
    /// Uses a pattern-matched query approach for type safety rather than dynamic SQL.
    pub async fn get_observations(&self, query: &ObservationQuery) -> WmsResult<Vec<Observation>> {
        #[derive(sqlx::FromRow)]
        struct ObsRow {
            id: Uuid,
            location_id: String,
            source: String,
            obs_time: DateTime<Utc>,
            receipt_time: Option<DateTime<Utc>>,
            temperature_k: Option<f32>,
            dewpoint_k: Option<f32>,
            wind_direction_deg: Option<i16>,
            wind_speed_ms: Option<f32>,
            wind_gust_ms: Option<f32>,
            altimeter_pa: Option<f32>,
            sea_level_pressure_pa: Option<f32>,
            visibility_m: Option<f32>,
            precip_1hr_mm: Option<f32>,
            relative_humidity_pct: Option<f32>,
            wave_height_m: Option<f32>,
            dominant_wave_period_s: Option<f32>,
            average_wave_period_s: Option<f32>,
            mean_wave_direction_deg: Option<i16>,
            water_temp_k: Option<f32>,
            tide_m: Option<f32>,
            water_column_height_m: Option<f32>,
            raw_text: Option<String>,
            flight_category: Option<String>,
            wx_string: Option<String>,
            cloud_layers: Option<serde_json::Value>,
            temperature_qc: Option<String>,
            dewpoint_qc: Option<String>,
            wind_qc: Option<String>,
            pressure_qc: Option<String>,
        }

        let rows = match (&query.location_id, &query.source, query.start_time, query.end_time) {
            (Some(loc_id), Some(source), Some(start), Some(end)) => {
                sqlx::query_as::<_, ObsRow>(
                    r#"
                     SELECT id, location_id, source, obs_time, receipt_time,
                           temperature_k, dewpoint_k, wind_direction_deg, wind_speed_ms, wind_gust_ms,
                           altimeter_pa, sea_level_pressure_pa, visibility_m, precip_1hr_mm, relative_humidity_pct,
                            wave_height_m, dominant_wave_period_s, average_wave_period_s,
                            mean_wave_direction_deg, water_temp_k, tide_m, water_column_height_m,
                            raw_text, flight_category, wx_string, cloud_layers,
                            temperature_qc, dewpoint_qc, wind_qc, pressure_qc
                    FROM observations
                    WHERE location_id = $1 AND source = $2 AND obs_time >= $3 AND obs_time <= $4
                    ORDER BY obs_time DESC
                    LIMIT $5
                    "#,
                )
                .bind(loc_id)
                .bind(source)
                .bind(start)
                .bind(end)
                .bind(query.limit.unwrap_or(1000))
                .fetch_all(&self.pool)
                .await
            }
            (Some(loc_id), None, Some(start), Some(end)) => {
                sqlx::query_as::<_, ObsRow>(
                    r#"
                     SELECT id, location_id, source, obs_time, receipt_time,
                           temperature_k, dewpoint_k, wind_direction_deg, wind_speed_ms, wind_gust_ms,
                           altimeter_pa, sea_level_pressure_pa, visibility_m, precip_1hr_mm, relative_humidity_pct,
                            wave_height_m, dominant_wave_period_s, average_wave_period_s,
                            mean_wave_direction_deg, water_temp_k, tide_m, water_column_height_m,
                            raw_text, flight_category, wx_string, cloud_layers,
                            temperature_qc, dewpoint_qc, wind_qc, pressure_qc
                    FROM observations
                    WHERE location_id = $1 AND obs_time >= $2 AND obs_time <= $3
                    ORDER BY obs_time DESC
                    LIMIT $4
                    "#,
                )
                .bind(loc_id)
                .bind(start)
                .bind(end)
                .bind(query.limit.unwrap_or(1000))
                .fetch_all(&self.pool)
                .await
            }
            (Some(loc_id), Some(source), None, None) => {
                sqlx::query_as::<_, ObsRow>(
                    r#"
                     SELECT id, location_id, source, obs_time, receipt_time,
                           temperature_k, dewpoint_k, wind_direction_deg, wind_speed_ms, wind_gust_ms,
                           altimeter_pa, sea_level_pressure_pa, visibility_m, precip_1hr_mm, relative_humidity_pct,
                            wave_height_m, dominant_wave_period_s, average_wave_period_s,
                            mean_wave_direction_deg, water_temp_k, tide_m, water_column_height_m,
                            raw_text, flight_category, wx_string, cloud_layers,
                            temperature_qc, dewpoint_qc, wind_qc, pressure_qc
                    FROM observations
                    WHERE location_id = $1 AND source = $2
                    ORDER BY obs_time DESC
                    LIMIT $3
                    "#,
                )
                .bind(loc_id)
                .bind(source)
                .bind(query.limit.unwrap_or(1000))
                .fetch_all(&self.pool)
                .await
            }
            (Some(loc_id), None, None, None) => {
                sqlx::query_as::<_, ObsRow>(
                    r#"
                     SELECT id, location_id, source, obs_time, receipt_time,
                           temperature_k, dewpoint_k, wind_direction_deg, wind_speed_ms, wind_gust_ms,
                           altimeter_pa, sea_level_pressure_pa, visibility_m, precip_1hr_mm, relative_humidity_pct,
                            wave_height_m, dominant_wave_period_s, average_wave_period_s,
                            mean_wave_direction_deg, water_temp_k, tide_m, water_column_height_m,
                            raw_text, flight_category, wx_string, cloud_layers,
                            temperature_qc, dewpoint_qc, wind_qc, pressure_qc
                    FROM observations
                    WHERE location_id = $1
                    ORDER BY obs_time DESC
                    LIMIT $2
                    "#,
                )
                .bind(loc_id)
                .bind(query.limit.unwrap_or(1000))
                .fetch_all(&self.pool)
                .await
            }
            _ => {
                // Default: get recent observations
                sqlx::query_as::<_, ObsRow>(
                    r#"
                     SELECT id, location_id, source, obs_time, receipt_time,
                           temperature_k, dewpoint_k, wind_direction_deg, wind_speed_ms, wind_gust_ms,
                           altimeter_pa, sea_level_pressure_pa, visibility_m, precip_1hr_mm, relative_humidity_pct,
                            wave_height_m, dominant_wave_period_s, average_wave_period_s,
                            mean_wave_direction_deg, water_temp_k, tide_m, water_column_height_m,
                            raw_text, flight_category, wx_string, cloud_layers,
                            temperature_qc, dewpoint_qc, wind_qc, pressure_qc
                    FROM observations
                    ORDER BY obs_time DESC
                    LIMIT $1
                    "#,
                )
                .bind(query.limit.unwrap_or(100))
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| WmsError::DatabaseError(format!("Get observations failed: {}", e)))?;

        Ok(rows
            .into_iter()
            .map(|r| Observation {
                id: Some(r.id),
                location_id: r.location_id,
                source: r.source,
                obs_time: r.obs_time,
                receipt_time: r.receipt_time,
                temperature_k: r.temperature_k,
                dewpoint_k: r.dewpoint_k,
                wind_direction_deg: r.wind_direction_deg,
                wind_speed_ms: r.wind_speed_ms,
                wind_gust_ms: r.wind_gust_ms,
                altimeter_pa: r.altimeter_pa,
                sea_level_pressure_pa: r.sea_level_pressure_pa,
                visibility_m: r.visibility_m,
                precip_1hr_mm: r.precip_1hr_mm,
                relative_humidity_pct: r.relative_humidity_pct,
                wave_height_m: r.wave_height_m,
                dominant_wave_period_s: r.dominant_wave_period_s,
                average_wave_period_s: r.average_wave_period_s,
                mean_wave_direction_deg: r.mean_wave_direction_deg,
                water_temp_k: r.water_temp_k,
                tide_m: r.tide_m,
                water_column_height_m: r.water_column_height_m,
                raw_text: r.raw_text,
                flight_category: r.flight_category,
                wx_string: r.wx_string,
                cloud_layers: r.cloud_layers,
                temperature_qc: r.temperature_qc.and_then(|s| s.chars().next()),
                dewpoint_qc: r.dewpoint_qc.and_then(|s| s.chars().next()),
                wind_qc: r.wind_qc.and_then(|s| s.chars().next()),
                pressure_qc: r.pressure_qc.and_then(|s| s.chars().next()),
            })
            .collect())
    }

    /// Get the latest observation for a location.
    pub async fn get_latest_observation(
        &self,
        location_id: &str,
        source: Option<&str>,
    ) -> WmsResult<Option<Observation>> {
        let query = ObservationQuery {
            location_id: Some(location_id.to_string()),
            source: source.map(|s| s.to_string()),
            limit: Some(1),
            ..Default::default()
        };

        let mut obs = self.get_observations(&query).await?;
        Ok(obs.pop())
    }

    /// Get observations within a radius of a point.
    pub async fn get_observations_in_radius(
        &self,
        lon: f64,
        lat: f64,
        radius_m: f64,
        source: Option<&str>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        limit: Option<i64>,
    ) -> WmsResult<Vec<(Location, Observation)>> {
        #[derive(sqlx::FromRow)]
        struct JoinedRow {
            // Location fields
            loc_id: String,
            loc_name: String,
            loc_description: Option<String>,
            loc_lon: f64,
            loc_lat: f64,
            loc_elevation_m: Option<f32>,
            loc_type: Option<String>,
            loc_country: Option<String>,
            loc_region: Option<String>,
            loc_properties: serde_json::Value,
            #[allow(dead_code)]
            distance_m: f64,
            // Observation fields
            obs_id: Uuid,
            obs_source: String,
            obs_time: DateTime<Utc>,
            receipt_time: Option<DateTime<Utc>>,
            temperature_k: Option<f32>,
            dewpoint_k: Option<f32>,
            wind_direction_deg: Option<i16>,
            wind_speed_ms: Option<f32>,
            wind_gust_ms: Option<f32>,
            altimeter_pa: Option<f32>,
            sea_level_pressure_pa: Option<f32>,
            visibility_m: Option<f32>,
            precip_1hr_mm: Option<f32>,
            relative_humidity_pct: Option<f32>,
            wave_height_m: Option<f32>,
            dominant_wave_period_s: Option<f32>,
            average_wave_period_s: Option<f32>,
            mean_wave_direction_deg: Option<i16>,
            water_temp_k: Option<f32>,
            tide_m: Option<f32>,
            water_column_height_m: Option<f32>,
            raw_text: Option<String>,
            flight_category: Option<String>,
            wx_string: Option<String>,
            cloud_layers: Option<serde_json::Value>,
        }

        let effective_limit = limit.unwrap_or(500);

        let rows = match (source, start_time, end_time) {
            (Some(src), Some(start), Some(end)) => {
                sqlx::query_as::<_, JoinedRow>(
                    r#"
                    SELECT
                        l.id as loc_id, l.name as loc_name, l.description as loc_description,
                        ST_X(l.location::geometry) as loc_lon, ST_Y(l.location::geometry) as loc_lat,
                        l.elevation_m as loc_elevation_m, l.location_type as loc_type,
                        l.country as loc_country, l.region as loc_region, l.properties as loc_properties,
                        ST_Distance(l.location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography) as distance_m,
                        o.id as obs_id, o.source as obs_source, o.obs_time, o.receipt_time,
                        o.temperature_k, o.dewpoint_k, o.wind_direction_deg, o.wind_speed_ms, o.wind_gust_ms,
                        o.altimeter_pa, o.sea_level_pressure_pa, o.visibility_m, o.precip_1hr_mm, o.relative_humidity_pct,
                        o.wave_height_m, o.dominant_wave_period_s, o.average_wave_period_s,
                        o.mean_wave_direction_deg, o.water_temp_k, o.tide_m, o.water_column_height_m,
                        o.raw_text, o.flight_category, o.wx_string, o.cloud_layers
                    FROM locations l
                    JOIN observations o ON l.id = o.location_id
                    WHERE ST_DWithin(l.location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography, $3)
                      AND o.source = $4
                      AND o.obs_time >= $5 AND o.obs_time <= $6
                    ORDER BY distance_m, o.obs_time DESC
                    LIMIT $7
                    "#,
                )
                .bind(lon)
                .bind(lat)
                .bind(radius_m)
                .bind(src)
                .bind(start)
                .bind(end)
                .bind(effective_limit)
                .fetch_all(&self.pool)
                .await
            }
            (Some(src), None, None) => {
                sqlx::query_as::<_, JoinedRow>(
                    r#"
                    SELECT
                        l.id as loc_id, l.name as loc_name, l.description as loc_description,
                        ST_X(l.location::geometry) as loc_lon, ST_Y(l.location::geometry) as loc_lat,
                        l.elevation_m as loc_elevation_m, l.location_type as loc_type,
                        l.country as loc_country, l.region as loc_region, l.properties as loc_properties,
                        ST_Distance(l.location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography) as distance_m,
                        o.id as obs_id, o.source as obs_source, o.obs_time, o.receipt_time,
                        o.temperature_k, o.dewpoint_k, o.wind_direction_deg, o.wind_speed_ms, o.wind_gust_ms,
                        o.altimeter_pa, o.sea_level_pressure_pa, o.visibility_m, o.precip_1hr_mm, o.relative_humidity_pct,
                        o.wave_height_m, o.dominant_wave_period_s, o.average_wave_period_s,
                        o.mean_wave_direction_deg, o.water_temp_k, o.tide_m, o.water_column_height_m,
                        o.raw_text, o.flight_category, o.wx_string, o.cloud_layers
                    FROM locations l
                    JOIN observations o ON l.id = o.location_id
                    WHERE ST_DWithin(l.location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography, $3)
                      AND o.source = $4
                    ORDER BY distance_m, o.obs_time DESC
                    LIMIT $5
                    "#,
                )
                .bind(lon)
                .bind(lat)
                .bind(radius_m)
                .bind(src)
                .bind(effective_limit)
                .fetch_all(&self.pool)
                .await
            }
            _ => {
                sqlx::query_as::<_, JoinedRow>(
                    r#"
                    SELECT
                        l.id as loc_id, l.name as loc_name, l.description as loc_description,
                        ST_X(l.location::geometry) as loc_lon, ST_Y(l.location::geometry) as loc_lat,
                        l.elevation_m as loc_elevation_m, l.location_type as loc_type,
                        l.country as loc_country, l.region as loc_region, l.properties as loc_properties,
                        ST_Distance(l.location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography) as distance_m,
                        o.id as obs_id, o.source as obs_source, o.obs_time, o.receipt_time,
                        o.temperature_k, o.dewpoint_k, o.wind_direction_deg, o.wind_speed_ms, o.wind_gust_ms,
                        o.altimeter_pa, o.sea_level_pressure_pa, o.visibility_m, o.precip_1hr_mm, o.relative_humidity_pct,
                        o.wave_height_m, o.dominant_wave_period_s, o.average_wave_period_s,
                        o.mean_wave_direction_deg, o.water_temp_k, o.tide_m, o.water_column_height_m,
                        o.raw_text, o.flight_category, o.wx_string, o.cloud_layers
                    FROM locations l
                    JOIN observations o ON l.id = o.location_id
                    WHERE ST_DWithin(l.location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography, $3)
                    ORDER BY distance_m, o.obs_time DESC
                    LIMIT $4
                    "#,
                )
                .bind(lon)
                .bind(lat)
                .bind(radius_m)
                .bind(effective_limit)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| WmsError::DatabaseError(format!("Get observations in radius failed: {}", e)))?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let location = Location {
                    id: r.loc_id.clone(),
                    name: r.loc_name,
                    description: r.loc_description,
                    lon: r.loc_lon,
                    lat: r.loc_lat,
                    elevation_m: r.loc_elevation_m,
                    location_type: r.loc_type,
                    country: r.loc_country,
                    region: r.loc_region,
                    properties: r.loc_properties,
                };
                let observation = Observation {
                    id: Some(r.obs_id),
                    location_id: r.loc_id,
                    source: r.obs_source,
                    obs_time: r.obs_time,
                    receipt_time: r.receipt_time,
                    temperature_k: r.temperature_k,
                    dewpoint_k: r.dewpoint_k,
                    wind_direction_deg: r.wind_direction_deg,
                    wind_speed_ms: r.wind_speed_ms,
                    wind_gust_ms: r.wind_gust_ms,
                    altimeter_pa: r.altimeter_pa,
                    sea_level_pressure_pa: r.sea_level_pressure_pa,
                    visibility_m: r.visibility_m,
                    precip_1hr_mm: r.precip_1hr_mm,
                    relative_humidity_pct: r.relative_humidity_pct,
                    wave_height_m: r.wave_height_m,
                    dominant_wave_period_s: r.dominant_wave_period_s,
                    average_wave_period_s: r.average_wave_period_s,
                    mean_wave_direction_deg: r.mean_wave_direction_deg,
                    water_temp_k: r.water_temp_k,
                    tide_m: r.tide_m,
                    water_column_height_m: r.water_column_height_m,
                    raw_text: r.raw_text,
                    flight_category: r.flight_category,
                    wx_string: r.wx_string,
                    cloud_layers: r.cloud_layers,
                    ..Default::default()
                };
                (location, observation)
            })
            .collect())
    }

    /// Count total observations.
    pub async fn count_observations(&self, source: Option<&str>) -> WmsResult<i64> {
        let count = if let Some(src) = source {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM observations WHERE source = $1")
                .bind(src)
                .fetch_one(&self.pool)
                .await
        } else {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM observations")
                .fetch_one(&self.pool)
                .await
        }
        .map_err(|e| WmsError::DatabaseError(format!("Count observations failed: {}", e)))?;

        Ok(count)
    }

    /// Get the time range of observations for a source.
    ///
    /// Returns (oldest_time, newest_time) or None if no observations exist.
    pub async fn get_observation_time_range(
        &self,
        source: &str,
    ) -> WmsResult<Option<(DateTime<Utc>, DateTime<Utc>)>> {
        #[derive(sqlx::FromRow)]
        struct TimeRange {
            min_time: Option<DateTime<Utc>>,
            max_time: Option<DateTime<Utc>>,
        }

        let range = sqlx::query_as::<_, TimeRange>(
            "SELECT MIN(obs_time) as min_time, MAX(obs_time) as max_time FROM observations WHERE source = $1",
        )
        .bind(source)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get time range failed: {}", e)))?;

        match (range.min_time, range.max_time) {
            (Some(min), Some(max)) => Ok(Some((min, max))),
            _ => Ok(None),
        }
    }

    /// Delete observations older than a given time.
    ///
    /// Returns the number of rows deleted.
    pub async fn delete_observations_before(
        &self,
        before: DateTime<Utc>,
        source: Option<&str>,
    ) -> WmsResult<u64> {
        let result = if let Some(src) = source {
            sqlx::query("DELETE FROM observations WHERE obs_time < $1 AND source = $2")
                .bind(before)
                .bind(src)
                .execute(&self.pool)
                .await
        } else {
            sqlx::query("DELETE FROM observations WHERE obs_time < $1")
                .bind(before)
                .execute(&self.pool)
                .await
        }
        .map_err(|e| WmsError::DatabaseError(format!("Delete observations failed: {}", e)))?;

        Ok(result.rows_affected())
    }

    /// Get locations that have observations from a specific source.
    pub async fn get_locations_with_observations(
        &self,
        source: &str,
        since: Option<DateTime<Utc>>,
    ) -> WmsResult<Vec<Location>> {
        #[derive(sqlx::FromRow)]
        struct LocationRow {
            id: String,
            name: String,
            description: Option<String>,
            lon: f64,
            lat: f64,
            elevation_m: Option<f32>,
            location_type: Option<String>,
            country: Option<String>,
            region: Option<String>,
            properties: serde_json::Value,
        }

        let rows = if let Some(since_time) = since {
            sqlx::query_as::<_, LocationRow>(
                r#"
                SELECT DISTINCT l.id, l.name, l.description,
                       ST_X(l.location::geometry) as lon, ST_Y(l.location::geometry) as lat,
                       l.elevation_m, l.location_type, l.country, l.region, l.properties
                FROM locations l
                JOIN observations o ON l.id = o.location_id
                WHERE o.source = $1 AND o.obs_time >= $2
                ORDER BY l.id
                "#,
            )
            .bind(source)
            .bind(since_time)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, LocationRow>(
                r#"
                SELECT DISTINCT l.id, l.name, l.description,
                       ST_X(l.location::geometry) as lon, ST_Y(l.location::geometry) as lat,
                       l.elevation_m, l.location_type, l.country, l.region, l.properties
                FROM locations l
                JOIN observations o ON l.id = o.location_id
                WHERE o.source = $1
                ORDER BY l.id
                "#,
            )
            .bind(source)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| {
            WmsError::DatabaseError(format!("Get locations with observations failed: {}", e))
        })?;

        Ok(rows
            .into_iter()
            .map(|r| Location {
                id: r.id,
                name: r.name,
                description: r.description,
                lon: r.lon,
                lat: r.lat,
                elevation_m: r.elevation_m,
                location_type: r.location_type,
                country: r.country,
                region: r.region,
                properties: r.properties,
            })
            .collect())
    }

    // ========== TAF Methods ==========

    /// Upsert a TAF forecast with its periods.
    ///
    /// Inserts a new TAF or updates if one already exists for this location/issue_time.
    /// All periods are replaced on update.
    pub async fn upsert_taf(
        &self,
        forecast: &TafForecast,
        periods: &[TafPeriod],
    ) -> WmsResult<Uuid> {
        // Start a transaction
        let mut tx =
            self.pool.begin().await.map_err(|e| {
                WmsError::DatabaseError(format!("Failed to start transaction: {}", e))
            })?;

        // Upsert the forecast header
        let taf_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO taf_forecasts (location_id, issue_time, valid_from, valid_to, raw_taf, remarks)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (location_id, issue_time) DO UPDATE SET
                valid_from = EXCLUDED.valid_from,
                valid_to = EXCLUDED.valid_to,
                raw_taf = EXCLUDED.raw_taf,
                remarks = EXCLUDED.remarks,
                ingested_at = NOW()
            RETURNING id
            "#,
        )
        .bind(&forecast.location_id)
        .bind(forecast.issue_time)
        .bind(forecast.valid_from)
        .bind(forecast.valid_to)
        .bind(&forecast.raw_taf)
        .bind(&forecast.remarks)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Upsert TAF forecast failed: {}", e)))?;

        // Delete existing periods (in case of update)
        sqlx::query("DELETE FROM taf_periods WHERE taf_id = $1")
            .bind(taf_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Delete TAF periods failed: {}", e)))?;

        // Insert all periods
        for (order, period) in periods.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO taf_periods (
                    taf_id, period_from, period_to, change_indicator, probability,
                    wind_direction_deg, wind_speed_ms, wind_gust_ms, visibility_m,
                    wx_string, cloud_layers, period_order
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                "#,
            )
            .bind(taf_id)
            .bind(period.period_from)
            .bind(period.period_to)
            .bind(&period.change_indicator)
            .bind(period.probability)
            .bind(period.wind_direction_deg)
            .bind(period.wind_speed_ms)
            .bind(period.wind_gust_ms)
            .bind(period.visibility_m)
            .bind(&period.wx_string)
            .bind(&period.cloud_layers)
            .bind(order as i16)
            .execute(&mut *tx)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Insert TAF period failed: {}", e)))?;
        }

        tx.commit().await.map_err(|e| {
            WmsError::DatabaseError(format!("Commit TAF transaction failed: {}", e))
        })?;

        Ok(taf_id)
    }

    /// Get the latest TAF for a location.
    pub async fn get_latest_taf(
        &self,
        location_id: &str,
    ) -> WmsResult<Option<TafForecastWithPeriods>> {
        #[derive(sqlx::FromRow)]
        struct TafRow {
            id: Uuid,
            location_id: String,
            issue_time: DateTime<Utc>,
            valid_from: DateTime<Utc>,
            valid_to: DateTime<Utc>,
            raw_taf: Option<String>,
            remarks: Option<String>,
        }

        let taf_row = sqlx::query_as::<_, TafRow>(
            r#"
            SELECT id, location_id, issue_time, valid_from, valid_to, raw_taf, remarks
            FROM taf_forecasts
            WHERE location_id = $1
            ORDER BY issue_time DESC
            LIMIT 1
            "#,
        )
        .bind(location_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get latest TAF failed: {}", e)))?;

        let Some(row) = taf_row else {
            return Ok(None);
        };

        let periods = self.get_taf_periods(row.id).await?;

        Ok(Some(TafForecastWithPeriods {
            forecast: TafForecast {
                id: Some(row.id),
                location_id: row.location_id,
                issue_time: row.issue_time,
                valid_from: row.valid_from,
                valid_to: row.valid_to,
                raw_taf: row.raw_taf,
                remarks: row.remarks,
            },
            periods,
        }))
    }

    /// Get TAFs valid at a specific time for a location.
    ///
    /// Returns TAFs where valid_from <= time <= valid_to.
    pub async fn get_tafs_valid_at(
        &self,
        location_id: &str,
        time: DateTime<Utc>,
    ) -> WmsResult<Vec<TafForecastWithPeriods>> {
        #[derive(sqlx::FromRow)]
        struct TafRow {
            id: Uuid,
            location_id: String,
            issue_time: DateTime<Utc>,
            valid_from: DateTime<Utc>,
            valid_to: DateTime<Utc>,
            raw_taf: Option<String>,
            remarks: Option<String>,
        }

        let taf_rows = sqlx::query_as::<_, TafRow>(
            r#"
            SELECT id, location_id, issue_time, valid_from, valid_to, raw_taf, remarks
            FROM taf_forecasts
            WHERE location_id = $1 AND valid_from <= $2 AND valid_to >= $2
            ORDER BY issue_time DESC
            "#,
        )
        .bind(location_id)
        .bind(time)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get TAFs valid at time failed: {}", e)))?;

        let mut results = Vec::new();
        for row in taf_rows {
            let periods = self.get_taf_periods(row.id).await?;
            results.push(TafForecastWithPeriods {
                forecast: TafForecast {
                    id: Some(row.id),
                    location_id: row.location_id,
                    issue_time: row.issue_time,
                    valid_from: row.valid_from,
                    valid_to: row.valid_to,
                    raw_taf: row.raw_taf,
                    remarks: row.remarks,
                },
                periods,
            });
        }

        Ok(results)
    }

    /// Get the latest TAF for each location within a radius.
    pub async fn get_latest_tafs_in_radius(
        &self,
        lon: f64,
        lat: f64,
        radius_m: f64,
        limit: Option<i64>,
    ) -> WmsResult<Vec<(Location, TafForecastWithPeriods)>> {
        #[derive(sqlx::FromRow)]
        struct JoinedRow {
            // Location fields
            loc_id: String,
            loc_name: String,
            loc_description: Option<String>,
            loc_lon: f64,
            loc_lat: f64,
            loc_elevation_m: Option<f32>,
            loc_type: Option<String>,
            loc_country: Option<String>,
            loc_region: Option<String>,
            loc_properties: serde_json::Value,
            #[allow(dead_code)]
            distance_m: f64,
            // TAF fields
            taf_id: Uuid,
            issue_time: DateTime<Utc>,
            valid_from: DateTime<Utc>,
            valid_to: DateTime<Utc>,
            raw_taf: Option<String>,
            remarks: Option<String>,
        }

        let effective_limit = limit.unwrap_or(100);

        // Get latest TAF per location using DISTINCT ON
        let rows = sqlx::query_as::<_, JoinedRow>(
            r#"
            SELECT DISTINCT ON (l.id)
                l.id as loc_id, l.name as loc_name, l.description as loc_description,
                ST_X(l.location::geometry) as loc_lon, ST_Y(l.location::geometry) as loc_lat,
                l.elevation_m as loc_elevation_m, l.location_type as loc_type,
                l.country as loc_country, l.region as loc_region, l.properties as loc_properties,
                ST_Distance(l.location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography) as distance_m,
                t.id as taf_id, t.issue_time, t.valid_from, t.valid_to, t.raw_taf, t.remarks
            FROM locations l
            JOIN taf_forecasts t ON l.id = t.location_id
            WHERE ST_DWithin(l.location, ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography, $3)
            ORDER BY l.id, t.issue_time DESC
            LIMIT $4
            "#,
        )
        .bind(lon)
        .bind(lat)
        .bind(radius_m)
        .bind(effective_limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get TAFs in radius failed: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            let periods = self.get_taf_periods(row.taf_id).await?;
            let location = Location {
                id: row.loc_id.clone(),
                name: row.loc_name,
                description: row.loc_description,
                lon: row.loc_lon,
                lat: row.loc_lat,
                elevation_m: row.loc_elevation_m,
                location_type: row.loc_type,
                country: row.loc_country,
                region: row.loc_region,
                properties: row.loc_properties,
            };
            let taf = TafForecastWithPeriods {
                forecast: TafForecast {
                    id: Some(row.taf_id),
                    location_id: row.loc_id,
                    issue_time: row.issue_time,
                    valid_from: row.valid_from,
                    valid_to: row.valid_to,
                    raw_taf: row.raw_taf,
                    remarks: row.remarks,
                },
                periods,
            };
            results.push((location, taf));
        }

        Ok(results)
    }

    /// Get the latest TAF for each location within a bounding box.
    pub async fn get_latest_tafs_in_bbox(
        &self,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
        limit: Option<i64>,
    ) -> WmsResult<Vec<(Location, TafForecastWithPeriods)>> {
        #[derive(sqlx::FromRow)]
        struct JoinedRow {
            // Location fields
            loc_id: String,
            loc_name: String,
            loc_description: Option<String>,
            loc_lon: f64,
            loc_lat: f64,
            loc_elevation_m: Option<f32>,
            loc_type: Option<String>,
            loc_country: Option<String>,
            loc_region: Option<String>,
            loc_properties: serde_json::Value,
            // TAF fields
            taf_id: Uuid,
            issue_time: DateTime<Utc>,
            valid_from: DateTime<Utc>,
            valid_to: DateTime<Utc>,
            raw_taf: Option<String>,
            remarks: Option<String>,
        }

        let effective_limit = limit.unwrap_or(100);

        let rows = sqlx::query_as::<_, JoinedRow>(
            r#"
            SELECT DISTINCT ON (l.id)
                l.id as loc_id, l.name as loc_name, l.description as loc_description,
                ST_X(l.location::geometry) as loc_lon, ST_Y(l.location::geometry) as loc_lat,
                l.elevation_m as loc_elevation_m, l.location_type as loc_type,
                l.country as loc_country, l.region as loc_region, l.properties as loc_properties,
                t.id as taf_id, t.issue_time, t.valid_from, t.valid_to, t.raw_taf, t.remarks
            FROM locations l
            JOIN taf_forecasts t ON l.id = t.location_id
            WHERE ST_Within(l.location::geometry, ST_MakeEnvelope($1, $2, $3, $4, 4326))
            ORDER BY l.id, t.issue_time DESC
            LIMIT $5
            "#,
        )
        .bind(min_lon)
        .bind(min_lat)
        .bind(max_lon)
        .bind(max_lat)
        .bind(effective_limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get TAFs in bbox failed: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            let periods = self.get_taf_periods(row.taf_id).await?;
            let location = Location {
                id: row.loc_id.clone(),
                name: row.loc_name,
                description: row.loc_description,
                lon: row.loc_lon,
                lat: row.loc_lat,
                elevation_m: row.loc_elevation_m,
                location_type: row.loc_type,
                country: row.loc_country,
                region: row.loc_region,
                properties: row.loc_properties,
            };
            let taf = TafForecastWithPeriods {
                forecast: TafForecast {
                    id: Some(row.taf_id),
                    location_id: row.loc_id,
                    issue_time: row.issue_time,
                    valid_from: row.valid_from,
                    valid_to: row.valid_to,
                    raw_taf: row.raw_taf,
                    remarks: row.remarks,
                },
                periods,
            };
            results.push((location, taf));
        }

        Ok(results)
    }

    /// Get locations that have TAF forecasts.
    pub async fn get_locations_with_tafs(&self) -> WmsResult<Vec<Location>> {
        #[derive(sqlx::FromRow)]
        struct LocationRow {
            id: String,
            name: String,
            description: Option<String>,
            lon: f64,
            lat: f64,
            elevation_m: Option<f32>,
            location_type: Option<String>,
            country: Option<String>,
            region: Option<String>,
            properties: serde_json::Value,
        }

        let rows = sqlx::query_as::<_, LocationRow>(
            r#"
            SELECT DISTINCT l.id, l.name, l.description,
                   ST_X(l.location::geometry) as lon, ST_Y(l.location::geometry) as lat,
                   l.elevation_m, l.location_type, l.country, l.region, l.properties
            FROM locations l
            JOIN taf_forecasts t ON l.id = t.location_id
            ORDER BY l.id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get locations with TAFs failed: {}", e)))?;

        Ok(rows
            .into_iter()
            .map(|r| Location {
                id: r.id,
                name: r.name,
                description: r.description,
                lon: r.lon,
                lat: r.lat,
                elevation_m: r.elevation_m,
                location_type: r.location_type,
                country: r.country,
                region: r.region,
                properties: r.properties,
            })
            .collect())
    }

    /// Delete TAFs older than a given time.
    pub async fn delete_tafs_before(&self, before: DateTime<Utc>) -> WmsResult<u64> {
        let result = sqlx::query("DELETE FROM taf_forecasts WHERE valid_to < $1")
            .bind(before)
            .execute(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Delete TAFs failed: {}", e)))?;

        Ok(result.rows_affected())
    }

    /// Count total TAF forecasts.
    pub async fn count_tafs(&self) -> WmsResult<i64> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM taf_forecasts")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Count TAFs failed: {}", e)))?;

        Ok(count)
    }

    /// Get the time range of TAF validity periods in the database.
    pub async fn get_taf_time_range(&self) -> WmsResult<Option<(DateTime<Utc>, DateTime<Utc>)>> {
        let result: Option<(DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
            r#"
            SELECT MIN(valid_from), MAX(valid_to)
            FROM taf_forecasts
            WHERE valid_to > NOW()
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get TAF time range failed: {}", e)))?;

        // The query returns (None, None) if no rows, which flatten handles
        Ok(result.and_then(|(min, max)| Some((min, max))))
    }

    /// Helper: Get periods for a TAF forecast.
    async fn get_taf_periods(&self, taf_id: Uuid) -> WmsResult<Vec<TafPeriod>> {
        #[derive(sqlx::FromRow)]
        struct PeriodRow {
            id: Uuid,
            period_from: DateTime<Utc>,
            period_to: DateTime<Utc>,
            change_indicator: Option<String>,
            probability: Option<i16>,
            wind_direction_deg: Option<i16>,
            wind_speed_ms: Option<f32>,
            wind_gust_ms: Option<f32>,
            visibility_m: Option<f32>,
            wx_string: Option<String>,
            cloud_layers: Option<serde_json::Value>,
        }

        let rows = sqlx::query_as::<_, PeriodRow>(
            r#"
            SELECT id, period_from, period_to, change_indicator, probability,
                   wind_direction_deg, wind_speed_ms, wind_gust_ms, visibility_m,
                   wx_string, cloud_layers
            FROM taf_periods
            WHERE taf_id = $1
            ORDER BY period_order
            "#,
        )
        .bind(taf_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Get TAF periods failed: {}", e)))?;

        Ok(rows
            .into_iter()
            .map(|r| TafPeriod {
                id: Some(r.id),
                period_from: r.period_from,
                period_to: r.period_to,
                change_indicator: r.change_indicator,
                probability: r.probability,
                wind_direction_deg: r.wind_direction_deg,
                wind_speed_ms: r.wind_speed_ms,
                wind_gust_ms: r.wind_gust_ms,
                visibility_m: r.visibility_m,
                wx_string: r.wx_string,
                cloud_layers: r.cloud_layers,
            })
            .collect())
    }
}

// =============================================================================
// TAF Data Structures
// =============================================================================

/// A TAF (Terminal Aerodrome Forecast) header.
///
/// Contains metadata about the forecast: when it was issued and its validity period.
/// The actual forecast data is in the associated `TafPeriod` records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TafForecast {
    /// Database ID (auto-generated).
    pub id: Option<Uuid>,
    /// Location ID (ICAO code, e.g., "KJFK").
    pub location_id: String,
    /// When this TAF was issued.
    pub issue_time: DateTime<Utc>,
    /// Start of validity period.
    pub valid_from: DateTime<Utc>,
    /// End of validity period.
    pub valid_to: DateTime<Utc>,
    /// Raw TAF text.
    pub raw_taf: Option<String>,
    /// Remarks section.
    pub remarks: Option<String>,
}

/// A single forecast period within a TAF.
///
/// TAFs consist of a base forecast plus change groups (FM, BECMG, TEMPO, PROB).
/// All values are in SI units for consistency with METAR observations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TafPeriod {
    /// Database ID (auto-generated).
    pub id: Option<Uuid>,
    /// Start of this period.
    pub period_from: DateTime<Utc>,
    /// End of this period.
    pub period_to: DateTime<Utc>,
    /// Change indicator: null (base forecast), "FM", "BECMG", "TEMPO", "PROB".
    pub change_indicator: Option<String>,
    /// Probability (30 or 40) for PROB groups.
    pub probability: Option<i16>,
    /// Wind direction in degrees (0-360).
    pub wind_direction_deg: Option<i16>,
    /// Wind speed in meters per second.
    pub wind_speed_ms: Option<f32>,
    /// Wind gust in meters per second.
    pub wind_gust_ms: Option<f32>,
    /// Visibility in meters.
    pub visibility_m: Option<f32>,
    /// Weather phenomena string (e.g., "RA", "BR", "-SHRA").
    pub wx_string: Option<String>,
    /// Cloud layers as JSON array.
    pub cloud_layers: Option<serde_json::Value>,
}

/// A TAF forecast with all its periods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TafForecastWithPeriods {
    /// The forecast header.
    pub forecast: TafForecast,
    /// Forecast periods in order.
    pub periods: Vec<TafPeriod>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_location_creation() {
        let loc = Location::new("KJFK", "JFK Airport", -73.7781, 40.6413)
            .with_type("airport")
            .with_country("US")
            .with_region("NY")
            .with_elevation(4.0);

        assert_eq!(loc.id, "KJFK");
        assert_eq!(loc.lon, -73.7781);
        assert_eq!(loc.lat, 40.6413);
        assert_eq!(loc.location_type, Some("airport".to_string()));
        assert_eq!(loc.country, Some("US".to_string()));
        assert_eq!(loc.elevation_m, Some(4.0));
    }

    #[test]
    fn test_observation_default() {
        let obs = Observation {
            location_id: "KJFK".to_string(),
            source: "metar".to_string(),
            obs_time: Utc::now(),
            temperature_k: Some(273.15),
            ..Default::default()
        };

        assert_eq!(obs.location_id, "KJFK");
        assert_eq!(obs.source, "metar");
        assert!(obs.temperature_k.is_some());
        assert!(obs.dewpoint_k.is_none());
    }
}
