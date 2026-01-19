//! Light pollution collection handler.
//!
//! Provides light pollution data (VIIRS nighttime lights) with derived Bortle scale.
//! Queries stored VIIRS radiance data and computes Bortle class on-the-fly.

use axum::{
    extract::{Extension, Query},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use edr_protocol::{
    coverage_json::CovJsonParameter,
    parameters::{I18nString, ObservedProperty, Unit, UnitSymbol},
    queries::PositionQuery,
    responses::ExceptionResponse,
    AreaQuery, CoverageJson,
};
use grid_processor::{BoundingBox, DatasetQuery};
use renderer::data_png::{DataPng8BitEncoder, DataPngEncoder};
use serde::Deserialize;
use std::sync::Arc;

use crate::content_negotiation::{check_png_not_supported, negotiate_format, OutputFormat};
use crate::metrics::{extract_client_ip, extract_user_agent, EndpointType, FormatType, Timer};
use crate::state::AppState;

/// Query parameters for light-pollution position endpoint.
#[derive(Debug, Deserialize)]
pub struct LightPollutionQueryParams {
    /// Coordinates as WKT POINT. Required.
    pub coords: Option<String>,

    /// Parameter name(s) to retrieve. If not provided, returns all parameters.
    #[serde(rename = "parameter-name")]
    pub parameter_name: Option<String>,

    /// Output format (for consistency with other endpoints).
    pub f: Option<String>,
}

/// Query parameters for light-pollution area endpoint.
#[derive(Debug, Deserialize)]
pub struct LightPollutionAreaParams {
    /// Coordinates as WKT POLYGON. Required.
    pub coords: Option<String>,

    /// Parameter name to retrieve. Only 'radiance' supported for area queries.
    #[serde(rename = "parameter-name")]
    pub parameter_name: Option<String>,

    /// Output format: 'png' for data PNG, 'json' for CoverageJSON.
    pub f: Option<String>,

    /// Coordinate reference system (default: CRS:84).
    pub crs: Option<String>,

    /// Output image width (default: 256, max: 2048).
    pub width: Option<u32>,

    /// Output image height (default: 256, max: 2048).
    pub height: Option<u32>,

    /// PNG bit depth: 16 (default) or 8.
    pub depth: Option<u8>,
}

/// Valid parameters for the light-pollution collection.
const VALID_PARAMS: &[&str] = &["radiance", "bortle_class"];

/// Default parameters to return when none specified.
const DEFAULT_PARAMS: &[&str] = &["radiance", "bortle_class"];

/// GET /edr/collections/light-pollution/position
pub async fn position_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<LightPollutionQueryParams>,
    headers: HeaderMap,
) -> Response {
    let timer = Timer::start();
    let client_ip = extract_client_ip(&headers);
    let user_agent = extract_user_agent(&headers);

    // Negotiate output format
    let output_format = match negotiate_format(&headers, params.f.as_deref()) {
        Ok(format) => format,
        Err(response) => {
            state
                .metrics
                .record_request(
                    EndpointType::Position,
                    Some("light-pollution"),
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

    // PNG not supported for light-pollution
    if let Some(response) = check_png_not_supported(output_format, "light-pollution") {
        return response;
    }

    // Parse coordinates
    let coords_str = match &params.coords {
        Some(c) if !c.trim().is_empty() => c.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("Missing required parameter: coords"),
            );
        }
    };

    let (lon, lat) = match PositionQuery::parse_coords(coords_str) {
        Ok((lon, lat)) => (lon, lat),
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid coords parameter: {}", e)),
            );
        }
    };

    // Validate lat/lon ranges
    if !(-90.0..=90.0).contains(&lat) {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!("Latitude must be between -90 and 90: {}", lat)),
        );
    }
    if !(-180.0..=180.0).contains(&lon) {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Longitude must be between -180 and 180: {}",
                lon
            )),
        );
    }

    // VIIRS coverage is -65 to 75 latitude
    if lat < -65.0 || lat > 75.0 {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "VIIRS coverage is limited to latitudes -65 to 75. Requested: {}",
                lat
            )),
        );
    }

    // Parse requested parameters
    let requested_params = parse_requested_parameters(params.parameter_name.as_deref());

    // Validate requested parameters
    if let Err(e) = validate_parameters(&requested_params) {
        return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e));
    }

    // Query radiance from catalog/storage
    let radiance = match query_viirs_radiance(&state, lon, lat).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            // No data available at this location (likely ocean or high latitude)
            return error_response(
                StatusCode::NOT_FOUND,
                ExceptionResponse::not_found("No light pollution data available at this location"),
            );
        }
        Err(e) => {
            tracing::error!("Failed to query VIIRS data: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to retrieve light pollution data"),
            );
        }
    };

    // Compute Bortle class from radiance
    let bortle_class = radiance_to_bortle(radiance);

    // Build CoverageJSON response
    let coverage = build_coverage_json(lon, lat, radiance, bortle_class, &requested_params);

    // Record metrics
    state
        .metrics
        .record_request(
            EndpointType::Position,
            Some("light-pollution"),
            &requested_params,
            FormatType::CoverageJson,
            timer.elapsed_us(),
            true,
            client_ip.as_deref(),
            user_agent.as_deref(),
        )
        .await;

    // Return response
    (StatusCode::OK, Json(coverage)).into_response()
}

