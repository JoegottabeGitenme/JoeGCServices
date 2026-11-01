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
use grid_processor::service::MATCH_TOLERANCE;
use grid_processor::{
    AreaData, AreaReadOptions, BoundingBox, DatasetQuery, GridProcessorError, GridRegion,
    ProjectionType, RowOrigin,
};
use renderer::data_png::{compute_data_range, DataPng8BitEncoder, DataPngEncoder};
use serde::Deserialize;
use std::sync::Arc;

use crate::availability::ModelAvailability;
use crate::cache_policy::{confirm_match, impl_time_selectors, TimeSelection, TimeSelectors};
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

    /// Cut a projected grid (HRRR) to the polygon's bounding box (PNG only; `true` or `false`).
    ///
    /// Without it a projected grid answers with the WHOLE model domain whatever polygon is sent,
    /// and `X-EDR-BBox` says which. With `crop=true` the image covers exactly the polygon's
    /// bounding box (all of a MULTIPOLYGON's polygons) at `width` x `height`, `X-EDR-BBox` is that
    /// box, and pixels outside the model domain are no-data. A lat/lon grid is always cut to the
    /// polygon, so it makes no difference.
    pub crop: Option<String>,

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

    /// linear-features-only: `?conditions=latest` merges trail-physics
    /// output into each feature's properties. See
    /// `linear_features::TrailAreaParams::conditions`.
    pub conditions: Option<String>,

    /// Feature collections (trails, storm events) and point-observation
    /// collections: maximum features returned. Ignored by gridded collections.
    pub limit: Option<i64>,

    /// linear-features-only: `feature_class` filter (e.g. `mtb_trail`).
    pub class: Option<String>,

    /// lightning-only: server-relative time window, an ISO-8601 duration
    /// (e.g. `PT10M`). See `lightning` module docs.
    pub window: Option<String>,

    /// lightning-only: change-feed cursor -- only flashes with `id` greater than this.
    pub after: Option<i64>,

    /// lightning-only: `goes-east` (default), `goes-west` or `both`.
    pub satellite: Option<String>,
}

// `run` is validated but the newest run is queried, so it does not pin the time
// for caching (see cache_policy).
impl_time_selectors!(AreaQueryParams, ignores_run);

// Pure translations into each backend's params -- see the matching comment in
// radius.rs for why these are functions and not inline struct literals.

fn trail_area_params(p: &AreaQueryParams) -> crate::handlers::linear_features::TrailAreaParams {
    crate::handlers::linear_features::TrailAreaParams {
        coords: p.coords.clone(),
        limit: p.limit,
        class: p.class.clone(),
        f: p.f.clone(),
        conditions: p.conditions.clone(),
    }
}

fn lightning_area_params(p: &AreaQueryParams) -> crate::handlers::lightning::LightningAreaParams {
    crate::handlers::lightning::LightningAreaParams {
        coords: p.coords.clone(),
        datetime: p.datetime.clone(),
        window: p.window.clone(),
        after: p.after,
        satellite: p.satellite.clone(),
        limit: p.limit,
        f: p.f.clone(),
    }
}

fn storm_area_params(p: &AreaQueryParams) -> crate::handlers::storm_events::StormAreaParams {
    crate::handlers::storm_events::StormAreaParams {
        coords: p.coords.clone(),
        datetime: p.datetime.clone(),
        limit: p.limit,
        f: p.f.clone(),
    }
}

