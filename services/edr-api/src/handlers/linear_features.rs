//! Linear-features feature-collection handlers (trails/tracks/bridleways).
//!
//! Backed by the PostGIS `linear_features` table (see
//! `storage::linear_features` and `crates/trail-sync`), served as GeoJSON.
//! Deliberately mirrors `storm_events.rs`'s shape: geometry arrives from
//! PostGIS pre-serialized as GeoJSON (`ST_AsGeoJSON`), embedded directly into
//! feature objects via `serde_json::Value`, no Rust geometry crate involved.
//!
//! Endpoints (dispatched from the generic radius/area/items handlers based on
//! `observation_source: linear_features` in the collection's EDR config):
//! - radius: features within a radius of a point
//! - area:   features within a bounding box
//! - items:  GeoJSON items (bbox + q name-search + paging) -- the map-viewport
//!           discovery query this collection exists for
//!
//! v1 scope note: unlike storm events, there is no per-feature temporal
//! extent (trails don't have a "begin_time") and no county-aggregate
//! endpoint -- discovery is bbox + name search only, per the trail-conditions
//! design session.

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::Response,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use storage::linear_features::LinearFeatureItem;

use crate::state::AppState;

/// Maximum features returned by a single radius/area/items query. Trail
/// counts per region are far smaller than storm-event history, so a lower
/// ceiling than storm_events' 10000 is plenty and keeps items responses light
/// for the map-viewport use case.
const DEFAULT_FEATURE_LIMIT: i64 = 1000;
const MAX_FEATURE_LIMIT: i64 = 5000;

// =============================================================================
// Query parameter structs
// =============================================================================

/// Parameters for a linear-features radius query.
#[derive(Debug, Deserialize, Default)]
pub struct TrailRadiusParams {
    pub coords: Option<String>,
    pub within: Option<String>,
    #[serde(rename = "within-units")]
    pub within_units: Option<String>,
    pub limit: Option<i64>,
    pub f: Option<String>,
}

/// Parameters for a linear-features area query.
#[derive(Debug, Deserialize, Default)]
pub struct TrailAreaParams {
    /// bbox as minLon,minLat,maxLon,maxLat.
    pub coords: Option<String>,
    pub limit: Option<i64>,
    pub f: Option<String>,
}

/// Parameters for a linear-features items query (OGC-Features-style).
#[derive(Debug, Deserialize, Default)]
pub struct TrailItemsParams {
    /// bbox as minLon,minLat,maxLon,maxLat.
    pub bbox: Option<String>,
    /// Name search (e.g. `?q=apex`). When present, takes precedence over
    /// bbox -- this is the "find a trail by name" query, not a viewport query.
    pub q: Option<String>,
    /// Optional `feature_class` filter (e.g. `mtb_trail`).
    pub class: Option<String>,
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    pub f: Option<String>,
}

// =============================================================================
// Handlers
// =============================================================================

/// Radius query for the trails collection.
pub async fn trail_radius_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<TrailRadiusParams>,
) -> Response {
    if let Err(resp) = resolve_trails_collection(&state, &collection_id).await {
        return resp;
    }

    let coords = params.coords.unwrap_or_default();
    let (lon, lat) = match parse_point_wkt(&coords) {
        Ok(c) => c,
        Err(e) => return bad_request(e),
    };
    let radius_m = match parse_radius(&params.within, params.within_units.as_deref()) {
        Ok(r) => r,
        Err(e) => return bad_request(e),
    };
    let limit = clamp_limit(params.limit);

    let features = match state
        .linear_feature_catalog
        .get_features_in_radius(None, lon, lat, radius_m, limit)
        .await
    {
        Ok(f) => f,
        Err(e) => return internal_error(format!("Radius query failed: {}", e)),
    };

    geojson_response(features_to_collection(features))
}

/// Area (bbox) query for the trails collection.
pub async fn trail_area_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<TrailAreaParams>,
) -> Response {
    if let Err(resp) = resolve_trails_collection(&state, &collection_id).await {
        return resp;
    }

    let coords = params.coords.unwrap_or_default();
    let (min_lon, min_lat, max_lon, max_lat) = match parse_bbox(&coords) {
        Ok(b) => b,
        Err(e) => return bad_request(e),
    };
    let limit = clamp_limit(params.limit);

    let features = match state
        .linear_feature_catalog
        .get_features_in_bbox(None, min_lon, min_lat, max_lon, max_lat, limit, 0)
        .await
    {
        Ok(f) => f,
        Err(e) => return internal_error(format!("Area query failed: {}", e)),
    };

    geojson_response(features_to_collection(features))
}

