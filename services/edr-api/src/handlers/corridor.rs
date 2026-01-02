//! Corridor query handler.
//!
//! Returns data along and around a path defined by the coords parameter.
//! A corridor is a volumetric region around a trajectory path with specified
//! width (horizontal) and optional height (vertical) dimensions.
//!
//! The response is a CoverageCollection containing multiple trajectories:
//! - The centerline trajectory
//! - Offset trajectories on either side (perpendicular to the path direction)
//!
//! Per OGC EDR spec, corridor queries require:
//! - coords: LINESTRING, LINESTRINGZ, LINESTRINGM, LINESTRINGZM or MULTI* variants
//! - corridor-width: Total width of corridor (trajectory is center)
//! - width-units: Units for width (km, mi, m, nm)
//!
//! Optional parameters:
//! - corridor-height: Total height of corridor (defaults to 0 for 2D corridor)
//! - height-units: Units for height (defaults to "m")
//!
//! The Z coordinate (in LINESTRINGZ/LINESTRINGZM) represents height.
//! The M coordinate (in LINESTRINGM/LINESTRINGZM) represents Unix epoch time.
//!
//! Error conditions (HTTP 400):
//! - Missing required parameters (coords, corridor-width, width-units)
//! - coords=LINESTRINGZ with z parameter (conflict)
//! - coords=LINESTRINGM with datetime parameter (conflict)
//! - coords=LINESTRINGZM with z or datetime parameter (conflict)
//! - Invalid unit values not in collection metadata

use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, TimeZone, Utc};
use edr_protocol::{
    coverage_json::{CovJsonParameter, CoverageCollection},
    parameters::Unit,
    queries::DateTimeQuery,
    responses::ExceptionResponse,
    CoverageJson, DistanceUnit, EdrFeatureCollection, PositionQuery, TrajectoryQuery, VerticalUnit,
};
use grid_processor::DatasetQuery;
use serde::Deserialize;
use std::sync::Arc;

use crate::config::LevelValue;
use crate::content_negotiation::{negotiate_format, OutputFormat};
use crate::limits::ResponseSizeEstimate;
use crate::state::AppState;

/// Query parameters for corridor endpoint.
#[derive(Debug, Deserialize)]
pub struct CorridorQueryParams {
    /// Coordinates as WKT LINESTRING or MULTILINESTRING. Required parameter.
    pub coords: Option<String>,

    /// Corridor width value. Required parameter.
    #[serde(rename = "corridor-width")]
    pub corridor_width: Option<String>,

    /// Units for corridor width (km, mi, m, nm). Required parameter.
    #[serde(rename = "width-units")]
    pub width_units: Option<String>,

    /// Corridor height value. Required parameter.
    #[serde(rename = "corridor-height")]
    pub corridor_height: Option<String>,

    /// Units for corridor height (m, km, hPa, mb, Pa). Required parameter.
    #[serde(rename = "height-units")]
    pub height_units: Option<String>,

    /// Vertical level(s) - only valid if coords doesn't include Z.
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
    // TODO: Stretch goal - resolution parameters
    // /// Number of positions across corridor width.
    // #[serde(rename = "resolution-x")]
    // pub resolution_x: Option<String>,
    //
    // /// Number of positions along trajectory path.
    // #[serde(rename = "resolution-y")]
    // pub resolution_y: Option<String>,
    //
    // /// Number of positions through corridor height.
    // #[serde(rename = "resolution-z")]
    // pub resolution_z: Option<String>,
}

/// Supported width units for corridor queries.
const SUPPORTED_WIDTH_UNITS: &[&str] = &["km", "m", "mi", "nm"];

/// Supported height units for corridor queries.
const SUPPORTED_HEIGHT_UNITS: &[&str] = &["m", "km", "hPa", "mb", "Pa"];

/// Default number of cross-section samples across the corridor width.
/// This creates 3 trajectories: left edge, centerline, right edge.
const DEFAULT_RESOLUTION_X: usize = 3;

/// Earth radius in kilometers for distance calculations.
const EARTH_RADIUS_KM: f64 = 6371.0;

