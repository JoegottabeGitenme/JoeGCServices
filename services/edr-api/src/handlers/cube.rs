//! Cube query handler.
//!
//! The cube query returns a 3D data cube defined by bbox and z parameters.
//! Per OGC EDR spec (Requirement A.28):
//! - bbox is REQUIRED
//! - z is REQUIRED
//! - Returns CoverageCollection with one Coverage per z-level

use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Utc};
use edr_protocol::{
    coverage_json::{
        CovJsonParameter, CoverageCollection, CoverageJson, CoverageType, Domain, DomainType,
        NdArray, ReferenceSystem, ReferenceSystemConnection, VerticalCoordinateSystem,
    },
    parameters::Unit,
    queries::{BboxQuery, DateTimeQuery},
    responses::ExceptionResponse,
    EdrFeatureCollection,
};
use grid_processor::{BoundingBox, DatasetQuery};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use crate::config::LevelValue;
use crate::content_negotiation::{negotiate_format, OutputFormat};
use crate::limits::ResponseSizeEstimate;
use crate::state::AppState;

/// WKT representation for EPSG:4326
const WGS84_WKT: &str = r#"GEOGCS["Unknown", DATUM["Unknown", SPHEROID["WGS_1984", 6378137.0, 298.257223563]], PRIMEM["Greenwich",0], UNIT["degree", 0.017453], AXIS["Lon", EAST], AXIS["Lat", NORTH]]"#;

/// Query parameters for cube endpoint.
#[derive(Debug, Deserialize)]
pub struct CubeQueryParams {
    /// Bounding box as west,south,east,north. REQUIRED.
    pub bbox: Option<String>,

    /// Vertical level(s). REQUIRED.
    pub z: Option<String>,

    /// Datetime instant or interval.
    pub datetime: Option<String>,

    /// Parameter name(s) to retrieve.
    #[serde(rename = "parameter-name")]
    pub parameter_name: Option<String>,

    /// Number of grid points along x-axis (0 = native resolution).
    #[serde(rename = "resolution-x")]
    pub resolution_x: Option<u32>,

    /// Number of grid points along y-axis (0 = native resolution).
    #[serde(rename = "resolution-y")]
    pub resolution_y: Option<u32>,

    /// Coordinate reference system.
    pub crs: Option<String>,

    /// Output format.
    pub f: Option<String>,
}

/// GET /edr/collections/:collection_id/cube
pub async fn cube_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<CubeQueryParams>,
    headers: HeaderMap,
) -> Response {
    cube_query(state, collection_id, None, params, headers).await
}

/// GET /edr/collections/:collection_id/instances/:instance_id/cube
pub async fn instance_cube_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, instance_id)): Path<(String, String)>,
    Query(params): Query<CubeQueryParams>,
    headers: HeaderMap,
) -> Response {
    cube_query(state, collection_id, Some(instance_id), params, headers).await
}

