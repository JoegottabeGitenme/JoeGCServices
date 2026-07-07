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
use serde::Deserialize;
use std::sync::Arc;

use crate::availability::ModelAvailability;
use crate::config::build_level_string;
use crate::content_negotiation::{check_png_not_supported, negotiate_format, OutputFormat};
use crate::handlers::forecast_params::{validate_not_observation_data, ForecastParams};
use crate::handlers::observations::{
    obs_location_query_handler, obs_locations_list_handler, ObsLocationQueryParams,
    ObsLocationsListParams,
};
use crate::limits::ResponseSizeEstimate;
use crate::location_cache::LocationCacheKey;
use crate::metrics::{
    extract_client_ip, extract_user_agent, format_from_output, EndpointType, Timer,
};
use crate::state::AppState;

/// Filter collection parameters to only those with available data.
fn filter_available_parameters(
    collection_def: &crate::config::CollectionDefinition,
    availability: &ModelAvailability,
) -> Vec<String> {
    collection_def
        .parameters
        .iter()
        .filter(|p| availability.has_parameter(&p.name))
        .map(|p| p.name.clone())
        .collect()
}
use crate::validation::validate_z_against_vertical_extent;

/// Query parameters for locations list endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct LocationsListParams {
    /// Output format.
    pub f: Option<String>,

    /// Populated-places filters.
    #[serde(rename = "min-population")]
    pub min_population: Option<i64>,
    pub bbox: Option<String>,
    pub limit: Option<i64>,
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

    /// Model run time (ISO8601). Required if forecast-hour is specified.
    /// Only applicable to forecast models (GFS, HRRR, etc.), not observation data.
    pub run: Option<String>,

    /// Forecast hour(s) from the model run.
    /// Formats: single (6), list (0,6,12), range (0/24), range+step (0/24/6).
    /// Requires 'run' to be specified.
    #[serde(rename = "forecast-hour")]
    pub forecast_hour: Option<String>,

    /// Backing collections to sample (populated-places collection only).
    pub collections: Option<String>,
}