/// Calculate the bearing (direction) from one point to another in radians.
/// Returns the bearing in radians (0 = North, π/2 = East).
fn calculate_bearing(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let lon1_rad = lon1.to_radians();
    let lat1_rad = lat1.to_radians();
    let lon2_rad = lon2.to_radians();
    let lat2_rad = lat2.to_radians();

    let d_lon = lon2_rad - lon1_rad;

    let x = d_lon.cos() * lat2_rad.cos();
    let y = lat1_rad.cos() * lat2_rad.sin() - lat1_rad.sin() * lat2_rad.cos() * d_lon.cos();

    y.atan2(x)
}

/// Calculate a point at a given distance and bearing from a starting point.
/// Uses the haversine formula for spherical Earth approximation.
fn destination_point(lon: f64, lat: f64, bearing_rad: f64, distance_km: f64) -> (f64, f64) {
    let lat_rad = lat.to_radians();
    let lon_rad = lon.to_radians();

    let angular_dist = distance_km / EARTH_RADIUS_KM;

    let lat2 = (lat_rad.sin() * angular_dist.cos()
        + lat_rad.cos() * angular_dist.sin() * bearing_rad.cos())
    .asin();

    let lon2 = lon_rad
        + (bearing_rad.sin() * angular_dist.sin() * lat_rad.cos())
            .atan2(angular_dist.cos() - lat_rad.sin() * lat2.sin());

    (lon2.to_degrees(), lat2.to_degrees())
}

/// Calculate perpendicular offset points for a waypoint on a trajectory.
/// Returns (left_offset_point, right_offset_point) at the specified distance.
fn calculate_perpendicular_offsets(
    prev_lon: Option<f64>,
    prev_lat: Option<f64>,
    lon: f64,
    lat: f64,
    next_lon: Option<f64>,
    next_lat: Option<f64>,
    offset_km: f64,
) -> ((f64, f64), (f64, f64)) {
    // Calculate the bearing at this point
    let bearing = if let (Some(next_lon), Some(next_lat)) = (next_lon, next_lat) {
        if let (Some(prev_lon), Some(prev_lat)) = (prev_lon, prev_lat) {
            // Average of incoming and outgoing bearings
            let b1 = calculate_bearing(prev_lon, prev_lat, lon, lat);
            let b2 = calculate_bearing(lon, lat, next_lon, next_lat);
            // Average the bearings (handling wrap-around)
            let sin_avg = (b1.sin() + b2.sin()) / 2.0;
            let cos_avg = (b1.cos() + b2.cos()) / 2.0;
            sin_avg.atan2(cos_avg)
        } else {
            // First point: use bearing to next
            calculate_bearing(lon, lat, next_lon, next_lat)
        }
    } else if let (Some(prev_lon), Some(prev_lat)) = (prev_lon, prev_lat) {
        // Last point: use bearing from previous
        calculate_bearing(prev_lon, prev_lat, lon, lat)
    } else {
        // Single point: default to east-west offset
        0.0
    };

    // Perpendicular bearings (90 degrees left and right)
    let left_bearing = bearing + std::f64::consts::FRAC_PI_2;
    let right_bearing = bearing - std::f64::consts::FRAC_PI_2;

    let left = destination_point(lon, lat, left_bearing, offset_km);
    let right = destination_point(lon, lat, right_bearing, offset_km);

    (left, right)
}

/// GET /edr/collections/:collection_id/corridor
pub async fn corridor_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<CorridorQueryParams>,
    headers: HeaderMap,
) -> Response {
    // Use latest instance
    corridor_query(state, collection_id, None, params, headers).await
}

/// GET /edr/collections/:collection_id/instances/:instance_id/corridor
pub async fn instance_corridor_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, instance_id)): Path<(String, String)>,
    Query(params): Query<CorridorQueryParams>,
    headers: HeaderMap,
) -> Response {
    corridor_query(state, collection_id, Some(instance_id), params, headers).await
}

