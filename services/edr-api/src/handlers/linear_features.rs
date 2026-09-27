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
use std::collections::HashMap;
use std::sync::Arc;

use storage::linear_features::LinearFeatureItem;
use storage::segment_conditions::SegmentCondition;

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
    /// `?conditions=latest` merges each feature's latest `segment_conditions`
    /// row (trail-physics output) into its properties, when one exists. Any
    /// other value (or absent) means geometry-only, the original behavior.
    pub conditions: Option<String>,
}

/// Parameters for a linear-features area query.
#[derive(Debug, Deserialize, Default)]
pub struct TrailAreaParams {
    /// bbox as minLon,minLat,maxLon,maxLat.
    pub coords: Option<String>,
    pub limit: Option<i64>,
    pub f: Option<String>,
    pub conditions: Option<String>,
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
    /// `?conditions=latest` -- see `TrailRadiusParams::conditions`.
    pub conditions: Option<String>,
}

/// Whether a `conditions` query param value requests condition merging.
/// Only `"latest"` is recognized (the only mode implemented -- a full
/// per-feature timeseries endpoint is a separate, not-yet-built query, see
/// `SegmentConditionsCatalog::get_timeseries_for_feature`); any other value
/// is silently treated as "no conditions requested" rather than a 400, since
/// this is an additive, optional enrichment, not a required parameter.
fn wants_latest_conditions(conditions: &Option<String>) -> bool {
    conditions.as_deref() == Some("latest")
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

    let conditions = if wants_latest_conditions(&params.conditions) {
        fetch_latest_conditions(&state, &features).await
    } else {
        HashMap::new()
    };

    geojson_response(features_to_collection(features, &conditions))
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

    let conditions = if wants_latest_conditions(&params.conditions) {
        fetch_latest_conditions(&state, &features).await
    } else {
        HashMap::new()
    };

    geojson_response(features_to_collection(features, &conditions))
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

    let conditions = if wants_latest_conditions(&params.conditions) {
        fetch_latest_conditions(&state, &features).await
    } else {
        HashMap::new()
    };

    let returned = features.len();
    let mut collection = features_to_collection(features, &conditions);
    collection["numberReturned"] = json!(returned);
    collection["timeStamp"] = json!(chrono::Utc::now().to_rfc3339());
    geojson_response(collection)
}

// =============================================================================
// Response builders
// =============================================================================

/// Batch-fetch the latest `segment_conditions` row for each of `features`'
/// feature_ids, keyed for O(1) lookup while building the response. Empty
/// input (or a query failure) yields an empty map -- conditions are an
/// additive enrichment, never a reason to fail a geometry query that would
/// otherwise have succeeded (matches this handler module's own existing
/// posture of logging query failures rather than propagating them for
/// secondary lookups).
async fn fetch_latest_conditions(
    state: &Arc<AppState>,
    features: &[LinearFeatureItem],
) -> HashMap<i64, SegmentCondition> {
    if features.is_empty() {
        return HashMap::new();
    }
    let feature_ids: Vec<i64> = features.iter().map(|f| f.feature_id).collect();
    match state
        .segment_conditions_catalog
        .get_latest_for_features(&feature_ids)
        .await
    {
        Ok(rows) => rows.into_iter().map(|c| (c.feature_id, c)).collect(),
        Err(e) => {
            tracing::warn!(
                "conditions=latest lookup failed, returning geometry only: {}",
                e
            );
            HashMap::new()
        }
    }
}

fn features_to_collection(
    features: Vec<LinearFeatureItem>,
    conditions: &HashMap<i64, SegmentCondition>,
) -> Value {
    let feats: Vec<Value> = features
        .into_iter()
        .map(|f| {
            let condition = conditions.get(&f.feature_id);
            feature_to_geojson(f, condition)
        })
        .collect();
    json!({
        "type": "FeatureCollection",
        "features": feats,
    })
}

