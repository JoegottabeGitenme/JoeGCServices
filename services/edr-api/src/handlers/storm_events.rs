//! Storm events feature-collection handlers (hail / wind / tornado).
//!
//! These collections are backed by the PostGIS `storm_events` table and served
//! as GeoJSON. Unlike point observations they carry mixed geometry: hail/wind
//! are Points and tornadoes are LineString tracks. Geometry arrives from
//! PostGIS pre-serialized as GeoJSON (`ST_AsGeoJSON`), so this module embeds it
//! directly into the feature objects via `serde_json::Value`.
//!
//! Endpoints (dispatched from the generic radius/area handlers, plus dedicated
//! routes for items/counties):
//! - radius:         reports within a radius of a point ("this house")
//! - area:           reports within a bounding box
//! - items:          GeoJSON items (bbox + datetime + paging) for the click-a-point UX
//! - counties:       county-aggregate counts + boundary geometry (monthly MV)
//! - counties/{fips} per-county individual events (cache-optimised, ETag'd)

use axum::{
    extract::{Extension, Path, Query},
    http::{header, StatusCode},
    response::Response,
};
use chrono::{DateTime, Datelike, Utc};
use edr_protocol::responses::ExceptionResponse;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use storage::storm_events::{CountyAggregate, CountyEventsResult, StormEventFeature};

use crate::state::AppState;

/// Default simplification tolerance (degrees) for county boundary geometry.
/// ~0.01 deg ≈ 1 km, good for choropleth display.
const DEFAULT_COUNTY_SIMPLIFY_TOL: f64 = 0.01;

/// Maximum features returned by a single radius/area/items query.
const DEFAULT_FEATURE_LIMIT: i64 = 2000;
const MAX_FEATURE_LIMIT: i64 = 10000;

// =============================================================================
// Query parameter structs
// =============================================================================

/// Parameters for a storm-events radius query.
#[derive(Debug, Deserialize, Default)]
pub struct StormRadiusParams {
    pub coords: Option<String>,
    pub within: Option<String>,
    #[serde(rename = "within-units")]
    pub within_units: Option<String>,
    pub datetime: Option<String>,
    pub limit: Option<i64>,
    pub f: Option<String>,
}

/// Parameters for a storm-events area query.
#[derive(Debug, Deserialize, Default)]
pub struct StormAreaParams {
    /// bbox as minLon,minLat,maxLon,maxLat (the EDR `coords` for these collections).
    pub coords: Option<String>,
    pub datetime: Option<String>,
    pub limit: Option<i64>,
    pub f: Option<String>,
}

/// Parameters for a storm-events items query (OGC-Features-style).
#[derive(Debug, Deserialize, Default)]
pub struct StormItemsParams {
    /// bbox as minLon,minLat,maxLon,maxLat.
    pub bbox: Option<String>,
    pub datetime: Option<String>,
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    pub f: Option<String>,
}

/// Parameters for the county-aggregate endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct CountiesParams {
    /// Year range or interval (e.g. `2015/2025`), or a single year.
    pub datetime: Option<String>,
    /// State abbreviation filter (e.g. `OK`).
    pub state: Option<String>,
    /// bbox filter as minLon,minLat,maxLon,maxLat.
    pub bbox: Option<String>,
    /// Whether to include (simplified) boundary geometry. Defaults to true.
    pub geometry: Option<bool>,
    /// Override simplification tolerance in degrees.
    pub simplify: Option<f64>,
}

/// Parameters for the per-county events endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct CountyEventsParams {
    /// Single year filter, e.g. `?year=2023`.
    pub year: Option<i32>,
    /// Year range, e.g. `?years=2015/2024`.  Either bare years ("2015/2024") or
    /// RFC3339 dates; the same format accepted by other storm-events endpoints.
    pub years: Option<String>,
}

// =============================================================================
// Handlers
// =============================================================================

