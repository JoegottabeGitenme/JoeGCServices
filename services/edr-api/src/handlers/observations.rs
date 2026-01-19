//! Observation query handlers for point observation data (METAR, TAF, etc.).
//!
//! These handlers query PostgreSQL directly for station-based observations,
//! rather than gridded data from MinIO/Zarr.
//!
//! Supported EDR endpoints:
//! - GET /collections/{id}/locations - List all stations
//! - GET /collections/{id}/locations/{locationId} - Get observations at station
//! - GET /collections/{id}/radius - Get observations within radius
//! - GET /collections/{id}/area - Get observations within bounding box

use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Duration, Utc};
use edr_protocol::{
    locations::{LocationGeometry, LocationProperties},
    responses::ExceptionResponse,
    LocationFeature, LocationFeatureCollection,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use storage::observations::{Location, Observation, ObservationQuery};

use crate::state::AppState;

/// Query parameters for observation locations list.
#[derive(Debug, Deserialize, Default)]
pub struct ObsLocationsListParams {
    /// Output format.
    pub f: Option<String>,
    /// Limit number of results
    pub limit: Option<i64>,
    /// Filter by observation source
    pub source: Option<String>,
}

/// Query parameters for observation location data query.
#[derive(Debug, Deserialize)]
pub struct ObsLocationQueryParams {
    /// Datetime instant or interval.
    pub datetime: Option<String>,
    /// Parameter name(s) to retrieve.
    #[serde(rename = "parameter-name")]
    pub parameter_name: Option<String>,
    /// Output format.
    pub f: Option<String>,
    /// Maximum number of observations to return.
    pub limit: Option<i64>,
}

/// Query parameters for radius query.
#[derive(Debug, Deserialize)]
pub struct ObsRadiusQueryParams {
    /// Center point as "POINT(lon lat)" WKT.
    pub coords: String,
    /// Search radius (with units, e.g., "100km" or "50mi").
    pub within: Option<String>,
    /// Datetime instant or interval.
    pub datetime: Option<String>,
    /// Parameter name(s) to retrieve.
    #[serde(rename = "parameter-name")]
    pub parameter_name: Option<String>,
    /// Output format.
    pub f: Option<String>,
    /// Maximum number of results.
    pub limit: Option<i64>,
}

/// Query parameters for area query.
#[derive(Debug, Deserialize)]
pub struct ObsAreaQueryParams {
    /// Bounding box or polygon as WKT.
    pub coords: String,
    /// Datetime instant or interval.
    pub datetime: Option<String>,
    /// Parameter name(s) to retrieve.
    #[serde(rename = "parameter-name")]
    pub parameter_name: Option<String>,
    /// Output format.
    pub f: Option<String>,
    /// Maximum number of results.
    pub limit: Option<i64>,
}

/// GeoJSON response for observation data.
#[derive(Debug, Serialize)]
pub struct ObservationFeatureCollection {
    #[serde(rename = "type")]
    pub collection_type: String,
    pub features: Vec<ObservationFeature>,
    #[serde(rename = "numberReturned")]
    pub number_returned: usize,
    #[serde(rename = "timeStamp")]
    pub timestamp: String,
}

/// Single observation as a GeoJSON Feature.
#[derive(Debug, Serialize)]
pub struct ObservationFeature {
    #[serde(rename = "type")]
    pub feature_type: String,
    pub id: String,
    pub geometry: LocationGeometry,
    pub properties: ObservationProperties,
}

/// Properties of an observation feature.
#[derive(Debug, Serialize)]
pub struct ObservationProperties {
    pub location_id: String,
    pub name: Option<String>,
    pub obs_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_k: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dewpoint_k: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_direction_deg: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_speed_ms: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_gust_ms: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility_m: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altimeter_pa: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sea_level_pressure_pa: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flight_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_text: Option<String>,
}

/// GET /edr/collections/:collection_id/locations (for observation collections)
///
/// Returns stations that have recent observations.
pub async fn obs_locations_list_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<ObsLocationsListParams>,
    _headers: HeaderMap,
) -> Response {
    let config = state.edr_config.read().await;

    // Find the collection and verify it's a point observation type
    let Some((model_config, _collection_def)) = config.find_collection(&collection_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        );
    };

    if !model_config.data_type.is_point_observation() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Collection {} is not a point observation collection",
                collection_id
            )),
        );
    }

    let source = model_config
        .observation_source
        .clone()
        .or(params.source.clone())
        .unwrap_or_else(|| "metar".to_string());

    // Get locations with recent observations (last 2 hours by default)
    let since = Some(Utc::now() - Duration::hours(2));
    let locations = match state
        .observation_catalog
        .get_locations_with_observations(&source, since)
        .await
    {
        Ok(locs) => locs,
        Err(e) => {
            tracing::error!("Failed to get observation locations: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to query locations"),
            );
        }
    };

    // Apply limit
    let limit = params.limit.unwrap_or(1000) as usize;
    let locations: Vec<_> = locations.into_iter().take(limit).collect();

    // Convert to GeoJSON FeatureCollection
    let features: Vec<LocationFeature> = locations
        .iter()
        .map(|loc| location_to_feature(loc, &state.base_url, &collection_id))
        .collect();

    let fc = LocationFeatureCollection {
        collection_type: "FeatureCollection".to_string(),
        features,
        number_returned: Some(locations.len()),
    };

    let json = match serde_json::to_string_pretty(&fc) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("Failed to serialize locations: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to serialize response"),
            );
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