/// GET /edr/collections/light-pollution/area
pub async fn area_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<LightPollutionAreaParams>,
    headers: HeaderMap,
) -> Response {
    let timer = Timer::start();
    let client_ip = extract_client_ip(&headers);
    let user_agent = extract_user_agent(&headers);

    // Negotiate output format
    let output_format = match negotiate_format(&headers, params.f.as_deref()) {
        Ok(format) => format,
        Err(response) => {
            state
                .metrics
                .record_request(
                    EndpointType::Area,
                    Some("light-pollution"),
                    &[],
                    FormatType::Png,
                    timer.elapsed_us(),
                    false,
                    client_ip.as_deref(),
                    user_agent.as_deref(),
                )
                .await;
            return response;
        }
    };

    // Parse coordinates
    let coords_str = match &params.coords {
        Some(c) if !c.trim().is_empty() => c.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("Missing required parameter: coords"),
            );
        }
    };

    // Parse polygon
    let polygon = match AreaQuery::parse_polygon(coords_str) {
        Ok(p) => p,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid polygon: {}", e)),
            );
        }
    };

    // Create AreaQuery to use its bbox method
    let area_query = AreaQuery {
        polygon: polygon.clone(),
        z: None,
        datetime: None,
        parameter_names: None,
        crs: None,
    };

    // Get bounding box from polygon
    let bbox_query = area_query.bbox();

    // Validate bbox is within VIIRS coverage (-65 to 75 latitude)
    if bbox_query.south < -65.0 || bbox_query.north > 75.0 {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "VIIRS coverage is limited to latitudes -65 to 75. Requested: {} to {}",
                bbox_query.south, bbox_query.north
            )),
        );
    }

    // Determine output dimensions (default 256, max 2048)
    let width = params.width.unwrap_or(256).min(2048) as usize;
    let height = params.height.unwrap_or(256).min(2048) as usize;

    // Create query for VIIRS radiance data
    // Try radiance_average first (the actual parameter name from VIIRS ingestion)
    let query = DatasetQuery::observation("viirs", "radiance_average").at_level("surface");

    // Convert to grid_processor BoundingBox
    let grid_bbox = BoundingBox::new(
        bbox_query.west,
        bbox_query.south,
        bbox_query.east,
        bbox_query.north,
    );

    // Read area data from grid service
    let region = match state
        .grid_data_service
        .read_region(&query, &grid_bbox, Some((width, height)))
        .await
    {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("Failed to read VIIRS area data: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to retrieve light pollution data"),
            );
        }
    };

    // Check if we got any data
    if region.data.is_empty() {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found("No light pollution data available in this area"),
        );
    }

    // Resample if the region is larger than the requested output size
    // VIIRS data doesn't have pyramid levels, so we need to downsample here
    let (final_data, final_width, final_height) = if region.width > width || region.height > height
    {
        let resampled = resample_bilinear(&region.data, region.width, region.height, width, height);
        (resampled, width, height)
    } else {
        (region.data.clone(), region.width, region.height)
    };

    // Convert Vec<f32> to Vec<Option<f32>> (NaN -> None)
    let data: Vec<Option<f32>> = final_data
        .iter()
        .map(|&v| if v.is_nan() { None } else { Some(v) })
        .collect();

    // Compute min/max for metadata
    let (min_val, max_val) = compute_min_max(&data);

    // Record metrics
    state
        .metrics
        .record_request(
            EndpointType::Area,
            Some("light-pollution"),
            &["radiance".to_string()],
            if matches!(output_format, OutputFormat::Png) {
                FormatType::Png
            } else {
                FormatType::CoverageJson
            },
            timer.elapsed_us(),
            true,
            client_ip.as_deref(),
            user_agent.as_deref(),
        )
        .await;

    // Return response based on format
    match output_format {
        OutputFormat::Png => {
            // Convert data to PNG using resampled dimensions
            let depth = params.depth.unwrap_or(16);
            let png_data = encode_radiance_png(&data, final_width, final_height, depth, &grid_bbox);
            let encoding_name = if depth == 8 { "8bit" } else { "16bit" };

            match png_data {
                Ok(encoded) => Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "image/png")
                    .header(header::CACHE_CONTROL, "max-age=3600")
                    .header("X-EDR-Parameter", "radiance")
                    .header("X-EDR-Units", "nW/cm^2/sr")
                    .header("X-EDR-Min", format!("{}", min_val))
                    .header("X-EDR-Max", format!("{}", max_val))
                    .header("X-EDR-Encoding", encoding_name)
                    .header(
                        "X-EDR-BBox",
                        format!(
                            "{},{},{},{}",
                            grid_bbox.min_lon,
                            grid_bbox.min_lat,
                            grid_bbox.max_lon,
                            grid_bbox.max_lat
                        ),
                    )
                    .header("X-EDR-Width", format!("{}", final_width))
                    .header("X-EDR-Height", format!("{}", final_height))
                    .body(encoded.png_bytes.into())
                    .unwrap(),
                Err(e) => {
                    tracing::error!("Failed to encode PNG: {}", e);
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ExceptionResponse::internal_error("Failed to encode image"),
                    )
                }
            }
        }
        _ => {
            // Return CoverageJSON for non-PNG formats
            let coverage = build_area_coverage_json(
                &data,
                final_width,
                final_height,
                &grid_bbox,
                min_val,
                max_val,
            );
            (StatusCode::OK, Json(coverage)).into_response()
        }
    }
}