/// Radius query for a storm-events collection.
pub async fn storm_radius_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<StormRadiusParams>,
) -> Response {
    let event_type = match resolve_event_type(&state, &collection_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    let coords = params.coords.unwrap_or_default();
    let (lon, lat) = match parse_point_wkt(&coords) {
        Ok(c) => c,
        Err(e) => return bad_request(e),
    };
    let radius_m = match parse_radius(&params.within, params.within_units.as_deref()) {
        Ok(r) => r,
        Err(e) => return bad_request(e),
    };
    let (start, end) = parse_datetime_range(&params.datetime);
    let limit = clamp_limit(params.limit);

    let features = match state
        .storm_event_catalog
        .get_events_in_radius(&event_type, lon, lat, radius_m, start, end, limit)
        .await
    {
        Ok(f) => f,
        Err(e) => return internal_error(format!("Radius query failed: {}", e)),
    };

    geojson_response(features_to_collection(features))
}

/// Area (bbox) query for a storm-events collection.
pub async fn storm_area_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<StormAreaParams>,
) -> Response {
    let event_type = match resolve_event_type(&state, &collection_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    let coords = params.coords.unwrap_or_default();
    let (min_lon, min_lat, max_lon, max_lat) = match parse_bbox(&coords) {
        Ok(b) => b,
        Err(e) => return bad_request(e),
    };
    let (start, end) = parse_datetime_range(&params.datetime);
    let limit = clamp_limit(params.limit);

    let features = match state
        .storm_event_catalog
        .get_events_in_bbox(
            &event_type,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            start,
            end,
            limit,
            0,
        )
        .await
    {
        Ok(f) => f,
        Err(e) => return internal_error(format!("Area query failed: {}", e)),
    };

    geojson_response(features_to_collection(features))
}

/// Items query (OGC-Features-style) for a storm-events collection.
pub async fn storm_items_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<StormItemsParams>,
) -> Response {
    let event_type = match resolve_event_type(&state, &collection_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    // bbox optional; default to global extent
    let (min_lon, min_lat, max_lon, max_lat) = match &params.bbox {
        Some(b) => match parse_bbox(b) {
            Ok(b) => b,
            Err(e) => return bad_request(e),
        },
        None => (-180.0, -90.0, 180.0, 90.0),
    };
    let (start, end) = parse_datetime_range(&params.datetime);
    let limit = clamp_limit(params.limit);
    let offset = params.offset.unwrap_or(0).max(0);

    let features = match state
        .storm_event_catalog
        .get_events_in_bbox(
            &event_type,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            start,
            end,
            limit,
            offset,
        )
        .await
    {
        Ok(f) => f,
        Err(e) => return internal_error(format!("Items query failed: {}", e)),
    };

    // Items responses include paging metadata alongside the FeatureCollection.
    let returned = features.len();
    let mut collection = features_to_collection(features);
    collection["numberReturned"] = json!(returned);
    collection["timeStamp"] = json!(Utc::now().to_rfc3339());
    geojson_response(collection)
}

/// County-aggregate endpoint: counts + (simplified) boundary geometry.
pub async fn storm_counties_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<CountiesParams>,
) -> Response {
    let event_type = match resolve_event_type(&state, &collection_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    let (start_year, end_year) = parse_year_range(&params.datetime);

    let bbox = match &params.bbox {
        Some(b) => match parse_bbox(b) {
            Ok(b) => Some(b),
            Err(e) => return bad_request(e),
        },
        None => None,
    };

    let include_geometry = params.geometry.unwrap_or(true);
    let tolerance = if include_geometry {
        Some(params.simplify.unwrap_or(DEFAULT_COUNTY_SIMPLIFY_TOL))
    } else {
        None
    };

    let aggregates = match state
        .storm_event_catalog
        .get_county_counts(
            &event_type,
            start_year,
            end_year,
            bbox,
            params.state.as_deref(),
            tolerance,
        )
        .await
    {
        Ok(a) => a,
        Err(e) => return internal_error(format!("County aggregate query failed: {}", e)),
    };

    geojson_response(counties_to_collection(aggregates))
}

/// Per-county events endpoint — returns individual event features for one county.
///
/// `GET /edr/collections/{type}/counties/{fips}`
/// `GET /edr/collections/{type}/counties/{fips}?year=2023`
/// `GET /edr/collections/{type}/counties/{fips}?years=2015/2024`
///
/// Designed for browser/CDN caching: the URL is stable and the response only
/// changes after the monthly ingest refresh.  An ETag derived from
/// `max(ingested_at)` enables efficient 304 revalidation.
pub async fn storm_county_events_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, fips)): Path<(String, String)>,
    Query(params): Query<CountyEventsParams>,
    headers: axum::http::HeaderMap,
) -> Response {
    let event_type = match resolve_event_type(&state, &collection_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    // FIPS pre-validation (5 digits). The storage layer also validates but we
    // want a 400 with a clear message rather than a DB error.
    if fips.len() != 5 || !fips.chars().all(|c| c.is_ascii_digit()) {
        return bad_request(format!(
            "Invalid county FIPS '{}': must be exactly 5 digits (e.g. '40109')",
            fips
        ));
    }

    let (start_year, end_year) = parse_county_year_params(&params);

    let result = match state
        .storm_event_catalog
        .get_events_by_county(&event_type, &fips, start_year, end_year)
        .await
    {
        Ok(r) => r,
        Err(e) if e.to_string().contains("Invalid county FIPS") => {
            return bad_request(e.to_string());
        }
        Err(e) => return internal_error(format!("County events query failed: {}", e)),
    };

    // Build the ETag from max(ingested_at) + year filter.
    // Stable until new events for this county are ingested.
    let etag_seed = format!(
        "{}-{}-{:?}-{:?}",
        fips,
        event_type,
        result.max_ingested_at,
        (start_year, end_year)
    );
    let etag = format!(
        "W/\"{:016x}\"",
        etag_seed.bytes().fold(14695981039346656037u64, |acc, b| {
            acc.wrapping_mul(1099511628211) ^ (b as u64)
        })
    );

    // Honour If-None-Match for cheap revalidation.
    if let Some(inm) = headers.get(header::IF_NONE_MATCH) {
        if inm.to_str().unwrap_or("") == etag {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, &etag)
                .body(axum::body::Body::empty())
                .unwrap();
        }
    }

    let collection = county_events_to_collection(&result);
    let json = match serde_json::to_string(&collection) {
        Ok(j) => j,
        Err(e) => return internal_error(format!("Serialization failed: {}", e)),
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        // Public caching: 24 h fresh, serve stale for up to 7 days while revalidating.
        // Historical storm data rarely changes; the ETag handles the monthly update.
        .header(
            header::CACHE_CONTROL,
            "public, max-age=86400, stale-while-revalidate=604800",
        )
        .header(header::ETAG, &etag)
        .body(json.into())
        .unwrap()
}

// =============================================================================
// Response builders
// =============================================================================

/// Build a GeoJSON FeatureCollection (as a serde_json Value) from storm events.
fn features_to_collection(features: Vec<StormEventFeature>) -> Value {
    let feats: Vec<Value> = features.into_iter().map(feature_to_geojson).collect();
    json!({
        "type": "FeatureCollection",
        "features": feats,
    })
}

/// Build the per-county response: a FeatureCollection with county metadata
/// alongside the standard `features` array.
fn county_events_to_collection(result: &CountyEventsResult) -> Value {
    let feats: Vec<Value> = result
        .features
        .iter()
        .map(|f| feature_to_geojson(f.clone()))
        .collect();
    json!({
        "type": "FeatureCollection",
        "county_fips": result.county_fips,
        "county_name": result.county_name,
        "state": result.state,
        "event_type": result.features.first().map(|f| &f.event_type),
        "numberReturned": feats.len(),
        "features": feats,
    })
}

/// Convert a single storm event into a GeoJSON Feature value.
fn feature_to_geojson(f: StormEventFeature) -> Value {
    // Geometry is already GeoJSON text from PostGIS; parse it back into a Value.
    let geometry: Value = serde_json::from_str(&f.geometry_geojson).unwrap_or(Value::Null);

    json!({
        "type": "Feature",
        "id": f.event_id,
        "geometry": geometry,
        "properties": {
            "event_id": f.event_id,
            "event_type": f.event_type,
            "datetime": f.begin_time.to_rfc3339(),
            "begin_time": f.begin_time.to_rfc3339(),
            "end_time": f.end_time.map(|t| t.to_rfc3339()),
            "magnitude": f.magnitude,
            "magnitude_unit": f.magnitude_unit,
            "tor_f_scale": f.tor_f_scale,
            "state": f.state,
            "county_name": f.cz_name,
            "county_fips": f.county_fips,
        },
    })
}

/// Build a GeoJSON FeatureCollection of county aggregates.
fn counties_to_collection(aggregates: Vec<CountyAggregate>) -> Value {
    let feats: Vec<Value> = aggregates.into_iter().map(county_to_geojson).collect();
    json!({
        "type": "FeatureCollection",
        "features": feats,
    })
}

/// Convert a county aggregate into a GeoJSON Feature value.
fn county_to_geojson(c: CountyAggregate) -> Value {
    let geometry: Value = c
        .geometry_geojson
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);

    json!({
        "type": "Feature",
        "id": c.county_fips,
        "geometry": geometry,
        "properties": {
            "county_fips": c.county_fips,
            "name": c.name,
            "state": c.state,
            "event_type": c.event_type,
            "total": c.total,
            "by_year": c.by_year,
        },
    })
}

