//! Position query handler.

use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Utc};
use edr_protocol::{
    coverage_json::CovJsonParameter,
    parameters::Unit,
    queries::{DateTimeQuery, TemporalInterpolationMethod},
    responses::ExceptionResponse,
    CoverageCollection, CoverageJson, EdrFeatureCollection, ParsedCoords,
    PositionQuery as ParsedPositionQuery,
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};

use crate::availability::ModelAvailability;
use crate::config::build_level_string;
use crate::content_negotiation::{check_png_not_supported, negotiate_format, OutputFormat};
use crate::limits::ResponseSizeEstimate;
use crate::metrics::{
    extract_client_ip, extract_user_agent, format_from_output, EndpointType, FormatType, Timer,
};
use crate::state::AppState;
use crate::temporal_interpolation::{
    calculate_weight, expand_interval_with_step, find_bracketing_times, linear_interpolate_f32,
    parse_iso8601_duration,
};
use crate::validation::validate_z_against_vertical_extent;

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

/// Query parameters for position endpoint.
#[derive(Debug, Deserialize)]
pub struct PositionQueryParams {
    /// Coordinates as WKT POINT or lon,lat. Required parameter.
    pub coords: Option<String>,

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

    /// Temporal interpolation method (none, nearest, linear).
    /// Defaults to "none" (no interpolation).
    pub interpolation: Option<String>,

    /// Step interval for generating times within a datetime range.
    /// Specified as an ISO 8601 duration (e.g., PT10M for 10 minutes, PT1H for 1 hour).
    /// Only used when datetime is an interval and interpolation is enabled.
    pub step: Option<String>,
}

/// GET /edr/collections/:collection_id/position
pub async fn position_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<PositionQueryParams>,
    headers: HeaderMap,
) -> Response {
    // Use latest instance
    position_query(state, collection_id, None, params, headers).await
}

/// GET /edr/collections/:collection_id/instances/:instance_id/position
pub async fn instance_position_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, instance_id)): Path<(String, String)>,
    Query(params): Query<PositionQueryParams>,
    headers: HeaderMap,
) -> Response {
    position_query(state, collection_id, Some(instance_id), params, headers).await
}