/// GET /edr/collections/:collection_id/locations
///
/// Returns a GeoJSON FeatureCollection of all available named locations.
/// For point observation collections (METAR, TAF, etc.), dispatches to observation handler.
pub async fn locations_list_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<LocationsListParams>,
    headers: HeaderMap,
) -> Response {
    // Check if this is a point data collection (METAR observations or TAF forecasts)
    {
        let config = state.edr_config.read().await;
        if let Some((model_config, _)) = config.find_collection(&collection_id) {
            if model_config.data_type.is_point_data() {
                // Dispatch to observation handler for both point observations and point forecasts
                let obs_params = ObsLocationsListParams {
                    f: params.f.clone(),
                    limit: None,
                    source: None,
                };
                return obs_locations_list_handler(
                    Extension(state.clone()),
                    Path(collection_id),
                    Query(obs_params),
                    headers,
                )
                .await;
            }
        }
    }

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
    let Some((model_config, _)) = config.find_collection(&collection_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        );
    };

    // Populated-places: list cities from PostGIS (filterable by population)
    if model_config.data_type.is_populated_places() {
        let pop_params = PopulatedListParams {
            min_population: params.min_population,
            bbox: params.bbox.clone(),
            limit: params.limit,
            f: params.f.clone(),
        };
        drop(config);
        return populated_locations_list_handler(
            Extension(state.clone()),
            Path(collection_id),
            Query(pop_params),
        )
        .await;
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
    let fc = LocationFeatureCollection::from_config_with_uris(
        locations,
        &state.base_url,
        &collection_id,
    );

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
/// For point observation collections (METAR, TAF, etc.), dispatches to observation handler.
pub async fn location_query_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, location_id)): Path<(String, String)>,
    Query(params): Query<LocationQueryParams>,
    headers: HeaderMap,
) -> Response {
    // Check if this is a point data collection (METAR observations or TAF forecasts)
    {
        let config = state.edr_config.read().await;
        if let Some((model_config, _)) = config.find_collection(&collection_id) {
            if model_config.data_type.is_populated_places() {
                let pop_params = PopulatedForecastParams {
                    collections: params.collections.clone(),
                    datetime: params.datetime.clone(),
                    ..Default::default()
                };
                drop(config);
                return populated_location_query_handler(
                    Extension(state.clone()),
                    Path((collection_id, location_id)),
                    Query(pop_params),
                )
                .await;
            }
            if model_config.data_type.is_point_data() {
                // Dispatch to observation handler for both point observations and point forecasts
                let obs_params = ObsLocationQueryParams {
                    datetime: params.datetime.clone(),
                    parameter_name: params.parameter_name.clone(),
                    f: params.f.clone(),
                    limit: None,
                };
                return obs_location_query_handler(
                    Extension(state.clone()),
                    Path((collection_id, location_id)),
                    Query(obs_params),
                    headers,
                )
                .await;
            }
        }
    }

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
    // Start timing and extract client info
    let timer = Timer::start();
    let client_ip = extract_client_ip(&headers);
    let user_agent = extract_user_agent(&headers);

    // Negotiate output format
    let output_format = match negotiate_format(&headers, params.f.as_deref()) {
        Ok(format) => format,
        Err(response) => return response,
    };

    // PNG output is not supported for locations queries (point data)
    if let Some(response) = check_png_not_supported(output_format, "locations") {
        return response;
    }

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

    // Get availability for this model
    let availability = state
        .availability_cache
        .get_model_availability(&state.catalog, &model_config.model)
        .await;

    let Some(availability) = availability else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!(
                "Collection {} has no available data",
                collection_id
            )),
        );
    };

    let available_params = filter_available_parameters(collection_def, &availability);

    if available_params.is_empty() {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!(
                "Collection {} has no parameters with available data",
                collection_id
            )),
        );
    }

    // Parse and validate forecast parameters (run, forecast-hour)
    let forecast_params =
        match ForecastParams::parse(params.run.as_deref(), params.forecast_hour.as_deref()) {
            Ok(fp) => fp,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, e);
            }
        };

    // Validate that forecast params are not used with observation data
    let is_observation_model = model_config.create_query("dummy").observation_data;
    if let Err(e) = validate_not_observation_data(&forecast_params, is_observation_model) {
        return error_response(StatusCode::BAD_REQUEST, e);
    }

    // Determine forecast query strategy
    let _forecast_strategy = forecast_params.strategy();

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
            Ok(values) => {
                // Validate z values against collection's vertical extent
                if let Err(e) = validate_z_against_vertical_extent(&values, collection_def) {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        ExceptionResponse::bad_request(e),
                    );
                }
                Some(values)
            }
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

    // Determine which parameters to query (filtered by availability)
    let params_to_query: Vec<_> = if requested_params.is_empty() {
        // Return all available parameters
        available_params.clone()
    } else {
        // Validate requested parameters exist and have data
        for param in &requested_params {
            if !available_params.contains(param) {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!(
                        "Parameter '{}' not available in collection. Available: {:?}",
                        param, available_params
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

                // Build the DatasetQuery using the appropriate type (forecast vs observation)
                let mut query = model_config.create_query(param_name);

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
                // Build the DatasetQuery for this specific valid time (forecast vs observation)
                let mut query = model_config
                    .create_query(param_name)
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
            // Build the DatasetQuery using the appropriate type (forecast vs observation)
            let mut query = model_config.create_query(param_name);

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
        OutputFormat::Png => {
            // PNG is rejected earlier in check_png_not_supported, this should never be reached
            unreachable!("PNG format should have been rejected earlier")
        }
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

    // Record successful request metrics
    state
        .metrics
        .record_request(
            EndpointType::Locations,
            Some(&collection_id),
            &params_to_query,
            format_from_output(&output_format),
            timer.elapsed_us(),
            true,
            client_ip.as_deref(),
            user_agent.as_deref(),
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

fn error_response(status: StatusCode, exc: ExceptionResponse) -> Response {
    let json = serde_json::to_string(&exc).unwrap_or_default();
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json.into())
        .unwrap()
}

// =============================================================================
// Global Locations Endpoints
// =============================================================================
//
// These endpoints provide a unified view of all known locations across both
// static config (for gridded data) and database (for point observations/forecasts).
//
// GET /edr/locations - List all known locations
// GET /edr/locations/{locationId} - Get all available data at a location

/// Query parameters for global locations list endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct GlobalLocationsListParams {
    /// Output format (json, geojson).
    pub f: Option<String>,
    /// Filter to locations with data in these collections (comma-separated).
    pub collections: Option<String>,
}

/// Query parameters for global location data endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct GlobalLocationDataParams {
    /// Output format (json, geojson, text).
    pub f: Option<String>,
    /// Collections to fetch data from (comma-separated).
    /// If not specified, returns only location metadata and available_collections.
    /// Example: ?collections=metar,taf,gfs-temperature
    pub collections: Option<String>,
}

/// Response structure for a location feature with available data.
#[derive(Debug, serde::Serialize)]
pub struct GlobalLocationFeature {
    #[serde(rename = "type")]
    pub feature_type: String,
    pub id: String,
    pub geometry: GlobalLocationGeometry,
    pub properties: GlobalLocationProperties,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<GlobalLocationLink>,
}

#[derive(Debug, serde::Serialize)]
pub struct GlobalLocationGeometry {
    #[serde(rename = "type")]
    pub geom_type: String,
    pub coordinates: [f64; 2],
}

#[derive(Debug, serde::Serialize)]
pub struct GlobalLocationProperties {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevation_m: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub available_collections: Vec<String>,
    /// Nested data (METAR, TAF, etc.) - only present in single location response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<GlobalLocationData>,
}

#[derive(Debug, serde::Serialize)]
pub struct GlobalLocationData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metar: Option<MetarData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taf: Option<TafData>,
    /// Gridded data collections (keyed by collection ID).
    /// Each contains the latest data at this location.
    #[serde(flatten)]
    pub gridded: std::collections::HashMap<String, GriddedCollectionData>,
}

/// Data from a gridded collection at a point location.
#[derive(Debug, serde::Serialize)]
pub struct GriddedCollectionData {
    /// Reference time (model run time) for forecast data, or observation time.
    pub reference_time: String,
    /// Valid time (when the forecast/observation is for).
    pub valid_time: String,
    /// Forecast hour (0 for observations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forecast_hour: Option<i32>,
    /// Parameter values keyed by level (e.g., "surface", "850 mb").
    /// Structure: { "parameter_name": { "level": value, ... }, ... }
    pub parameters: std::collections::HashMap<String, GriddedParameterData>,
}

/// Data for a single parameter at multiple levels.
#[derive(Debug, serde::Serialize)]
pub struct GriddedParameterData {
    /// Unit of measurement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Values at each level.
    pub values: std::collections::HashMap<String, Option<f32>>,
}

#[derive(Debug, serde::Serialize)]
pub struct MetarData {
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
    pub wave_height_m: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dominant_wave_period_s: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_wave_period_s: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_wave_direction_deg: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub water_temp_k: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tide_m: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub water_column_height_m: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flight_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_layers: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_observation: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct TafData {
    pub issue_time: String,
    pub valid_from: String,
    pub valid_to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_taf: Option<String>,
    pub periods: Vec<TafPeriodData>,
}

#[derive(Debug, serde::Serialize)]
pub struct TafPeriodData {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probability: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_direction_deg: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_speed_ms: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_gust_ms: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility_m: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_layers: Option<serde_json::Value>,
}

#[derive(Debug, serde::Serialize)]
pub struct GlobalLocationLink {
    pub rel: String,
    pub href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct GlobalLocationsCollection {
    #[serde(rename = "type")]
    pub collection_type: String,
    pub features: Vec<GlobalLocationFeature>,
    #[serde(rename = "numberReturned")]
    pub number_returned: usize,
    pub links: Vec<GlobalLocationLink>,
}

/// GET /edr/locations - List all known locations
///
/// Returns a merged list of locations from:
/// - Database (airports with METAR/TAF data)
/// - Static config (predefined locations for gridded data)
///
/// Database locations take precedence for coordinates when IDs overlap.
pub async fn global_locations_list_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<GlobalLocationsListParams>,
) -> Response {
    let base_url = &state.base_url;

    // Parse collection filter if provided
    let collection_filter: Option<Vec<String>> = params.collections.as_ref().map(|c| {
        c.split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    });

    // Get locations from database
    let db_locations = state
        .observation_catalog
        .get_locations(None)
        .await
        .unwrap_or_default();

    // Get locations from static config
    let config = state.edr_config.read().await;
    let static_locations = &config.locations.locations;

    // Merge locations: database takes precedence for coordinates
    let mut locations_map: std::collections::HashMap<String, GlobalLocationFeature> =
        std::collections::HashMap::new();

    // First add static locations
    for loc in static_locations {
        let id = loc.id.to_uppercase();
        let feature = GlobalLocationFeature {
            feature_type: "Feature".to_string(),
            id: id.clone(),
            geometry: GlobalLocationGeometry {
                geom_type: "Point".to_string(),
                coordinates: [loc.coords[0], loc.coords[1]],
            },
            properties: GlobalLocationProperties {
                name: loc.name.clone(),
                description: loc.description.clone(),
                location_type: loc.properties.get("type").cloned(),
                elevation_m: None,
                country: loc.properties.get("country").cloned(),
                available_collections: Vec::new(), // Not calculated for list view
                data: None,
            },
            links: vec![GlobalLocationLink {
                rel: "self".to_string(),
                href: format!("{}/locations/{}", base_url, id),
                title: None,
            }],
        };
        locations_map.insert(id, feature);
    }

    // Then add/override with database locations (more accurate coords)
    for loc in db_locations {
        let id = loc.id.to_uppercase();
        let feature = GlobalLocationFeature {
            feature_type: "Feature".to_string(),
            id: id.clone(),
            geometry: GlobalLocationGeometry {
                geom_type: "Point".to_string(),
                coordinates: [loc.lon, loc.lat],
            },
            properties: GlobalLocationProperties {
                name: loc.name.clone(),
                description: loc.description.clone(),
                location_type: loc.location_type.clone(),
                elevation_m: loc.elevation_m,
                country: loc.country.clone(),
                available_collections: Vec::new(), // Not calculated for list view
                data: None,
            },
            links: vec![GlobalLocationLink {
                rel: "self".to_string(),
                href: format!("{}/locations/{}", base_url, id),
                title: None,
            }],
        };
        locations_map.insert(id, feature);
    }

    // If collection filter is specified, filter locations
    let mut features: Vec<GlobalLocationFeature> = if let Some(ref filter) = collection_filter {
        // For point data collections, check if location has data
        let mut filtered = Vec::new();
        for (id, feature) in locations_map {
            let mut has_matching_collection = false;

            for coll in filter {
                if coll == "metar" {
                    // Check if location has METAR data
                    if let Ok(Some(_)) = state
                        .observation_catalog
                        .get_latest_observation(&id, Some("metar"))
                        .await
                    {
                        has_matching_collection = true;
                        break;
                    }
                } else if coll == "taf" {
                    // Check if location has TAF data
                    if let Ok(Some(_)) = state.observation_catalog.get_latest_taf(&id).await {
                        has_matching_collection = true;
                        break;
                    }
                } else {
                    // For gridded collections, check if location is within bbox
                    // This is expensive, so for list view we assume static locations
                    // are valid for gridded collections they were configured for
                    has_matching_collection = true;
                    break;
                }
            }

            if has_matching_collection {
                filtered.push(feature);
            }
        }
        filtered
    } else {
        locations_map.into_values().collect()
    };

    // Sort by ID for consistent ordering
    features.sort_by(|a, b| a.id.cmp(&b.id));

    let response = GlobalLocationsCollection {
        collection_type: "FeatureCollection".to_string(),
        number_returned: features.len(),
        features,
        links: vec![GlobalLocationLink {
            rel: "self".to_string(),
            href: format!("{}/locations", base_url),
            title: None,
        }],
    };

    let json = serde_json::to_string_pretty(&response).unwrap_or_default();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

/// GET /edr/locations/{locationId} - Get all available data at a location
///
/// Returns the location metadata along with data from requested collections.
///
/// If `?collections=` is not specified, returns only location metadata and
/// the list of available collections (no actual data).
///
/// If `?collections=metar,taf,gfs-temperature` is specified, fetches the latest
/// data from those collections at this location.
pub async fn global_location_data_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(location_id): Path<String>,
    Query(params): Query<GlobalLocationDataParams>,
) -> Response {
    let base_url = &state.base_url;
    let location_id = location_id.to_uppercase();

    // Try to find location in database first (more accurate coords)
    let db_location = state
        .observation_catalog
        .get_location(&location_id)
        .await
        .ok()
        .flatten();

    // Try static config as fallback
    let config = state.edr_config.read().await;
    let static_location = config.locations.find(&location_id);

    // Determine location info (database takes precedence)
    let (lon, lat, name, description, location_type, elevation_m, country) =
        if let Some(ref loc) = db_location {
            (
                loc.lon,
                loc.lat,
                loc.name.clone(),
                loc.description.clone(),
                loc.location_type.clone(),
                loc.elevation_m,
                loc.country.clone(),
            )
        } else if let Some(loc) = static_location {
            (
                loc.coords[0],
                loc.coords[1],
                loc.name.clone(),
                loc.description.clone(),
                loc.properties.get("type").cloned(),
                None,
                loc.properties.get("country").cloned(),
            )
        } else {
            return error_response(
                StatusCode::NOT_FOUND,
                ExceptionResponse::not_found(format!("Location not found: {}", location_id)),
            );
        };

    // Parse requested collections from query parameter
    let requested_collections: Option<Vec<String>> = params.collections.as_ref().map(|c| {
        c.split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    });

    // Check collection count limit
    if let Some(ref colls) = requested_collections {
        let max_collections = config
            .server
            .global_limits
            .max_collections_per_location_request;
        if colls.len() > max_collections {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "Too many collections requested: {} exceeds limit of {}",
                    colls.len(),
                    max_collections
                )),
            );
        }
    }

    // Determine all available collections at this location
    let mut available_collections = Vec::new();
    let mut links = vec![GlobalLocationLink {
        rel: "self".to_string(),
        href: format!("{}/locations/{}", base_url, location_id),
        title: None,
    }];

    // Check for METAR availability
    let has_metar = state
        .observation_catalog
        .get_latest_observation(&location_id, Some("metar"))
        .await
        .ok()
        .flatten()
        .is_some();
    if has_metar {
        available_collections.push("metar".to_string());
        links.push(GlobalLocationLink {
            rel: "collection".to_string(),
            href: format!("{}/collections/metar/locations/{}", base_url, location_id),
            title: Some("METAR observations".to_string()),
        });
    }

    // Check for TAF availability
    let has_taf = state
        .observation_catalog
        .get_latest_taf(&location_id)
        .await
        .ok()
        .flatten()
        .is_some();
    if has_taf {
        available_collections.push("taf".to_string());
        links.push(GlobalLocationLink {
            rel: "collection".to_string(),
            href: format!("{}/collections/taf/locations/{}", base_url, location_id),
            title: Some("TAF forecasts".to_string()),
        });
    }

    // Check gridded collections - see which have data covering this point
    for (model_id, model_config) in &config.models {
        if model_config.data_type.is_point_data() {
            continue; // Already handled above
        }

        // Get the model's actual data extent from catalog
        if let Ok(bbox) = state.catalog.get_model_bbox(model_id).await {
            // Normalize longitude for models using 0-360 convention (e.g., GFS)
            // If bbox extends beyond 180, the model uses 0-360 convention
            let check_lon = if bbox.max_x > 180.0 {
                if lon < 0.0 {
                    lon + 360.0
                } else {
                    lon
                }
            } else {
                lon
            };

            // Check if point is within bbox
            if check_lon >= bbox.min_x
                && check_lon <= bbox.max_x
                && lat >= bbox.min_y
                && lat <= bbox.max_y
            {
                // Add all collections from this model
                for coll in &model_config.collections {
                    available_collections.push(coll.id.clone());
                    links.push(GlobalLocationLink {
                        rel: "collection".to_string(),
                        href: format!(
                            "{}/collections/{}/position?coords=POINT({} {})",
                            base_url, coll.id, lon, lat
                        ),
                        title: Some(coll.title.clone()),
                    });
                }
            }
        }
    }

    // Sort collections for consistent output
    available_collections.sort();
    available_collections.dedup();

    // If no collections requested, return metadata only
    if requested_collections.is_none() {
        return build_location_metadata_response(
            &location_id,
            lon,
            lat,
            name,
            description,
            location_type,
            elevation_m,
            country,
            available_collections,
            links,
            &params,
        );
    }

    let requested = requested_collections.unwrap();

    // Validate requested collections exist
    for coll in &requested {
        if !available_collections.contains(coll) {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "Collection '{}' is not available at this location. Available: {:?}",
                    coll, available_collections
                )),
            );
        }
    }

    // Fetch data for requested collections
    let mut metar_data: Option<MetarData> = None;
    let mut taf_data: Option<TafData> = None;
    let mut gridded_data: std::collections::HashMap<String, GriddedCollectionData> =
        std::collections::HashMap::new();

    for coll_id in &requested {
        if coll_id == "metar" || coll_id == "ndbc" || coll_id == "dart" {
            // Fetch observation data (METAR, NDBC, or DART)
            let source = if coll_id == "ndbc" {
                "ndbc"
            } else if coll_id == "dart" {
                "dart"
            } else {
                "metar"
            };
            if let Ok(Some(obs)) = state
                .observation_catalog
                .get_latest_observation(&location_id, Some(source))
                .await
            {
                metar_data = Some(MetarData {
                    obs_time: obs.obs_time.to_rfc3339(),
                    temperature_k: obs.temperature_k,
                    dewpoint_k: obs.dewpoint_k,
                    wind_direction_deg: obs.wind_direction_deg,
                    wind_speed_ms: obs.wind_speed_ms,
                    wind_gust_ms: obs.wind_gust_ms,
                    visibility_m: obs.visibility_m,
                    altimeter_pa: obs.altimeter_pa,
                    sea_level_pressure_pa: obs.sea_level_pressure_pa,
                    wave_height_m: obs.wave_height_m,
                    dominant_wave_period_s: obs.dominant_wave_period_s,
                    average_wave_period_s: obs.average_wave_period_s,
                    mean_wave_direction_deg: obs.mean_wave_direction_deg,
                    water_temp_k: obs.water_temp_k,
                    tide_m: obs.tide_m,
                    water_column_height_m: obs.water_column_height_m,
                    flight_category: obs.flight_category.clone(),
                    cloud_layers: obs.cloud_layers.clone(),
                    raw_observation: obs.raw_text.clone(),
                });
            }
        } else if coll_id == "taf" {
            // Fetch TAF data
            if let Ok(Some(taf)) = state.observation_catalog.get_latest_taf(&location_id).await {
                let periods: Vec<TafPeriodData> = taf
                    .periods
                    .iter()
                    .map(|p| TafPeriodData {
                        from: p.period_from.to_rfc3339(),
                        to: p.period_to.to_rfc3339(),
                        change_type: p.change_indicator.clone(),
                        probability: p.probability,
                        wind_direction_deg: p.wind_direction_deg,
                        wind_speed_ms: p.wind_speed_ms,
                        wind_gust_ms: p.wind_gust_ms,
                        visibility_m: p.visibility_m,
                        cloud_layers: p.cloud_layers.clone(),
                    })
                    .collect();

                taf_data = Some(TafData {
                    issue_time: taf.forecast.issue_time.to_rfc3339(),
                    valid_from: taf.forecast.valid_from.to_rfc3339(),
                    valid_to: taf.forecast.valid_to.to_rfc3339(),
                    raw_taf: taf.forecast.raw_taf.clone(),
                    periods,
                });
            }
        } else {
            // Fetch gridded data for this collection
            if let Some(gridded) =
                fetch_gridded_collection_data(&state, &config, coll_id, lon, lat).await
            {
                gridded_data.insert(coll_id.clone(), gridded);
            }
        }
    }

    // Check for text format output
    let wants_text = params
        .f
        .as_ref()
        .map(|f| f == "text" || f == "text/plain")
        .unwrap_or(false);

    if wants_text {
        return build_location_text_response(
            &location_id,
            lon,
            lat,
            &name,
            elevation_m,
            &available_collections,
            &metar_data,
            &taf_data,
            &gridded_data,
        );
    }

    // Build GeoJSON response with data
    let data = GlobalLocationData {
        metar: metar_data,
        taf: taf_data,
        gridded: gridded_data,
    };

    let feature = GlobalLocationFeature {
        feature_type: "Feature".to_string(),
        id: location_id.clone(),
        geometry: GlobalLocationGeometry {
            geom_type: "Point".to_string(),
            coordinates: [lon, lat],
        },
        properties: GlobalLocationProperties {
            name,
            description,
            location_type,
            elevation_m,
            country,
            available_collections,
            data: Some(data),
        },
        links,
    };

    let json = serde_json::to_string_pretty(&feature).unwrap_or_default();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

