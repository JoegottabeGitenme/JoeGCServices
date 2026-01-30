//! Radius query handler.
//!
//! Returns data within a defined radius of a specified coordinate point.
//! Per OGC EDR spec, radius queries require:
//! - coords: POINT or MULTIPOINT (center of circle)
//! - within: radius value (e.g., "100")
//! - within-units: unit for radius (km, mi, m, nm)

use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Utc};
use edr_protocol::{
    coverage_json::CovJsonParameter, parameters::Unit, queries::DateTimeQuery,
    responses::ExceptionResponse, CoverageJson, DistanceUnit, EdrFeatureCollection, ParsedCoords,
    PositionQuery, RadiusQuery,
};
use grid_processor::BoundingBox;
use serde::Deserialize;
use std::sync::Arc;

use crate::availability::ModelAvailability;
use crate::config::build_level_string;
use crate::content_negotiation::{check_png_not_supported, negotiate_format, OutputFormat};
use crate::handlers::forecast_params::{validate_not_observation_data, ForecastParams};
use crate::handlers::observations::{obs_radius_query_handler, ObsRadiusQueryParams};
use crate::limits::ResponseSizeEstimate;
use crate::metrics::{
    extract_client_ip, extract_user_agent, format_from_output, EndpointType, Timer,
};
use crate::state::AppState;
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

/// Query parameters for radius endpoint.
#[derive(Debug, Deserialize)]
pub struct RadiusQueryParams {
    /// Coordinates as WKT POINT or MULTIPOINT. Required parameter.
    pub coords: Option<String>,

    /// Radius value. Required parameter.
    pub within: Option<String>,

    /// Distance units for the within parameter. Required parameter.
    #[serde(rename = "within-units")]
    pub within_units: Option<String>,

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
}

/// GET /edr/collections/:collection_id/radius
///
/// For point observation collections (METAR, TAF, etc.), dispatches to observation handler.
pub async fn radius_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<RadiusQueryParams>,
    headers: HeaderMap,
) -> Response {
    // Check if this is a point data collection (METAR observations or TAF forecasts)
    {
        let config = state.edr_config.read().await;
        if let Some((model_config, _)) = config.find_collection(&collection_id) {
            if model_config.data_type.is_point_data() {
                // Convert the coords to format observation handler expects
                let obs_params = ObsRadiusQueryParams {
                    coords: params.coords.clone().unwrap_or_default(),
                    within: params.within.clone(),
                    datetime: params.datetime.clone(),
                    parameter_name: params.parameter_name.clone(),
                    f: params.f.clone(),
                    limit: None,
                };
                return obs_radius_query_handler(
                    Extension(state.clone()),
                    Path(collection_id),
                    Query(obs_params),
                    headers,
                )
                .await;
            }
        }
    }

    // Use latest instance
    radius_query(state, collection_id, None, params, headers).await
}

/// GET /edr/collections/:collection_id/instances/:instance_id/radius
pub async fn instance_radius_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, instance_id)): Path<(String, String)>,
    Query(params): Query<RadiusQueryParams>,
    headers: HeaderMap,
) -> Response {
    radius_query(state, collection_id, Some(instance_id), params, headers).await
}