async fn corridor_query(
    state: Arc<AppState>,
    collection_id: String,
    instance_id: Option<String>,
    params: CorridorQueryParams,
    headers: HeaderMap,
) -> Response {
    // Negotiate output format based on Accept header and f parameter
    let output_format = match negotiate_format(&headers, params.f.as_deref()) {
        Ok(format) => format,
        Err(response) => {
            return response;
        }
    };

    let config = state.edr_config.read().await;

    // Find the collection
    let Some((model_config, collection_def)) = config.find_collection(&collection_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        );
    };

    // ===== Validate Required Parameters =====

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

    // Check for required corridor-width parameter
    let corridor_width_str = match &params.corridor_width {
        Some(w) if !w.trim().is_empty() => w.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("Missing required parameter: corridor-width"),
            );
        }
    };

    // Check for required width-units parameter
    let width_units_str = match &params.width_units {
        Some(u) if !u.trim().is_empty() => u.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("Missing required parameter: width-units"),
            );
        }
    };

    // corridor-height is optional - default to 0 if not provided (2D corridor)
    let corridor_height_str = params
        .corridor_height
        .as_ref()
        .map(|h| h.as_str())
        .filter(|h| !h.trim().is_empty())
        .unwrap_or("0");

    // height-units is optional - default to "m" if not provided
    let height_units_str = params
        .height_units
        .as_ref()
        .map(|u| u.as_str())
        .filter(|u| !u.trim().is_empty())
        .unwrap_or("m");

    // ===== Parse and Validate Units =====

    // Validate width-units is in supported list
    if !SUPPORTED_WIDTH_UNITS
        .iter()
        .any(|u| u.eq_ignore_ascii_case(width_units_str))
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Invalid width-units '{}'. Supported units: {}",
                width_units_str,
                SUPPORTED_WIDTH_UNITS.join(", ")
            )),
        );
    }

    // Validate height-units is in supported list
    if !SUPPORTED_HEIGHT_UNITS
        .iter()
        .any(|u| u.eq_ignore_ascii_case(height_units_str))
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Invalid height-units '{}'. Supported units: {}",
                height_units_str,
                SUPPORTED_HEIGHT_UNITS.join(", ")
            )),
        );
    }

    // Parse width units
    let width_units = match DistanceUnit::parse(width_units_str) {
        Ok(u) => u,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid width-units: {}", e)),
            );
        }
    };

    // Parse height units
    let _height_units = match VerticalUnit::parse(height_units_str) {
        Ok(u) => u,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid height-units: {}", e)),
            );
        }
    };

    // ===== Parse Corridor Dimensions =====

    let corridor_width: f64 = match corridor_width_str.trim().parse() {
        Ok(v) if v > 0.0 => v,
        Ok(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("corridor-width must be a positive number"),
            );
        }
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "Invalid corridor-width '{}'. Expected a number.",
                    corridor_width_str
                )),
            );
        }
    };

    let _corridor_height: f64 = match corridor_height_str.trim().parse() {
        Ok(v) if v >= 0.0 => v, // Allow 0 for 2D corridors (no vertical extent)
        Ok(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("corridor-height must be a non-negative number"),
            );
        }
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "Invalid corridor-height '{}'. Expected a number.",
                    corridor_height_str
                )),
            );
        }
    };

    // ===== Parse Coordinates =====

    // Parse the corridor coordinates (reuse trajectory parsing)
    let parsed_trajectory = match TrajectoryQuery::parse_coords(coords_str) {
        Ok(t) => t,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "Invalid coordinates: {}. Expected LINESTRING, LINESTRINGZ, LINESTRINGM, LINESTRINGZM, or MULTI* variant.",
                    e
                )),
            );
        }
    };

    let line_type = parsed_trajectory.line_type;
    let waypoints = parsed_trajectory.waypoints;

    if waypoints.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request("Corridor must contain at least one waypoint"),
        );
    }

    // ===== Check for Coordinate/Parameter Conflicts =====

    // Check for conflicting z parameter when coords already has Z
    if line_type.has_z() && params.z.is_some() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(
                "Cannot specify 'z' parameter when coords contains Z coordinates (LINESTRINGZ/LINESTRINGZM). \
                 Use either embedded Z coordinates or the z query parameter, not both."
            ),
        );
    }

    // Check for conflicting datetime parameter when coords already has M
    if line_type.has_m() && params.datetime.is_some() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(
                "Cannot specify 'datetime' parameter when coords contains M coordinates (LINESTRINGM/LINESTRINGZM). \
                 Use either embedded M coordinates or the datetime query parameter, not both."
            ),
        );
    }

    // ===== Parse Optional Parameters =====

    // Parse vertical levels (only if not embedded in coords)
    let z_values = if line_type.has_z() {
        // Z values are embedded in each waypoint
        None
    } else if let Some(ref z) = params.z {
        match PositionQuery::parse_z(z) {
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
        .map(|p| PositionQuery::parse_parameter_names(p))
        .unwrap_or_default();

    // Determine which parameters to query
    let params_to_query: Vec<_> = if requested_params.is_empty() {
        collection_def
            .parameters
            .iter()
            .map(|p| p.name.clone())
            .collect()
    } else {
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

    // Get the list of times to query
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

    // ===== Check Response Size Limits =====

    let num_waypoints = waypoints.len();
    let num_levels = z_values.as_ref().map(|v| v.len()).unwrap_or(1);
    let num_times = if time_strings.is_empty() {
        1
    } else {
        time_strings.len()
    };

    // Use trajectory estimate for corridor (same structure)
    let estimate = ResponseSizeEstimate::for_trajectory(
        params_to_query.len(),
        num_waypoints,
        num_times,
        num_levels,
    );

    if let Err(limit_err) = estimate.check_limits(&model_config.limits) {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            ExceptionResponse::payload_too_large(limit_err.to_string()),
        );
    }

    // ===== Parse Instance ID =====

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

    // ===== Build CoverageJSON CoverageCollection Response =====
    //
    // A corridor query returns a CoverageCollection with multiple trajectories:
    // - Left edge trajectory (offset perpendicular to path)
    // - Centerline trajectory (the original path)
    // - Right edge trajectory (offset perpendicular to path)
    //
    // Each coverage contains the same parameters sampled at different positions.

    // Calculate corridor half-width for offset trajectories
    let half_width_km = width_units.to_kilometers(corridor_width) / 2.0;

    // Z values - either from embedded coords or from z parameter
    let z_axis: Option<Vec<f64>> = if line_type.has_z() {
        Some(waypoints.iter().filter_map(|wp| wp.z).collect())
    } else {
        z_values.clone()
    };

    // Time values - from embedded M coords or from datetime parameter
    let t_values: Option<Vec<String>> = if line_type.has_m() {
        // Convert Unix epoch timestamps to ISO8601
        let times: Vec<String> = waypoints
            .iter()
            .filter_map(|wp| wp.m)
            .map(|epoch| {
                Utc.timestamp_opt(epoch, 0)
                    .single()
                    .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                    .unwrap_or_default()
            })
            .filter(|s| !s.is_empty())
            .collect();
        if times.is_empty() {
            None
        } else {
            Some(times)
        }
    } else if !time_strings.is_empty() {
        Some(time_strings.clone())
    } else {
        None
    };

    // Determine the z value to use for queries
    let query_z_value = if line_type.has_z() {
        waypoints.first().and_then(|wp| wp.z)
    } else {
        z_values.as_ref().and_then(|v| v.first().copied())
    };

    // Determine the time to use for queries
    let query_time: Option<DateTime<Utc>> = if line_type.has_m() {
        waypoints
            .first()
            .and_then(|wp| wp.m)
            .and_then(|epoch| Utc.timestamp_opt(epoch, 0).single())
    } else if !time_strings.is_empty() {
        time_strings
            .first()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
    } else {
        None
    };

    // Generate offset trajectories (left edge, centerline, right edge)
    // Each trajectory is a list of (lon, lat) points
    let mut trajectories: Vec<Vec<(f64, f64)>> = Vec::with_capacity(DEFAULT_RESOLUTION_X);

    // Calculate left edge, centerline, and right edge coordinates
    let mut left_coords: Vec<(f64, f64)> = Vec::with_capacity(waypoints.len());
    let mut center_coords: Vec<(f64, f64)> = Vec::with_capacity(waypoints.len());
    let mut right_coords: Vec<(f64, f64)> = Vec::with_capacity(waypoints.len());

    for (i, wp) in waypoints.iter().enumerate() {
        let prev = if i > 0 {
            (Some(waypoints[i - 1].lon), Some(waypoints[i - 1].lat))
        } else {
            (None, None)
        };
        let next = if i < waypoints.len() - 1 {
            (Some(waypoints[i + 1].lon), Some(waypoints[i + 1].lat))
        } else {
            (None, None)
        };

        let (left, right) = calculate_perpendicular_offsets(
            prev.0,
            prev.1,
            wp.lon,
            wp.lat,
            next.0,
            next.1,
            half_width_km,
        );

        left_coords.push(left);
        center_coords.push((wp.lon, wp.lat));
        right_coords.push(right);
    }

    trajectories.push(left_coords);
    trajectories.push(center_coords);
    trajectories.push(right_coords);

    // Build shared parameter definitions (used by all coverages)
    let mut shared_params: std::collections::HashMap<String, CovJsonParameter> =
        std::collections::HashMap::new();

    // Pre-fetch metadata for all parameters to build shared definitions
    for param_name in &params_to_query {
        let param_def = collection_def
            .parameters
            .iter()
            .find(|p| p.name == *param_name);

        let level_str = build_level_string(&collection_def.level_filter, param_def, query_z_value);

        let mut query = DatasetQuery::forecast(&model_config.model, param_name);
        if let Some(level) = &level_str {
            query = query.at_level(level);
        }
        if let Some(valid_time) = query_time {
            query = query.at_valid_time(valid_time);
        }
        if let Some(ref_time) = reference_time {
            query = query.at_run(ref_time);
        }

        let metadata = state.grid_data_service.get_metadata(&query).await.ok();
        let units_str = metadata
            .as_ref()
            .map(|m| m.units.clone())
            .unwrap_or_default();

        let unit = Unit::from_symbol(&units_str);
        let cov_param = CovJsonParameter::new(param_name).with_unit(unit);
        shared_params.insert(param_name.clone(), cov_param);
    }

    // Create a coverage for each trajectory (left, center, right)
    let mut coverages: Vec<CoverageJson> = Vec::with_capacity(trajectories.len());

    for traj_coords in &trajectories {
        let x_values: Vec<f64> = traj_coords.iter().map(|(lon, _)| *lon).collect();
        let y_values: Vec<f64> = traj_coords.iter().map(|(_, lat)| *lat).collect();

        // Create base coverage with trajectory domain
        let mut coverage = CoverageJson {
            type_: edr_protocol::coverage_json::CoverageType::Coverage,
            domain: edr_protocol::Domain::trajectory(
                x_values.clone(),
                y_values.clone(),
                t_values.clone(),
                z_axis.clone(),
            ),
            parameters: None, // Parameters defined at collection level
            ranges: Some(std::collections::HashMap::new()),
        };

        // Query data at each point in this trajectory for each parameter
        for param_name in &params_to_query {
            let param_def = collection_def
                .parameters
                .iter()
                .find(|p| p.name == *param_name);

            let level_str =
                build_level_string(&collection_def.level_filter, param_def, query_z_value);

            let mut query = DatasetQuery::forecast(&model_config.model, param_name);

            if let Some(level) = &level_str {
                query = query.at_level(level);
            }

            if let Some(valid_time) = query_time {
                query = query.at_valid_time(valid_time);
            }

            if let Some(ref_time) = reference_time {
                query = query.at_run(ref_time);
            }

            let mut values: Vec<Option<f32>> = Vec::with_capacity(traj_coords.len());

            for (i, (lon, lat)) in traj_coords.iter().enumerate() {
                // For corridor queries with embedded time (M coords), use waypoint time
                let wp_query = if line_type.has_m() && i < waypoints.len() {
                    if let Some(epoch) = waypoints[i].m {
                        if let Some(dt) = Utc.timestamp_opt(epoch, 0).single() {
                            query.clone().at_valid_time(dt)
                        } else {
                            query.clone()
                        }
                    } else {
                        query.clone()
                    }
                } else {
                    query.clone()
                };

                // Sample at this point
                let value = match state
                    .grid_data_service
                    .read_point(&wp_query, *lon, *lat)
                    .await
                {
                    Ok(point_value) => match point_value.value {
                        Some(v) if !v.is_nan() => Some(v),
                        _ => None,
                    },
                    Err(e) => {
                        tracing::debug!(
                            "Failed to sample {} at ({}, {}): {}",
                            param_name,
                            lon,
                            lat,
                            e
                        );
                        None
                    }
                };

                values.push(value);
            }

            // Add the range to this coverage
            let shape = vec![traj_coords.len()];
            let axis_names = vec!["composite".to_string()];

            if let Some(ref mut ranges) = coverage.ranges {
                ranges.insert(
                    param_name.clone(),
                    edr_protocol::coverage_json::NdArray::with_missing(values, shape, axis_names),
                );
            }
        }

        coverages.push(coverage);
    }

    // Build the CoverageCollection with Trajectory domain type
    let mut collection = CoverageCollection::new()
        .with_domain_type(edr_protocol::coverage_json::DomainType::Trajectory)
        .with_parameters(shared_params);

    // Add all coverages
    for cov in coverages {
        collection = collection.with_coverage(cov);
    }

    // Serialize response based on requested format
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
                tracing::error!("Failed to serialize CoverageJSON: {}", e);
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ExceptionResponse::internal_error("Failed to serialize response"),
                );
            }
        },
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "max-age=300")
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
            // GRIB2 Table 4.5: 212-214=low, 222-224=middle, 232-234=high
            // (x2=bottom, x3=top, x4=layer; some products use different codes)
            if let Some(code) = level_filter.level_code {
                match code {
                    212 | 213 | 214 => Some("low cloud layer".to_string()),
                    222 | 223 | 224 => Some("middle cloud layer".to_string()),
                    232 | 233 | 234 => Some("high cloud layer".to_string()),
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
    use super::*;

    #[test]
    fn test_supported_width_units() {
        assert!(SUPPORTED_WIDTH_UNITS.contains(&"km"));
        assert!(SUPPORTED_WIDTH_UNITS.contains(&"m"));
        assert!(SUPPORTED_WIDTH_UNITS.contains(&"mi"));
        assert!(SUPPORTED_WIDTH_UNITS.contains(&"nm"));
    }

    #[test]
    fn test_supported_height_units() {
        assert!(SUPPORTED_HEIGHT_UNITS.contains(&"m"));
        assert!(SUPPORTED_HEIGHT_UNITS.contains(&"km"));
        assert!(SUPPORTED_HEIGHT_UNITS.contains(&"hPa"));
        assert!(SUPPORTED_HEIGHT_UNITS.contains(&"mb"));
        assert!(SUPPORTED_HEIGHT_UNITS.contains(&"Pa"));
    }

    #[test]
    fn test_width_unit_validation() {
        assert!(SUPPORTED_WIDTH_UNITS
            .iter()
            .any(|u| u.eq_ignore_ascii_case("KM")));
        assert!(SUPPORTED_WIDTH_UNITS
            .iter()
            .any(|u| u.eq_ignore_ascii_case("Km")));
        assert!(!SUPPORTED_WIDTH_UNITS
            .iter()
            .any(|u| u.eq_ignore_ascii_case("hPa")));
    }

    #[test]
    fn test_height_unit_validation() {
        assert!(SUPPORTED_HEIGHT_UNITS
            .iter()
            .any(|u| u.eq_ignore_ascii_case("HPA")));
        assert!(SUPPORTED_HEIGHT_UNITS
            .iter()
            .any(|u| u.eq_ignore_ascii_case("MB")));
        assert!(!SUPPORTED_HEIGHT_UNITS
            .iter()
            .any(|u| u.eq_ignore_ascii_case("nm")));
    }
}
