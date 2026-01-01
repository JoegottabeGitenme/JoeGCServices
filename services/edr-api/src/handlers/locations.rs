//! Locations query handler.
//!
//! The locations endpoint provides two main functions:
//! 1. List all available named locations (GET /collections/{id}/locations)
//! 2. Query data at a specific named location (GET /collections/{id}/locations/{locationId})
//!
//! Named locations allow clients to query data using human-readable identifiers
//! (like airport codes or city names) instead of raw coordinates.

use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use edr_protocol::{
    coverage_json::CovJsonParameter, parameters::Unit, queries::DateTimeQuery,
    responses::ExceptionResponse, CoverageJson, EdrFeatureCollection, LocationFeatureCollection,
    PositionQuery as ParsedPositionQuery,
};
use grid_processor::DatasetQuery;
use serde::Deserialize;
use std::sync::Arc;

use crate::config::LevelValue;
use crate::content_negotiation::{negotiate_format, OutputFormat};
use crate::limits::ResponseSizeEstimate;
use crate::location_cache::LocationCacheKey;
use crate::state::AppState;

/// Query parameters for locations list endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct LocationsListParams {
    /// Output format.
    pub f: Option<String>,
}

/// Query parameters for location data query endpoint.
#[derive(Debug, Deserialize)]
pub struct LocationQueryParams {
    /// Vertical level(s).
    pub z: Option<String>,

    /// Datetime instant or interval.
    pub datetime: Option<String>,

    /// Parameter name(s) to retrieve.
    #[serde(rename = "parameter-name")]
    pub parameter_name: Option<String>,

    /// Coordinate reference system.
    pub crs: Option<String>,

    /// Output format.
    pub f: Option<String>,
}

/// GET /edr/collections/:collection_id/locations
///
/// Returns a GeoJSON FeatureCollection of all available named locations.
pub async fn locations_list_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<LocationsListParams>,
    headers: HeaderMap,
) -> Response {
    locations_list(state, collection_id, None, params, headers).await
}

/// GET /edr/collections/:collection_id/instances/:instance_id/locations
///
/// Returns a GeoJSON FeatureCollection of all available named locations for an instance.
pub async fn instance_locations_list_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, _instance_id)): Path<(String, String)>,
    Query(params): Query<LocationsListParams>,
    headers: HeaderMap,
) -> Response {
    // Locations are global, not instance-specific, but we validate the collection
    locations_list(state, collection_id, None, params, headers).await
}

async fn locations_list(
    state: Arc<AppState>,
    collection_id: String,
    _instance_id: Option<String>,
    params: LocationsListParams,
    _headers: HeaderMap,
) -> Response {
    let config = state.edr_config.read().await;

    // Validate the collection exists
    if config.find_collection(&collection_id).is_none() {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        );
    }

    // Get all locations from config
    let locations = &config.locations;

    if locations.is_empty() {
        // Return empty feature collection
        let fc = LocationFeatureCollection::from_locations(&[]);
        let json = serde_json::to_string_pretty(&fc).unwrap_or_default();

        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/geo+json")
            .header(header::CACHE_CONTROL, "max-age=3600")
            .body(json.into())
            .unwrap();
    }

    // Build GeoJSON FeatureCollection with URI-style IDs per OGC EDR spec
    let fc = LocationFeatureCollection::from_config_with_uris(locations, &state.base_url, &collection_id);

    // Determine output format
    let content_type = match params.f.as_deref() {
        Some("json") | Some("application/json") => "application/json",
        _ => "application/geo+json", // Default to GeoJSON
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
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "max-age=3600") // Locations are static
        .body(json.into())
        .unwrap()
}

/// GET /edr/collections/:collection_id/locations/:location_id
///
/// Query data at a specific named location.
pub async fn location_query_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, location_id)): Path<(String, String)>,
    Query(params): Query<LocationQueryParams>,
    headers: HeaderMap,
) -> Response {
    location_query(state, collection_id, None, location_id, params, headers).await
}

/// GET /edr/collections/:collection_id/instances/:instance_id/locations/:location_id
///
/// Query data at a specific named location for an instance.
pub async fn instance_location_query_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, instance_id, location_id)): Path<(String, String, String)>,
    Query(params): Query<LocationQueryParams>,
    headers: HeaderMap,
) -> Response {
    location_query(
        state,
        collection_id,
        Some(instance_id),
        location_id,
        params,
        headers,
    )
    .await
}