/// Build a metadata-only response (when no collections are requested).
fn build_location_metadata_response(
    location_id: &str,
    lon: f64,
    lat: f64,
    name: String,
    description: Option<String>,
    location_type: Option<String>,
    elevation_m: Option<f32>,
    country: Option<String>,
    available_collections: Vec<String>,
    links: Vec<GlobalLocationLink>,
    params: &GlobalLocationDataParams,
) -> Response {
    // Check for text format output
    let wants_text = params
        .f
        .as_ref()
        .map(|f| f == "text" || f == "text/plain")
        .unwrap_or(false);

    if wants_text {
        let mut text = format!(
            "Location: {} - {}\nCoordinates: {:.4}°N, {:.4}°W\n",
            location_id,
            name,
            lat,
            lon.abs()
        );
        if let Some(elev) = elevation_m {
            text.push_str(&format!("Elevation: {}m\n", elev));
        }
        text.push('\n');

        if available_collections.is_empty() {
            text.push_str("No data available at this location.\n");
        } else {
            text.push_str(&format!(
                "Available collections: {}\n\n",
                available_collections.join(", ")
            ));
            text.push_str(
                "Use ?collections=<name>,<name> to fetch data from specific collections.\n",
            );
        }

        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(header::CACHE_CONTROL, "max-age=60")
            .body(text.into())
            .unwrap();
    }

    // GeoJSON response without data
    let feature = GlobalLocationFeature {
        feature_type: "Feature".to_string(),
        id: location_id.to_string(),
        geometry: GlobalLocationGeometry {
            geom_type: "Point".to_string(),
            coordinates: [lon, lat],
        },
        properties: GlobalLocationProperties {
            name,
            description,
            location_type,
            elevation_m,
            country,
            available_collections,
            data: None,
        },
        links,
    };

    let json = serde_json::to_string_pretty(&feature).unwrap_or_default();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

/// Build a text format response with data.
fn build_location_text_response(
    location_id: &str,
    lon: f64,
    lat: f64,
    name: &str,
    elevation_m: Option<f32>,
    available_collections: &[String],
    metar_data: &Option<MetarData>,
    taf_data: &Option<TafData>,
    gridded_data: &std::collections::HashMap<String, GriddedCollectionData>,
) -> Response {
    let mut text = format!(
        "Location: {} - {}\nCoordinates: {:.4}°N, {:.4}°W\n",
        location_id,
        name,
        lat,
        lon.abs()
    );
    if let Some(elev) = elevation_m {
        text.push_str(&format!("Elevation: {}m\n", elev));
    }
    text.push('\n');

    if let Some(ref metar) = metar_data {
        text.push_str("=== METAR ===\n");
        if let Some(ref raw) = metar.raw_observation {
            text.push_str(raw);
        } else {
            text.push_str(&format!("Time: {}\n", metar.obs_time));
            if let Some(t) = metar.temperature_k {
                text.push_str(&format!("Temperature: {:.1}K ({:.1}°C)\n", t, t - 273.15));
            }
            if let Some(w) = metar.wind_speed_ms {
                text.push_str(&format!("Wind: {:.1} m/s", w));
                if let Some(d) = metar.wind_direction_deg {
                    text.push_str(&format!(" from {}°", d));
                }
                text.push('\n');
            }
        }
        text.push_str("\n\n");
    }

    if let Some(ref taf) = taf_data {
        text.push_str("=== TAF ===\n");
        if let Some(ref raw) = taf.raw_taf {
            text.push_str(raw);
        } else {
            text.push_str(&format!("Issued: {}\n", taf.issue_time));
            text.push_str(&format!("Valid: {} to {}\n", taf.valid_from, taf.valid_to));
        }
        text.push_str("\n\n");
    }

    // Gridded data
    for (coll_id, data) in gridded_data {
        text.push_str(&format!("=== {} ===\n", coll_id.to_uppercase()));
        text.push_str(&format!("Reference time: {}\n", data.reference_time));
        text.push_str(&format!("Valid time: {}\n", data.valid_time));
        if let Some(fh) = data.forecast_hour {
            text.push_str(&format!("Forecast hour: {}\n", fh));
        }
        for (param_name, param_data) in &data.parameters {
            text.push_str(&format!("  {}:\n", param_name));
            for (level, value) in &param_data.values {
                if let Some(v) = value {
                    let unit_str = param_data.unit.as_deref().unwrap_or("");
                    text.push_str(&format!("    {}: {:.2} {}\n", level, v, unit_str));
                } else {
                    text.push_str(&format!("    {}: null\n", level));
                }
            }
        }
        text.push('\n');
    }

    if available_collections.is_empty() {
        text.push_str("No data available at this location.\n");
    } else {
        text.push_str(&format!(
            "Available collections: {}\n",
            available_collections.join(", ")
        ));
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(text.into())
        .unwrap()
}

/// Fetch gridded data for a collection at a specific point.
///
/// Returns the latest data (closest to now) with all available levels.
async fn fetch_gridded_collection_data(
    state: &Arc<AppState>,
    config: &crate::config::EdrConfig,
    collection_id: &str,
    lon: f64,
    lat: f64,
) -> Option<GriddedCollectionData> {
    fetch_gridded_collection_data_at(state, config, collection_id, lon, lat, None).await
}

// ============================================================================
// Populated Places collection (data_type: populated_places)
//
// A coordinate registry of US cities. /locations discovers places; /locations/{id}
// and /radius resolve places to coordinates and proxy point-forecast data from
// configured gridded collections (default GFS). Holds no weather data itself.
// ============================================================================

/// Query params for the populated-places /locations list.
#[derive(Debug, Deserialize, Default)]
pub struct PopulatedListParams {
    /// Minimum population filter (defaults to the collection's configured floor).
    #[serde(rename = "min-population")]
    pub min_population: Option<i64>,
    /// Bounding box filter: "min_lon,min_lat,max_lon,max_lat".
    pub bbox: Option<String>,
    /// Max places to return (capped by config).
    pub limit: Option<i64>,
    pub f: Option<String>,
}

/// Query params for populated-places forecast (per-place and radius).
#[derive(Debug, Deserialize, Default)]
pub struct PopulatedForecastParams {
    /// WKT point for /radius, e.g. "POINT(-86.8 33.5)".
    pub coords: Option<String>,
    /// Comma-separated gridded collections to sample (default from config).
    pub collections: Option<String>,
    /// Target valid time (ISO8601). Omit for closest-to-now.
    pub datetime: Option<String>,
    /// Radius (for /radius), e.g. "100km" or "50" (+ within-units).
    pub within: Option<String>,
    #[serde(rename = "within-units")]
    pub within_units: Option<String>,
    /// Minimum population filter (for /radius).
    #[serde(rename = "min-population")]
    pub min_population: Option<i64>,
    pub limit: Option<i64>,
    pub f: Option<String>,
}

/// Resolve the backing collections for a request: explicit `?collections=` or
/// the collection's configured default.
fn resolve_backing_collections(
    requested: Option<&str>,
    model_config: &crate::config::ModelEdrConfig,
) -> Vec<String> {
    if let Some(list) = requested {
        let v: Vec<String> = list
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    model_config
        .default_backing_collections
        .clone()
        .unwrap_or_else(|| vec!["gfs-surface".to_string()])
}

/// Build a GeoJSON point feature for a populated place, with population/state
/// properties and ready-to-use forecast links.
fn populated_place_feature(
    loc: &storage::observations::Location,
    base_url: &str,
    default_collections: &[String],
) -> serde_json::Value {
    let pop = loc.properties.get("population").cloned();
    let state = loc.region.clone().or_else(|| {
        loc.properties
            .get("state")
            .and_then(|v| v.as_str().map(String::from))
    });

    // Pre-built forecast links per default backing collection
    let forecast_links: Vec<serde_json::Value> = default_collections
        .iter()
        .map(|c| {
            serde_json::json!({
                "collection": c,
                "href": format!(
                    "{}/collections/{}/position?coords=POINT({} {})",
                    base_url, c, loc.lon, loc.lat
                ),
            })
        })
        .collect();

    serde_json::json!({
        "type": "Feature",
        "id": loc.id,
        "geometry": { "type": "Point", "coordinates": [loc.lon, loc.lat] },
        "properties": {
            "name": loc.name,
            "state": state,
            "population": pop,
            "forecast": format!("{}/collections/populated/locations/{}", base_url, loc.id),
            "forecast_links": forecast_links,
        },
    })
}

/// GET /edr/collections/populated/locations
///
/// Discover populated places (GeoJSON FeatureCollection). Filter with
/// ?min-population= and ?bbox=; capped by config.
pub async fn populated_locations_list_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(_collection_id): Path<String>,
    Query(params): Query<PopulatedListParams>,
) -> Response {
    let config = state.edr_config.read().await;
    let Some((model_config, _)) = config.find_collection("populated") else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found("Collection not found: populated"),
        );
    };
    let limits = &model_config.limits;
    let min_pop = params
        .min_population
        .unwrap_or(limits.default_min_population);
    let limit = params
        .limit
        .unwrap_or(limits.max_places_per_request)
        .clamp(1, limits.max_places_per_request);

    let bbox = match &params.bbox {
        Some(s) => match parse_bbox(s) {
            Ok(b) => Some(b),
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e))
            }
        },
        None => None,
    };

    let default_collections = model_config
        .default_backing_collections
        .clone()
        .unwrap_or_else(|| vec!["gfs-surface".to_string()]);

    let places = match state
        .observation_catalog
        .get_populated_places(min_pop, bbox, limit)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("get_populated_places failed: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to query populated places"),
            );
        }
    };

    let features: Vec<serde_json::Value> = places
        .iter()
        .map(|p| populated_place_feature(p, &state.base_url, &default_collections))
        .collect();

    let fc = serde_json::json!({
        "type": "FeatureCollection",
        "numberReturned": features.len(),
        "min_population": min_pop,
        "features": features,
    });

    json_response(fc, "max-age=86400")
}