/// Resample grid data using bilinear interpolation.
fn resample_bilinear(
    data: &[f32],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
) -> Vec<f32> {
    let mut result = vec![f32::NAN; dst_width * dst_height];

    let x_ratio = src_width as f64 / dst_width as f64;
    let y_ratio = src_height as f64 / dst_height as f64;

    for dst_y in 0..dst_height {
        for dst_x in 0..dst_width {
            // Map destination pixel to source coordinates
            let src_x = dst_x as f64 * x_ratio;
            let src_y = dst_y as f64 * y_ratio;

            let x0 = src_x.floor() as usize;
            let y0 = src_y.floor() as usize;
            let x1 = (x0 + 1).min(src_width - 1);
            let y1 = (y0 + 1).min(src_height - 1);

            let dx = (src_x - x0 as f64) as f32;
            let dy = (src_y - y0 as f64) as f32;

            // Get four corner values
            let v00 = data[y0 * src_width + x0];
            let v10 = data[y0 * src_width + x1];
            let v01 = data[y1 * src_width + x0];
            let v11 = data[y1 * src_width + x1];

            // If any corner is NaN, use nearest neighbor
            if v00.is_nan() || v10.is_nan() || v01.is_nan() || v11.is_nan() {
                // Nearest neighbor fallback
                let nearest_x = (src_x + 0.5) as usize;
                let nearest_y = (src_y + 0.5) as usize;
                let nearest_x = nearest_x.min(src_width - 1);
                let nearest_y = nearest_y.min(src_height - 1);
                result[dst_y * dst_width + dst_x] = data[nearest_y * src_width + nearest_x];
            } else {
                // Bilinear interpolation
                let v0 = v00 * (1.0 - dx) + v10 * dx;
                let v1 = v01 * (1.0 - dx) + v11 * dx;
                result[dst_y * dst_width + dst_x] = v0 * (1.0 - dy) + v1 * dy;
            }
        }
    }

    result
}

/// Compute min/max values from data, ignoring None values.
fn compute_min_max(data: &[Option<f32>]) -> (f32, f32) {
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for v in data.iter().flatten() {
        if *v < min {
            min = *v;
        }
        if *v > max {
            max = *v;
        }
    }
    // Default to 0-300 if no valid data
    if min > max {
        (0.0, 300.0)
    } else {
        (min.min(0.0), max.max(300.0))
    }
}