// =============================================================================
// Helpers
// =============================================================================

/// Resolve and validate the event type for a collection id. Returns the event
/// type string (`"hail"`, `"wind"`, `"tornado"`) or an error response.
async fn resolve_event_type(
    state: &Arc<AppState>,
    collection_id: &str,
) -> Result<String, Response> {
    let config = state.edr_config.read().await;
    let Some((model_config, _)) = config.find_collection(collection_id) else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        ));
    };
    if !model_config.data_type.is_feature_data() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Collection {} is not a feature collection",
                collection_id
            )),
        ));
    }
    // The collection id is the event type for storm-event collections.
    Ok(collection_id.to_string())
}

fn clamp_limit(limit: Option<i64>) -> i64 {
    limit
        .unwrap_or(DEFAULT_FEATURE_LIMIT)
        .clamp(1, MAX_FEATURE_LIMIT)
}

/// Parse `POINT(lon lat)` or `lon,lat`.
fn parse_point_wkt(coords: &str) -> Result<(f64, f64), String> {
    let coords = coords.trim();
    if coords.to_uppercase().starts_with("POINT") {
        let inner = coords
            .trim_start_matches(|c: char| !c.is_ascii_digit() && c != '-' && c != '.')
            .trim_end_matches(')');
        let parts: Vec<&str> = inner.split_whitespace().collect();
        if parts.len() >= 2 {
            let lon = parts[0]
                .parse()
                .map_err(|_| "Invalid longitude".to_string())?;
            let lat = parts[1]
                .parse()
                .map_err(|_| "Invalid latitude".to_string())?;
            return Ok((lon, lat));
        }
    }
    let parts: Vec<&str> = coords.split(',').collect();
    if parts.len() >= 2 {
        let lon = parts[0]
            .trim()
            .parse()
            .map_err(|_| "Invalid longitude".to_string())?;
        let lat = parts[1]
            .trim()
            .parse()
            .map_err(|_| "Invalid latitude".to_string())?;
        return Ok((lon, lat));
    }
    Err("Invalid coordinates. Use POINT(lon lat) or lon,lat".to_string())
}