/// GET /edr/collections/populated/locations/{id}
///
/// Fetch point-forecast data at a city, proxied from backing collections.
pub async fn populated_location_query_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((_collection_id, location_id)): Path<(String, String)>,
    Query(params): Query<PopulatedForecastParams>,
) -> Response {
    let location_id = location_id.to_uppercase();
    let config = state.edr_config.read().await;
    let Some((model_config, _)) = config.find_collection("populated") else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found("Collection not found: populated"),
        );
    };

    let place = match state.observation_catalog.get_populated_place(&location_id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                ExceptionResponse::not_found(format!(
                    "Populated place not found: {}. Use GET /collections/populated/locations to list places.",
                    location_id
                )),
            )
        }
        Err(e) => {
            tracing::error!("get_populated_place failed: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to query place"),
            );
        }
    };

    let target_time = match parse_optional_datetime(&params.datetime) {
        Ok(t) => t,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e))
        }
    };

    let backing = resolve_backing_collections(params.collections.as_deref(), model_config);
    let max_collections = model_config.limits.max_collections_per_request;
    if backing.len() > max_collections {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Too many collections: {} exceeds limit of {}",
                backing.len(),
                max_collections
            )),
        );
    }

    let feature = build_place_forecast_feature(
        &state,
        &config,
        &place,
        &backing,
        target_time,
        &state.base_url,
    )
    .await;

    json_response(feature, "max-age=300")
}