/// Encode radiance data as a data PNG.
fn encode_radiance_png(
    data: &[Option<f32>],
    width: usize,
    height: usize,
    depth: u8,
    bbox: &BoundingBox,
) -> Result<renderer::data_png::EncodedDataPng, String> {
    // Compute data range for scaling
    let (min, max) = compute_min_max(data);

    let bbox_arr = [bbox.min_lon, bbox.min_lat, bbox.max_lon, bbox.max_lat];

    if depth == 8 {
        let encoder = DataPng8BitEncoder::new(min, max);
        encoder
            .encode_with_metadata(data, width, height, "radiance", "nW/cm^2/sr", bbox_arr)
            .map_err(|e| format!("PNG encoding failed: {}", e))
    } else {
        let encoder = DataPngEncoder::new(min, max);
        encoder
            .encode(data, width, height)
            .map_err(|e| format!("PNG encoding failed: {}", e))
    }
}

/// Build CoverageJSON for area response.
fn build_area_coverage_json(
    data: &[Option<f32>],
    width: usize,
    height: usize,
    bbox: &BoundingBox,
    _min: f32,
    _max: f32,
) -> CoverageJson {
    use edr_protocol::coverage_json::{Domain, NdArray};
    use std::collections::HashMap;

    // Generate x (longitude) values
    let x_step = (bbox.max_lon - bbox.min_lon) / width as f64;
    let x_values: Vec<f64> = (0..width)
        .map(|i| bbox.min_lon + (i as f64 + 0.5) * x_step)
        .collect();

    // Generate y (latitude) values - from north to south (top to bottom)
    let y_step = (bbox.max_lat - bbox.min_lat) / height as f64;
    let y_values: Vec<f64> = (0..height)
        .map(|i| bbox.max_lat - (i as f64 + 0.5) * y_step)
        .collect();

    // Create grid domain
    let domain = Domain::grid(
        x_values, y_values, None, // time
        None, // z
    );

    // Add radiance parameter
    let param = CovJsonParameter {
        type_: "Parameter".to_string(),
        description: Some(I18nString::english(
            "Average nighttime radiance from VIIRS Day/Night Band",
        )),
        observed_property: ObservedProperty {
            id: None,
            label: Some(I18nString::english("Radiance")),
            description: Some(I18nString::english(
                "Nighttime radiance measured by VIIRS satellite",
            )),
            categories: None,
        },
        unit: Some(Unit {
            label: Some(I18nString::english("nW/cm^2/sr")),
            symbol: Some(UnitSymbol::Simple("nW/cm^2/sr".to_string())),
        }),
    };

    let mut params = HashMap::new();
    params.insert("radiance".to_string(), param);

    // Create data range (y*x order for grid)
    let shape = vec![height, width];
    let axis_names = vec!["y".to_string(), "x".to_string()];
    let range = NdArray::with_missing(data.to_vec(), shape, axis_names);

    let mut ranges = HashMap::new();
    ranges.insert("radiance".to_string(), range);

    CoverageJson {
        type_: edr_protocol::coverage_json::CoverageType::Coverage,
        domain,
        parameters: Some(params),
        ranges: Some(ranges),
    }
}

/// Parse requested parameter names from query string.
fn parse_requested_parameters(parameter_name: Option<&str>) -> Vec<String> {
    match parameter_name {
        None => DEFAULT_PARAMS.iter().map(|s| s.to_string()).collect(),
        Some(params) => params.split(',').map(|s| s.trim().to_string()).collect(),
    }
}

/// Validate that requested parameters are supported.
fn validate_parameters(params: &[String]) -> Result<(), String> {
    for param in params {
        if !VALID_PARAMS.contains(&param.as_str()) {
            return Err(format!(
                "Invalid parameter name: {}. Valid parameters: {}",
                param,
                VALID_PARAMS.join(", ")
            ));
        }
    }
    Ok(())
}

