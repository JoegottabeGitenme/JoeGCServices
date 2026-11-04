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
use serde::Deserialize;
use std::sync::Arc;

use crate::availability::ModelAvailability;
use crate::cache_policy::{impl_time_selectors, TimeSelectors};
use crate::config::build_level_string;
use crate::content_negotiation::{check_png_not_supported, negotiate_format, OutputFormat};
use crate::handlers::forecast_params::{validate_not_observation_data, ForecastParams};
use crate::metrics::{
    extract_client_ip, extract_user_agent, format_from_output, EndpointType, Timer,
};

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
use crate::limits::{check_trajectory_points, ResponseSizeEstimate};
use crate::state::AppState;
use crate::validation::validate_z_against_vertical_extent;

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

    /// Model run time (ISO8601). Required if forecast-hour is specified.
    /// Only applicable to forecast models (GFS, HRRR, etc.), not observation data.
    pub run: Option<String>,

    /// Forecast hour(s) from the model run.
    /// Formats: single (6), list (0,6,12), range (0/24), range+step (0/24/6).
    /// Requires 'run' to be specified.
    #[serde(rename = "forecast-hour")]
    pub forecast_hour: Option<String>,
}

// `run` is validated but the newest run is queried, so it does not pin the time
// for caching (see cache_policy).
impl_time_selectors!(TrajectoryQueryParams, ignores_run);

/// How many waypoints of one parameter are read at the same time. Each read is a catalog lookup
/// and a blocking chunk read that waits on the grid service's permits; this keeps one route from
/// taking them all.
const TRAJECTORY_READ_CONCURRENCY: usize = 8;

/// Whether a `datetime` names more than one time.
fn selects_several_times(dq: &DateTimeQuery) -> bool {
    match dq {
        DateTimeQuery::Instant(_) => false,
        DateTimeQuery::List(times) => times.len() > 1,
        DateTimeQuery::Interval { .. } => true,
    }
}

/// `f` applied to every item, at most `limit` at a time, each on its own task so that blocking
/// work really runs in parallel. The results are in the order of `items`; an item whose task
/// failed (it panicked) is `None`.
async fn map_bounded<I, T, F, Fut>(items: Vec<I>, limit: usize, f: F) -> Vec<Option<T>>
where
    F: Fn(I) -> Fut,
    Fut: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    use futures::StreamExt;
    futures::stream::iter(items)
        .map(|item| tokio::spawn(f(item)))
        .buffered(limit.max(1))
        .map(|joined| joined.ok())
        .collect()
        .await
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

    // PNG output is not supported for trajectory queries
    if let Some(response) = check_png_not_supported(output_format, "trajectory") {
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

    if let Err(limit_err) = check_trajectory_points(&model_config.limits, waypoints.len()) {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(limit_err.to_string()),
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

    // Validate embedded Z coordinates against collection's vertical extent
    if line_type.has_z() {
        let embedded_z_values: Vec<f64> = waypoints.iter().filter_map(|wp| wp.z).collect();
        if let Err(e) = validate_z_against_vertical_extent(&embedded_z_values, collection_def) {
            return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e));
        }
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

    // Without M every waypoint is read at one time (a trajectory has one value per waypoint), so
    // an interval or a list would only label the response with times it does not hold.
    if !line_type.has_m() {
        if datetime_query.as_ref().is_some_and(selects_several_times) {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "A trajectory without M coordinates is sampled at a single time, but datetime '{}' selects \
                     several. Give one instant, or put each waypoint's time in the coordinates \
                     (LINESTRINGM or LINESTRINGZM, M in Unix seconds).",
                    params.datetime.as_deref().unwrap_or_default()
                )),
            );
        }
    }

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

        // Build the DatasetQuery using the appropriate type (forecast vs observation)
        let mut query = model_config.create_query(param_name);

        if let Some(level) = &level_str {
            query = query.at_level(level);
        }

        query = query.in_run_at(reference_time, query_time);

        // Sample values at each waypoint. With embedded time (M coords) each waypoint is read at
        // its own valid time. The reads run concurrently (a route is up to a few hundred of them,
        // each a catalog lookup and a chunk read), in waypoint order.
        let queries: Vec<_> = waypoints
            .iter()
            .map(|waypoint| {
                let at = line_type
                    .has_m()
                    .then(|| {
                        waypoint
                            .m
                            .and_then(|epoch| Utc.timestamp_opt(epoch, 0).single())
                    })
                    .flatten();
                let wp_query = match at {
                    Some(dt) => query.clone().in_run_at(reference_time, Some(dt)),
                    None => query.clone(),
                };
                (wp_query, waypoint.lon, waypoint.lat)
            })
            .collect();
        let values: Vec<Option<f32>> = map_bounded(queries, TRAJECTORY_READ_CONCURRENCY, {
            let state = Arc::clone(&state);
            let param_name = param_name.clone();
            move |(wp_query, lon, lat)| {
                let state = Arc::clone(&state);
                let param_name = param_name.clone();
                async move {
                    match state
                        .grid_data_service
                        .read_point(&wp_query, lon, lat)
                        .await
                    {
                        // PointValue has an Option<f32> value field
                        Ok(point_value) => point_value.value.filter(|v| !v.is_nan()),
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
                    }
                }
            }
        })
        .await
        .into_iter()
        .map(Option::flatten)
        .collect();

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
            EndpointType::Trajectory,
            Some(&collection_id),
            &params_to_query,
            format_from_output(&output_format),
            timer.elapsed_us(),
            true,
            client_ip.as_deref(),
            user_agent.as_deref(),
        )
        .await;

    // Whether this query named its time decides how long a CDN may keep the
    // response. A LINESTRING M query carries its times in the coordinates. See
    // cache_policy.
    let cache_control = params
        .requested_time(instance_id.as_deref())
        .with_embedded_times(line_type.has_m())
        .cache_control(300);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache_control)
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
    fn only_a_single_instant_is_one_time() {
        let parse = |s: &str| DateTimeQuery::parse(s).unwrap();
        assert!(!selects_several_times(&parse("2026-11-04T12:00:00Z")));
        assert!(selects_several_times(&parse(
            "2026-11-04T12:00:00Z,2026-11-04T13:00:00Z"
        )));
        assert!(selects_several_times(&parse(
            "2026-11-04T12:00:00Z/2026-11-04T18:00:00Z"
        )));
        assert!(selects_several_times(&parse("2026-11-04T12:00:00Z/..")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn map_bounded_keeps_the_order_and_the_bound() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let items: Vec<u64> = (0..40).collect();
        let out = map_bounded(items, 8, {
            let (running, peak) = (Arc::clone(&running), Arc::clone(&peak));
            move |i: u64| {
                let (running, peak) = (Arc::clone(&running), Arc::clone(&peak));
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    // the early items finish last
                    tokio::time::sleep(std::time::Duration::from_millis(40 - i)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    i * 2
                }
            }
        })
        .await;
        assert_eq!(out, (0..40).map(|i| Some(i * 2)).collect::<Vec<_>>());
        let peak = peak.load(Ordering::SeqCst);
        assert!(
            (2..=8).contains(&peak),
            "reads overlap, but never more than the bound: {peak}"
        );
    }

    #[tokio::test]
    async fn map_bounded_marks_a_failed_task_and_goes_on() {
        let out = map_bounded(vec![1, 2, 3], 2, |i: i32| async move {
            if i == 2 {
                panic!("a read failed");
            }
            i
        })
        .await;
        assert_eq!(out, vec![Some(1), None, Some(3)]);
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
