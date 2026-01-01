//! Trajectory query handler.
//!
//! Returns data along a path defined by the coords parameter.
//! Per OGC EDR spec, trajectory queries require:
//! - coords: LINESTRING, LINESTRINGZ, LINESTRINGM, LINESTRINGZM
//!           or MULTI* variants (path along which to sample data)
//!
//! The Z coordinate (in LINESTRINGZ/LINESTRINGZM) represents height.
//! The M coordinate (in LINESTRINGM/LINESTRINGZM) represents Unix epoch time.
//!
//! If coords contains Z, the `z` query param MUST NOT be used.
//! If coords contains M, the `datetime` query param provides additional filtering.

use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, TimeZone, Utc};
use edr_protocol::{
    coverage_json::CovJsonParameter, parameters::Unit, queries::DateTimeQuery,
    responses::ExceptionResponse, CoverageJson, EdrFeatureCollection, PositionQuery,
    TrajectoryQuery,
};
use grid_processor::DatasetQuery;
use serde::Deserialize;
use std::sync::Arc;

use crate::config::LevelValue;
use crate::content_negotiation::{negotiate_format, OutputFormat};
use crate::limits::ResponseSizeEstimate;
use crate::state::AppState;

/// Query parameters for trajectory endpoint.
#[derive(Debug, Deserialize)]
pub struct TrajectoryQueryParams {
    /// Coordinates as WKT LINESTRING or MULTILINESTRING. Required parameter.
    pub coords: Option<String>,

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
}

/// GET /edr/collections/:collection_id/trajectory
pub async fn trajectory_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<TrajectoryQueryParams>,
    headers: HeaderMap,
) -> Response {
    // Use latest instance
    trajectory_query(state, collection_id, None, params, headers).await
}

/// GET /edr/collections/:collection_id/instances/:instance_id/trajectory
pub async fn instance_trajectory_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, instance_id)): Path<(String, String)>,
    Query(params): Query<TrajectoryQueryParams>,
    headers: HeaderMap,
) -> Response {
    trajectory_query(state, collection_id, Some(instance_id), params, headers).await
}