/// Query VIIRS radiance value from storage using the grid data service.
///
/// Returns the radiance value in nW/cm^2/sr at the given location,
/// or None if no data is available (e.g., ocean, high latitude).
async fn query_viirs_radiance(state: &AppState, lon: f64, lat: f64) -> Result<Option<f32>, String> {
    // Create a query for VIIRS radiance data
    // VIIRS is a static dataset treated as observation data
    // Try multiple parameter names since VIIRS files may be named differently
    // (radiance, radiance_average, radiance_median)
    let param_names = ["radiance_average", "radiance_median", "radiance"];

    for param in param_names {
        let query = DatasetQuery::observation("viirs", param).at_level("surface");
        tracing::debug!(
            "Querying VIIRS: model=viirs, param={}, level=surface",
            param
        );
        match state.grid_data_service.read_point(&query, lon, lat).await {
            Ok(point_value) => {
                tracing::debug!("Got point value: {:?}", point_value.value);
                match point_value.value {
                    Some(v) if v.is_nan() => {
                        tracing::debug!("Value is NaN, trying next param");
                        continue;
                    }
                    Some(v) => {
                        tracing::info!("Found VIIRS radiance: {} at ({}, {})", v, lon, lat);
                        return Ok(Some(v));
                    }
                    None => {
                        tracing::debug!("Value is None, trying next param");
                        continue;
                    }
                }
            }
            Err(e) => {
                tracing::debug!("Error querying param {}: {}", param, e);
                continue;
            }
        }
    }

    // None of the parameters had data
    tracing::debug!("No VIIRS data found at ({}, {})", lon, lat);
    Ok(None)
}

#[allow(dead_code)]
async fn query_viirs_radiance_single(
    state: &AppState,
    lon: f64,
    lat: f64,
) -> Result<Option<f32>, String> {
    // Original single-parameter query for reference
    let query = DatasetQuery::observation("viirs", "radiance").at_level("surface");

    // Use the grid data service to read the point value
    match state.grid_data_service.read_point(&query, lon, lat).await {
        Ok(point_value) => {
            // Check for NaN (no data at this location)
            match point_value.value {
                Some(v) if v.is_nan() => Ok(None),
                Some(v) => Ok(Some(v)),
                None => Ok(None),
            }
        }
        Err(e) => {
            // Log the error for debugging
            tracing::debug!("Failed to read VIIRS data at ({}, {}): {}", lon, lat, e);
            // Return None instead of error for "no data" cases
            // The grid data service may return an error if no dataset is found
            Ok(None)
        }
    }
}

/// Convert VIIRS radiance to Bortle dark-sky scale.
///
/// Based on research correlating VIIRS DNB radiance to naked-eye limiting magnitude
/// and the Bortle Dark-Sky Scale.
///
/// References:
/// - Falchi et al. 2016 "The new world atlas of artificial night sky brightness"
/// - Bortle, John E. 2001 "The Bortle Dark-Sky Scale"
pub fn radiance_to_bortle(radiance_nw: f32) -> u8 {
    // Approximate mapping based on VIIRS radiance (nW/cm^2/sr)
    // These thresholds are approximations based on published research
    match radiance_nw {
        r if r < 0.25 => 1, // Excellent dark-sky site (mag limit ~7.6-8.0)
        r if r < 0.5 => 2,  // Typical truly dark site (mag limit ~7.1-7.5)
        r if r < 1.0 => 3,  // Rural sky (mag limit ~6.6-7.0)
        r if r < 2.0 => 4,  // Rural/suburban transition (mag limit ~6.1-6.5)
        r if r < 4.0 => 5,  // Suburban sky (mag limit ~5.6-6.0)
        r if r < 8.0 => 6,  // Bright suburban sky (mag limit ~5.1-5.5)
        r if r < 20.0 => 7, // Suburban/urban transition (mag limit ~4.6-5.0)
        r if r < 50.0 => 8, // City sky (mag limit ~4.1-4.5)
        _ => 9,             // Inner-city sky (mag limit ~4.0 or less)
    }
}

/// Get description for Bortle class.
pub fn bortle_description(class: u8) -> &'static str {
    match class {
        1 => "Excellent dark-sky site",
        2 => "Typical truly dark site",
        3 => "Rural sky",
        4 => "Rural/suburban transition",
        5 => "Suburban sky",
        6 => "Bright suburban sky",
        7 => "Suburban/urban transition",
        8 => "City sky",
        9 => "Inner-city sky",
        _ => "Unknown",
    }
}

/// Build CoverageJSON response for light pollution data.
fn build_coverage_json(
    lon: f64,
    lat: f64,
    radiance: f32,
    bortle_class: u8,
    requested_params: &[String],
) -> CoverageJson {
    let mut coverage = CoverageJson::point(lon, lat, None, None);

    for param_name in requested_params {
        match param_name.as_str() {
            "radiance" => {
                coverage = add_radiance_parameter(coverage, radiance);
            }
            "bortle_class" => {
                coverage = add_bortle_parameter(coverage, bortle_class);
            }
            _ => {}
        }
    }

    coverage
}