/// GET /edr/collections/populated/radius
///
/// Fetch point forecasts for all populated places within a radius of a point.
pub async fn populated_radius_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(_collection_id): Path<String>,
    Query(params): Query<PopulatedForecastParams>,
) -> Response {
    let config = state.edr_config.read().await;
    let Some((model_config, _)) = config.find_collection("populated") else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found("Collection not found: populated"),
        );
    };
    let limits = &model_config.limits;

    let (lon, lat) = match parse_wkt_point(params.coords.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e))
        }
    };

    let radius_m = match parse_within(&params.within, params.within_units.as_deref()) {
        Ok(m) => m,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e))
        }
    };
    if let Some(max_km) = limits.max_radius_km {
        if radius_m > max_km * 1000.0 {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Radius exceeds limit of {} km", max_km)),
            );
        }
    }

    let target_time = match parse_optional_datetime(&params.datetime) {
        Ok(t) => t,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e))
        }
    };

    let min_pop = params
        .min_population
        .unwrap_or(limits.default_min_population);
    let limit = params
        .limit
        .unwrap_or(limits.max_places_per_request)
        .clamp(1, limits.max_places_per_request);

    let backing = resolve_backing_collections(params.collections.as_deref(), model_config);
    if backing.len() > limits.max_collections_per_request {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Too many collections: {} exceeds limit of {}",
                backing.len(),
                limits.max_collections_per_request
            )),
        );
    }

    let places = match state
        .observation_catalog
        .get_populated_places_in_radius(lon, lat, radius_m, min_pop, limit)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("get_populated_places_in_radius failed: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to query places"),
            );
        }
    };

    let mut features = Vec::with_capacity(places.len());
    for place in &places {
        features.push(
            build_place_forecast_feature(
                &state,
                &config,
                place,
                &backing,
                target_time,
                &state.base_url,
            )
            .await,
        );
    }

    let fc = serde_json::json!({
        "type": "FeatureCollection",
        "numberReturned": features.len(),
        "min_population": min_pop,
        "features": features,
    });

    json_response(fc, "max-age=300")
}