/// GET /edr/collections/:collection_id/locations/:location_id (for observation collections)
///
/// Returns observations at a specific station.
pub async fn obs_location_query_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, location_id)): Path<(String, String)>,
    Query(params): Query<ObsLocationQueryParams>,
    _headers: HeaderMap,
) -> Response {
    let config = state.edr_config.read().await;

    // Find the collection
    let Some((model_config, _collection_def)) = config.find_collection(&collection_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        );
    };

    if !model_config.data_type.is_point_observation() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Collection {} is not a point observation collection",
                collection_id
            )),
        );
    }

    // Get the location
    let location = match state.observation_catalog.get_location(&location_id).await {
        Ok(Some(loc)) => loc,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                ExceptionResponse::not_found(format!("Location not found: {}", location_id)),
            );
        }
        Err(e) => {
            tracing::error!("Failed to get location: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to query location"),
            );
        }
    };

    // Parse datetime range
    let (start_time, end_time) = parse_datetime_range(&params.datetime);

    // Build observation query
    let query = ObservationQuery {
        location_id: Some(location_id.clone()),
        source: model_config.observation_source.clone(),
        sources: None,
        start_time,
        end_time,
        limit: params.limit.or(Some(100)),
    };

    // Get observations
    let observations = match state.observation_catalog.get_observations(&query).await {
        Ok(obs) => obs,
        Err(e) => {
            tracing::error!("Failed to get observations: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to query observations"),
            );
        }
    };

    // Convert to GeoJSON
    let fc = observations_to_geojson(&location, &observations);

    let json = match serde_json::to_string_pretty(&fc) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("Failed to serialize observations: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to serialize response"),
            );
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

/// GET /edr/collections/:collection_id/radius (for observation collections)
///
/// Returns observations within a radius of a point.
pub async fn obs_radius_query_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<ObsRadiusQueryParams>,
    _headers: HeaderMap,
) -> Response {
    let config = state.edr_config.read().await;

    // Find the collection
    let Some((model_config, _collection_def)) = config.find_collection(&collection_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        );
    };

    if !model_config.data_type.is_point_observation() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Collection {} is not a point observation collection",
                collection_id
            )),
        );
    }

    // Parse coordinates (POINT(lon lat))
    let (lon, lat) = match parse_point_wkt(&params.coords) {
        Ok(coords) => coords,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e));
        }
    };

    // Parse radius (default 100km)
    let radius_m = match parse_radius(&params.within) {
        Ok(r) => r,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e));
        }
    };

    // Parse datetime range
    let (start_time, end_time) = parse_datetime_range(&params.datetime);

    // Get observations in radius
    let observations = match state
        .observation_catalog
        .get_observations_in_radius(
            lon,
            lat,
            radius_m,
            model_config.observation_source.as_deref(),
            start_time,
            end_time,
            params.limit.or(Some(100)),
        )
        .await
    {
        Ok(obs) => obs,
        Err(e) => {
            tracing::error!("Failed to get observations in radius: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to query observations"),
            );
        }
    };

    // Convert to GeoJSON (need locations for coordinates)
    let fc = radius_observations_to_geojson(&observations, lon, lat, radius_m);

    let json = match serde_json::to_string_pretty(&fc) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("Failed to serialize observations: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to serialize response"),
            );
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