async fn location_query(
    state: Arc<AppState>,
    collection_id: String,
    instance_id: Option<String>,
    location_id: String,
    params: LocationQueryParams,
    headers: HeaderMap,
) -> Response {
    // Negotiate output format
    let output_format = match negotiate_format(&headers, params.f.as_deref()) {
        Ok(format) => format,
        Err(response) => return response,
    };

    // Build cache key early to check cache before expensive operations
    // Include format in cache key to ensure different formats are cached separately
    let cache_key = LocationCacheKey::new(
        &collection_id,
        &location_id,
        instance_id.clone(),
        params.datetime.clone(),
        params.parameter_name.clone(),
        params.z.clone(),
        params.f.clone(),
    );

    // Check cache first
    if let Some((cached_data, cached_content_type)) = state.location_cache.get(&cache_key).await {
        tracing::debug!(
            "Cache hit for location query: {}/{}",
            collection_id,
            location_id
        );
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, cached_content_type)
            .header(header::CACHE_CONTROL, "max-age=300")
            .header("X-Cache", "HIT")
            .body(axum::body::Body::from(cached_data))
            .unwrap();
    }

    let config = state.edr_config.read().await;

    // Find the collection
    let Some((model_config, collection_def)) = config.find_collection(&collection_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        );
    };

    // Find the location by ID
    let Some(location) = config.locations.find(&location_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!(
                "Location not found: {}. Use GET /collections/{}/locations to list available locations.",
                location_id, collection_id
            )),
        );
    };

    let lon = location.lon();
    let lat = location.lat();

    tracing::debug!(
        "Cache miss for location query: {}/{} at ({}, {})",
        location_id,
        location.name,
        lon,
        lat
    );

    // Parse vertical levels
    let z_values = if let Some(ref z) = params.z {
        match ParsedPositionQuery::parse_z(z) {
            Ok(values) => Some(values),
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!("Invalid z parameter: {}", e)),
                );
            }
        }
    } else {
        None
    };

    // Parse datetime
    let datetime_query = if let Some(ref dt) = params.datetime {
        match DateTimeQuery::parse(dt) {
            Ok(dt) => Some(dt),
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!("Invalid datetime: {}", e)),
                );
            }
        }
    } else {
        None
    };

    // Parse parameter names
    let requested_params = params
        .parameter_name
        .as_ref()
        .map(|p| ParsedPositionQuery::parse_parameter_names(p))
        .unwrap_or_default();

    // Determine which parameters to query
    let params_to_query: Vec<_> = if requested_params.is_empty() {
        collection_def
            .parameters
            .iter()
            .map(|p| p.name.clone())
            .collect()
    } else {
        // Validate requested parameters
        let available: Vec<_> = collection_def
            .parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        for param in &requested_params {
            if !available.contains(&param.as_str()) {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!(
                        "Parameter '{}' not available in collection. Available: {:?}",
                        param, available
                    )),
                );
            }
        }
        requested_params
    };

    // Expand datetime interval if needed
    let time_strings: Vec<String> = if let Some(ref dq) = datetime_query {
        if dq.is_interval() {
            let model_name = &model_config.model;
            let available_times: Vec<String> = state
                .catalog
                .get_model_valid_times(model_name)
                .await
                .ok()
                .unwrap_or_default()
                .into_iter()
                .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .collect();

            dq.expand_against_available_times(&available_times)
        } else {
            dq.to_vec()
        }
    } else {
        Vec::new()
    };

    // Check response size limits
    let num_levels = z_values.as_ref().map(|v| v.len()).unwrap_or(1);
    let num_times = if time_strings.is_empty() {
        1
    } else {
        time_strings.len()
    };
    let estimate = ResponseSizeEstimate::for_position(params_to_query.len(), num_times, num_levels);

    if let Err(limit_err) = estimate.check_limits(&model_config.limits) {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            ExceptionResponse::payload_too_large(limit_err.to_string()),
        );
    }

    // Parse and validate instance_id
    let reference_time = if let Some(ref id) = instance_id {
        match chrono::DateTime::parse_from_rfc3339(id) {
            Ok(dt) => {
                let ref_time = dt.with_timezone(&chrono::Utc);

                // Validate instance exists
                let model_name = &model_config.model;
                match state.catalog.get_model_runs_with_counts(model_name).await {
                    Ok(runs) => {
                        let run_exists = runs.iter().any(|(rt, _)| *rt == ref_time);
                        if !run_exists {
                            return error_response(
                                StatusCode::NOT_FOUND,
                                ExceptionResponse::not_found(format!(
                                    "Instance not found: {} for collection {}",
                                    id, collection_id
                                )),
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to validate instance: {}", e);
                    }
                }

                Some(ref_time)
            }
            Err(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!("Invalid instance ID format: {}", id)),
                );
            }
        }
    } else {
        None
    };

    // Determine query type
    let is_multi_z = z_values.as_ref().map(|v| v.len() > 1).unwrap_or(false);
    let z_val = z_values.as_ref().and_then(|v| v.first().copied());
    let is_multi_time = datetime_query
        .as_ref()
        .map(|dq| dq.is_multi_time())
        .unwrap_or(false);

    // Parse time strings
    let parsed_times: Vec<DateTime<Utc>> = time_strings
        .iter()
        .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .collect();

    // Build CoverageJSON response
    let mut coverage = if is_multi_z {
        let datetime_str = time_strings.first().cloned();
        CoverageJson::vertical_profile(lon, lat, datetime_str, z_values.clone().unwrap_or_default())
    } else if is_multi_time && !time_strings.is_empty() {
        CoverageJson::point_series(lon, lat, time_strings.clone(), z_val)
    } else {
        let datetime_str = time_strings.first().cloned();
        CoverageJson::point(lon, lat, datetime_str, z_val)
    };

    // Query each parameter
    for param_name in &params_to_query {
        let param_def = collection_def
            .parameters
            .iter()
            .find(|p| p.name == *param_name);

        // Handle multi-z (VerticalProfile)
        if is_multi_z {
            let z_vals = z_values.as_ref().unwrap();
            let mut values: Vec<Option<f32>> = Vec::with_capacity(z_vals.len());
            let mut units_str = String::new();

            for z in z_vals {
                let level_str =
                    build_level_string(&collection_def.level_filter, param_def, Some(*z));

                let mut query = DatasetQuery::forecast(&model_config.model, param_name);

                if let Some(level) = &level_str {
                    query = query.at_level(level);
                }

                if let Some(valid_time) = parsed_times.first() {
                    query = query.at_valid_time(*valid_time);
                }

                if let Some(ref_time) = reference_time {
                    query = query.at_run(ref_time);
                }

                match state.grid_data_service.read_point(&query, lon, lat).await {
                    Ok(point_value) => {
                        if units_str.is_empty() {
                            units_str = point_value.units.clone();
                        }
                        values.push(point_value.value);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to query {}/{} at location {} for z={}: {}",
                            model_config.model,
                            param_name,
                            location_id,
                            z,
                            e
                        );
                        values.push(None);
                    }
                }
            }

            let unit = Unit::from_symbol(&units_str);
            let cov_param = CovJsonParameter::new(param_name).with_unit(unit);
            coverage = coverage.with_vertical_profile_data(param_name, cov_param, values);
            continue;
        }

        let level_str = build_level_string(&collection_def.level_filter, param_def, z_val);

        // Handle multi-time (PointSeries)
        if is_multi_time && !parsed_times.is_empty() {
            let mut values: Vec<Option<f32>> = Vec::with_capacity(parsed_times.len());
            let mut units_str = String::new();

            for valid_time in &parsed_times {
                let mut query = DatasetQuery::forecast(&model_config.model, param_name)
                    .at_valid_time(*valid_time);

                if let Some(level) = &level_str {
                    query = query.at_level(level);
                }

                if let Some(ref_time) = reference_time {
                    query = query.at_run(ref_time);
                }

                match state.grid_data_service.read_point(&query, lon, lat).await {
                    Ok(point_value) => {
                        if units_str.is_empty() {
                            units_str = point_value.units.clone();
                        }
                        values.push(point_value.value);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to query {}/{} at location {} for time {}: {}",
                            model_config.model,
                            param_name,
                            location_id,
                            valid_time,
                            e
                        );
                        values.push(None);
                    }
                }
            }

            let unit = Unit::from_symbol(&units_str);
            let cov_param = CovJsonParameter::new(param_name).with_unit(unit);
            coverage = coverage.with_time_series(param_name, cov_param, values);
        } else {
            // Single time query
            let mut query = DatasetQuery::forecast(&model_config.model, param_name);

            if let Some(level) = &level_str {
                query = query.at_level(level);
            }

            if let Some(valid_time) = parsed_times.first() {
                query = query.at_valid_time(*valid_time);
            }

            if let Some(ref_time) = reference_time {
                query = query.at_run(ref_time);
            }

            match state.grid_data_service.read_point(&query, lon, lat).await {
                Ok(point_value) => {
                    let unit = Unit::from_symbol(&point_value.units);
                    let cov_param = CovJsonParameter::new(param_name).with_unit(unit);

                    if let Some(val) = point_value.value {
                        coverage = coverage.with_parameter(param_name, cov_param, val);
                    } else {
                        coverage = coverage.with_parameter_null(param_name, cov_param);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to query {}/{} at location {}: {}",
                        model_config.model,
                        param_name,
                        location_id,
                        e
                    );
                    let cov_param = CovJsonParameter::new(param_name);
                    coverage = coverage.with_parameter_null(param_name, cov_param);
                }
            }
        }
    }

    // Serialize response
    let (json, content_type) = match output_format {
        OutputFormat::GeoJson => {
            let geojson = EdrFeatureCollection::from(&coverage);
            match serde_json::to_string_pretty(&geojson) {
                Ok(j) => (j, output_format.content_type()),
                Err(e) => {
                    tracing::error!("Failed to serialize GeoJSON: {}", e);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ExceptionResponse::internal_error("Failed to serialize response"),
                    );
                }
            }
        }
        OutputFormat::CoverageJson => match serde_json::to_string_pretty(&coverage) {
            Ok(j) => (j, output_format.content_type()),
            Err(e) => {
                tracing::error!("Failed to serialize CoverageJSON: {}", e);
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ExceptionResponse::internal_error("Failed to serialize response"),
                );
            }
        },
    };

    // Cache the response
    state
        .location_cache
        .put(
            &cache_key,
            Bytes::from(json.clone()),
            content_type.to_string(),
        )
        .await;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "max-age=300")
        .header("X-Cache", "MISS")
        .body(json.into())
        .unwrap()
}