fn obs_area_params(p: &AreaQueryParams) -> ObsAreaQueryParams {
    ObsAreaQueryParams {
        coords: p.coords.clone().unwrap_or_default(),
        datetime: p.datetime.clone(),
        parameter_name: p.parameter_name.clone(),
        f: p.f.clone(),
        limit: p.limit,
    }
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
    // or a feature collection (storm events: hail/wind/tornado).
    {
        let config = state.edr_config.read().await;
        if let Some((model_config, _)) = config.find_collection(&collection_id) {
            if model_config.data_type.is_feature_data() {
                if model_config.observation_source.as_deref() == Some("lightning") {
                    let lightning_params = lightning_area_params(&params);
                    drop(config);
                    return crate::handlers::lightning::lightning_area_handler(
                        Extension(state.clone()),
                        Path(collection_id),
                        Query(lightning_params),
                    )
                    .await;
                }
                if model_config.observation_source.as_deref() == Some("linear_features") {
                    let trail_params = trail_area_params(&params);
                    drop(config);
                    return crate::handlers::linear_features::trail_area_handler(
                        Extension(state.clone()),
                        Path(collection_id),
                        Query(trail_params),
                    )
                    .await;
                }
                let storm_params = storm_area_params(&params);
                drop(config);
                return crate::handlers::storm_events::storm_area_handler(
                    Extension(state.clone()),
                    Path(collection_id),
                    Query(storm_params),
                )
                .await;
            }
            if model_config.data_type.is_point_data() {
                // Convert the coords to format observation handler expects
                let obs_params = obs_area_params(&params);
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

    // Whether this query named its time decides how long a CDN may keep the
    // response. See cache_policy.
    let time_selection = params.requested_time(instance_id.as_deref()).selection();

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

    // Parse crop (PNG only; the default is the whole domain for a projected grid)
    let crop = match params.crop.as_deref().map(str::trim) {
        None => false,
        Some(v) if v.eq_ignore_ascii_case("false") => false,
        Some(v) if v.eq_ignore_ascii_case("true") => {
            if output_format != OutputFormat::Png {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request("crop is only supported for PNG output (f=png)"),
                );
            }
            true
        }
        Some(v) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "Invalid crop value: {}. Must be true or false.",
                    v
                )),
            );
        }
    };

    // Get the bbox of the polygon for grid queries
    let bbox = area_query_struct.bbox();
    // The box a cropped image covers: the request's, every polygon of a MULTIPOLYGON (the grid
    // queries above have always been made for the first polygon's box)
    let crop_box = crop.then(|| union_bbox(&all_area_queries));

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

    // An instance is one run, and a `datetime` list or interval was expanded against the times of
    // EVERY run of the model (above), so its first time may be one this run does not hold: the
    // model's earliest instant is usually an older run's. Keep the times the run does hold; a span
    // with none of them is a time the run does not hold, as for a single instant.
    let time_strings = match (reference_time, datetime_query.as_ref()) {
        (Some(run), Some(dq)) if dq.is_multi_time() => {
            let run_times = match state
                .catalog
                .get_run_valid_times(&model_config.model, run)
                .await
            {
                Ok(times) => times,
                Err(e) => {
                    tracing::error!("Failed to look up the times of instance {}: {}", run, e);
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ExceptionResponse::internal_error("Failed to look up the instance's times"),
                    );
                }
            };
            let held = times_the_run_holds(dq, &run_times);
            if held.is_empty() {
                return error_response(
                    StatusCode::NOT_FOUND,
                    ExceptionResponse::not_found(format!(
                        "Instance {} of collection {} holds no data for datetime {}",
                        run.format("%Y-%m-%dT%H:%M:%SZ"),
                        collection_id,
                        params.datetime.as_deref().unwrap_or_default()
                    )),
                );
            }
            held
        }
        _ => time_strings,
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

    // The dataset query for one parameter: its level, and the time and run the request names.
    // An instance is one model run, so it is read at exactly that run (its first forecast when no
    // time is named); a time alone is the nearest dataset of any run.
    let query_for = |param_name: &str| {
        let param_def = collection_def
            .parameters
            .iter()
            .find(|p| p.name == param_name);
        let level_str = build_level_string(&collection_def.level_filter, param_def, z_val);

        // Build the DatasetQuery using the appropriate type (forecast vs observation)
        let mut query = model_config.create_query(param_name);

        if let Some(level) = &level_str {
            query = query.at_level(level);
        }

        match (reference_time, query_time) {
            (Some(run), valid_time) => query.in_instance(run, valid_time),
            (None, Some(valid_time)) => query.at_valid_time(valid_time),
            (None, None) => query,
        }
    };

    // Read the region
    let grid_bbox = BoundingBox::new(bbox.west, bbox.south, bbox.east, bbox.north);

    // Read the first parameter ONCE: the dataset's metadata (units, row order, projection), the
    // time of the dataset the catalog matched and its cells come from one catalog lookup and one
    // read. A PNG is made from this alone; a CoverageJSON body takes its grid axes and its first
    // parameter from it and reads the rest.
    let read = match state
        .grid_data_service
        .read_area(
            &query_for(first_param),
            &grid_bbox,
            AreaReadOptions {
                output_size: requested_dimensions,
                crop: crop_box,
            },
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return handle_grid_read_error(e, "read region");
        }
    };
    let projection_type = read.metadata.projection;

    // A datetime resolves to the NEAREST dataset, so the response is that instant's (and may be
    // cached as such) only if the dataset matched is valid at it. A datetime that named no
    // instant we could look up (an interval with nothing in it) was answered with the newest
    // dataset.
    let time_selection = match (time_selection, query_time) {
        (TimeSelection::Explicit, None)
            if params.datetime.is_some() && reference_time.is_none() =>
        {
            TimeSelection::Latest
        }
        _ => confirm_match(time_selection, query_time, read.valid_time),
    };

    // A cropped image is the box around EVERY polygon of a MULTIPOLYGON; any other image is the
    // first polygon's box (or the whole domain), which is all a rectangular first polygon clips to
    let spans_every_polygon =
        matches!(read.data, AreaData::Window(_)) && all_area_queries.len() > 1;

    // The cells as a regular lat/lon grid.
    let region = match read.data {
        // A projected grid cut to the box: sampled onto the box itself, at the size asked for
        // (the grid's own cell spacing when none was)
        AreaData::Window(window) => {
            let (width, height) = requested_dimensions.unwrap_or(window.cells);
            tracing::debug!(
                model = model_config.model.as_str(),
                projection = %projection_type,
                level = read.level,
                native_size = format!("{}x{}", window.width, window.height),
                "Reprojecting cropped grid window to geographic coordinates"
            );
            window.resample(
                &crop_box.unwrap_or(grid_bbox),
                width.clamp(1, MAX_PNG_DIMENSION),
                height.clamp(1, MAX_PNG_DIMENSION),
            )
        }
        AreaData::Region(region) => {
            // Check if we got any data (region may be empty if polygon is outside data bounds)
            if output_format == OutputFormat::Png && (region.width == 0 || region.height == 0) {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!(
                        "No data available for the requested area. The polygon may be outside the collection's geographic bounds. \
                         Requested bbox: [{:.2}, {:.2}, {:.2}, {:.2}]",
                        grid_bbox.min_lon, grid_bbox.min_lat, grid_bbox.max_lon, grid_bbox.max_lat
                    )),
                );
            }
            into_geographic(region, projection_type, &model_config.model, &grid_bbox)
        }
    };

    if output_format == OutputFormat::Png {
        // PNG output - encode data as 16-bit PNG for GPU shaders
        // We already validated that there's exactly one parameter
        let param_name = &params_to_query[0];

        // After reprojection, row_origin is always North (our resampling outputs north-up)
        let row_origin = if projection_type.requires_projection_transform() {
            RowOrigin::North
        } else {
            read.metadata.row_origin
        };

        let valid_range = collection_def
            .parameters
            .iter()
            .find(|p| p.name == *param_name)
            .and_then(|p| p.valid_range)
            .map(|range| (range.min, range.max));

        let png = match encode_area_png(
            &region,
            row_origin,
            &polygon,
            &all_area_queries,
            spans_every_polygon,
            valid_range,
            requested_dimensions,
            params.depth,
            output_crs,
            param_name,
            &read.metadata.units,
        ) {
            Ok(png) => png,
            Err(failure) => return failure.into_response(),
        };

        // Build response with metadata headers
        // Use per-model cache policy for PNG responses (capped for the latest data)
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

        // Which dataset this is: the catalog's match for the request, which is not always the time
        // that was asked for (nearest scan or step, or the newest when none was). An observation's
        // run time is its own valid time, so only a forecast says which run it came from.
        let mut response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/png")
            .header(
                header::CACHE_CONTROL,
                time_selection.cache_control(png_cache_max_age),
            )
            .header("X-EDR-Valid-Time", rfc3339_utc(read.valid_time));
        if !model_config.data_type.is_observation() {
            response = response.header("X-EDR-Reference-Time", rfc3339_utc(read.reference_time));
        }
        return response
            .header("X-EDR-Parameter", param_name.as_str())
            .header("X-EDR-Units", &read.metadata.units)
            .header("X-EDR-Min", format!("{}", png.min))
            .header("X-EDR-Max", format!("{}", png.max))
            .header("X-EDR-Encoding", png.encoding)
            .header(
                "X-EDR-BBox",
                format!(
                    "{},{},{},{}",
                    png.bbox[0], png.bbox[1], png.bbox[2], png.bbox[3]
                ),
            )
            .header("X-EDR-CRS", output_crs.code())
            .header("X-EDR-Width", format!("{}", png.width))
            .header("X-EDR-Height", format!("{}", png.height))
            .body(png.bytes.into())
            .unwrap();
    }

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

    // For each parameter, query the data and add to coverage. The first parameter is the one
    // already read above; the others are read here.
    let mut first_read = Some((read.metadata.units.clone(), region));
    for param_name in &params_to_query {
        let loaded = match first_read.take() {
            Some(first) => Ok(first),
            None => {
                read_geographic(
                    &state,
                    &query_for(param_name),
                    &grid_bbox,
                    &model_config.model,
                )
                .await
            }
        };

        match loaded {
            Ok((units_str, param_region)) => {
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
    if output_format == OutputFormat::GeoJson {
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
                    .header(header::CACHE_CONTROL, time_selection.cache_control(300))
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
    } else {
        match serde_json::to_string_pretty(&coverage) {
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
                    .header(header::CACHE_CONTROL, time_selection.cache_control(300))
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
        }
    }
}

/// `time` as RFC 3339 in UTC with a `Z`, to the second (MRMS scans have seconds).
fn rfc3339_utc(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The cells of an area read as a regular lat/lon grid: a 0-360 grid cut at the prime meridian
/// and the dateline, and a projected grid (Lambert, Polar Stereographic, ...) resampled so that
/// EDR output coordinates are uniformly spaced in lat/lon, which is what clients expect.
fn into_geographic(
    region: GridRegion,
    projection_type: ProjectionType,
    model: &str,
    grid_bbox: &BoundingBox,
) -> GridRegion {
    // If the request crosses the prime meridian on a 0-360 grid (like GFS),
    // extract just the requested region. The grid reader returns the full grid
    // in this case, so we need to extract the appropriate columns.
    let region = extract_prime_meridian_region(&region, grid_bbox);

    // If the request uses extended notation crossing the dateline (e.g., 140° to 235°),
    // convert the output bbox to -180/180 format (e.g., 140° to -125°).
    let region = extract_dateline_region(&region, grid_bbox);

    if projection_type.requires_projection_transform() {
        tracing::debug!(
            model = model,
            projection = %projection_type,
            native_size = format!("{}x{}", region.width, region.height),
            "Reprojecting grid data to geographic coordinates"
        );
        resample_to_geographic(&region, projection_type, model, None)
    } else {
        region
    }
}

/// The smallest box that holds the bounding box of every polygon of a request.
fn union_bbox(polygons: &[AreaQuery]) -> BoundingBox {
    let boxes: Vec<_> = polygons.iter().map(AreaQuery::bbox).collect();
    BoundingBox::new(
        boxes.iter().map(|b| b.west).fold(f64::INFINITY, f64::min),
        boxes.iter().map(|b| b.south).fold(f64::INFINITY, f64::min),
        boxes
            .iter()
            .map(|b| b.east)
            .fold(f64::NEG_INFINITY, f64::max),
        boxes
            .iter()
            .map(|b| b.north)
            .fold(f64::NEG_INFINITY, f64::max),
    )
}

/// The times of a `datetime` list or interval that a model run holds (`run_times`, which includes
/// every parameter's), in the order the request gave them (an interval: ascending). A time of a
/// list is held if the run has an instant within [`MATCH_TOLERANCE`] of it.
fn times_the_run_holds(datetime: &DateTimeQuery, run_times: &[DateTime<Utc>]) -> Vec<String> {
    if datetime.is_interval() {
        let held: Vec<String> = run_times
            .iter()
            .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .collect();
        return datetime.expand_against_available_times(&held);
    }
    datetime
        .to_vec()
        .into_iter()
        .filter(|time| {
            DateTime::parse_from_rfc3339(time).is_ok_and(|asked| {
                let asked = asked.with_timezone(&Utc);
                run_times
                    .iter()
                    .any(|t| (*t - asked).abs() <= MATCH_TOLERANCE)
            })
        })
        .collect()
}

/// Read one more parameter of a CoverageJSON body: its units and its cells as a regular lat/lon
/// grid. Always the whole read; only a PNG is cropped or sized.
async fn read_geographic(
    state: &AppState,
    query: &DatasetQuery,
    grid_bbox: &BoundingBox,
    model: &str,
) -> Result<(String, GridRegion), GridProcessorError> {
    let read = state
        .grid_data_service
        .read_area(query, grid_bbox, AreaReadOptions::default())
        .await?;
    match read.data {
        AreaData::Region(region) => Ok((
            read.metadata.units,
            into_geographic(region, read.metadata.projection, model, grid_bbox),
        )),
        AreaData::Window(_) => Err(GridProcessorError::read_failed(
            "a window came back from a read that did not ask for one",
        )),
    }
}

/// Why an area PNG could not be made.
enum PngFailure {
    /// `depth` was neither 8 nor 16
    BadDepth(u8),
    /// The encoder refused the image
    Encode(String),
}

impl PngFailure {
    fn into_response(self) -> Response {
        match self {
            PngFailure::BadDepth(depth) => error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!(
                    "Invalid depth value: {}. Must be 8 or 16.",
                    depth
                )),
            ),
            PngFailure::Encode(e) => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error(format!("Failed to encode PNG: {}", e)),
            ),
        }
    }
}