/// Add radiance parameter to coverage.
fn add_radiance_parameter(coverage: CoverageJson, value: f32) -> CoverageJson {
    let param = CovJsonParameter {
        type_: "Parameter".to_string(),
        description: Some(I18nString::english(
            "Average nighttime radiance from VIIRS Day/Night Band",
        )),
        observed_property: ObservedProperty {
            id: None,
            label: Some(I18nString::english("Radiance")),
            description: Some(I18nString::english(
                "Nighttime radiance measured by VIIRS satellite",
            )),
            categories: None,
        },
        unit: Some(Unit {
            label: Some(I18nString::english("nW/cm^2/sr")),
            symbol: Some(UnitSymbol::Simple("nW/cm^2/sr".to_string())),
        }),
    };

    coverage.with_parameter("radiance", param, value)
}

/// Add Bortle class parameter to coverage.
fn add_bortle_parameter(coverage: CoverageJson, value: u8) -> CoverageJson {
    let description = bortle_description(value);

    let param = CovJsonParameter {
        type_: "Parameter".to_string(),
        description: Some(I18nString::english(&format!(
            "Bortle Dark-Sky Scale (1-9). Current: {} - {}",
            value, description
        ))),
        observed_property: ObservedProperty {
            id: None,
            label: Some(I18nString::english("Bortle Class")),
            description: Some(I18nString::english(
                "The Bortle scale measures night sky brightness from 1 (darkest) to 9 (brightest)",
            )),
            categories: None,
        },
        unit: None, // Dimensionless categorical
    };

    coverage.with_parameter("bortle_class", param, value as f32)
}

/// Helper to build error responses.
fn error_response(status: StatusCode, exception: ExceptionResponse) -> Response {
    let json = serde_json::to_string(&exception).unwrap_or_default();
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(json.into())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_radiance_to_bortle() {
        // Test boundary cases
        assert_eq!(radiance_to_bortle(0.1), 1);
        assert_eq!(radiance_to_bortle(0.24), 1);
        assert_eq!(radiance_to_bortle(0.25), 2);
        assert_eq!(radiance_to_bortle(0.49), 2);
        assert_eq!(radiance_to_bortle(0.5), 3);
        assert_eq!(radiance_to_bortle(0.99), 3);
        assert_eq!(radiance_to_bortle(1.0), 4);
        assert_eq!(radiance_to_bortle(1.99), 4);
        assert_eq!(radiance_to_bortle(2.0), 5);
        assert_eq!(radiance_to_bortle(3.99), 5);
        assert_eq!(radiance_to_bortle(4.0), 6);
        assert_eq!(radiance_to_bortle(7.99), 6);
        assert_eq!(radiance_to_bortle(8.0), 7);
        assert_eq!(radiance_to_bortle(19.99), 7);
        assert_eq!(radiance_to_bortle(20.0), 8);
        assert_eq!(radiance_to_bortle(49.99), 8);
        assert_eq!(radiance_to_bortle(50.0), 9);
        assert_eq!(radiance_to_bortle(100.0), 9);
    }

    #[test]
    fn test_bortle_description() {
        assert_eq!(bortle_description(1), "Excellent dark-sky site");
        assert_eq!(bortle_description(5), "Suburban sky");
        assert_eq!(bortle_description(9), "Inner-city sky");
        assert_eq!(bortle_description(10), "Unknown");
    }

    #[test]
    fn test_parse_requested_parameters_default() {
        let params = parse_requested_parameters(None);
        assert!(params.contains(&"radiance".to_string()));
        assert!(params.contains(&"bortle_class".to_string()));
    }

    #[test]
    fn test_parse_requested_parameters_custom() {
        let params = parse_requested_parameters(Some("radiance"));
        assert_eq!(params, vec!["radiance".to_string()]);

        let params = parse_requested_parameters(Some("radiance,bortle_class"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_validate_parameters() {
        assert!(validate_parameters(&["radiance".to_string()]).is_ok());
        assert!(validate_parameters(&["bortle_class".to_string()]).is_ok());
        assert!(validate_parameters(&["radiance".to_string(), "bortle_class".to_string()]).is_ok());
        assert!(validate_parameters(&["invalid".to_string()]).is_err());
    }
}