/// Parse a radius value + optional units into meters. Supports a units suffix on
/// the value (e.g. `50km`) or a separate `within-units` parameter.
fn parse_radius(within: &Option<String>, units: Option<&str>) -> Result<f64, String> {
    let default_m = 100_000.0;
    let Some(within) = within else {
        return Ok(default_m);
    };
    let within = within.trim().to_lowercase();

    // Units suffix on the value wins; otherwise use the units parameter.
    let (num_str, unit) = if within.ends_with("km") {
        (within.trim_end_matches("km").trim(), "km")
    } else if within.ends_with("mi") {
        (within.trim_end_matches("mi").trim(), "mi")
    } else if within.ends_with("nm") {
        (within.trim_end_matches("nm").trim(), "nm")
    } else if within.ends_with('m') {
        (within.trim_end_matches('m').trim(), "m")
    } else {
        (within.as_str(), units.unwrap_or("km"))
    };

    let num: f64 = num_str
        .parse()
        .map_err(|_| "Invalid radius value".to_string())?;
    let meters = match unit.to_lowercase().as_str() {
        "km" => num * 1000.0,
        "mi" => num * 1609.34,
        "nm" => num * 1852.0,
        "m" => num,
        _ => num * 1000.0,
    };
    Ok(meters)
}

/// Parse `minLon,minLat,maxLon,maxLat`.
fn parse_bbox(coords: &str) -> Result<(f64, f64, f64, f64), String> {
    let parts: Vec<&str> = coords.trim().split(',').collect();
    if parts.len() >= 4 {
        let min_lon = parts[0]
            .trim()
            .parse()
            .map_err(|_| "Invalid min longitude".to_string())?;
        let min_lat = parts[1]
            .trim()
            .parse()
            .map_err(|_| "Invalid min latitude".to_string())?;
        let max_lon = parts[2]
            .trim()
            .parse()
            .map_err(|_| "Invalid max longitude".to_string())?;
        let max_lat = parts[3]
            .trim()
            .parse()
            .map_err(|_| "Invalid max latitude".to_string())?;
        return Ok((min_lon, min_lat, max_lon, max_lat));
    }
    Err("Invalid bbox. Use minLon,minLat,maxLon,maxLat".to_string())
}