async fn radius_query(
    state: Arc<AppState>,
    collection_id: String,
    instance_id: Option<String>,
    params: RadiusQueryParams,
    headers: HeaderMap,
) -> Response {
    // Start timing and extract client info
    let timer = Timer::start();
    let client_ip = extract_client_ip(&headers);
    let user_agent = extract_user_agent(&headers);

    // Negotiate output format based on Accept header and f parameter
    let output_format = match negotiate_format(&headers, params.f.as_deref()) {
        Ok(format) => format,
        Err(response) => {
            return response;
        }
    };

    // PNG output is not supported for radius queries
    if let Some(response) = check_png_not_supported(output_format, "radius") {
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

    // Check for required within parameter
    let within_str = match &params.within {
        Some(w) if !w.trim().is_empty() => w.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("Missing required parameter: within"),
            );
        }
    };

    // Check for required within-units parameter
    let within_units_str = match &params.within_units {
        Some(u) if !u.trim().is_empty() => u.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("Missing required parameter: within-units"),
            );
        }
    };

    // Parse the radius value
    let within_value = match RadiusQuery::parse_within(within_str) {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid within parameter: {}", e)),
            );
        }
    };

    // Parse the distance units
    let distance_unit = match DistanceUnit::parse(within_units_str) {
        Ok(u) => u,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid within-units parameter: {}", e)),
            );
        }
    };

    // Parse coordinates - supports both POINT and MULTIPOINT
    let parsed_coords = match PositionQuery::parse_coords_multi(coords_str) {
        Ok(c) => c,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "Invalid coordinates: {}. Expected POINT or MULTIPOINT.",
                    e
                )),
            );
        }
    };

    // Extract center points for processing
    let center_points: Vec<(f64, f64)> = match &parsed_coords {
        ParsedCoords::Single(lon, lat) => vec![(*lon, *lat)],
        ParsedCoords::Multi(points) => points.clone(),
    };

    // Create RadiusQuery objects for each center point
    let radius_queries: Vec<RadiusQuery> = center_points
        .iter()
        .map(|(lon, lat)| RadiusQuery::new(*lon, *lat, within_value, distance_unit))
        .collect();

    // Check radius size limit
    let radius_km = distance_unit.to_kilometers(within_value);
    let max_radius_km = model_config.limits.max_radius_km.unwrap_or(500.0);
    if radius_km > max_radius_km {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            ExceptionResponse::payload_too_large(format!(
                "Radius too large: {:.2} km exceeds limit of {:.2} km",
                radius_km, max_radius_km
            )),
        );
    }

    // Parse vertical levels
    let z_values = if let Some(ref z) = params.z {
        match PositionQuery::parse_z(z) {
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
        .map(|p| PositionQuery::parse_parameter_names(p))
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

    // For radius query, we compute the union bounding box of all circles
    let union_bbox = compute_union_bbox(&radius_queries);

    // Get the list of times to query
    let time_strings: Vec<String> = if let Some(ref dq) = datetime_query {
        if dq.is_interval() {
            // Fetch available times from catalog to expand the interval
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

    // Conservative resolution estimate for grid points
    let resolution = 0.05;

    let estimate = ResponseSizeEstimate::for_radius(
        params_to_query.len(),
        num_times,
        num_levels,
        radius_km,
        resolution,
    );

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

    // For multi-z queries, we include all z values in the domain
    let is_multi_z = z_values.as_ref().map(|v| v.len() > 1).unwrap_or(false);
    let z_val = z_values.as_ref().and_then(|v| v.first().copied());

    // Parse time strings to DateTime<Utc>
    let parsed_times: Vec<DateTime<Utc>> = time_strings
        .iter()
        .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .collect();

    // Use the first time for the query (or None for latest)
    let query_time = parsed_times.first().copied();

    // Query the grid data for the first parameter to get grid coordinates
    let first_param = match params_to_query.first() {
        Some(p) => p,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("No parameters specified"),
            );
        }
    };

    // Find the parameter definition
    let param_def = collection_def
        .parameters
        .iter()
        .find(|p| p.name == *first_param);

    // Build the level string
    let level_str = build_level_string(&collection_def.level_filter, param_def, z_val);

    // Build the DatasetQuery using the appropriate type (forecast vs observation)
    let mut query = model_config.create_query(first_param);

    if let Some(level) = &level_str {
        query = query.at_level(level);
    }

    if let Some(valid_time) = query_time {
        query = query.at_valid_time(valid_time);
    }

    if let Some(ref_time) = reference_time {
        query = query.at_run(ref_time);
    }

    // Read the region using the union bounding box
    let grid_bbox = BoundingBox::new(
        union_bbox.west,
        union_bbox.south,
        union_bbox.east,
        union_bbox.north,
    );

    let region = match state
        .grid_data_service
        .read_region(&query, &grid_bbox, None)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to read region: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error(format!("Failed to read data: {}", e)),
            );
        }
    };

    // Build x and y coordinate arrays from the grid metadata
    let x_values: Vec<f64> = (0..region.width)
        .map(|i| region.bbox.min_lon + (i as f64 + 0.5) * region.resolution.0)
        .collect();
    let y_values: Vec<f64> = (0..region.height)
        .map(|j| region.bbox.max_lat - (j as f64 + 0.5) * region.resolution.1)
        .collect();

    // Build the time axis
    let t_values = if !time_strings.is_empty() {
        Some(time_strings.clone())
    } else {
        None
    };

    // Build z axis - include all requested z values
    let z_axis = if is_multi_z {
        z_values.clone()
    } else {
        z_val.map(|z| vec![z])
    };

    // Create CoverageJSON with Grid domain
    let mut coverage = CoverageJson {
        type_: edr_protocol::coverage_json::CoverageType::Coverage,
        domain: edr_protocol::Domain::grid(x_values.clone(), y_values.clone(), t_values, z_axis),
        parameters: Some(std::collections::HashMap::new()),
        ranges: Some(std::collections::HashMap::new()),
    };

    // For each parameter, query the data and add to coverage
    for param_name in &params_to_query {
        // Find the parameter definition
        let param_def = collection_def
            .parameters
            .iter()
            .find(|p| p.name == *param_name);

        // Build the level string
        let level_str = build_level_string(&collection_def.level_filter, param_def, z_val);

        // Build the DatasetQuery using the appropriate type (forecast vs observation)
        let mut query = model_config.create_query(param_name);

        if let Some(level) = &level_str {
            query = query.at_level(level);
        }

        if let Some(valid_time) = query_time {
            query = query.at_valid_time(valid_time);
        }

        if let Some(ref_time) = reference_time {
            query = query.at_run(ref_time);
        }

        // Get metadata for units
        let metadata = state.grid_data_service.get_metadata(&query).await.ok();
        let units_str = metadata
            .as_ref()
            .map(|m| m.units.clone())
            .unwrap_or_default();

        // Read the region for this parameter
        match state
            .grid_data_service
            .read_region(&query, &grid_bbox, None)
            .await
        {
            Ok(param_region) => {
                // Apply radius mask - set values outside all circles to null
                // Uses Haversine distance for accurate distance calculation
                let mut values: Vec<Option<f32>> = Vec::with_capacity(param_region.data.len());

                for (idx, &value) in param_region.data.iter().enumerate() {
                    let row = idx / param_region.width;
                    let col = idx % param_region.width;

                    // Calculate lon/lat for this grid cell
                    let lon =
                        param_region.bbox.min_lon + (col as f64 + 0.5) * param_region.resolution.0;
                    let lat =
                        param_region.bbox.max_lat - (row as f64 + 0.5) * param_region.resolution.1;

                    // Check if point is inside any of the radius circles (union)
                    let inside_any = radius_queries.iter().any(|rq| rq.contains_point(lon, lat));
                    if inside_any {
                        if value.is_nan() {
                            values.push(None);
                        } else {
                            values.push(Some(value));
                        }
                    } else {
                        values.push(None);
                    }
                }

                let unit = Unit::from_symbol(&units_str);
                let cov_param = CovJsonParameter::new(param_name).with_unit(unit);

                // Add the parameter and data
                let shape = vec![y_values.len(), x_values.len()];
                let axis_names = vec!["y".to_string(), "x".to_string()];

                coverage = coverage.with_parameter_array_nullable(
                    param_name, cov_param, values, shape, axis_names,
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to query {}/{}: {}",
                    model_config.model,
                    param_name,
                    e
                );
                // Add parameter with null values
                let cov_param = CovJsonParameter::new(param_name);
                let null_values: Vec<Option<f32>> = vec![None; y_values.len() * x_values.len()];
                let shape = vec![y_values.len(), x_values.len()];
                let axis_names = vec!["y".to_string(), "x".to_string()];

                coverage = coverage.with_parameter_array_nullable(
                    param_name,
                    cov_param,
                    null_values,
                    shape,
                    axis_names,
                );
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
            EndpointType::Radius,
            Some(&collection_id),
            &params_to_query,
            format_from_output(&output_format),
            timer.elapsed_us(),
            true,
            client_ip.as_deref(),
            user_agent.as_deref(),
        )
        .await;

    // Record geographic extent for heatmap (use union bbox of all radius queries)
    let union_bbox = compute_union_bbox(&radius_queries);
    state
        .metrics
        .record_query_extent(
            union_bbox.west,
            union_bbox.south,
            union_bbox.east,
            union_bbox.north,
            "radius",
            &collection_id,
        )
        .await;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "max-age=300")
        .body(json.into())
        .unwrap()
}