async fn cube_query(
    state: Arc<AppState>,
    collection_id: String,
    instance_id: Option<String>,
    params: CubeQueryParams,
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

    // Check for required bbox parameter (OGC EDR Requirement A.28.D/F)
    let bbox_str = match &params.bbox {
        Some(b) if !b.trim().is_empty() => b.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(
                    "Missing required parameter: bbox. Cube queries require a bounding box.",
                ),
            );
        }
    };

    // Parse bbox
    let bbox = match BboxQuery::parse(bbox_str) {
        Ok(b) => b,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid bbox parameter: {}", e)),
            );
        }
    };

    // Check for required z parameter (OGC EDR Requirement A.28.G/H)
    let z_str = match &params.z {
        Some(z) if !z.trim().is_empty() => z.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(
                    "Missing required parameter: z. Cube queries require vertical level(s).",
                ),
            );
        }
    };

    // Parse vertical levels
    let z_values = match edr_protocol::PositionQuery::parse_z(z_str) {
        Ok(values) => values,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid z parameter: {}", e)),
            );
        }
    };

    if z_values.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request("z parameter must contain at least one level"),
        );
    }

    // Check area size limit
    let area_sq_degrees = bbox.area_sq_degrees();
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
    let num_times = if time_strings.is_empty() {
        1
    } else {
        time_strings.len()
    };

    let resolution = 0.05; // Conservative estimate
    let estimate = ResponseSizeEstimate::for_area(
        params_to_query.len(),
        num_times,
        z_values.len(),
        area_sq_degrees,
        resolution,
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

                // Validate instance exists
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

    // Parse time strings to DateTime<Utc>
    let parsed_times: Vec<DateTime<Utc>> = time_strings
        .iter()
        .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .collect();

    let query_time = parsed_times.first().copied();

    // Determine grid resolution
    // Use a reasonable default max grid cells (100x100 = 10000)
    let max_grid_cells = 10000_usize;
    let (res_x, res_y) = calculate_resolution(&params, &bbox, max_grid_cells);

    // Build grid bounding box
    let grid_bbox = BoundingBox::new(bbox.west, bbox.south, bbox.east, bbox.north);

    // Determine vertical CRS type based on collection
    let level_type = &collection_def.level_filter.level_type;
    let is_isobaric = level_type == "isobaric" || level_type.contains("pressure");

    // Build shared parameters for the collection
    let mut shared_params: HashMap<String, CovJsonParameter> = HashMap::new();
    for param_name in &params_to_query {
        let param_def = collection_def
            .parameters
            .iter()
            .find(|p| p.name == *param_name);

        let cov_param = if let Some(pd) = param_def {
            CovJsonParameter::new(&pd.name)
        } else {
            CovJsonParameter::new(param_name)
        };

        shared_params.insert(param_name.clone(), cov_param);
    }

    // Build referencing for the collection
    let referencing = build_cube_referencing(is_isobaric);

    // Build a Coverage for each z-level
    let mut coverages: Vec<CoverageJson> = Vec::new();

    for z_val in &z_values {
        // Get the first param to determine grid size
        let first_param = match params_to_query.first() {
            Some(p) => p,
            None => continue,
        };

        let param_def = collection_def
            .parameters
            .iter()
            .find(|p| p.name == *first_param);

        let level_str = build_level_string(&collection_def.level_filter, param_def, Some(*z_val));

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

        // Read region to get grid dimensions
        let region = match state
            .grid_data_service
            .read_region(&query, &grid_bbox, None)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Failed to read region for z={}: {}", z_val, e);
                continue;
            }
        };

        // Calculate output grid dimensions based on resolution params
        let (out_width, out_height) = if res_x > 0 && res_y > 0 {
            (res_x as usize, res_y as usize)
        } else {
            (region.width, region.height)
        };

        // Build domain with Regular axes (start/stop/num format)
        let t_value = if !time_strings.is_empty() {
            Some(time_strings[0].clone())
        } else {
            None
        };

        let domain = Domain::cube_grid(
            bbox.west,
            bbox.east,
            out_width,
            // Note: y goes from north to south (top to bottom)
            bbox.north,
            bbox.south,
            out_height,
            t_value.clone(),
            *z_val,
        );

        // Build ranges for each parameter
        let mut ranges: HashMap<String, NdArray> = HashMap::new();

        for param_name in &params_to_query {
            let param_def = collection_def
                .parameters
                .iter()
                .find(|p| p.name == *param_name);

            let level_str =
                build_level_string(&collection_def.level_filter, param_def, Some(*z_val));

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

            // Update shared parameter with units if we have them
            if !units_str.is_empty() {
                if let Some(shared_param) = shared_params.get_mut(param_name) {
                    if shared_param.unit.is_none() {
                        *shared_param = shared_param
                            .clone()
                            .with_unit(Unit::from_symbol(&units_str));
                    }
                }
            }

            // Read the region
            match state
                .grid_data_service
                .read_region(&query, &grid_bbox, None)
                .await
            {
                Ok(param_region) => {
                    // Resample if needed
                    let values =
                        if out_width == param_region.width && out_height == param_region.height {
                            // Native resolution
                            param_region
                                .data
                                .iter()
                                .map(|&v| if v.is_nan() { None } else { Some(v) })
                                .collect()
                        } else {
                            // Resample to requested resolution
                            resample_grid(
                                &param_region.data,
                                param_region.width,
                                param_region.height,
                                out_width,
                                out_height,
                            )
                        };

                    // Shape: [t, y, x, z] per the IBL example
                    let shape = vec![1, out_height, out_width, 1];
                    let axis_names = vec![
                        "t".to_string(),
                        "y".to_string(),
                        "x".to_string(),
                        "z".to_string(),
                    ];

                    ranges.insert(
                        param_name.clone(),
                        NdArray::with_missing(values, shape, axis_names),
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to query {}/{} at z={}: {}",
                        model_config.model,
                        param_name,
                        z_val,
                        e
                    );

                    // Add null values
                    let null_values: Vec<Option<f32>> = vec![None; out_height * out_width];
                    let shape = vec![1, out_height, out_width, 1];
                    let axis_names = vec![
                        "t".to_string(),
                        "y".to_string(),
                        "x".to_string(),
                        "z".to_string(),
                    ];

                    ranges.insert(
                        param_name.clone(),
                        NdArray::with_missing(null_values, shape, axis_names),
                    );
                }
            }
        }

        // Create coverage for this z-level
        let coverage = CoverageJson {
            type_: CoverageType::Coverage,
            domain,
            parameters: None, // Parameters are at collection level
            ranges: Some(ranges),
        };

        coverages.push(coverage);
    }

    // Build the CoverageCollection
    let collection = CoverageCollection::new()
        .with_domain_type(DomainType::Grid)
        .with_parameters(shared_params)
        .with_referencing(referencing);

    // Add all coverages
    let mut final_collection = collection;
    for cov in coverages {
        final_collection = final_collection.with_coverage(cov);
    }

    // Serialize response based on requested format
    let (json, content_type) = match output_format {
        OutputFormat::GeoJson => {
            let geojson = EdrFeatureCollection::from(&final_collection);
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
        OutputFormat::CoverageJson => match serde_json::to_string_pretty(&final_collection) {
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

/// Build referencing for cube response.
fn build_cube_referencing(is_isobaric: bool) -> Vec<ReferenceSystemConnection> {
    let refs = vec![
        // Geographic CRS for x/y
        ReferenceSystemConnection {
            coordinates: vec!["y".to_string(), "x".to_string()],
            system: ReferenceSystem::geographic_with_wkt(
                "http://www.opengis.net/def/crs/EPSG/0/4326",
                WGS84_WKT,
            ),
        },
        // Vertical CRS for z
        ReferenceSystemConnection {
            coordinates: vec!["z".to_string()],
            system: if is_isobaric {
                ReferenceSystem::vertical_with_cs(VerticalCoordinateSystem::isobaric())
            } else {
                ReferenceSystem::vertical_with_cs(VerticalCoordinateSystem::height_above_ground())
            },
        },
        // Temporal CRS for t
        ReferenceSystemConnection {
            coordinates: vec!["t".to_string()],
            system: ReferenceSystem::temporal_gregorian(),
        },
    ];

    refs
}

/// Calculate output resolution based on params and limits.
fn calculate_resolution(
    params: &CubeQueryParams,
    bbox: &BboxQuery,
    max_cells: usize,
) -> (u32, u32) {
    let res_x = params.resolution_x.unwrap_or(0);
    let res_y = params.resolution_y.unwrap_or(0);

    if res_x == 0 && res_y == 0 {
        // Use native resolution (return 0,0 to signal this)
        return (0, 0);
    }

    // If only one dimension specified, calculate the other to maintain aspect ratio
    let bbox_width = (bbox.east - bbox.west).abs();
    let bbox_height = (bbox.north - bbox.south).abs();
    let aspect = bbox_width / bbox_height;

    let (out_x, out_y) = if res_x > 0 && res_y > 0 {
        (res_x, res_y)
    } else if res_x > 0 {
        let y = (res_x as f64 / aspect).round() as u32;
        (res_x, y.max(1))
    } else {
        let x = (res_y as f64 * aspect).round() as u32;
        (x.max(1), res_y)
    };

    // Ensure we don't exceed max cells
    let total = out_x as usize * out_y as usize;
    if total > max_cells {
        let scale = (max_cells as f64 / total as f64).sqrt();
        let new_x = (out_x as f64 * scale).round() as u32;
        let new_y = (out_y as f64 * scale).round() as u32;
        (new_x.max(1), new_y.max(1))
    } else {
        (out_x, out_y)
    }
}

/// Resample grid data to a new resolution using nearest neighbor.
fn resample_grid(
    data: &[f32],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
) -> Vec<Option<f32>> {
    let mut result = Vec::with_capacity(dst_width * dst_height);

    for dst_y in 0..dst_height {
        for dst_x in 0..dst_width {
            // Map destination coords to source coords
            let src_x = (dst_x as f64 * (src_width - 1) as f64 / (dst_width - 1).max(1) as f64)
                .round() as usize;
            let src_y = (dst_y as f64 * (src_height - 1) as f64 / (dst_height - 1).max(1) as f64)
                .round() as usize;

            let src_x = src_x.min(src_width - 1);
            let src_y = src_y.min(src_height - 1);

            let idx = src_y * src_width + src_x;
            let value = data[idx];

            if value.is_nan() {
                result.push(None);
            } else {
                result.push(Some(value));
            }
        }
    }

    result
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
    fn test_calculate_resolution_native() {
        let params = CubeQueryParams {
            bbox: Some("-100,35,-95,40".to_string()),
            z: Some("850".to_string()),
            datetime: None,
            parameter_name: None,
            resolution_x: None,
            resolution_y: None,
            crs: None,
            f: None,
        };
        let bbox = BboxQuery::parse("-100,35,-95,40").unwrap();

        let (x, y) = calculate_resolution(&params, &bbox, 10000);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn test_calculate_resolution_specified() {
        let params = CubeQueryParams {
            bbox: Some("-100,35,-95,40".to_string()),
            z: Some("850".to_string()),
            datetime: None,
            parameter_name: None,
            resolution_x: Some(10),
            resolution_y: Some(10),
            crs: None,
            f: None,
        };
        let bbox = BboxQuery::parse("-100,35,-95,40").unwrap();

        let (x, y) = calculate_resolution(&params, &bbox, 10000);
        assert_eq!(x, 10);
        assert_eq!(y, 10);
    }

    #[test]
    fn test_calculate_resolution_exceeds_limit() {
        let params = CubeQueryParams {
            bbox: Some("-100,35,-95,40".to_string()),
            z: Some("850".to_string()),
            datetime: None,
            parameter_name: None,
            resolution_x: Some(200),
            resolution_y: Some(200),
            crs: None,
            f: None,
        };
        let bbox = BboxQuery::parse("-100,35,-95,40").unwrap();

        let (x, y) = calculate_resolution(&params, &bbox, 1000);
        // Should be scaled down - the algorithm scales by sqrt, so we expect roughly sqrt(1000/40000) * 200 = ~31
        // Allow some tolerance since we use max(1) and rounding
        assert!(x < 200 && y < 200);
        assert!(x > 0 && y > 0);
    }

    #[test]
    fn test_resample_grid() {
        let data = vec![1.0, 2.0, 3.0, 4.0]; // 2x2 grid
        let result = resample_grid(&data, 2, 2, 4, 4);

        assert_eq!(result.len(), 16);
        // Corners should match original values
        assert_eq!(result[0], Some(1.0)); // top-left
        assert_eq!(result[3], Some(2.0)); // top-right
        assert_eq!(result[12], Some(3.0)); // bottom-left
        assert_eq!(result[15], Some(4.0)); // bottom-right
    }

    #[test]
    fn test_resample_grid_with_nan() {
        let data = vec![1.0, f32::NAN, 3.0, 4.0];
        let result = resample_grid(&data, 2, 2, 2, 2);

        assert_eq!(result[0], Some(1.0));
        assert_eq!(result[1], None); // NaN becomes None
        assert_eq!(result[2], Some(3.0));
        assert_eq!(result[3], Some(4.0));
    }
}