/// Build a GeoJSON feature for a place with forecast values flattened into
/// properties (one nested object per backing collection).
async fn build_place_forecast_feature(
    state: &Arc<AppState>,
    config: &crate::config::EdrConfig,
    place: &storage::observations::Location,
    backing: &[String],
    target_time: Option<DateTime<Utc>>,
    _base_url: &str,
) -> serde_json::Value {
    let mut forecasts = serde_json::Map::new();
    for coll in backing {
        if let Some(data) =
            fetch_gridded_collection_data_at(state, config, coll, place.lon, place.lat, target_time)
                .await
        {
            forecasts.insert(
                coll.clone(),
                serde_json::to_value(&data).unwrap_or(serde_json::Value::Null),
            );
        }
    }

    let pop = place.properties.get("population").cloned();
    let st = place.region.clone().or_else(|| {
        place
            .properties
            .get("state")
            .and_then(|v| v.as_str().map(String::from))
    });

    serde_json::json!({
        "type": "Feature",
        "id": place.id,
        "geometry": { "type": "Point", "coordinates": [place.lon, place.lat] },
        "properties": {
            "name": place.name,
            "state": st,
            "population": pop,
            "forecasts": forecasts,
        },
    })
}

// ---- small helpers for the populated handlers ----

fn json_response(value: serde_json::Value, cache: &str) -> Response {
    let json = serde_json::to_string(&value).unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        .header(header::CACHE_CONTROL, cache)
        .body(json.into())
        .unwrap()
}