/// GET /edr/collections/:collection_id/area (for observation collections)
///
/// Returns observations within a bounding box.
pub async fn obs_area_query_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<ObsAreaQueryParams>,
    _headers: HeaderMap,
) -> Response {
    let config = state.edr_config.read().await;

    // Find the collection
    let Some((model_config, _collection_def)) = config.find_collection(&collection_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        );
    };

    if !model_config.data_type.is_point_observation() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Collection {} is not a point observation collection",
                collection_id
            )),
        );
    }

    // Parse bbox from coords (BBOX format or POLYGON WKT)
    let bbox = match parse_bbox(&params.coords) {
        Ok(b) => b,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e));
        }
    };

    // Parse datetime range
    let (start_time, end_time) = parse_datetime_range(&params.datetime);

    // Get locations in bbox
    let locations = match state
        .observation_catalog
        .get_locations_in_bbox(bbox.0, bbox.1, bbox.2, bbox.3)
        .await
    {
        Ok(locs) => locs,
        Err(e) => {
            tracing::error!("Failed to get locations in bbox: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to query locations"),
            );
        }
    };

    // Apply limit
    let limit = params.limit.unwrap_or(1000) as usize;
    let locations: Vec<_> = locations.into_iter().take(limit).collect();

    // Get latest observation for each location
    let mut features = Vec::new();
    for loc in &locations {
        let query = ObservationQuery {
            location_id: Some(loc.id.clone()),
            source: model_config.observation_source.clone(),
            sources: None,
            start_time,
            end_time,
            limit: Some(1),
        };

        if let Ok(obs) = state.observation_catalog.get_observations(&query).await {
            if let Some(ob) = obs.first() {
                features.push(observation_to_feature(loc, ob));
            }
        }
    }

    let fc = ObservationFeatureCollection {
        collection_type: "FeatureCollection".to_string(),
        number_returned: features.len(),
        features,
        timestamp: Utc::now().to_rfc3339(),
    };

    let json = match serde_json::to_string_pretty(&fc) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("Failed to serialize observations: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to serialize response"),
            );
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

// ============================================================================
// Helper Functions
// ============================================================================

fn error_response(status: StatusCode, exc: ExceptionResponse) -> Response {
    let json = serde_json::to_string(&exc).unwrap_or_default();
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json.into())
        .unwrap()
}

fn location_to_feature(loc: &Location, base_url: &str, collection_id: &str) -> LocationFeature {
    let uri_id = format!(
        "{}/collections/{}/locations/{}",
        base_url, collection_id, loc.id
    );

    let mut extra = HashMap::new();
    if let Some(ref lt) = loc.location_type {
        extra.insert("type".to_string(), serde_json::Value::String(lt.clone()));
    }
    if let Some(ref country) = loc.country {
        extra.insert(
            "country".to_string(),
            serde_json::Value::String(country.clone()),
        );
    }
    if let Some(elev) = loc.elevation_m {
        extra.insert(
            "elevation_m".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(elev as f64).unwrap()),
        );
    }

    LocationFeature {
        feature_type: "Feature".to_string(),
        id: uri_id,
        geometry: LocationGeometry {
            geometry_type: "Point".to_string(),
            coordinates: vec![loc.lon, loc.lat],
        },
        properties: LocationProperties {
            name: loc.name.clone(),
            description: loc.description.clone(),
            datetime: None,
            extra,
        },
    }
}