/// A PNG made from an area, and the numbers its headers report.
struct AreaPng {
    bytes: Vec<u8>,
    /// `uint16` or `uint8`
    encoding: &'static str,
    /// The value range the PNG's pixel values are scaled over
    min: f32,
    max: f32,
    /// Extent of the image in the output CRS: west, south, east, north
    bbox: [f64; 4],
    width: usize,
    height: usize,
}

/// Mask `region` to the polygon, size it, and encode it as a data PNG.
///
/// `region` is a regular lat/lon grid (see [`into_geographic`]) whose row order is `row_origin`.
/// `spans_every_polygon` is whether it is the box around all of a MULTIPOLYGON's polygons, see
/// [`mask_to_polygons`].
#[allow(clippy::too_many_arguments)]
fn encode_area_png(
    region: &GridRegion,
    row_origin: RowOrigin,
    polygon: &[(f64, f64)],
    all_area_queries: &[AreaQuery],
    spans_every_polygon: bool,
    valid_range: Option<(f32, f32)>,
    requested_dimensions: Option<(usize, usize)>,
    depth: Option<u8>,
    output_crs: edr_protocol::OutputCrs,
    param_name: &str,
    units: &str,
) -> Result<AreaPng, PngFailure> {
    let masked_data = mask_to_polygons(
        region,
        row_origin,
        polygon,
        all_area_queries,
        spans_every_polygon,
    );

    // For PNG output, we need north at the top of the image.
    // If row_origin is South (row 0 = south), flip the rows so north is at top.
    let masked_data = if row_origin == RowOrigin::South {
        flip_rows(&masked_data, region.width, region.height)
    } else {
        masked_data
    };

    // Determine value range for PNG encoding:
    // - If valid_range is configured, use it for consistent colormap rendering
    // - Otherwise, compute from the actual data (legacy behavior)
    let (min_val, max_val) = if let Some(range) = valid_range {
        range
    } else {
        compute_data_range(&masked_data)
    };

    // Apply resampling if requested dimensions differ from source
    let (output_data, output_width, output_height) =
        if let Some((req_width, req_height)) = requested_dimensions {
            if req_width != region.width || req_height != region.height {
                let resampled = resample_nearest(
                    &masked_data,
                    region.width,
                    region.height,
                    req_width,
                    req_height,
                );
                (resampled, req_width, req_height)
            } else {
                (masked_data, region.width, region.height)
            }
        } else {
            (masked_data, region.width, region.height)
        };

    // Determine encoding bit depth (default to 16-bit)
    let use_8bit = depth == Some(8);
    if let Some(depth) = depth {
        if depth != 8 && depth != 16 {
            return Err(PngFailure::BadDepth(depth));
        }
    }

    // Transform bbox to the requested output CRS
    let png_bbox = if output_crs.is_projected() {
        // Transform WGS84 bbox corners to Web Mercator
        let (min_x, min_y) =
            edr_protocol::crs::wgs84_to_mercator(region.bbox.min_lon, region.bbox.min_lat);
        let (max_x, max_y) =
            edr_protocol::crs::wgs84_to_mercator(region.bbox.max_lon, region.bbox.max_lat);
        [min_x, min_y, max_x, max_y]
    } else {
        // Keep in WGS84 degrees
        [
            region.bbox.min_lon,
            region.bbox.min_lat,
            region.bbox.max_lon,
            region.bbox.max_lat,
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
            units,
            png_bbox,
        ) {
            Ok(e) => (e.png_bytes, "uint8"),
            Err(e) => {
                tracing::error!("Failed to encode 8-bit PNG: {}", e);
                return Err(PngFailure::Encode(e));
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
            units,
            png_bbox,
        ) {
            Ok(e) => (e.png_bytes, "uint16"),
            Err(e) => {
                tracing::error!("Failed to encode 16-bit PNG: {}", e);
                return Err(PngFailure::Encode(e));
            }
        }
    };

    Ok(AreaPng {
        bytes: png_bytes,
        encoding: encoding_name,
        min: min_val,
        max: max_val,
        bbox: png_bbox,
        width: output_width,
        height: output_height,
    })
}

/// The cells of `region` inside the request's polygons, `None` elsewhere (and where there is no
/// data), in the row order of `region`.
///
/// A rectangular `polygon` (the first of the request) is not checked cell by cell when `region` is
/// the box that was read for it: it holds only cells inside. A region that is the box around ALL of
/// a MULTIPOLYGON's polygons (`spans_every_polygon`, a cropped image) also holds cells outside
/// every polygon, even when the first polygon is a rectangle, so it is always checked.
fn mask_to_polygons(
    region: &GridRegion,
    row_origin: RowOrigin,
    polygon: &[(f64, f64)],
    all_area_queries: &[AreaQuery],
    spans_every_polygon: bool,
) -> Vec<Option<f32>> {
    // Check if the polygon is a simple rectangle (common case for bbox queries).
    // For rectangles, we can skip the expensive polygon containment check entirely
    // since read_region already returns only data within the requested bbox.
    let simple_rect = is_simple_rectangle(polygon).filter(|_| !spans_every_polygon);

    // Apply polygon mask - set values outside polygon to None
    if simple_rect.is_some() {
        // For simple rectangles, skip containment check - read_region already
        // returned data clipped to the bbox. Just convert NaN to None.
        region
            .data
            .iter()
            .map(|&value| if value.is_nan() { None } else { Some(value) })
            .collect()
    } else {
        // For complex polygons, we need to check each pixel against the polygon
        let mut masked = Vec::with_capacity(region.data.len());

        for (idx, &value) in region.data.iter().enumerate() {
            let row = idx / region.width;
            let col = idx % region.width;

            // Calculate lon/lat for this grid cell (uniformly spaced after reprojection)
            // Latitude calculation depends on row_origin:
            // - RowOrigin::North: row 0 is at max_lat (top), increases going south
            // - RowOrigin::South: row 0 is at min_lat (bottom), increases going north
            let lon = region.bbox.min_lon + (col as f64 + 0.5) * region.resolution.0;
            let lat = match row_origin {
                RowOrigin::North => region.bbox.max_lat - (row as f64 + 0.5) * region.resolution.1,
                RowOrigin::South => region.bbox.min_lat + (row as f64 + 0.5) * region.resolution.1,
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

/// Handle a GridProcessorError from a read_region/read_point call, returning
/// an appropriate HTTP response. DataUnavailable errors get a 404 instead of 500.
fn handle_grid_read_error(e: GridProcessorError, context: &str) -> Response {
    if e.is_data_unavailable() {
        tracing::warn!("Data unavailable during {}: {}", context, e);
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!(
                "No data currently available. The data may not have been ingested yet \
                 or may have expired. Details: {}",
                e
            )),
        );
    }
    if matches!(e, GridProcessorError::NotFound(_)) {
        tracing::warn!("Dataset not found during {}: {}", context, e);
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(e.to_string()),
        );
    }
    tracing::error!("Failed to {}: {}", context, e);
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        ExceptionResponse::internal_error(format!("Failed to {}: {}", context, e)),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn times_are_rfc3339_utc_with_z() {
        let t: super::DateTime<super::Utc> = "2026-11-01T15:02:00+00:00".parse().unwrap();
        assert_eq!(super::rfc3339_utc(t), "2026-11-01T15:02:00Z");
        let t: super::DateTime<super::Utc> = "2026-10-09T23:00:41.250Z".parse().unwrap();
        assert_eq!(super::rfc3339_utc(t), "2026-10-09T23:00:41Z");
    }

    #[test]
    fn lightning_translation_threads_every_field() {
        let p = parse_area("coords=-106,39,-105,40&datetime=2026-10-07T20:00:00Z/..&limit=9&window=PT30M&after=7&satellite=goes-west");
        let l = lightning_area_params(&p);
        assert_eq!(l.coords.as_deref(), Some("-106,39,-105,40"));
        assert_eq!(l.datetime.as_deref(), Some("2026-10-07T20:00:00Z/.."));
        assert_eq!(
            (
                l.limit,
                l.window.as_deref(),
                l.after,
                l.satellite.as_deref()
            ),
            (Some(9), Some("PT30M"), Some(7), Some("goes-west"))
        );
    }

    // ---- Session 14: limit/class were silently dropped by this dispatcher ----

    fn parse_area(qs: &str) -> AreaQueryParams {
        let uri: axum::http::Uri = format!("/area?{}", qs).parse().unwrap();
        Query::<AreaQueryParams>::try_from_uri(&uri).unwrap().0
    }

    #[test]
    fn limit_and_class_survive_real_query_string_parsing() {
        let p = parse_area("coords=-105.3,39.9,-105.2,40.0&limit=3&class=hiking_trail");
        assert_eq!(p.limit, Some(3));
        assert_eq!(p.class.as_deref(), Some("hiking_trail"));
    }

    #[test]
    fn trail_translation_threads_every_field_including_limit_and_class() {
        let p = parse_area(
            "coords=-105.3,39.9,-105.2,40.0&limit=3&class=hiking_trail&conditions=latest&f=geojson",
        );
        let t = trail_area_params(&p);
        assert_eq!(t.limit, Some(3));
        assert_eq!(t.class.as_deref(), Some("hiking_trail"));
        assert_eq!(t.conditions.as_deref(), Some("latest"));
        assert_eq!(t.coords.as_deref(), Some("-105.3,39.9,-105.2,40.0"));
        assert_eq!(t.f.as_deref(), Some("geojson"));
    }

    #[test]
    fn crop_is_read_from_the_query_string_and_off_by_default() {
        let on = parse_area("coords=-106,38.5,-104.5,40.9&f=png&crop=true");
        assert_eq!(on.crop.as_deref(), Some("true"));
        assert_eq!(parse_area("coords=-106,38.5,-104.5,40.9&f=png").crop, None);
        // `crop` is a PNG option, not a pin on the time: the cache policy ignores it
        assert_eq!(on.requested_time(None).cache_control(900), "max-age=60");
    }

    /// The polygons of a request, as the handler holds them
    fn polygons_of(wkt: &str) -> Vec<AreaQuery> {
        match AreaQuery::parse_polygon_multi(wkt).unwrap() {
            ParsedPolygons::Single(p) => vec![p],
            ParsedPolygons::Multi(p) => p,
        }
        .into_iter()
        .map(|polygon| AreaQuery {
            polygon,
            z: None,
            datetime: None,
            parameter_names: None,
            crs: None,
        })
        .collect()
    }

    #[test]
    fn a_cropped_image_covers_every_polygon_of_a_multipolygon() {
        let boxes = polygons_of;

        let one = union_bbox(&boxes("POLYGON((-106 39,-104 39,-104 40,-106 40,-106 39))"));
        assert_eq!(
            (one.min_lon, one.min_lat, one.max_lon, one.max_lat),
            (-106.0, 39.0, -104.0, 40.0)
        );

        let two = union_bbox(&boxes(
            "MULTIPOLYGON(((-106 39,-104 39,-104 40,-106 40,-106 39)),\
             ((-90 35,-88 35,-88 36,-90 36,-90 35)))",
        ));
        assert_eq!(
            (two.min_lon, two.min_lat, two.max_lon, two.max_lat),
            (-106.0, 35.0, -88.0, 40.0)
        );
    }

    #[test]
    fn a_cropped_multipolygon_is_masked_outside_every_polygon_whatever_its_first_polygon() {
        // Two rectangles 2 x 1.2 and 2 x 1 degrees, far apart, in a 154 x 72 image of the box
        // around both (0.1 degree pixels, rows from the north)
        let polygons = polygons_of(
            "MULTIPOLYGON(((-113.4 44,-111.4 44,-111.4 45.2,-113.4 45.2,-113.4 44)),\
             ((-100 38,-98 38,-98 39,-100 39,-100 38)))",
        );
        let (width, height) = (154, 72);
        let union = union_bbox(&polygons);
        let image = GridRegion::new(
            vec![1.0; width * height],
            width,
            height,
            union,
            (union.width() / width as f64, union.height() / height as f64),
        );
        let first = &polygons[0].polygon;
        let kept = |masked: &[Option<f32>]| masked.iter().flatten().count();

        // the image is bigger than the first polygon: only the pixels in a polygon are data
        let masked = mask_to_polygons(&image, RowOrigin::North, first, &polygons, true);
        assert_eq!(kept(&masked), 20 * 12 + 20 * 10);
        assert!(masked[5 * width + 10].is_some(), "in the first polygon");
        assert!(masked[65 * width + 144].is_some(), "in the second polygon");
        assert!(masked[5 * width + 144].is_none(), "in neither");
        assert!(masked[40 * width + 77].is_none(), "in neither");

        // a read of the first polygon's box (a lat/lon grid, or crop off) is that rectangle:
        // nothing in it to mask, so nothing is checked, as ever
        let own_box =
            polygons_of("POLYGON((-113.4 44,-111.4 44,-111.4 45.2,-113.4 45.2,-113.4 44))");
        let own = GridRegion::new(vec![1.0; 20 * 12], 20, 12, union_bbox(&own_box), (0.1, 0.1));
        let masked = mask_to_polygons(&own, RowOrigin::North, first, &polygons, false);
        assert_eq!(kept(&masked), 20 * 12);
        // ...and so is the whole domain for a rectangle, whatever else is in the request
        let masked = mask_to_polygons(&image, RowOrigin::North, first, &polygons, false);
        assert_eq!(kept(&masked), width * height);

        // a first polygon that is not a rectangle is always checked
        let triangles = polygons_of(
            "MULTIPOLYGON(((-113.4 44,-111.4 44,-112.4 45.2,-113.4 44)),\
             ((-100 38,-98 38,-98 39,-100 39,-100 38)))",
        );
        let masked = mask_to_polygons(
            &image,
            RowOrigin::North,
            &triangles[0].polygon,
            &triangles,
            false,
        );
        assert!(kept(&masked) < 20 * 12 + 20 * 10);
        assert!(masked[5 * width + 144].is_none(), "in neither");
    }

    #[test]
    fn an_instances_datetime_span_is_cut_to_the_times_the_run_holds() {
        use chrono::TimeZone;
        // A run that starts at 18:15; the model's earliest instant (17:45) is an older run's
        let at = |h, m| Utc.with_ymd_and_hms(2026, 10, 11, h, m, 0).unwrap();
        let run = [at(18, 15), at(18, 30), at(19, 0)];
        let held =
            |datetime: &str| times_the_run_holds(&DateTimeQuery::parse(datetime).unwrap(), &run);

        // an interval: the run's own times inside it, ascending, whatever the model holds
        assert_eq!(
            held("2026-10-11T17:30:00Z/2026-10-11T18:30:00Z"),
            ["2026-10-11T18:15:00Z", "2026-10-11T18:30:00Z"]
        );
        assert_eq!(held("../2026-10-11T18:20:00Z"), ["2026-10-11T18:15:00Z"]);
        assert_eq!(
            held("2026-10-11T18:20:00Z/.."),
            ["2026-10-11T18:30:00Z", "2026-10-11T19:00:00Z"]
        );
        // a list: the entries the run holds, in the order given
        assert_eq!(
            held("2026-10-11T19:00:00Z,2026-10-11T17:45:00Z,2026-10-11T18:15:00Z"),
            ["2026-10-11T19:00:00Z", "2026-10-11T18:15:00Z"]
        );
        // within the catalog's one second, not beyond it
        assert_eq!(
            held("2026-10-11T18:30:01Z,2026-10-11T18:30:02Z"),
            ["2026-10-11T18:30:01Z"]
        );
        // nothing the run holds: nothing (the handler answers 404)
        assert!(held("2030-01-01T00:00:00Z/2030-01-02T00:00:00Z").is_empty());
        assert!(held("2026-10-11T17:00:00Z,2026-10-11T17:45:00Z").is_empty());
        assert_eq!(
            held("2026-10-11T18:15:00Z/2026-10-11T18:20:00Z"),
            ["2026-10-11T18:15:00Z"]
        );
    }

    #[test]
    fn storm_and_observation_translations_thread_limit() {
        let p = parse_area("coords=-97.1,35.0,-97.0,35.1&limit=7");
        assert_eq!(storm_area_params(&p).limit, Some(7));
        assert_eq!(obs_area_params(&p).limit, Some(7));
    }

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