/// Build a catalog-compatible level string from EDR config.
fn build_level_string(
    level_filter: &crate::config::LevelFilter,
    param_def: Option<&crate::config::ParameterDefinition>,
    z_value: Option<f64>,
) -> Option<String> {
    let level_value = z_value.or_else(|| {
        param_def
            .and_then(|p| p.levels.first())
            .and_then(|l| match l {
                LevelValue::Numeric(n) => Some(*n),
                LevelValue::Named(_) => None,
            })
    });

    match level_filter.level_type.as_str() {
        "surface" => Some("surface".to_string()),
        "mean_sea_level" => Some("mean sea level".to_string()),
        "entire_atmosphere" => Some("entire atmosphere".to_string()),
        "isobaric" => level_value.map(|v| format!("{} mb", v as i32)),
        "height_above_ground" => level_value.map(|v| format!("{} m above ground", v as i32)),
        "cloud_layer" => {
            if let Some(code) = level_filter.level_code {
                match code {
                    212 => Some("low cloud layer".to_string()),
                    222 => Some("middle cloud layer".to_string()),
                    232 => Some("high cloud layer".to_string()),
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => param_def
            .and_then(|p| p.levels.first())
            .and_then(|l| match l {
                LevelValue::Named(name) => Some(name.clone()),
                LevelValue::Numeric(_) => None,
            }),
    }
}

fn error_response(status: StatusCode, exc: ExceptionResponse) -> Response {
    let json = serde_json::to_string(&exc).unwrap_or_default();
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json.into())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use edr_protocol::Location;

    #[test]
    fn test_location_lookup() {
        let config = edr_protocol::LocationsConfig {
            locations: vec![
                Location::new("KJFK", "JFK Airport", -73.7781, 40.6413),
                Location::new("KLAX", "LAX Airport", -118.4085, 33.9416),
            ],
        };

        assert!(config.find("KJFK").is_some());
        assert!(config.find("kjfk").is_some()); // Case insensitive
        assert!(config.find("UNKNOWN").is_none());
    }
}