/// Parse a datetime instant or interval into start/end. For historical storm
/// events, the default (no datetime) is unbounded (entire archive).
fn parse_datetime_range(
    datetime: &Option<String>,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let Some(dt) = datetime else {
        return (None, None);
    };
    let dt = dt.trim();
    if dt == ".." || dt.is_empty() {
        return (None, None);
    }
    if dt.contains('/') {
        let parts: Vec<&str> = dt.split('/').collect();
        if parts.len() == 2 {
            let start = parse_dt_bound(parts[0]);
            let end = parse_dt_bound(parts[1]);
            return (start, end);
        }
    }
    // Single instant: treat as [instant, instant]
    let inst = parse_dt_bound(dt);
    (inst, inst)
}

/// Parse one bound of a datetime interval. Accepts RFC3339, a bare year, or
/// `..` (open). Returns None for open/invalid bounds.
fn parse_dt_bound(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s == ".." || s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Bare year, e.g. "2015"
    if let Ok(year) = s.parse::<i32>() {
        return Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).single();
    }
    None
}

/// Parse a year range for county aggregation from a datetime parameter.
fn parse_year_range(datetime: &Option<String>) -> (Option<i32>, Option<i32>) {
    let (start, end) = parse_datetime_range(datetime);
    (start.map(|d| d.year()), end.map(|d| d.year()))
}

/// Parse year filter params for the per-county events endpoint.
///
/// Priority: `year` (single year) > `years` (range) > unbounded.
fn parse_county_year_params(params: &CountyEventsParams) -> (Option<i32>, Option<i32>) {
    if let Some(y) = params.year {
        return (Some(y), Some(y));
    }
    if let Some(ref range) = params.years {
        return parse_year_range(&Some(range.clone()));
    }
    (None, None)
}

fn geojson_response(value: Value) -> Response {
    match serde_json::to_string(&value) {
        Ok(json) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/geo+json")
            .header(header::CACHE_CONTROL, "max-age=3600")
            .body(json.into())
            .unwrap(),
        Err(e) => internal_error(format!("Serialization failed: {}", e)),
    }
}

fn bad_request(msg: impl Into<String>) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        ExceptionResponse::bad_request(msg.into()),
    )
}

fn internal_error(msg: impl Into<String>) -> Response {
    let msg = msg.into();
    tracing::error!("{}", msg);
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        ExceptionResponse::internal_error("Internal error"),
    )
}

fn error_response(status: StatusCode, exc: ExceptionResponse) -> Response {
    let json = serde_json::to_string(&exc).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json.into())
        .unwrap()
}

// Bring TimeZone trait into scope for with_ymd_and_hms.
use chrono::TimeZone;
