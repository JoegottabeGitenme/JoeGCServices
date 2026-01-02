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
use grid_processor::{BoundingBox, DatasetQuery};
use serde::Deserialize;
use std::sync::Arc;

use crate::config::LevelValue;
use crate::content_negotiation::{negotiate_format, OutputFormat};
use crate::limits::ResponseSizeEstimate;
use crate::state::AppState;

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
}

/// GET /edr/collections/:collection_id/area
pub async fn area_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<AreaQueryParams>,
    headers: HeaderMap,
) -> Response {
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

    // Check area size limit
    let area_sq_degrees = area_query_struct.area_sq_degrees();
    let max_area = model_config.limits.max_area_sq_degrees.unwrap_or(100.0);
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
        .map(|p| edr_protocol::PositionQuery::parse_parameter_names(p))
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

    // Build the DatasetQuery
    let mut query = DatasetQuery::forecast(&model_config.model, first_param);

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

        // Build the DatasetQuery
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
                // Apply polygon mask - set values outside polygon to null
                let mut values: Vec<Option<f32>> = Vec::with_capacity(param_region.data.len());

                for (idx, &value) in param_region.data.iter().enumerate() {
                    let row = idx / param_region.width;
                    let col = idx % param_region.width;

                    // Calculate lon/lat for this grid cell
                    let lon =
                        param_region.bbox.min_lon + (col as f64 + 0.5) * param_region.resolution.0;
                    let lat =
                        param_region.bbox.max_lat - (row as f64 + 0.5) * param_region.resolution.1;

                    // Check if point is inside any polygon (union of all polygons for MULTIPOLYGON)
                    let inside_any = all_area_queries
                        .iter()
                        .any(|aq| aq.contains_point(lon, lat));
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
}