async fn trajectory_query(
    state: Arc<AppState>,
    collection_id: String,
    instance_id: Option<String>,
    params: TrajectoryQueryParams,
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

    // Parse the trajectory coordinates
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
            ExceptionResponse::bad_request("Trajectory must contain at least one waypoint"),
        );
    }

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
    // Per OGC EDR spec: An error SHALL be thrown if coords=LINESTRINGM and datetime is specified
    if line_type.has_m() && params.datetime.is_some() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(
                "Cannot specify 'datetime' parameter when coords contains M coordinates (LINESTRINGM/LINESTRINGZM). \
                 Use either embedded M coordinates or the datetime query parameter, not both."
            ),
        );
    }

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

    // Check response size limits
    let num_waypoints = waypoints.len();
    let num_levels = z_values.as_ref().map(|v| v.len()).unwrap_or(1);
    let num_times = if time_strings.is_empty() {
        1
    } else {
        time_strings.len()
    };

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

    // Parse instance_id if provided
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

    // Build coordinate arrays for CoverageJSON Trajectory domain
    let x_values: Vec<f64> = waypoints.iter().map(|wp| wp.lon).collect();
    let y_values: Vec<f64> = waypoints.iter().map(|wp| wp.lat).collect();

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

    // Create CoverageJSON with Trajectory domain
    let mut coverage = CoverageJson {
        type_: edr_protocol::coverage_json::CoverageType::Coverage,
        domain: edr_protocol::Domain::trajectory(
            x_values.clone(),
            y_values.clone(),
            t_values.clone(),
            z_axis.clone(),
        ),
        parameters: Some(std::collections::HashMap::new()),
        ranges: Some(std::collections::HashMap::new()),
    };

    // Query data at each waypoint for each parameter
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

        // Sample values at each waypoint
        let mut values: Vec<Option<f32>> = Vec::with_capacity(waypoints.len());

        for waypoint in &waypoints {
            // For trajectory queries with embedded time (M coords), we might need
            // to query at different valid times for each waypoint.
            // For simplicity, we'll sample at the waypoint location using the first time.
            // A more sophisticated implementation could query each waypoint at its embedded time.

            let wp_query = if line_type.has_m() {
                // Use the waypoint's embedded time
                if let Some(epoch) = waypoint.m {
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

            match state
                .grid_data_service
                .read_point(&wp_query, waypoint.lon, waypoint.lat)
                .await
            {
                Ok(point_value) => {
                    // PointValue has an Option<f32> value field
                    match point_value.value {
                        Some(v) if !v.is_nan() => values.push(Some(v)),
                        _ => values.push(None),
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        "Failed to sample {} at ({}, {}): {}",
                        param_name,
                        waypoint.lon,
                        waypoint.lat,
                        e
                    );
                    values.push(None);
                }
            }
        }

        // Get metadata for units
        let metadata = state.grid_data_service.get_metadata(&query).await.ok();
        let units_str = metadata
            .as_ref()
            .map(|m| m.units.clone())
            .unwrap_or_default();

        let unit = Unit::from_symbol(&units_str);
        let cov_param = CovJsonParameter::new(param_name).with_unit(unit);

        // For trajectory, the shape is just [num_waypoints] since each waypoint has one value
        let shape = vec![waypoints.len()];
        let axis_names = vec!["composite".to_string()]; // Composite axis for trajectory

        coverage = coverage
            .with_parameter_array_nullable(param_name, cov_param, values, shape, axis_names);
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
        OutputFormat::CoverageJson => {
            match serde_json::to_string_pretty(&coverage) {
                Ok(j) => (j, output_format.content_type()),
                Err(e) => {
                    tracing::error!("Failed to serialize CoverageJSON: {}", e);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ExceptionResponse::internal_error("Failed to serialize response"),
                    );
                }
            }
        }
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
    use super::*;
    use edr_protocol::LineStringType;

    #[test]
    fn test_parse_linestring() {
        let result =
            TrajectoryQuery::parse_coords("LINESTRING(-3.53 50.72, -3.35 50.92, -3.11 51.02)");
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.waypoints.len(), 3);
        assert_eq!(parsed.line_type, LineStringType::LineString);
    }

    #[test]
    fn test_parse_linestringz() {
        let result = TrajectoryQuery::parse_coords("LINESTRINGZ(-3.53 50.72 100, -3.35 50.92 200)");
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.waypoints.len(), 2);
        assert_eq!(parsed.line_type, LineStringType::LineStringZ);
        assert!(parsed.line_type.has_z());
    }

    #[test]
    fn test_parse_linestringm() {
        let result = TrajectoryQuery::parse_coords(
            "LINESTRINGM(-3.53 50.72 1560507000, -3.35 50.92 1560508800)",
        );
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.waypoints.len(), 2);
        assert_eq!(parsed.line_type, LineStringType::LineStringM);
        assert!(parsed.line_type.has_m());
    }

    #[test]
    fn test_parse_linestringzm() {
        let result = TrajectoryQuery::parse_coords(
            "LINESTRINGZM(-3.53 50.72 100 1560507000, -3.35 50.92 200 1560508800)",
        );
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.waypoints.len(), 2);
        assert_eq!(parsed.line_type, LineStringType::LineStringZM);
        assert!(parsed.line_type.has_z());
        assert!(parsed.line_type.has_m());
    }

    #[test]
    fn test_parse_invalid_linestring() {
        // POINT is not valid for trajectory
        let result = TrajectoryQuery::parse_coords("POINT(-3.53 50.72)");
        assert!(result.is_err());

        // POLYGON is not valid for trajectory
        let result = TrajectoryQuery::parse_coords(
            "POLYGON((-3.53 50.72, -3.35 50.92, -3.11 51.02, -3.53 50.72))",
        );
        assert!(result.is_err());
    }
}