fn feature_to_geojson(f: LinearFeatureItem, condition: Option<&SegmentCondition>) -> Value {
    let geometry: Value = serde_json::from_str(&f.geometry_geojson).unwrap_or(Value::Null);

    let mut properties = json!({
        "feature_id": f.feature_id,
        "feature_class": f.feature_class,
        "name": f.name,
        "system": f.system,
        "region": f.region,
        "active": f.active,
        "updated_at": f.updated_at.to_rfc3339(),
        "tags": f.tags,
    });

    // Only present when a segment_conditions row actually exists for this
    // feature (Session 13: currently only true for trails inside WS1's
    // Boulder-area pilot coverage -- see pipelines/static/README.md) --
    // deliberately no `"conditions": null` spam for the (currently
    // overwhelming) majority of trails outside that coverage.
    if let Some(c) = condition {
        properties["conditions"] = json!({
            "run_time": c.run_time.to_rfc3339(),
            "valid_time": c.valid_time.to_rfc3339(),
            "forecast_hour": c.forecast_hour,
            "soil_moisture": c.soil_moisture,
            "frozen_fraction": c.frozen_fraction,
            "confidence": c.confidence,
            "model_version": c.model_version,
        });
    }

    json!({
        "type": "Feature",
        "id": f.feature_id,
        "geometry": geometry,
        "properties": properties,
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn sample_feature(feature_id: i64) -> LinearFeatureItem {
        LinearFeatureItem {
            feature_id,
            feature_class: "mtb_trail".to_string(),
            name: Some("Test Trail".to_string()),
            system: Some("Test System".to_string()),
            geometry_geojson:
                r#"{"type":"LineString","coordinates":[[-105.3,40.0],[-105.29,40.01]]}"#.to_string(),
            tags: json!({}),
            region: "colorado".to_string(),
            active: true,
            updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn sample_condition(feature_id: i64) -> SegmentCondition {
        SegmentCondition {
            feature_id,
            run_time: Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap(),
            valid_time: Utc.with_ymd_and_hms(2026, 1, 15, 15, 0, 0).unwrap(),
            forecast_hour: 3,
            soil_moisture: Some(0.23),
            frozen_fraction: Some(0.0),
            frost_depth_m: None,
            swe_mm: None,
            softness_index: None,
            confidence: Some(1.0),
            model_version: "trail-physics-v1".to_string(),
        }
    }

    #[test]
    fn wants_latest_conditions_recognizes_only_latest() {
        assert!(wants_latest_conditions(&Some("latest".to_string())));
        assert!(!wants_latest_conditions(&Some("bogus".to_string())));
        assert!(!wants_latest_conditions(&None));
    }

    #[test]
    fn feature_without_condition_has_no_conditions_property() {
        let feature = sample_feature(1);
        let geojson = feature_to_geojson(feature, None);
        assert!(geojson["properties"].get("conditions").is_none());
    }

    #[test]
    fn feature_with_condition_gets_merged_conditions_property() {
        let feature = sample_feature(1);
        let condition = sample_condition(1);
        let geojson = feature_to_geojson(feature, Some(&condition));

        let conditions = &geojson["properties"]["conditions"];
        // f32 -> f64 -> JSON round-trip isn't bit-exact (0.23f32 as f64 !=
        // 0.23f64 literally) -- compare with tolerance, not ==.
        let soil_moisture = conditions["soil_moisture"].as_f64().unwrap();
        assert!((soil_moisture - 0.23).abs() < 1e-6);
        assert_eq!(conditions["confidence"], json!(1.0));
        assert_eq!(conditions["forecast_hour"], json!(3));
        assert_eq!(conditions["model_version"], json!("trail-physics-v1"));
        assert_eq!(conditions["valid_time"], json!("2026-01-15T15:00:00+00:00"));
    }

    #[test]
    fn feature_geometry_and_base_properties_unaffected_by_condition_merge() {
        let feature = sample_feature(42);
        let condition = sample_condition(42);
        let geojson = feature_to_geojson(feature, Some(&condition));
        assert_eq!(geojson["id"], json!(42));
        assert_eq!(geojson["properties"]["feature_id"], json!(42));
        assert_eq!(geojson["properties"]["name"], json!("Test Trail"));
        assert_eq!(geojson["geometry"]["type"], json!("LineString"));
    }

    #[test]
    fn features_to_collection_only_merges_matching_feature_ids() {
        let features = vec![sample_feature(1), sample_feature(2)];
        let mut conditions = HashMap::new();
        conditions.insert(1, sample_condition(1));
        // Feature 2 has no matching condition -- must not error or borrow
        // some other feature's row.

        let collection = features_to_collection(features, &conditions);
        let feats = collection["features"].as_array().unwrap();
        assert_eq!(feats.len(), 2);

        let f1 = feats.iter().find(|f| f["id"] == json!(1)).unwrap();
        assert!(f1["properties"].get("conditions").is_some());

        let f2 = feats.iter().find(|f| f["id"] == json!(2)).unwrap();
        assert!(f2["properties"].get("conditions").is_none());
    }
}
