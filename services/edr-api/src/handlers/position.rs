//! Position query handler.

use std::sync::Arc;
use axum::{
    extract::{Extension, Path, Query},
    http::{header, StatusCode},
    response::Response,
};
use serde::Deserialize;
use edr_protocol::{
    CoverageJson,
    PositionQuery as ParsedPositionQuery,
    responses::ExceptionResponse,
    coverage_json::CovJsonParameter,
    queries::DateTimeQuery,
    parameters::Unit,
};
use grid_processor::DatasetQuery;

use crate::state::AppState;
use crate::config::LevelValue;
use crate::limits::ResponseSizeEstimate;

/// Query parameters for position endpoint.
#[derive(Debug, Deserialize)]
pub struct PositionQueryParams {
    /// Coordinates as WKT POINT or lon,lat.
    pub coords: String,

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
) -> Response {
    // Use latest instance
    position_query(state, collection_id, None, params).await
}

/// GET /edr/collections/:collection_id/instances/:instance_id/position
pub async fn instance_position_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, instance_id)): Path<(String, String)>,
    Query(params): Query<PositionQueryParams>,
) -> Response {
    position_query(state, collection_id, Some(instance_id), params).await
}

async fn position_query(
    state: Arc<AppState>,
    collection_id: String,
    instance_id: Option<String>,
    params: PositionQueryParams,
) -> Response {
    let config = state.edr_config.read().await;

    // Find the collection
    let Some((model_config, collection_def)) = config.find_collection(&collection_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Collection not found: {}", collection_id)),
        );
    };

    // Parse coordinates
    let (lon, lat) = match ParsedPositionQuery::parse_coords(&params.coords) {
        Ok(coords) => coords,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid coordinates: {}", e)),
            );
        }
    };

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

    // Parse datetime
    let _datetime = if let Some(ref dt) = params.datetime {
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
    let requested_params = params.parameter_name
        .as_ref()
        .map(|p| ParsedPositionQuery::parse_parameter_names(p))
        .unwrap_or_default();

    // Determine which parameters to query
    let params_to_query: Vec<_> = if requested_params.is_empty() {
        // Return all parameters in collection
        collection_def.parameters.iter().map(|p| p.name.clone()).collect()
    } else {
        // Validate requested parameters exist in collection
        let available: Vec<_> = collection_def.parameters.iter().map(|p| p.name.as_str()).collect();
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

    // Check response size limits
    let num_levels = z_values.as_ref().map(|v| v.len()).unwrap_or(1);
    let num_times = 1; // For now, single time
    let estimate = ResponseSizeEstimate::for_position(params_to_query.len(), num_times, num_levels);

    if let Err(limit_err) = estimate.check_limits(&model_config.limits) {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            ExceptionResponse::payload_too_large(limit_err.to_string()),
        );
    }

    // Parse instance_id if provided
    let _reference_time = if let Some(ref id) = instance_id {
        match chrono::DateTime::parse_from_rfc3339(id) {
            Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
            Err(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    ExceptionResponse::bad_request(format!(
                        "Invalid instance ID format: {}",
                        id
                    )),
                );
            }
        }
    } else {
        None
    };

    // Build CoverageJSON response
    let datetime_str = params.datetime.clone();
    let z_val = z_values.as_ref().and_then(|v| v.first().copied());

    let mut coverage = CoverageJson::point(lon, lat, datetime_str, z_val);

    // For each parameter, query the data
    for param_name in &params_to_query {
        // Find the parameter definition in the collection to get level info
        let param_def = collection_def.parameters.iter().find(|p| p.name == *param_name);
        
        // Build the level string for catalog lookup
        let level_str = build_level_string(&collection_def.level_filter, param_def, z_val);
        
        // Build the DatasetQuery
        let mut query = DatasetQuery::forecast(&model_config.model, param_name);
        
        if let Some(level) = &level_str {
            query = query.at_level(level);
        }
        
        // Use the reference time if provided
        if let Some(ref_time) = _reference_time {
            query = query.at_run(ref_time);
        }
        
        // Query the actual data
        match state.grid_data_service.read_point(&query, lon, lat).await {
            Ok(point_value) => {
                let unit = Unit::from_symbol(&point_value.units);
                let cov_param = CovJsonParameter::new(param_name)
                    .with_unit(unit);

                if let Some(val) = point_value.value {
                    coverage = coverage.with_parameter(param_name, cov_param, val);
                } else {
                    // No data at this point (outside grid or fill value)
                    tracing::debug!(
                        "No data value at ({}, {}) for {}/{}",
                        lon, lat, model_config.model, param_name
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
                    model_config.model, param_name, lon, lat, e
                );
                // Add parameter with null value
                let cov_param = CovJsonParameter::new(param_name);
                coverage = coverage.with_parameter_null(param_name, cov_param);
            }
        }
    }

    // Serialize response
    let json = match serde_json::to_string_pretty(&coverage) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("Failed to serialize CoverageJSON: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ExceptionResponse::internal_error("Failed to serialize response"),
            );
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.cov+json")
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
        param_def.and_then(|p| p.levels.first()).and_then(|l| match l {
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
            param_def.and_then(|p| p.levels.first()).and_then(|l| match l {
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
    fn test_coverage_json_creation() {
        let coverage = CoverageJson::point(-97.5, 35.2, Some("2024-12-29T12:00:00Z".to_string()), Some(2.0));

        let json = serde_json::to_string(&coverage).unwrap();
        assert!(json.contains("\"type\":\"Coverage\""));
        assert!(json.contains("\"domainType\":\"Point\""));
    }
}