fn parse_bbox(s: &str) -> Result<(f64, f64, f64, f64), String> {
    let parts: Vec<f64> = s
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "bbox must be numeric: min_lon,min_lat,max_lon,max_lat".to_string())?;
    if parts.len() != 4 {
        return Err("bbox must have 4 comma-separated values".to_string());
    }
    Ok((parts[0], parts[1], parts[2], parts[3]))
}

fn parse_optional_datetime(dt: &Option<String>) -> Result<Option<DateTime<Utc>>, String> {
    match dt {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => DateTime::parse_from_rfc3339(s.trim())
            .map(|t| Some(t.with_timezone(&Utc)))
            .map_err(|_| format!("Invalid datetime '{}': expected ISO8601/RFC3339", s)),
    }
}

/// Parse a `within` distance (e.g. "100km", "50000m", or bare number with
/// `within-units`) into meters. Defaults to km when units are absent.
fn parse_within(within: &Option<String>, units: Option<&str>) -> Result<f64, String> {
    let raw = within
        .as_deref()
        .ok_or_else(|| "radius requires `within`".to_string())?
        .trim();

    // Split trailing unit letters from the number
    let (num_str, unit_str) = raw
        .find(|c: char| c.is_alphabetic())
        .map(|i| (&raw[..i], &raw[i..]))
        .unwrap_or((raw, ""));

    let value: f64 = num_str
        .trim()
        .parse()
        .map_err(|_| format!("Invalid radius value: {}", raw))?;

    let unit = if !unit_str.is_empty() {
        unit_str.trim()
    } else {
        units.unwrap_or("km")
    };

    let meters = match unit.to_lowercase().as_str() {
        "km" | "kilometers" | "kilometres" => value * 1000.0,
        "m" | "meters" | "metres" => value,
        "mi" | "miles" => value * 1609.344,
        other => return Err(format!("Unsupported radius unit: {}", other)),
    };
    Ok(meters)
}

/// Parse a WKT `POINT(lon lat)` into (lon, lat).
fn parse_wkt_point(coords: Option<&str>) -> Result<(f64, f64), String> {
    let s = coords
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .ok_or_else(|| "radius requires `coords=POINT(lon lat)`".to_string())?;

    let inner = s
        .trim()
        .strip_prefix("POINT")
        .or_else(|| s.strip_prefix("point"))
        .map(|r| r.trim())
        .and_then(|r| r.strip_prefix('('))
        .and_then(|r| r.strip_suffix(')'))
        .ok_or_else(|| format!("Invalid WKT point: {}", s))?;

    let nums: Vec<f64> = inner
        .split_whitespace()
        .map(|p| p.parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("Invalid WKT point coordinates: {}", s))?;

    if nums.len() != 2 {
        return Err(format!("POINT must have lon and lat: {}", s));
    }
    Ok((nums[0], nums[1]))
}

