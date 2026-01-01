//! Position query handler.

use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Utc};
use edr_protocol::{
    coverage_json::CovJsonParameter, parameters::Unit, queries::DateTimeQuery,
    responses::ExceptionResponse, CoverageCollection, CoverageJson, EdrFeatureCollection,
    ParsedCoords, PositionQuery as ParsedPositionQuery,
};
use grid_processor::DatasetQuery;
use serde::Deserialize;
use std::sync::Arc;

use crate::config::LevelValue;
use crate::content_negotiation::{negotiate_format, OutputFormat};
use crate::limits::ResponseSizeEstimate;
use crate::state::AppState;

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
        // Return all parameters in collection
        collection_def
            .parameters
            .iter()
            .map(|p| p.name.clone())
            .collect()
    } else {
        // Validate requested parameters exist in collection
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
        };

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

                let mut query = DatasetQuery::forecast(&model_config.model, param_name);

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
            let mut values: Vec<Option<f32>> = Vec::with_capacity(parsed_times.len());
            let mut units_str = String::new();

            for valid_time in &parsed_times {
                // Build the DatasetQuery for this specific valid time
                let mut query = DatasetQuery::forecast(&model_config.model, param_name)
                    .at_valid_time(*valid_time);

                if let Some(level) = &level_str {
                    query = query.at_level(level);
                }

                // Use the reference time if provided (instance query)
                if let Some(ref_time) = reference_time {
                    query = query.at_run(ref_time);
                }

                // Query the data for this time
                match state.grid_data_service.read_point(&query, lon, lat).await {
                    Ok(point_value) => {
                        if units_str.is_empty() {
                            units_str = point_value.units.clone();
                        }
                        values.push(point_value.value);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to query {}/{} at ({}, {}) for time {}: {}",
                            model_config.model,
                            param_name,
                            lon,
                            lat,
                            valid_time,
                            e
                        );
                        values.push(None);
                    }
                }
            }

            // Add the time series data
            let unit = Unit::from_symbol(&units_str);
            let cov_param = CovJsonParameter::new(param_name).with_unit(unit);
            coverage = coverage.with_time_series(param_name, cov_param, values);
        } else {
            // Single time query (original behavior)
            let mut query = DatasetQuery::forecast(&model_config.model, param_name);

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
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "max-age=300")
        .body(json.into())
        .unwrap()
}

/// Build a catalog-compatible level string from EDR config.
///
/// Maps EDR level types and values to the format used in the catalog:
/// - "surface" for surface level
/// - "2 m above ground" for height_above_ground
/// - "500 mb" for isobaric levels
/// - "entire atmosphere" for entire_atmosphere
fn build_level_string(
    level_filter: &crate::config::LevelFilter,
    param_def: Option<&crate::config::ParameterDefinition>,
    z_value: Option<f64>,
) -> Option<String> {
    // Use z_value if provided, otherwise use the first level from param definition
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
        "isobaric" => {
            // Isobaric levels stored as "XXX mb"
            level_value.map(|v| format!("{} mb", v as i32))
        }
        "height_above_ground" => {
            // Height above ground stored as "X m above ground"
            level_value.map(|v| format!("{} m above ground", v as i32))
        }
        "cloud_layer" => {
            // Map cloud layer codes to names
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
        _ => {
            // Unknown level type, try to use named level from param
            param_def
                .and_then(|p| p.levels.first())
                .and_then(|l| match l {
                    LevelValue::Named(name) => Some(name.clone()),
                    LevelValue::Numeric(_) => None,
                })
        }
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