fn observation_to_feature(loc: &Location, obs: &Observation) -> ObservationFeature {
    ObservationFeature {
        feature_type: "Feature".to_string(),
        id: format!("{}:{}", obs.location_id, obs.obs_time.to_rfc3339()),
        geometry: LocationGeometry {
            geometry_type: "Point".to_string(),
            coordinates: vec![loc.lon, loc.lat],
        },
        properties: ObservationProperties {
            location_id: obs.location_id.clone(),
            name: Some(loc.name.clone()),
            obs_time: obs.obs_time.to_rfc3339(),
            temperature_k: obs.temperature_k,
            dewpoint_k: obs.dewpoint_k,
            wind_direction_deg: obs.wind_direction_deg,
            wind_speed_ms: obs.wind_speed_ms,
            wind_gust_ms: obs.wind_gust_ms,
            visibility_m: obs.visibility_m,
            altimeter_pa: obs.altimeter_pa,
            sea_level_pressure_pa: obs.sea_level_pressure_pa,
            flight_category: obs.flight_category.clone(),
            raw_text: obs.raw_text.clone(),
        },
    }
}

fn observations_to_geojson(
    loc: &Location,
    observations: &[Observation],
) -> ObservationFeatureCollection {
    let features: Vec<ObservationFeature> = observations
        .iter()
        .map(|obs| observation_to_feature(loc, obs))
        .collect();

    ObservationFeatureCollection {
        collection_type: "FeatureCollection".to_string(),
        number_returned: features.len(),
        features,
        timestamp: Utc::now().to_rfc3339(),
    }
}

fn radius_observations_to_geojson(
    observations: &[(Location, Observation)],
    _center_lon: f64,
    _center_lat: f64,
    _radius_m: f64,
) -> ObservationFeatureCollection {
    let features: Vec<ObservationFeature> = observations
        .iter()
        .map(|(loc, obs)| observation_to_feature(loc, obs))
        .collect();

    ObservationFeatureCollection {
        collection_type: "FeatureCollection".to_string(),
        number_returned: features.len(),
        features,
        timestamp: Utc::now().to_rfc3339(),
    }
}

/// Parse a POINT(lon lat) WKT string.
fn parse_point_wkt(coords: &str) -> Result<(f64, f64), String> {
    let coords = coords.trim();

    // Handle "POINT(lon lat)" format
    if coords.to_uppercase().starts_with("POINT") {
        let inner = coords
            .trim_start_matches(|c: char| !c.is_ascii_digit() && c != '-' && c != '.')
            .trim_end_matches(')');

        let parts: Vec<&str> = inner.split_whitespace().collect();
        if parts.len() >= 2 {
            let lon: f64 = parts[0]
                .parse()
                .map_err(|_| "Invalid longitude".to_string())?;
            let lat: f64 = parts[1]
                .parse()
                .map_err(|_| "Invalid latitude".to_string())?;
            return Ok((lon, lat));
        }
    }

    // Handle "lon,lat" format
    let parts: Vec<&str> = coords.split(',').collect();
    if parts.len() >= 2 {
        let lon: f64 = parts[0]
            .trim()
            .parse()
            .map_err(|_| "Invalid longitude".to_string())?;
        let lat: f64 = parts[1]
            .trim()
            .parse()
            .map_err(|_| "Invalid latitude".to_string())?;
        return Ok((lon, lat));
    }

    Err("Invalid coordinates format. Use POINT(lon lat) or lon,lat".to_string())
}

/// Parse a radius string like "100km" or "50mi".
fn parse_radius(within: &Option<String>) -> Result<f64, String> {
    let default_radius_m = 100_000.0; // 100 km default

    let Some(within) = within else {
        return Ok(default_radius_m);
    };

    let within = within.trim().to_lowercase();

    if within.ends_with("km") {
        let num: f64 = within
            .trim_end_matches("km")
            .trim()
            .parse()
            .map_err(|_| "Invalid radius value".to_string())?;
        Ok(num * 1000.0)
    } else if within.ends_with("mi") {
        let num: f64 = within
            .trim_end_matches("mi")
            .trim()
            .parse()
            .map_err(|_| "Invalid radius value".to_string())?;
        Ok(num * 1609.34)
    } else if within.ends_with("m") {
        let num: f64 = within
            .trim_end_matches("m")
            .trim()
            .parse()
            .map_err(|_| "Invalid radius value".to_string())?;
        Ok(num)
    } else {
        // Assume km if no unit
        let num: f64 = within
            .parse()
            .map_err(|_| "Invalid radius value".to_string())?;
        Ok(num * 1000.0)
    }
}