/// Items query (OGC-Features-style) for the trails collection.
///
/// `?q=` triggers a name search instead of the bbox query -- this is the
/// "find Apex" lookup, as distinct from "what trails are in this viewport".
pub async fn trail_items_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<TrailItemsParams>,
) -> Response {
    if let Err(resp) = resolve_trails_collection(&state, &collection_id).await {
        return resp;
    }

    let limit = clamp_limit(params.limit);
    let offset = params.offset.unwrap_or(0).max(0);
    let class = params.class.as_deref();

    let features = if let Some(q) = params.q.as_deref().filter(|s| !s.trim().is_empty()) {
        match state
            .linear_feature_catalog
            .search_features(q, class, limit)
            .await
        {
            Ok(f) => f,
            Err(e) => return internal_error(format!("Search query failed: {}", e)),
        }
    } else {
        let (min_lon, min_lat, max_lon, max_lat) = match &params.bbox {
            Some(b) => match parse_bbox(b) {
                Ok(b) => b,
                Err(e) => return bad_request(e),
            },
            None => (-180.0, -90.0, 180.0, 90.0),
        };
        match state
            .linear_feature_catalog
            .get_features_in_bbox(class, min_lon, min_lat, max_lon, max_lat, limit, offset)
            .await
        {
            Ok(f) => f,
            Err(e) => return internal_error(format!("Items query failed: {}", e)),
        }
    };

    let returned = features.len();
    let mut collection = features_to_collection(features);
    collection["numberReturned"] = json!(returned);
    collection["timeStamp"] = json!(chrono::Utc::now().to_rfc3339());
    geojson_response(collection)
}

// =============================================================================
// Response builders
// =============================================================================

fn features_to_collection(features: Vec<LinearFeatureItem>) -> Value {
    let feats: Vec<Value> = features.into_iter().map(feature_to_geojson).collect();
    json!({
        "type": "FeatureCollection",
        "features": feats,
    })
}

fn feature_to_geojson(f: LinearFeatureItem) -> Value {
    let geometry: Value = serde_json::from_str(&f.geometry_geojson).unwrap_or(Value::Null);

    json!({
        "type": "Feature",
        "id": f.feature_id,
        "geometry": geometry,
        "properties": {
            "feature_id": f.feature_id,
            "feature_class": f.feature_class,
            "name": f.name,
            "system": f.system,
            "region": f.region,
            "active": f.active,
            "updated_at": f.updated_at.to_rfc3339(),
            "tags": f.tags,
        },
    })
}

// =============================================================================
// Helpers
// =============================================================================

/// Validate that `collection_id` is a configured feature collection backed by
/// `linear_features` (as opposed to `storm_events`). Unlike storm events
/// (where the collection id *is* the lookup key, `event_type`), all
/// linear-feature collections query the same table today, so there is
/// nothing to resolve besides validating the collection exists and is wired
/// to this backend -- this function exists primarily to give a clean 404/400
/// instead of a confusing empty result if it's ever misconfigured.
async fn resolve_trails_collection(
    state: &Arc<AppState>,
    collection_id: &str,
) -> Result<(), Response> {
    let config = state.edr_config.read().await;
    let Some((model_config, _)) = config.find_collection(collection_id) else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            edr_protocol::responses::ExceptionResponse::not_found(format!(
                "Collection not found: {}",
                collection_id
            )),
        ));
    };
    if !model_config.data_type.is_feature_data()
        || model_config.observation_source.as_deref() != Some("linear_features")
    {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            edr_protocol::responses::ExceptionResponse::bad_request(format!(
                "Collection {} is not a linear-features collection",
                collection_id
            )),
        ));
    }
    Ok(())
}

fn clamp_limit(limit: Option<i64>) -> i64 {
    limit
        .unwrap_or(DEFAULT_FEATURE_LIMIT)
        .clamp(1, MAX_FEATURE_LIMIT)
}

/// Parse `POINT(lon lat)` or `lon,lat`. Identical to storm_events' parser;
/// duplicated rather than shared to keep the two feature-collection backends
/// independently editable (same rationale as the rest of this module).
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

fn parse_radius(within: &Option<String>, units: Option<&str>) -> Result<f64, String> {
    let default_m = 10_000.0; // 10 km default -- trails are local, unlike storm events' 100km
    let Some(within) = within else {
        return Ok(default_m);
    };
    let within = within.trim().to_lowercase();

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

fn geojson_response(value: Value) -> Response {
    match serde_json::to_string(&value) {
        Ok(json) => Response::builder()
            .status(StatusCode::OK)
            .header(axum::http::header::CONTENT_TYPE, "application/geo+json")
            .header(axum::http::header::CACHE_CONTROL, "max-age=3600")
            .body(json.into())
            .unwrap(),
        Err(e) => internal_error(format!("Serialization failed: {}", e)),
    }
}

fn bad_request(msg: impl Into<String>) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        edr_protocol::responses::ExceptionResponse::bad_request(msg.into()),
    )
}

fn internal_error(msg: impl Into<String>) -> Response {
    let msg = msg.into();
    tracing::error!("{}", msg);
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        edr_protocol::responses::ExceptionResponse::internal_error("Internal error"),
    )
}

fn error_response(status: StatusCode, exc: edr_protocol::responses::ExceptionResponse) -> Response {
    let json = serde_json::to_string(&exc).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(json.into())
        .unwrap()
}
