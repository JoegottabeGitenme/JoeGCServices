//! Area query handler.

use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Utc};
use edr_protocol::{
    coverage_json::CovJsonParameter, parameters::Unit, queries::DateTimeQuery,
    responses::ExceptionResponse, AreaQuery, CoverageJson, EdrFeatureCollection, ParsedPolygons,
};
use grid_processor::{BoundingBox, ProjectionType, RowOrigin};
use renderer::data_png::{compute_data_range, DataPng8BitEncoder, DataPngEncoder};
use serde::Deserialize;
use std::sync::Arc;

use crate::availability::ModelAvailability;
use crate::config::build_level_string;
use crate::content_negotiation::{negotiate_format, OutputFormat};
use crate::handlers::forecast_params::{validate_not_observation_data, ForecastParams};
use crate::handlers::observations::{obs_area_query_handler, ObsAreaQueryParams};
use crate::limits::ResponseSizeEstimate;
use crate::metrics::{
    extract_client_ip, extract_user_agent, format_from_output, EndpointType, Timer,
};
use crate::resampling::{
    extract_dateline_region, extract_prime_meridian_region, resample_to_geographic,
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

/// Resample data using nearest-neighbor interpolation.
///
/// This is fast and preserves discrete values well. For smoother results
/// with continuous data like temperature, bilinear interpolation would be better.
///
/// TODO: Add bilinear interpolation option for parameters that benefit from smoothing.
fn resample_nearest(
    data: &[Option<f32>],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
) -> Vec<Option<f32>> {
    // Handle edge cases: empty source data or zero dimensions
    if data.is_empty() || src_width == 0 || src_height == 0 {
        return vec![None; dst_width * dst_height];
    }

    let mut result = Vec::with_capacity(dst_width * dst_height);

    for dst_y in 0..dst_height {
        for dst_x in 0..dst_width {
            // Map destination pixel to source pixel (nearest neighbor)
            let src_x = (dst_x as f64 * src_width as f64 / dst_width as f64) as usize;
            let src_y = (dst_y as f64 * src_height as f64 / dst_height as f64) as usize;

            // Clamp to valid range
            let src_x = src_x.min(src_width - 1);
            let src_y = src_y.min(src_height - 1);

            let src_idx = src_y * src_width + src_x;
            result.push(data[src_idx]);
        }
    }

    result
}

/// Flip rows vertically (reverse row order).
///
/// Used to convert RowOrigin::South grids (row 0 = south) to image coordinates
/// where row 0 should be at the top (north).
fn flip_rows(data: &[Option<f32>], width: usize, height: usize) -> Vec<Option<f32>> {
    let mut result = Vec::with_capacity(data.len());

    // Iterate rows in reverse order (last row first)
    for row in (0..height).rev() {
        let row_start = row * width;
        let row_end = row_start + width;
        result.extend_from_slice(&data[row_start..row_end]);
    }

    result
}

/// Maximum allowed PNG dimension (width or height)
const MAX_PNG_DIMENSION: usize = 4096;

/// Small epsilon for floating-point bbox containment checks.
/// This prevents edge artifacts when checking if pixels are inside a rectangular region.
const BBOX_EPSILON: f64 = 1e-9;

/// Check if a polygon is a simple axis-aligned rectangle (4 corners + closing point).
/// Returns the bbox if it is, None otherwise.
fn is_simple_rectangle(polygon: &[(f64, f64)]) -> Option<(f64, f64, f64, f64)> {
    // A simple rectangle has 5 points (4 corners + closing point that matches first)
    if polygon.len() != 5 {
        return None;
    }

    // First and last point should be the same (closed polygon)
    let first = polygon[0];
    let last = polygon[4];
    if (first.0 - last.0).abs() > BBOX_EPSILON || (first.1 - last.1).abs() > BBOX_EPSILON {
        return None;
    }

    // Extract all unique x and y coordinates
    let mut x_coords: Vec<f64> = polygon[..4].iter().map(|p| p.0).collect();
    let mut y_coords: Vec<f64> = polygon[..4].iter().map(|p| p.1).collect();
    x_coords.sort_by(|a, b| a.partial_cmp(b).unwrap());
    y_coords.sort_by(|a, b| a.partial_cmp(b).unwrap());
    x_coords.dedup_by(|a, b| (*a - *b).abs() < BBOX_EPSILON);
    y_coords.dedup_by(|a, b| (*a - *b).abs() < BBOX_EPSILON);

    // A rectangle should have exactly 2 unique x values and 2 unique y values
    if x_coords.len() != 2 || y_coords.len() != 2 {
        return None;
    }

    Some((x_coords[0], y_coords[0], x_coords[1], y_coords[1]))
}

/// Check if a point is inside a bbox with epsilon tolerance.
///
/// Note: For simple rectangle queries (common case), we skip this check entirely
/// because read_region already returns data clipped to the requested bbox. This
/// avoids edge artifacts where grid cell centers fall just outside the requested
/// polygon due to grid alignment differences.
#[allow(dead_code)]
fn bbox_contains_point(west: f64, south: f64, east: f64, north: f64, lon: f64, lat: f64) -> bool {
    lon >= west - BBOX_EPSILON
        && lon <= east + BBOX_EPSILON
        && lat >= south - BBOX_EPSILON
        && lat <= north + BBOX_EPSILON
}

/// Query parameters for area endpoint.
#[derive(Debug, Deserialize)]
pub struct AreaQueryParams {
    /// Coordinates as WKT POLYGON. Required parameter.
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

    /// Requested output width in pixels (PNG only, max 4096).
    /// If specified, height must also be specified.
    pub width: Option<u32>,

    /// Requested output height in pixels (PNG only, max 4096).
    /// If specified, width must also be specified.
    pub height: Option<u32>,

    /// PNG encoding bit depth (PNG only).
    /// - `16` (default): 16-bit precision using RG channels (65536 values)
    /// - `8`: 8-bit grayscale+alpha (256 values, ~50% smaller files)
    pub depth: Option<u8>,

    /// Model run time (ISO8601). Required if forecast-hour is specified.
    /// Only applicable to forecast models (GFS, HRRR, etc.), not observation data.
    pub run: Option<String>,

    /// Forecast hour(s) from the model run.
    /// Formats: single (6), list (0,6,12), range (0/24), range+step (0/24/6).
    /// Requires 'run' to be specified.
    #[serde(rename = "forecast-hour")]
    pub forecast_hour: Option<String>,
}

/// GET /edr/collections/:collection_id/area
///
/// For point observation collections (METAR, TAF, etc.), dispatches to observation handler.
pub async fn area_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<AreaQueryParams>,
    headers: HeaderMap,
) -> Response {
    // Check if this is a point data collection (METAR observations or TAF forecasts)
    {
        let config = state.edr_config.read().await;
        if let Some((model_config, _)) = config.find_collection(&collection_id) {
            if model_config.data_type.is_point_data() {
                // Convert the coords to format observation handler expects
                let obs_params = ObsAreaQueryParams {
                    coords: params.coords.clone().unwrap_or_default(),
                    datetime: params.datetime.clone(),
                    parameter_name: params.parameter_name.clone(),
                    f: params.f.clone(),
                    limit: None,
                };
                return obs_area_query_handler(
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
    area_query(state, collection_id, None, params, headers).await
}

/// GET /edr/collections/:collection_id/instances/:instance_id/area
pub async fn instance_area_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, instance_id)): Path<(String, String)>,
    Query(params): Query<AreaQueryParams>,
    headers: HeaderMap,
) -> Response {
    area_query(state, collection_id, Some(instance_id), params, headers).await
}

async fn area_query(
    state: Arc<AppState>,
    collection_id: String,
    instance_id: Option<String>,
    params: AreaQueryParams,
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

    // Parse polygon coordinates - supports both POLYGON and MULTIPOLYGON
    let parsed_polygons = match AreaQuery::parse_polygon_multi(coords_str) {
        Ok(p) => p,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid coordinates: {}", e)),
            );
        }
    };

    // Extract polygons for processing
    let polygons: Vec<Vec<(f64, f64)>> = match &parsed_polygons {
        ParsedPolygons::Single(polygon) => vec![polygon.clone()],
        ParsedPolygons::Multi(polygons) => polygons.clone(),
    };

    // Use first polygon for primary calculations (union for point-in-polygon checks)
    let polygon = polygons.first().cloned().unwrap_or_default();

    // Create AreaQuery for calculations
    let area_query_struct = AreaQuery {
        polygon: polygon.clone(),
        z: None,
        datetime: None,
        parameter_names: None,
        crs: None,
    };

    // For MULTIPOLYGON, we create additional AreaQuery structs for contains_point checks
    let all_area_queries: Vec<AreaQuery> = polygons
        .iter()
        .map(|p| AreaQuery {
            polygon: p.clone(),
            z: None,
            datetime: None,
            parameter_names: None,
            crs: None,
        })
        .collect();

    // Check for crossing notation in any polygon (antimeridian crossing with ambiguous coordinates)
    // Users should use extended notation (e.g., 170 to 190 instead of 170 to -170)
    for aq in &all_area_queries {
        if let Some((from_lon, to_lon)) = aq.uses_crossing_notation() {
            let suggested_to = if to_lon < 0.0 {
                to_lon + 360.0
            } else {
                to_lon - 360.0
            };
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "Antimeridian-crossing polygons must use extended notation. \
                     Your polygon has an edge from {:.1}° to {:.1}° which crosses the antimeridian \
                     using ambiguous 'crossing notation'. Please use extended notation instead: \
                     the coordinate {:.1}° should be specified as {:.1}° \
                     (add 360 to negative longitudes, or subtract 360 from positive longitudes \
                     that cross the antimeridian). \
                     Example: POLYGON((170 35, 190 35, 190 45, 170 45, 170 35)) instead of \
                     POLYGON((170 35, -170 35, -170 45, 170 45, 170 35))",
                    from_lon, to_lon, to_lon, suggested_to
                )),
            );
        }
    }

    // Check area size limit
    // Use higher limit for PNG queries (typically used for large regional coverage)
    let area_sq_degrees = area_query_struct.area_sq_degrees();
    let max_area = if output_format == OutputFormat::Png {
        model_config
            .limits
            .max_area_sq_degrees_png
            .or(model_config.limits.max_area_sq_degrees)
            .unwrap_or(100.0)
    } else {
        model_config.limits.max_area_sq_degrees.unwrap_or(100.0)
    };
    if area_sq_degrees > max_area {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            ExceptionResponse::payload_too_large(format!(
                "Area too large: {:.2} sq degrees exceeds limit of {:.2}",
                area_sq_degrees, max_area
            )),
        );
    }

    // Parse vertical levels
    let z_values = if let Some(ref z) = params.z {
        match edr_protocol::PositionQuery::parse_z(z) {
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

    // Parse output CRS (defaults to CRS:84)
    let output_crs = if let Some(ref crs_str) = params.crs {
        match edr_protocol::OutputCrs::from_str(crs_str) {
            Ok(crs) => crs,
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(e.to_string()),
                );
            }
        }
    } else {
        edr_protocol::OutputCrs::default() // CRS:84
    };

    // Parse parameter names
    let requested_params = params
        .parameter_name
        .as_ref()
        .map(|p| edr_protocol::PositionQuery::parse_parameter_names(p))
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

    // PNG output requires exactly one parameter
    if output_format == OutputFormat::Png && params_to_query.len() != 1 {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "PNG output requires exactly one parameter. Use parameter-name to select a single parameter. Requested: {}",
                if params_to_query.is_empty() { "none".to_string() } else { params_to_query.join(", ") }
            )),
        );
    }

    // Validate width/height parameters (PNG only)
    let requested_dimensions: Option<(usize, usize)> = match (params.width, params.height) {
        (Some(w), Some(h)) => {
            if output_format != OutputFormat::Png {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(
                        "width and height parameters are only supported for PNG output (f=png)",
                    ),
                );
            }
            let w = w as usize;
            let h = h as usize;
            if w == 0 || h == 0 {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request("width and height must be greater than 0"),
                );
            }
            if w > MAX_PNG_DIMENSION || h > MAX_PNG_DIMENSION {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!(
                        "width and height must not exceed {}. Requested: {}x{}",
                        MAX_PNG_DIMENSION, w, h
                    )),
                );
            }
            Some((w, h))
        }
        (Some(_), None) | (None, Some(_)) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(
                    "Both width and height must be specified together, or neither",
                ),
            );
        }
        (None, None) => None,
    };

    // Get the bbox of the polygon for grid queries
    let bbox = area_query_struct.bbox();

    // Get the list of times to query
    // For interval queries (especially open-ended ones), expand against available times
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

    // Check response size limits (skip for PNG - binary PNG is much smaller than JSON)
    if output_format != OutputFormat::Png {
        // Estimate grid size based on bbox (assume ~0.03 degree resolution for HRRR, ~0.25 for GFS)
        let resolution = 0.05; // Conservative estimate

        let estimate = ResponseSizeEstimate::for_area(
            params_to_query.len(),
            num_times,
            num_levels,
            area_sq_degrees,
            resolution,
        );

        if let Err(limit_err) = estimate.check_limits(&model_config.limits) {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                ExceptionResponse::payload_too_large(limit_err.to_string()),
            );
        }
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
    // TODO: Full multi-z support would return a 3D grid (z, y, x) with data for each level
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

    // Read the region
    let grid_bbox = BoundingBox::new(bbox.west, bbox.south, bbox.east, bbox.north);

    // Get metadata to determine projection type
    let first_metadata = state.grid_data_service.get_metadata(&query).await.ok();
    let projection_type = first_metadata
        .as_ref()
        .map(|m| m.projection)
        .unwrap_or(ProjectionType::Geographic);

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

    // If the request crosses the prime meridian on a 0-360 grid (like GFS),
    // extract just the requested region. The grid reader returns the full grid
    // in this case, so we need to extract the appropriate columns.
    let region = extract_prime_meridian_region(&region, &grid_bbox);

    // If the request uses extended notation crossing the dateline (e.g., 140° to 235°),
    // convert the output bbox to -180/180 format (e.g., 140° to -125°).
    let region = extract_dateline_region(&region, &grid_bbox);

    // If the grid uses a projected coordinate system (Lambert, Polar Stereographic, etc.),
    // resample it to a regular geographic grid. This ensures EDR output coordinates
    // are uniformly spaced in lat/lon, which is what clients expect.
    let region = if projection_type.requires_projection_transform() {
        tracing::debug!(
            model = model_config.model.as_str(),
            projection = %projection_type,
            native_size = format!("{}x{}", region.width, region.height),
            "Reprojecting grid data to geographic coordinates"
        );
        resample_to_geographic(&region, projection_type, &model_config.model, None)
    } else {
        region
    };

    // Build x and y coordinate arrays from the (possibly reprojected) grid
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
    // Transform coordinates to the requested output CRS
    let mut coverage = CoverageJson {
        type_: edr_protocol::coverage_json::CoverageType::Coverage,
        domain: edr_protocol::Domain::grid_with_crs(
            x_values.clone(),
            y_values.clone(),
            t_values,
            z_axis,
            output_crs,
        ),
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
                // Handle prime meridian crossing for 0-360 grids
                let param_region = extract_prime_meridian_region(&param_region, &grid_bbox);

                // If the grid uses a projected coordinate system, resample to geographic
                let param_region = if projection_type.requires_projection_transform() {
                    resample_to_geographic(
                        &param_region,
                        projection_type,
                        &model_config.model,
                        None,
                    )
                } else {
                    param_region
                };

                // Check if the polygon is a simple rectangle (common case for bbox queries).
                // For rectangles, we can skip the expensive polygon containment check entirely
                // since read_region already returns only data within the requested bbox.
                let simple_rect = is_simple_rectangle(&polygon);

                // Apply polygon mask - set values outside polygon to null
                let values: Vec<Option<f32>> = if simple_rect.is_some() {
                    // For simple rectangles, skip containment check - read_region already
                    // returned data clipped to the bbox. Just convert NaN to None.
                    param_region
                        .data
                        .iter()
                        .map(|&value| if value.is_nan() { None } else { Some(value) })
                        .collect()
                } else {
                    // For complex polygons, we need to check each pixel against the polygon
                    let mut masked = Vec::with_capacity(param_region.data.len());

                    for (idx, &value) in param_region.data.iter().enumerate() {
                        let row = idx / param_region.width;
                        let col = idx % param_region.width;

                        // Calculate lon/lat for this grid cell (now uniformly spaced after reprojection)
                        let lon = param_region.bbox.min_lon
                            + (col as f64 + 0.5) * param_region.resolution.0;
                        let lat = param_region.bbox.max_lat
                            - (row as f64 + 0.5) * param_region.resolution.1;

                        // Normalize longitude to -180/180 range for polygon comparison
                        // (grid may use 0-360 convention like GFS)
                        let lon_normalized = if lon > 180.0 { lon - 360.0 } else { lon };

                        // Check if point is inside the polygon using ray casting algorithm
                        let inside = all_area_queries
                            .iter()
                            .any(|aq| aq.contains_point(lon_normalized, lat));

                        if inside {
                            if value.is_nan() {
                                masked.push(None);
                            } else {
                                masked.push(Some(value));
                            }
                        } else {
                            masked.push(None);
                        }
                    }
                    masked
                };

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
    match output_format {
        OutputFormat::Png => {
            // PNG output - encode data as 16-bit PNG for GPU shaders
            // We already validated that there's exactly one parameter
            //
            // Note: The query-building logic below mirrors the JSON path above (lines ~436-449).
            // While this is duplication, extracting it adds complexity since:
            // - JSON path iterates multiple parameters in a loop with different error handling
            // - PNG path handles exactly one parameter with different response formatting
            // The duplication is intentional to keep each code path self-contained and readable.
            let param_name = &params_to_query[0];

            // Find the parameter definition for units
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

            // Get metadata for units, row origin, and projection
            let metadata = state.grid_data_service.get_metadata(&query).await.ok();
            let units_str = metadata
                .as_ref()
                .map(|m| m.units.clone())
                .unwrap_or_default();
            let row_origin = metadata
                .as_ref()
                .map(|m| m.row_origin)
                .unwrap_or(RowOrigin::North);
            let png_projection_type = metadata
                .as_ref()
                .map(|m| m.projection)
                .unwrap_or(ProjectionType::Geographic);

            // Read the region for this parameter
            let param_region = match state
                .grid_data_service
                .read_region(&query, &grid_bbox, None)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("Failed to read region for PNG: {}", e);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ExceptionResponse::internal_error(format!("Failed to read data: {}", e)),
                    );
                }
            };

            // Check if we got any data (region may be empty if polygon is outside data bounds)
            if param_region.width == 0 || param_region.height == 0 {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!(
                        "No data available for the requested area. The polygon may be outside the collection's geographic bounds. \
                         Requested bbox: [{:.2}, {:.2}, {:.2}, {:.2}]",
                        grid_bbox.min_lon, grid_bbox.min_lat, grid_bbox.max_lon, grid_bbox.max_lat
                    )),
                );
            }

            // Handle prime meridian crossing for 0-360 grids (like GFS)
            let param_region = extract_prime_meridian_region(&param_region, &grid_bbox);

            // Handle dateline crossing for extended notation (e.g., 140° to 235°)
            let param_region = extract_dateline_region(&param_region, &grid_bbox);

            // If the grid uses a projected coordinate system, resample to geographic.
            // This ensures the PNG output has uniformly-spaced lat/lon pixels.
            let param_region = if png_projection_type.requires_projection_transform() {
                tracing::debug!(
                    model = model_config.model.as_str(),
                    projection = %png_projection_type,
                    native_size = format!("{}x{}", param_region.width, param_region.height),
                    "Reprojecting grid data to geographic coordinates for PNG"
                );
                resample_to_geographic(
                    &param_region,
                    png_projection_type,
                    &model_config.model,
                    None,
                )
            } else {
                param_region
            };

            // After reprojection, row_origin is always North (our resampling outputs north-up)
            let row_origin = if png_projection_type.requires_projection_transform() {
                RowOrigin::North
            } else {
                row_origin
            };

            // Check if the polygon is a simple rectangle (common case for bbox queries).
            // For rectangles, we can skip the expensive polygon containment check entirely
            // since read_region already returns only data within the requested bbox.
            let simple_rect = is_simple_rectangle(&polygon);

            // Apply polygon mask - set values outside polygon to None
            let masked_data: Vec<Option<f32>> = if simple_rect.is_some() {
                // For simple rectangles, skip containment check - read_region already
                // returned data clipped to the bbox. Just convert NaN to None.
                param_region
                    .data
                    .iter()
                    .map(|&value| if value.is_nan() { None } else { Some(value) })
                    .collect()
            } else {
                // For complex polygons, we need to check each pixel against the polygon
                let mut masked = Vec::with_capacity(param_region.data.len());

                for (idx, &value) in param_region.data.iter().enumerate() {
                    let row = idx / param_region.width;
                    let col = idx % param_region.width;

                    // Calculate lon/lat for this grid cell (uniformly spaced after reprojection)
                    // Latitude calculation depends on row_origin:
                    // - RowOrigin::North: row 0 is at max_lat (top), increases going south
                    // - RowOrigin::South: row 0 is at min_lat (bottom), increases going north
                    let lon =
                        param_region.bbox.min_lon + (col as f64 + 0.5) * param_region.resolution.0;
                    let lat = match row_origin {
                        RowOrigin::North => {
                            param_region.bbox.max_lat
                                - (row as f64 + 0.5) * param_region.resolution.1
                        }
                        RowOrigin::South => {
                            param_region.bbox.min_lat
                                + (row as f64 + 0.5) * param_region.resolution.1
                        }
                    };

                    // Normalize longitude to -180/180 range for polygon comparison
                    // (grid may use 0-360 convention like GFS)
                    let lon_normalized = if lon > 180.0 { lon - 360.0 } else { lon };

                    // Check if point is inside the polygon using ray casting algorithm
                    let inside = all_area_queries
                        .iter()
                        .any(|aq| aq.contains_point(lon_normalized, lat));

                    if inside && !value.is_nan() {
                        masked.push(Some(value));
                    } else {
                        masked.push(None);
                    }
                }
                masked
            };

            // For PNG output, we need north at the top of the image.
            // If row_origin is South (row 0 = south), flip the rows so north is at top.
            let masked_data = if row_origin == RowOrigin::South {
                flip_rows(&masked_data, param_region.width, param_region.height)
            } else {
                masked_data
            };

            // Determine value range for PNG encoding:
            // - If valid_range is configured, use it for consistent colormap rendering
            // - Otherwise, compute from the actual data (legacy behavior)
            let (min_val, max_val) = if let Some(range) = param_def.and_then(|p| p.valid_range) {
                (range.min, range.max)
            } else {
                compute_data_range(&masked_data)
            };

            // Apply resampling if requested dimensions differ from source
            let (output_data, output_width, output_height) =
                if let Some((req_width, req_height)) = requested_dimensions {
                    if req_width != param_region.width || req_height != param_region.height {
                        let resampled = resample_nearest(
                            &masked_data,
                            param_region.width,
                            param_region.height,
                            req_width,
                            req_height,
                        );
                        (resampled, req_width, req_height)
                    } else {
                        (masked_data, param_region.width, param_region.height)
                    }
                } else {
                    (masked_data, param_region.width, param_region.height)
                };

            // Determine encoding bit depth (default to 16-bit)
            let use_8bit = params.depth == Some(8);
            if let Some(depth) = params.depth {
                if depth != 8 && depth != 16 {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        ExceptionResponse::bad_request(format!(
                            "Invalid depth value: {}. Must be 8 or 16.",
                            depth
                        )),
                    );
                }
            }

            // Transform bbox to the requested output CRS
            let png_bbox = if output_crs.is_projected() {
                // Transform WGS84 bbox corners to Web Mercator
                let (min_x, min_y) = edr_protocol::crs::wgs84_to_mercator(
                    param_region.bbox.min_lon,
                    param_region.bbox.min_lat,
                );
                let (max_x, max_y) = edr_protocol::crs::wgs84_to_mercator(
                    param_region.bbox.max_lon,
                    param_region.bbox.max_lat,
                );
                [min_x, min_y, max_x, max_y]
            } else {
                // Keep in WGS84 degrees
                [
                    param_region.bbox.min_lon,
                    param_region.bbox.min_lat,
                    param_region.bbox.max_lon,
                    param_region.bbox.max_lat,
                ]
            };

            // Encode to PNG using appropriate encoder
            let (png_bytes, encoding_name) = if use_8bit {
                // 8-bit grayscale+alpha (~50% smaller, 256 values)
                let encoder = DataPng8BitEncoder::new(min_val, max_val);
                match encoder.encode_with_metadata(
                    &output_data,
                    output_width,
                    output_height,
                    param_name,
                    &units_str,
                    png_bbox,
                ) {
                    Ok(e) => (e.png_bytes, "uint8"),
                    Err(e) => {
                        tracing::error!("Failed to encode 8-bit PNG: {}", e);
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            ExceptionResponse::internal_error(format!(
                                "Failed to encode PNG: {}",
                                e
                            )),
                        );
                    }
                }
            } else {
                // 16-bit RGBA (default, 65536 values)
                let encoder = DataPngEncoder::new(min_val, max_val);
                match encoder.encode_with_metadata(
                    &output_data,
                    output_width,
                    output_height,
                    param_name,
                    &units_str,
                    png_bbox,
                ) {
                    Ok(e) => (e.png_bytes, "uint16"),
                    Err(e) => {
                        tracing::error!("Failed to encode 16-bit PNG: {}", e);
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            ExceptionResponse::internal_error(format!(
                                "Failed to encode PNG: {}",
                                e
                            )),
                        );
                    }
                }
            };

            // Build response with metadata headers
            // Use per-model cache policy for PNG responses
            let png_cache_max_age = model_config.settings.cache_policy.png_max_age;

            // Record successful request metrics
            state
                .metrics
                .record_request(
                    EndpointType::Area,
                    Some(&collection_id),
                    &params_to_query,
                    format_from_output(&output_format),
                    timer.elapsed_us(),
                    true,
                    client_ip.as_deref(),
                    user_agent.as_deref(),
                )
                .await;

            // Record geographic extent for heatmap
            let bbox = area_query_struct.bbox();
            state
                .metrics
                .record_query_extent(
                    bbox.west,
                    bbox.south,
                    bbox.east,
                    bbox.north,
                    "area",
                    &collection_id,
                )
                .await;

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .header(
                    header::CACHE_CONTROL,
                    format!("max-age={}", png_cache_max_age),
                )
                .header("X-EDR-Parameter", param_name.as_str())
                .header("X-EDR-Units", &units_str)
                .header("X-EDR-Min", format!("{}", min_val))
                .header("X-EDR-Max", format!("{}", max_val))
                .header("X-EDR-Encoding", encoding_name)
                .header(
                    "X-EDR-BBox",
                    format!(
                        "{},{},{},{}",
                        png_bbox[0], png_bbox[1], png_bbox[2], png_bbox[3]
                    ),
                )
                .header("X-EDR-CRS", output_crs.code())
                .header("X-EDR-Width", format!("{}", output_width))
                .header("X-EDR-Height", format!("{}", output_height))
                .body(png_bytes.into())
                .unwrap()
        }
        OutputFormat::GeoJson => {
            let geojson = EdrFeatureCollection::from(&coverage);
            match serde_json::to_string_pretty(&geojson) {
                Ok(json) => {
                    // Record successful request metrics
                    state
                        .metrics
                        .record_request(
                            EndpointType::Area,
                            Some(&collection_id),
                            &params_to_query,
                            format_from_output(&output_format),
                            timer.elapsed_us(),
                            true,
                            client_ip.as_deref(),
                            user_agent.as_deref(),
                        )
                        .await;

                    // Record geographic extent for heatmap
                    let bbox = area_query_struct.bbox();
                    state
                        .metrics
                        .record_query_extent(
                            bbox.west,
                            bbox.south,
                            bbox.east,
                            bbox.north,
                            "area",
                            &collection_id,
                        )
                        .await;

                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, output_format.content_type())
                        .header(header::CACHE_CONTROL, "max-age=300")
                        .body(json.into())
                        .unwrap()
                }
                Err(e) => {
                    tracing::error!("Failed to serialize GeoJSON: {}", e);
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ExceptionResponse::internal_error("Failed to serialize response"),
                    )
                }
            }
        }
        OutputFormat::CoverageJson => match serde_json::to_string_pretty(&coverage) {
            Ok(json) => {
                // Record successful request metrics
                state
                    .metrics
                    .record_request(
                        EndpointType::Area,
                        Some(&collection_id),
                        &params_to_query,
                        format_from_output(&output_format),
                        timer.elapsed_us(),
                        true,
                        client_ip.as_deref(),
                        user_agent.as_deref(),
                    )
                    .await;

                // Record geographic extent for heatmap
                let bbox = area_query_struct.bbox();
                state
                    .metrics
                    .record_query_extent(
                        bbox.west,
                        bbox.south,
                        bbox.east,
                        bbox.north,
                        "area",
                        &collection_id,
                    )
                    .await;

                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, output_format.content_type())
                    .header(header::CACHE_CONTROL, "max-age=300")
                    .body(json.into())
                    .unwrap()
            }
            Err(e) => {
                tracing::error!("Failed to serialize CoverageJSON: {}", e);
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ExceptionResponse::internal_error("Failed to serialize response"),
                )
            }
        },
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
    fn test_parse_polygon() {
        let polygon =
            AreaQuery::parse_polygon("POLYGON((-100 35, -98 35, -98 37, -100 37, -100 35))")
                .unwrap();
        assert_eq!(polygon.len(), 5);
        assert_eq!(polygon[0], (-100.0, 35.0));
    }

    #[test]
    fn test_polygon_bbox() {
        let area_query = AreaQuery {
            polygon: vec![
                (-100.0, 35.0),
                (-98.0, 35.0),
                (-98.0, 37.0),
                (-100.0, 37.0),
                (-100.0, 35.0),
            ],
            z: None,
            datetime: None,
            parameter_names: None,
            crs: None,
        };

        let bbox = area_query.bbox();
        assert_eq!(bbox.west, -100.0);
        assert_eq!(bbox.east, -98.0);
        assert_eq!(bbox.south, 35.0);
        assert_eq!(bbox.north, 37.0);
    }

    #[test]
    fn test_polygon_contains_point() {
        let area_query = AreaQuery {
            polygon: vec![
                (-100.0, 35.0),
                (-98.0, 35.0),
                (-98.0, 37.0),
                (-100.0, 37.0),
                (-100.0, 35.0),
            ],
            z: None,
            datetime: None,
            parameter_names: None,
            crs: None,
        };

        assert!(area_query.contains_point(-99.0, 36.0));
        assert!(!area_query.contains_point(-101.0, 36.0));
    }

    #[test]
    fn test_png_encoder_integration() {
        // Test that the PNG encoder works with typical area query data
        let data: Vec<Option<f32>> = vec![
            Some(10.0),
            Some(15.0),
            None, // Outside polygon
            Some(20.0),
            Some(12.5),
            Some(17.5),
            None,
            Some(22.5),
            Some(11.0),
        ];

        let (min_val, max_val) = compute_data_range(&data);
        assert_eq!(min_val, 10.0);
        assert_eq!(max_val, 22.5);

        let encoder = DataPngEncoder::new(min_val, max_val);
        let result = encoder.encode_with_metadata(
            &data,
            3,
            3,
            "temperature",
            "K",
            [-100.0, 35.0, -98.0, 37.0],
        );

        assert!(result.is_ok());
        let encoded = result.unwrap();

        // Verify PNG signature
        assert_eq!(&encoded.png_bytes[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);

        // Verify metadata
        assert_eq!(encoded.metadata.parameter_name, "temperature");
        assert_eq!(encoded.metadata.units, "K");
        assert_eq!(encoded.metadata.width, 3);
        assert_eq!(encoded.metadata.height, 3);
    }

    #[test]
    fn test_png_encoder_all_null_data() {
        // Test handling of completely masked/no-data regions
        let data: Vec<Option<f32>> = vec![None, None, None, None];

        let (min_val, max_val) = compute_data_range(&data);
        // Should use default range for all-null data
        assert_eq!(min_val, 0.0);
        assert_eq!(max_val, 1.0);

        let encoder = DataPngEncoder::new(min_val, max_val);
        let result = encoder.encode(&data, 2, 2);

        assert!(result.is_ok());
        // Should produce a valid (transparent) PNG
        let encoded = result.unwrap();
        assert!(!encoded.png_bytes.is_empty());
    }

    #[test]
    fn test_resample_nearest_upsample() {
        // 2x2 -> 4x4 (double size)
        let data: Vec<Option<f32>> = vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)];

        let result = resample_nearest(&data, 2, 2, 4, 4);
        assert_eq!(result.len(), 16);

        // Top-left quadrant should all be 1.0
        assert_eq!(result[0], Some(1.0));
        assert_eq!(result[1], Some(1.0));
        assert_eq!(result[4], Some(1.0));
        assert_eq!(result[5], Some(1.0));

        // Bottom-right quadrant should all be 4.0
        assert_eq!(result[10], Some(4.0));
        assert_eq!(result[11], Some(4.0));
        assert_eq!(result[14], Some(4.0));
        assert_eq!(result[15], Some(4.0));
    }

    #[test]
    fn test_resample_nearest_downsample() {
        // 4x4 -> 2x2 (half size)
        let data: Vec<Option<f32>> = vec![
            Some(1.0),
            Some(1.0),
            Some(2.0),
            Some(2.0),
            Some(1.0),
            Some(1.0),
            Some(2.0),
            Some(2.0),
            Some(3.0),
            Some(3.0),
            Some(4.0),
            Some(4.0),
            Some(3.0),
            Some(3.0),
            Some(4.0),
            Some(4.0),
        ];

        let result = resample_nearest(&data, 4, 4, 2, 2);
        assert_eq!(result.len(), 4);

        // Should pick center-ish pixels
        assert_eq!(result[0], Some(1.0));
        assert_eq!(result[1], Some(2.0));
        assert_eq!(result[2], Some(3.0));
        assert_eq!(result[3], Some(4.0));
    }

    #[test]
    fn test_resample_nearest_preserves_none() {
        // Ensure None values are preserved through resampling
        let data: Vec<Option<f32>> = vec![Some(1.0), None, None, Some(4.0)];

        let result = resample_nearest(&data, 2, 2, 4, 4);
        assert_eq!(result.len(), 16);

        // Top-left should be Some(1.0)
        assert_eq!(result[0], Some(1.0));

        // Top-right quadrant should be None
        assert_eq!(result[2], None);
        assert_eq!(result[3], None);

        // Bottom-right should be Some(4.0)
        assert_eq!(result[15], Some(4.0));
    }

    #[test]
    fn test_resample_nearest_same_size() {
        // Same size should return equivalent data
        let data: Vec<Option<f32>> = vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)];

        let result = resample_nearest(&data, 2, 2, 2, 2);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], Some(1.0));
        assert_eq!(result[1], Some(2.0));
        assert_eq!(result[2], Some(3.0));
        assert_eq!(result[3], Some(4.0));
    }

    #[test]
    fn test_max_png_dimension_constant() {
        assert_eq!(MAX_PNG_DIMENSION, 4096);
    }

    #[test]
    fn test_flip_rows_2x2() {
        // Input: row 0 = [1, 2], row 1 = [3, 4]
        // Output: row 0 = [3, 4], row 1 = [1, 2]
        let data: Vec<Option<f32>> = vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)];

        let result = flip_rows(&data, 2, 2);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], Some(3.0)); // First row is now last row
        assert_eq!(result[1], Some(4.0));
        assert_eq!(result[2], Some(1.0)); // Last row is now first row
        assert_eq!(result[3], Some(2.0));
    }

    #[test]
    fn test_flip_rows_3x2() {
        // Input: row 0 = [1, 2, 3], row 1 = [4, 5, 6]
        // Output: row 0 = [4, 5, 6], row 1 = [1, 2, 3]
        let data: Vec<Option<f32>> = vec![
            Some(1.0),
            Some(2.0),
            Some(3.0),
            Some(4.0),
            Some(5.0),
            Some(6.0),
        ];

        let result = flip_rows(&data, 3, 2);
        assert_eq!(result.len(), 6);
        assert_eq!(result[0], Some(4.0));
        assert_eq!(result[1], Some(5.0));
        assert_eq!(result[2], Some(6.0));
        assert_eq!(result[3], Some(1.0));
        assert_eq!(result[4], Some(2.0));
        assert_eq!(result[5], Some(3.0));
    }

    #[test]
    fn test_flip_rows_preserves_none() {
        // Ensure None values are preserved
        let data: Vec<Option<f32>> = vec![Some(1.0), None, None, Some(4.0)];

        let result = flip_rows(&data, 2, 2);
        assert_eq!(result[0], None);
        assert_eq!(result[1], Some(4.0));
        assert_eq!(result[2], Some(1.0));
        assert_eq!(result[3], None);
    }

    #[test]
    fn test_is_simple_rectangle() {
        // Standard rectangle (counterclockwise)
        let rect = vec![
            (-10.0, 45.0),
            (10.0, 45.0),
            (10.0, 55.0),
            (-10.0, 55.0),
            (-10.0, 45.0), // closing point
        ];
        let result = is_simple_rectangle(&rect);
        assert!(result.is_some());
        let (west, south, east, north) = result.unwrap();
        assert!((west - (-10.0)).abs() < 1e-6);
        assert!((south - 45.0).abs() < 1e-6);
        assert!((east - 10.0).abs() < 1e-6);
        assert!((north - 55.0).abs() < 1e-6);
    }

    #[test]
    fn test_is_simple_rectangle_clockwise() {
        // Rectangle with clockwise winding
        let rect = vec![
            (-10.0, 45.0),
            (-10.0, 55.0),
            (10.0, 55.0),
            (10.0, 45.0),
            (-10.0, 45.0),
        ];
        let result = is_simple_rectangle(&rect);
        assert!(result.is_some());
    }

    #[test]
    fn test_is_simple_rectangle_not_closed() {
        // Not closed (missing closing point)
        let rect = vec![(-10.0, 45.0), (10.0, 45.0), (10.0, 55.0), (-10.0, 55.0)];
        let result = is_simple_rectangle(&rect);
        assert!(result.is_none());
    }

    #[test]
    fn test_is_simple_rectangle_triangle() {
        // Triangle, not rectangle
        let tri = vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0), (0.0, 0.0)];
        let result = is_simple_rectangle(&tri);
        assert!(result.is_none());
    }

    #[test]
    fn test_is_simple_rectangle_diamond() {
        // Diamond shape (4 corners but not axis-aligned)
        let diamond = vec![(0.0, 5.0), (5.0, 0.0), (10.0, 5.0), (5.0, 10.0), (0.0, 5.0)];
        let result = is_simple_rectangle(&diamond);
        assert!(result.is_none());
    }

    #[test]
    fn test_bbox_contains_point_inside() {
        assert!(bbox_contains_point(-10.0, 45.0, 10.0, 55.0, 0.0, 50.0));
    }

    #[test]
    fn test_bbox_contains_point_on_edge() {
        // Points exactly on the edge should be included (with epsilon)
        assert!(bbox_contains_point(-10.0, 45.0, 10.0, 55.0, -10.0, 50.0)); // west edge
        assert!(bbox_contains_point(-10.0, 45.0, 10.0, 55.0, 10.0, 50.0)); // east edge
        assert!(bbox_contains_point(-10.0, 45.0, 10.0, 55.0, 0.0, 45.0)); // south edge
        assert!(bbox_contains_point(-10.0, 45.0, 10.0, 55.0, 0.0, 55.0)); // north edge
    }

    #[test]
    fn test_bbox_contains_point_corners() {
        // Corners should be included
        assert!(bbox_contains_point(-10.0, 45.0, 10.0, 55.0, -10.0, 45.0)); // SW
        assert!(bbox_contains_point(-10.0, 45.0, 10.0, 55.0, 10.0, 45.0)); // SE
        assert!(bbox_contains_point(-10.0, 45.0, 10.0, 55.0, -10.0, 55.0)); // NW
        assert!(bbox_contains_point(-10.0, 45.0, 10.0, 55.0, 10.0, 55.0)); // NE
    }

    #[test]
    fn test_bbox_contains_point_outside() {
        assert!(!bbox_contains_point(-10.0, 45.0, 10.0, 55.0, -20.0, 50.0)); // west
        assert!(!bbox_contains_point(-10.0, 45.0, 10.0, 55.0, 20.0, 50.0)); // east
        assert!(!bbox_contains_point(-10.0, 45.0, 10.0, 55.0, 0.0, 40.0)); // south
        assert!(!bbox_contains_point(-10.0, 45.0, 10.0, 55.0, 0.0, 60.0)); // north
    }
}