/// Parse a bbox from coords parameter.
/// Supports: "minLon,minLat,maxLon,maxLat" or BBOX WKT.
fn parse_bbox(coords: &str) -> Result<(f64, f64, f64, f64), String> {
    let coords = coords.trim();

    // Handle comma-separated format
    let parts: Vec<&str> = coords.split(',').collect();
    if parts.len() >= 4 {
        let min_lon: f64 = parts[0]
            .trim()
            .parse()
            .map_err(|_| "Invalid min longitude".to_string())?;
        let min_lat: f64 = parts[1]
            .trim()
            .parse()
            .map_err(|_| "Invalid min latitude".to_string())?;
        let max_lon: f64 = parts[2]
            .trim()
            .parse()
            .map_err(|_| "Invalid max longitude".to_string())?;
        let max_lat: f64 = parts[3]
            .trim()
            .parse()
            .map_err(|_| "Invalid max latitude".to_string())?;
        return Ok((min_lon, min_lat, max_lon, max_lat));
    }

    Err("Invalid bbox format. Use minLon,minLat,maxLon,maxLat".to_string())
}

/// Parse datetime parameter into start/end times.
fn parse_datetime_range(
    datetime: &Option<String>,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let Some(dt) = datetime else {
        // Default to last 2 hours
        let end = Utc::now();
        let start = end - Duration::hours(2);
        return (Some(start), Some(end));
    };

    // Handle interval (start/end)
    if dt.contains('/') {
        let parts: Vec<&str> = dt.split('/').collect();
        if parts.len() == 2 {
            let start = DateTime::parse_from_rfc3339(parts[0])
                .ok()
                .map(|t| t.with_timezone(&Utc));
            let end = if parts[1] == ".." {
                Some(Utc::now())
            } else {
                DateTime::parse_from_rfc3339(parts[1])
                    .ok()
                    .map(|t| t.with_timezone(&Utc))
            };
            return (start, end);
        }
    }

    // Handle single instant
    if let Ok(instant) = DateTime::parse_from_rfc3339(dt) {
        let instant = instant.with_timezone(&Utc);
        // Return observations within 1 hour of the instant
        return (
            Some(instant - Duration::minutes(30)),
            Some(instant + Duration::minutes(30)),
        );
    }

    // Fallback to last 2 hours
    let end = Utc::now();
    let start = end - Duration::hours(2);
    (Some(start), Some(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_point_wkt() {
        assert_eq!(parse_point_wkt("POINT(-73.7 40.6)").unwrap(), (-73.7, 40.6));
        assert_eq!(
            parse_point_wkt("POINT( -73.7 40.6 )").unwrap(),
            (-73.7, 40.6)
        );
        assert_eq!(parse_point_wkt("-73.7,40.6").unwrap(), (-73.7, 40.6));
    }

    #[test]
    fn test_parse_radius() {
        assert_eq!(parse_radius(&Some("100km".to_string())).unwrap(), 100_000.0);
        assert_eq!(
            parse_radius(&Some("50mi".to_string())).unwrap(),
            50.0 * 1609.34
        );
        assert_eq!(parse_radius(&Some("5000m".to_string())).unwrap(), 5000.0);
        assert_eq!(parse_radius(&Some("100".to_string())).unwrap(), 100_000.0);
        assert_eq!(parse_radius(&None).unwrap(), 100_000.0);
    }

    #[test]
    fn test_parse_bbox() {
        let bbox = parse_bbox("-100,30,-90,40").unwrap();
        assert_eq!(bbox, (-100.0, 30.0, -90.0, 40.0));
    }
}