/// Sample a gridded collection's parameters at a point, optionally at a target
/// valid time (`None` = closest to now). Shared by the global location proxy
/// and the populated-places collection.
async fn fetch_gridded_collection_data_at(
    state: &Arc<AppState>,
    config: &crate::config::EdrConfig,
    collection_id: &str,
    lon: f64,
    lat: f64,
    target_time: Option<DateTime<Utc>>,
) -> Option<GriddedCollectionData> {
    // Find the collection definition
    let (model_config, collection_def) = config.find_collection(collection_id)?;

    let is_observation = model_config.data_type.is_observation();
    let model_name = &model_config.model;

    let mut parameters: std::collections::HashMap<String, GriddedParameterData> =
        std::collections::HashMap::new();
    let mut reference_time: Option<DateTime<Utc>> = None;
    let mut valid_time: Option<DateTime<Utc>> = None;
    let mut forecast_hour: Option<i32> = None;

    // Query each parameter in the collection
    for param_def in &collection_def.parameters {
        let param_name = &param_def.name;

        // Get all levels for this parameter at the target time (or closest to now)
        let entries = match (is_observation, target_time) {
            (true, Some(t)) => state
                .catalog
                .get_all_levels_observation_at_time(model_name, param_name, t)
                .await
                .ok()?,
            (true, None) => state
                .catalog
                .get_all_levels_observation_closest_to_now(model_name, param_name)
                .await
                .ok()?,
            (false, Some(t)) => state
                .catalog
                .get_all_levels_forecast_at_time(model_name, param_name, t)
                .await
                .ok()?,
            (false, None) => state
                .catalog
                .get_all_levels_forecast_closest_to_now(model_name, param_name)
                .await
                .ok()?,
        };

        if entries.is_empty() {
            continue;
        }

        // Use the first entry to set reference/valid times
        if reference_time.is_none() {
            let first = &entries[0];
            reference_time = Some(first.reference_time);
            forecast_hour = Some(first.forecast_hour as i32);
            // Calculate valid_time for forecast data
            valid_time =
                Some(first.reference_time + chrono::Duration::hours(first.forecast_hour as i64));
        }

        // Extract values at each level
        let mut level_values: std::collections::HashMap<String, Option<f32>> =
            std::collections::HashMap::new();
        let mut unit_str: Option<String> = None;

        for entry in &entries {
            // Build query for this specific entry
            let query = if is_observation {
                grid_processor::DatasetQuery::observation(model_name, param_name)
                    .at_level(&entry.level)
                    .at_time(entry.reference_time)
            } else {
                grid_processor::DatasetQuery::forecast(model_name, param_name)
                    .at_level(&entry.level)
                    .at_run(entry.reference_time)
                    .at_forecast_hour(entry.forecast_hour as u32)
            };

            // Read the point value
            match state.grid_data_service.read_point(&query, lon, lat).await {
                Ok(point_value) => {
                    level_values.insert(entry.level.clone(), point_value.value);
                    if unit_str.is_none() && !point_value.units.is_empty() {
                        unit_str = Some(point_value.units.clone());
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to read point value for {}/{} at level {}: {}",
                        model_name,
                        param_name,
                        entry.level,
                        e
                    );
                    level_values.insert(entry.level.clone(), None);
                }
            }
        }

        if !level_values.is_empty() {
            parameters.insert(
                param_name.clone(),
                GriddedParameterData {
                    unit: unit_str,
                    values: level_values,
                },
            );
        }
    }

    if parameters.is_empty() {
        return None;
    }

    Some(GriddedCollectionData {
        reference_time: reference_time?.to_rfc3339(),
        valid_time: valid_time?.to_rfc3339(),
        forecast_hour: if is_observation { None } else { forecast_hour },
        parameters,
    })
}

// TODO: Add pagination (limit, offset) for /edr/locations when location count grows large
// TODO: Add spatial filtering (bbox, within) to /edr/locations
// TODO: Cache collection availability per location with TTL
// TODO: Add gridded data point extraction at /edr/locations/{id}

#[cfg(test)]
mod tests {
    use edr_protocol::Location;

    #[test]
    fn test_parse_wkt_point() {
        use super::parse_wkt_point;
        assert_eq!(
            parse_wkt_point(Some("POINT(-86.8 33.5)")).unwrap(),
            (-86.8, 33.5)
        );
        assert_eq!(
            parse_wkt_point(Some("point(-100 40)")).unwrap(),
            (-100.0, 40.0)
        );
        assert!(parse_wkt_point(Some("POINT(-86.8)")).is_err());
        assert!(parse_wkt_point(Some("not wkt")).is_err());
        assert!(parse_wkt_point(None).is_err());
        assert!(parse_wkt_point(Some("")).is_err());
    }

    #[test]
    fn test_parse_within() {
        use super::parse_within;
        assert_eq!(
            parse_within(&Some("100km".to_string()), None).unwrap(),
            100_000.0
        );
        assert_eq!(
            parse_within(&Some("50000m".to_string()), None).unwrap(),
            50_000.0
        );
        assert_eq!(
            parse_within(&Some("50".to_string()), Some("km")).unwrap(),
            50_000.0
        );
        assert_eq!(
            parse_within(&Some("10".to_string()), Some("mi")).unwrap(),
            16093.44
        );
        assert!(parse_within(&None, None).is_err());
        assert!(parse_within(&Some("5furlongs".to_string()), None).is_err());
    }

    #[test]
    fn test_parse_bbox() {
        use super::parse_bbox;
        assert_eq!(
            parse_bbox("-100,30,-90,40").unwrap(),
            (-100.0, 30.0, -90.0, 40.0)
        );
        assert!(parse_bbox("-100,30,-90").is_err());
        assert!(parse_bbox("a,b,c,d").is_err());
    }

    #[test]
    fn test_parse_optional_datetime() {
        use super::parse_optional_datetime;
        assert!(parse_optional_datetime(&None).unwrap().is_none());
        assert!(parse_optional_datetime(&Some("".to_string()))
            .unwrap()
            .is_none());
        assert!(
            parse_optional_datetime(&Some("2026-07-07T12:00:00Z".to_string()))
                .unwrap()
                .is_some()
        );
        assert!(parse_optional_datetime(&Some("garbage".to_string())).is_err());
    }

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