/// Compute the union bounding box for multiple radius queries.
fn compute_union_bbox(radius_queries: &[RadiusQuery]) -> edr_protocol::BboxQuery {
    let mut west = f64::MAX;
    let mut south = f64::MAX;
    let mut east = f64::MIN;
    let mut north = f64::MIN;

    for rq in radius_queries {
        let bbox = rq.bounding_box();
        west = west.min(bbox.west);
        south = south.min(bbox.south);
        east = east.max(bbox.east);
        north = north.max(bbox.north);
    }

    edr_protocol::BboxQuery {
        west,
        south,
        east,
        north,
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
    use super::*;

    #[test]
    fn test_distance_unit_parsing() {
        assert!(DistanceUnit::parse("km").is_ok());
        assert!(DistanceUnit::parse("mi").is_ok());
        assert!(DistanceUnit::parse("m").is_ok());
        assert!(DistanceUnit::parse("nm").is_ok());
        assert!(DistanceUnit::parse("invalid").is_err());
    }

    #[test]
    fn test_radius_query_creation() {
        let rq = RadiusQuery::new(-97.5, 35.2, 100.0, DistanceUnit::Kilometers);
        assert_eq!(rq.center_lon, -97.5);
        assert_eq!(rq.center_lat, 35.2);
        assert!((rq.radius_km() - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_union_bbox() {
        let queries = vec![
            RadiusQuery::new(-100.0, 35.0, 50.0, DistanceUnit::Kilometers),
            RadiusQuery::new(-95.0, 40.0, 50.0, DistanceUnit::Kilometers),
        ];

        let bbox = compute_union_bbox(&queries);

        // The union should encompass both circles
        assert!(bbox.west < -100.0);
        assert!(bbox.east > -95.0);
        assert!(bbox.south < 35.0);
        assert!(bbox.north > 40.0);
    }

    #[test]
    fn test_radius_contains_point() {
        let rq = RadiusQuery::new(-97.5, 35.5, 100.0, DistanceUnit::Kilometers);

        // Center should be inside
        assert!(rq.contains_point(-97.5, 35.5));

        // Point far away should be outside
        assert!(!rq.contains_point(-90.0, 35.5));
    }
}