async fn position_query(
    state: Arc<AppState>,
    collection_id: String,
    instance_id: Option<String>,
    params: PositionQueryParams,
    headers: HeaderMap,
) -> Response {
    // Start timing
    let timer = Timer::start();
    let client_ip = extract_client_ip(&headers);
    let user_agent = extract_user_agent(&headers);

    // Debug: log the Accept header
    if let Some(accept) = headers.get(header::ACCEPT) {
        tracing::debug!("Position query Accept header: {:?}", accept);
    } else {
        tracing::debug!("Position query: No Accept header present");
    }

    // Negotiate output format based on Accept header and f parameter
    let output_format = match negotiate_format(&headers, params.f.as_deref()) {
        Ok(format) => format,
        Err(response) => {
            tracing::debug!("Format negotiation failed");
            // Record error metrics
            state
                .metrics
                .record_request(
                    EndpointType::Position,
                    Some(&collection_id),
                    &[],
                    FormatType::CoverageJson,
                    timer.elapsed_us(),
                    false,
                    client_ip.as_deref(),
                    user_agent.as_deref(),
                )
                .await;
            return response;
        }
    };

    // PNG output is not supported for position queries
    if let Some(response) = check_png_not_supported(output_format, "position") {
        return response;
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

    // Return 404 if model has no data
    let Some(availability) = availability else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!(
                "Collection {} has no available data",
                collection_id
            )),
        );
    };

    // Get available parameters (filter to only those with data)
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

    // Check for required coords parameter
    let coords_str = match &params.coords {
        Some(c) if !c.trim().is_empty() => c.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("Missing required parameter: coords"),
            );
        }
    };

    // Parse coordinates - supports both POINT and MULTIPOINT
    let parsed_coords = match ParsedPositionQuery::parse_coords_multi(coords_str) {
        Ok(coords) => coords,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid coordinates: {}", e)),
            );
        }
    };

    // Extract points - single point or multiple points
    let points: Vec<(f64, f64)> = match &parsed_coords {
        ParsedCoords::Single(lon, lat) => vec![(*lon, *lat)],
        ParsedCoords::Multi(pts) => {
            if pts.is_empty() {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request("MULTIPOINT must contain at least one point"),
                );
            }
            pts.clone()
        }
    };

    let is_multipoint = points.len() > 1;
    let (lon, lat) = points[0]; // Use first point for single-point calculations

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

    // Parse datetime - now supports lists and intervals
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

    // Parse interpolation method
    let interpolation_method = if let Some(ref interp_str) = params.interpolation {
        match TemporalInterpolationMethod::parse(interp_str) {
            Ok(method) => method,
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!(
                        "Invalid interpolation parameter: {}",
                        e
                    )),
                );
            }
        }
    } else {
        TemporalInterpolationMethod::None
    };

    // Parse step duration if provided
    let step_duration = if let Some(ref step_str) = params.step {
        match parse_iso8601_duration(step_str) {
            Some(duration) => Some(duration),
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!(
                        "Invalid step parameter '{}'. Expected ISO 8601 duration (e.g., PT10M, PT1H)",
                        step_str
                    )),
                );
            }
        }
    } else {
        None
    };

    // Fetch available times and temporal extent from catalog
    let model_name = &model_config.model;
    let available_time_strings: Vec<String> = state
        .catalog
        .get_model_valid_times(model_name)
        .await
        .ok()
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .collect();

    let available_times: Vec<DateTime<Utc>> = available_time_strings
        .iter()
        .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .collect();

    // Get temporal extent from catalog for validation
    let temporal_extent = state
        .catalog
        .get_model_temporal_extent(model_name)
        .await
        .ok()
        .flatten();

    // Get the list of target times (times user wants in the response)
    let mut target_times: Vec<DateTime<Utc>> = Vec::new();

    if let Some(ref dq) = datetime_query {
        // Check if we need to generate times with step parameter
        if dq.is_interval()
            && step_duration.is_some()
            && interpolation_method.requires_interpolation()
        {
            // Generate times at step intervals within the datetime range
            let interval_bounds = dq.to_vec();
            if interval_bounds.len() == 2 {
                let start = DateTime::parse_from_rfc3339(&interval_bounds[0])
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok();
                let end = DateTime::parse_from_rfc3339(&interval_bounds[1])
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok();

                if let (Some(start), Some(end)) = (start, end) {
                    // Validate times are within temporal extent
                    if let Some((extent_start, extent_end)) = temporal_extent {
                        if start < extent_start || end > extent_end {
                            return error_response(
                                StatusCode::BAD_REQUEST,
                                ExceptionResponse::bad_request(format!(
                                    "Requested datetime range {}/{} exceeds collection temporal extent {}/{}",
                                    interval_bounds[0], interval_bounds[1],
                                    extent_start.format("%Y-%m-%dT%H:%M:%SZ"),
                                    extent_end.format("%Y-%m-%dT%H:%M:%SZ")
                                )),
                            );
                        }
                    }

                    target_times = expand_interval_with_step(start, end, step_duration.unwrap());
                } else {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        ExceptionResponse::bad_request("Invalid datetime interval format"),
                    );
                }
            } else {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(
                        "Step parameter requires a datetime interval with start and end",
                    ),
                );
            }
        } else if dq.is_interval() {
            // Standard interval expansion without step
            let expanded = dq.expand_against_available_times(&available_time_strings);
            target_times = expanded
                .iter()
                .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .collect();
        } else {
            // Instant or list - parse directly
            let time_vec = dq.to_vec();
            target_times = time_vec
                .iter()
                .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .collect();
        }
    }

    // Build interpolation plan: map each target time to how it should be queried
    // Each entry: (target_time, query_strategy)
    enum QueryStrategy {
        Exact(DateTime<Utc>),                           // Query exact time
        Interpolate(DateTime<Utc>, DateTime<Utc>, f64), // (before, after, weight)
        Nearest(DateTime<Utc>),                         // Use nearest time
    }

    let mut query_plan: Vec<(DateTime<Utc>, QueryStrategy)> = Vec::new();

    for target_time in &target_times {
        if available_times.contains(target_time) {
            // Exact match - no interpolation needed
            query_plan.push((*target_time, QueryStrategy::Exact(*target_time)));
        } else if interpolation_method.requires_interpolation() {
            // Need interpolation
            let (before, after) = find_bracketing_times(*target_time, &available_times);

            match interpolation_method {
                TemporalInterpolationMethod::Linear => {
                    if let (Some(before_time), Some(after_time)) = (before, after) {
                        let weight = calculate_weight(before_time, after_time, *target_time);
                        query_plan.push((
                            *target_time,
                            QueryStrategy::Interpolate(before_time, after_time, weight),
                        ));
                    } else if let Some(nearest) = before.or(after) {
                        // Edge case: target outside range, use nearest
                        query_plan.push((*target_time, QueryStrategy::Nearest(nearest)));
                    }
                }
                TemporalInterpolationMethod::Nearest => {
                    // Find nearest time
                    let nearest = if let (Some(before_time), Some(after_time)) = (before, after) {
                        let dist_before = (*target_time - before_time).num_seconds().abs();
                        let dist_after = (after_time - *target_time).num_seconds().abs();
                        if dist_before <= dist_after {
                            before_time
                        } else {
                            after_time
                        }
                    } else {
                        before.or(after).unwrap_or(available_times[0])
                    };
                    query_plan.push((*target_time, QueryStrategy::Nearest(nearest)));
                }
                TemporalInterpolationMethod::None => {
                    // Should not reach here
                    query_plan.push((*target_time, QueryStrategy::Exact(*target_time)));
                }
            }
        } else {
            // No interpolation, but time doesn't exist - skip it
            // (This matches current behavior where non-existent times are silently dropped)
        }
    }

    // Legacy time_strings for compatibility with existing code
    let time_strings: Vec<String> = target_times
        .iter()
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .collect();

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

    // Parse instance_id if provided and validate it exists
    let reference_time = if let Some(ref id) = instance_id {
        match chrono::DateTime::parse_from_rfc3339(id) {
            Ok(dt) => {
                let ref_time = dt.with_timezone(&chrono::Utc);

                // Validate that this instance actually exists
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
                        // Continue anyway - the query will fail if instance doesn't exist
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

    // Determine if this is a multi-z query (VerticalProfile)
    let is_multi_z = z_values.as_ref().map(|v| v.len() > 1).unwrap_or(false);
    let z_val = z_values.as_ref().and_then(|v| v.first().copied());

    // Determine if this is a multi-time query (PointSeries) or single time (Point)
    let is_multi_time = datetime_query
        .as_ref()
        .map(|dq| dq.is_multi_time())
        .unwrap_or(false);

    // Parse time strings to DateTime<Utc>
    let parsed_times: Vec<DateTime<Utc>> = time_strings
        .iter()
        .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .collect();

    // Handle MULTIPOINT - return CoverageCollection with one Coverage per point
    if is_multipoint {
        let mut collection = CoverageCollection::new();
        let datetime_str = time_strings.first().cloned();

        for (pt_lon, pt_lat) in &points {
            let mut point_coverage =
                CoverageJson::point(*pt_lon, *pt_lat, datetime_str.clone(), z_val);

            // Query each parameter for this point
            for param_name in &params_to_query {
                let param_def = collection_def
                    .parameters
                    .iter()
                    .find(|p| p.name == *param_name);

                let level_str = build_level_string(&collection_def.level_filter, param_def, z_val);

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

                match state
                    .grid_data_service
                    .read_point(&query, *pt_lon, *pt_lat)
                    .await
                {
                    Ok(point_value) => {
                        let unit = Unit::from_symbol(&point_value.units);
                        let cov_param = CovJsonParameter::new(param_name).with_unit(unit);

                        if let Some(val) = point_value.value {
                            point_coverage =
                                point_coverage.with_parameter(param_name, cov_param, val);
                        } else {
                            point_coverage =
                                point_coverage.with_parameter_null(param_name, cov_param);
                        }
                    }
                    Err(_) => {
                        let cov_param = CovJsonParameter::new(param_name);
                        point_coverage = point_coverage.with_parameter_null(param_name, cov_param);
                    }
                }
            }

            collection = collection.with_coverage(point_coverage);
        }

        // Serialize based on requested format
        let (json, content_type) = match output_format {
            OutputFormat::GeoJson => {
                let geojson = EdrFeatureCollection::from(&collection);
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
            OutputFormat::CoverageJson => match serde_json::to_string_pretty(&collection) {
                Ok(j) => (j, output_format.content_type()),
                Err(e) => {
                    tracing::error!("Failed to serialize CoverageCollection: {}", e);
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

        // Record successful MULTIPOINT request metrics
        state
            .metrics
            .record_request(
                EndpointType::Position,
                Some(&collection_id),
                &params_to_query,
                format_from_output(&output_format),
                timer.elapsed_us(),
                true,
                client_ip.as_deref(),
                user_agent.as_deref(),
            )
            .await;

        // Record geographic locations for each point
        for (pt_lon, pt_lat) in &points {
            state
                .metrics
                .record_point_query(*pt_lon, *pt_lat, &collection_id)
                .await;
        }

        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, "max-age=300")
            .body(json.into())
            .unwrap();
    }

    // Build CoverageJSON response - Point, PointSeries, or VerticalProfile
    let mut coverage = if is_multi_z {
        // VerticalProfile: multiple z levels at a single point
        let datetime_str = time_strings.first().cloned();
        CoverageJson::vertical_profile(lon, lat, datetime_str, z_values.clone().unwrap_or_default())
    } else if is_multi_time && !time_strings.is_empty() {
        CoverageJson::point_series(lon, lat, time_strings.clone(), z_val)
    } else {
        let datetime_str = time_strings.first().cloned();
        CoverageJson::point(lon, lat, datetime_str, z_val)
    };

    // For each parameter, query the data
    for param_name in &params_to_query {
        // Find the parameter definition in the collection to get level info
        let param_def = collection_def
            .parameters
            .iter()
            .find(|p| p.name == *param_name);

        // Handle multi-z queries (VerticalProfile)
        if is_multi_z {
            let z_vals = z_values.as_ref().unwrap();
            let mut values: Vec<Option<f32>> = Vec::with_capacity(z_vals.len());
            let mut units_str = String::new();

            for z in z_vals {
                // Build the level string for this z value
                let level_str =
                    build_level_string(&collection_def.level_filter, param_def, Some(*z));

                // Build the DatasetQuery using the appropriate type (forecast vs observation)
                let mut query = model_config.create_query(param_name);

                if let Some(level) = &level_str {
                    query = query.at_level(level);
                }

                // Use the first parsed time if available
                if let Some(valid_time) = parsed_times.first() {
                    query = query.at_valid_time(*valid_time);
                }

                // Use the reference time if provided (instance query)
                if let Some(ref_time) = reference_time {
                    query = query.at_run(ref_time);
                }

                // Query the data for this z level
                match state.grid_data_service.read_point(&query, lon, lat).await {
                    Ok(point_value) => {
                        if units_str.is_empty() {
                            units_str = point_value.units.clone();
                        }
                        values.push(point_value.value);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to query {}/{} at ({}, {}) for z={}: {}",
                            model_config.model,
                            param_name,
                            lon,
                            lat,
                            z,
                            e
                        );
                        values.push(None);
                    }
                }
            }

            // Add the vertical profile data
            let unit = Unit::from_symbol(&units_str);
            let cov_param = CovJsonParameter::new(param_name).with_unit(unit);
            coverage = coverage.with_vertical_profile_data(param_name, cov_param, values);
            continue;
        }

        // Build the level string for catalog lookup
        let level_str = build_level_string(&collection_def.level_filter, param_def, z_val);

        if is_multi_time && !parsed_times.is_empty() {
            // Multi-time query: query each time and build an array
            let mut values: Vec<Option<f32>> = Vec::with_capacity(query_plan.len());
            let mut units_str = String::new();

            // Cache for queried values to avoid duplicate queries when interpolating
            let mut value_cache: HashMap<DateTime<Utc>, Option<f32>> = HashMap::new();

            for (_target_time, strategy) in &query_plan {
                let interpolated_value: Option<f32> = match strategy {
                    QueryStrategy::Exact(query_time) | QueryStrategy::Nearest(query_time) => {
                        // Check cache first
                        if let Some(cached_val) = value_cache.get(query_time) {
                            *cached_val
                        } else {
                            // Build the DatasetQuery
                            let mut query = model_config
                                .create_query(param_name)
                                .at_valid_time(*query_time);

                            if let Some(level) = &level_str {
                                query = query.at_level(level);
                            }

                            if let Some(ref_time) = reference_time {
                                query = query.at_run(ref_time);
                            }

                            // Query the data
                            let result =
                                match state.grid_data_service.read_point(&query, lon, lat).await {
                                    Ok(point_value) => {
                                        if units_str.is_empty() {
                                            units_str = point_value.units.clone();
                                        }
                                        point_value.value
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to query {}/{} at ({}, {}) for time {}: {}",
                                            model_config.model,
                                            param_name,
                                            lon,
                                            lat,
                                            query_time,
                                            e
                                        );
                                        None
                                    }
                                };

                            // Cache the result
                            value_cache.insert(*query_time, result);
                            result
                        }
                    }
                    QueryStrategy::Interpolate(before_time, after_time, weight) => {
                        // Query before and after times
                        let before_val = if let Some(cached) = value_cache.get(before_time) {
                            *cached
                        } else {
                            let mut query = model_config
                                .create_query(param_name)
                                .at_valid_time(*before_time);

                            if let Some(level) = &level_str {
                                query = query.at_level(level);
                            }

                            if let Some(ref_time) = reference_time {
                                query = query.at_run(ref_time);
                            }

                            let result =
                                match state.grid_data_service.read_point(&query, lon, lat).await {
                                    Ok(point_value) => {
                                        if units_str.is_empty() {
                                            units_str = point_value.units.clone();
                                        }
                                        point_value.value
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to query {}/{} at ({}, {}) for time {}: {}",
                                            model_config.model,
                                            param_name,
                                            lon,
                                            lat,
                                            before_time,
                                            e
                                        );
                                        None
                                    }
                                };

                            value_cache.insert(*before_time, result);
                            result
                        };

                        let after_val = if let Some(cached) = value_cache.get(after_time) {
                            *cached
                        } else {
                            let mut query = model_config
                                .create_query(param_name)
                                .at_valid_time(*after_time);

                            if let Some(level) = &level_str {
                                query = query.at_level(level);
                            }

                            if let Some(ref_time) = reference_time {
                                query = query.at_run(ref_time);
                            }

                            let result =
                                match state.grid_data_service.read_point(&query, lon, lat).await {
                                    Ok(point_value) => {
                                        if units_str.is_empty() {
                                            units_str = point_value.units.clone();
                                        }
                                        point_value.value
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to query {}/{} at ({}, {}) for time {}: {}",
                                            model_config.model,
                                            param_name,
                                            lon,
                                            lat,
                                            after_time,
                                            e
                                        );
                                        None
                                    }
                                };

                            value_cache.insert(*after_time, result);
                            result
                        };

                        // Perform linear interpolation
                        match (before_val, after_val) {
                            (Some(b), Some(a)) => Some(linear_interpolate_f32(b, a, *weight)),
                            (Some(b), None) => Some(b), // Fall back to before value
                            (None, Some(a)) => Some(a), // Fall back to after value
                            (None, None) => None,       // No data available
                        }
                    }
                };

                values.push(interpolated_value);
            }

            // Add the time series data
            let unit = Unit::from_symbol(&units_str);
            let cov_param = CovJsonParameter::new(param_name).with_unit(unit);
            coverage = coverage.with_time_series(param_name, cov_param, values);
        } else {
            // Single time query (original behavior)
            // Build the DatasetQuery using the appropriate type (forecast vs observation)
            let mut query = model_config.create_query(param_name);

            if let Some(level) = &level_str {
                query = query.at_level(level);
            }

            // If we have a single parsed time, use it
            if let Some(valid_time) = parsed_times.first() {
                query = query.at_valid_time(*valid_time);
            }

            // Use the reference time if provided
            if let Some(ref_time) = reference_time {
                query = query.at_run(ref_time);
            }

            // Query the actual data
            match state.grid_data_service.read_point(&query, lon, lat).await {
                Ok(point_value) => {
                    let unit = Unit::from_symbol(&point_value.units);
                    let cov_param = CovJsonParameter::new(param_name).with_unit(unit);

                    if let Some(val) = point_value.value {
                        coverage = coverage.with_parameter(param_name, cov_param, val);
                    } else {
                        // No data at this point (outside grid or fill value)
                        tracing::debug!(
                            "No data value at ({}, {}) for {}/{}",
                            lon,
                            lat,
                            model_config.model,
                            param_name
                        );
                        let cov_param = CovJsonParameter::new(param_name)
                            .with_unit(Unit::from_symbol(&point_value.units));
                        coverage = coverage.with_parameter_null(param_name, cov_param);
                    }
                }
                Err(e) => {
                    // Log the error but continue with other parameters
                    tracing::warn!(
                        "Failed to query {}/{} at ({}, {}): {}",
                        model_config.model,
                        param_name,
                        lon,
                        lat,
                        e
                    );
                    // Add parameter with null value
                    let cov_param = CovJsonParameter::new(param_name);
                    coverage = coverage.with_parameter_null(param_name, cov_param);
                }
            }
        }
    }

    // Serialize response based on requested format
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

    // Record successful request metrics
    state
        .metrics
        .record_request(
            EndpointType::Position,
            Some(&collection_id),
            &params_to_query,
            format_from_output(&output_format),
            timer.elapsed_us(),
            true,
            client_ip.as_deref(),
            user_agent.as_deref(),
        )
        .await;

    // Record geographic location for heatmap
    state
        .metrics
        .record_point_query(lon, lat, &collection_id)
        .await;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "max-age=300")
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

#[cfg(test)]
mod tests {
    use super::*;
    use edr_protocol::PositionQuery;

    #[test]
    fn test_parse_wkt_point() {
        let (lon, lat) = PositionQuery::parse_coords("POINT(-97.5 35.2)").unwrap();
        assert_eq!(lon, -97.5);
        assert_eq!(lat, 35.2);
    }

    #[test]
    fn test_parse_simple_coords() {
        let (lon, lat) = PositionQuery::parse_coords("-97.5,35.2").unwrap();
        assert_eq!(lon, -97.5);
        assert_eq!(lat, 35.2);
    }

    #[test]
    fn test_parse_invalid_coords() {
        let result = PositionQuery::parse_coords("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_z_single() {
        let z = PositionQuery::parse_z("850").unwrap();
        assert_eq!(z, vec![850.0]);
    }

    #[test]
    fn test_parse_z_multiple() {
        let z = PositionQuery::parse_z("850,700,500").unwrap();
        assert_eq!(z, vec![850.0, 700.0, 500.0]);
    }

    #[test]
    fn test_parse_datetime() {
        let dt = DateTimeQuery::parse("2024-12-29T12:00:00Z").unwrap();
        assert!(matches!(dt, DateTimeQuery::Instant(_)));
    }

    #[test]
    fn test_parse_datetime_interval() {
        let dt = DateTimeQuery::parse("2024-12-29T00:00:00Z/2024-12-29T23:59:59Z").unwrap();
        assert!(matches!(dt, DateTimeQuery::Interval { .. }));
    }

    #[test]
    fn test_parse_datetime_list() {
        let dt =
            DateTimeQuery::parse("2024-12-29T12:00:00Z,2024-12-29T13:00:00Z,2024-12-29T14:00:00Z")
                .unwrap();
        assert!(matches!(dt, DateTimeQuery::List(_)));
        assert!(dt.is_multi_time());
        assert_eq!(dt.len(), 3);
    }

    #[test]
    fn test_coverage_json_creation() {
        let coverage = CoverageJson::point(
            -97.5,
            35.2,
            Some("2024-12-29T12:00:00Z".to_string()),
            Some(2.0),
        );

        let json = serde_json::to_string(&coverage).unwrap();
        assert!(json.contains("\"type\":\"Coverage\""));
        assert!(json.contains("\"domainType\":\"Point\""));
    }

    #[test]
    fn test_point_series_coverage_json_creation() {
        let times = vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
            "2024-12-29T14:00:00Z".to_string(),
        ];

        let coverage = CoverageJson::point_series(-97.5, 35.2, times, Some(2.0));

        let json = serde_json::to_string(&coverage).unwrap();
        assert!(json.contains("\"type\":\"Coverage\""));
        assert!(json.contains("\"domainType\":\"PointSeries\""));
    }

    #[test]
    fn test_datetime_to_vec() {
        let dt =
            DateTimeQuery::parse("2024-12-29T12:00:00Z,2024-12-29T13:00:00Z,2024-12-29T14:00:00Z")
                .unwrap();

        let times = dt.to_vec();
        assert_eq!(times.len(), 3);
        assert_eq!(times[0], "2024-12-29T12:00:00Z");
        assert_eq!(times[1], "2024-12-29T13:00:00Z");
        assert_eq!(times[2], "2024-12-29T14:00:00Z");
    }
}
