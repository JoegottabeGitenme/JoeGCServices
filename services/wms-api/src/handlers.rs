//! HTTP request handlers for WMS and WMTS.

use axum::{
    extract::{Extension, Path, Query},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, instrument, error};

use storage::CacheKey;
use wms_common::{tile::web_mercator_tile_matrix_set, BoundingBox, CrsCode, TileCoord};
use bytes::Bytes;
use renderer::gradient;

use crate::state::AppState;

// ============================================================================
// WMS Handlers
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct WmsParams {
    #[serde(rename = "SERVICE", alias = "service")]
    service: Option<String>,
    #[serde(rename = "REQUEST", alias = "request")]
    request: Option<String>,
    #[serde(rename = "VERSION", alias = "version")]
    version: Option<String>,
    #[serde(rename = "LAYERS", alias = "layers")]
    layers: Option<String>,
    #[serde(rename = "STYLES", alias = "styles")]
    styles: Option<String>,
    #[serde(rename = "CRS", alias = "SRS", alias = "crs", alias = "srs")]
    crs: Option<String>,
    #[serde(rename = "BBOX", alias = "bbox")]
    bbox: Option<String>,
    #[serde(rename = "WIDTH", alias = "width")]
    width: Option<u32>,
    #[serde(rename = "HEIGHT", alias = "height")]
    height: Option<u32>,
    #[serde(rename = "FORMAT", alias = "format")]
    format: Option<String>,
    #[serde(rename = "TIME", alias = "time")]
    time: Option<String>,
    #[serde(rename = "TRANSPARENT", alias = "transparent")]
    transparent: Option<String>,
    // GetFeatureInfo parameters
    #[serde(rename = "QUERY_LAYERS", alias = "query_layers")]
    query_layers: Option<String>,
    #[serde(rename = "INFO_FORMAT", alias = "info_format")]
    info_format: Option<String>,
    #[serde(rename = "I", alias = "i", alias = "X", alias = "x")]
    i: Option<u32>,
    #[serde(rename = "J", alias = "j", alias = "Y", alias = "y")]
    j: Option<u32>,
    #[serde(rename = "FEATURE_COUNT", alias = "feature_count")]
    feature_count: Option<u32>,
}

#[instrument(skip(state))]
pub async fn wms_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<WmsParams>,
) -> Response {
    // Normalize service parameter to uppercase for comparison
    let service = params.service.as_deref().map(|s| s.to_uppercase());
    if service.as_deref() != Some("WMS") {
        return wms_exception(
            "InvalidParameterValue",
            "SERVICE must be WMS",
            StatusCode::BAD_REQUEST,
        );
    }

    // Normalize request parameter to match pattern
    let request = params.request.as_deref().map(|s| s.to_uppercase());
    match request.as_deref() {
        Some("GETCAPABILITIES") => wms_get_capabilities(state, params).await,
        Some("GETMAP") => wms_get_map(state, params).await,
        Some("GETFEATUREINFO") => wms_get_feature_info(state, params).await,
        Some(req) => wms_exception(
            "OperationNotSupported",
            &format!("Unknown request: {}", req),
            StatusCode::BAD_REQUEST,
        ),
        None => wms_exception(
            "MissingParameterValue",
            "REQUEST is required",
            StatusCode::BAD_REQUEST,
        ),
    }
}

async fn wms_get_capabilities(state: Arc<AppState>, params: WmsParams) -> Response {
    let version = params.version.as_deref().unwrap_or("1.3.0");
    let models = state.catalog.list_models().await.unwrap_or_default();
    
    // Get parameters and dimensions for each model
    let mut model_params = HashMap::new();
    let mut model_dimensions: HashMap<String, (Vec<String>, Vec<i32>)> = HashMap::new();
    for model in &models {
        let params_list = state.catalog.list_parameters(model).await.unwrap_or_default();
        model_params.insert(model.clone(), params_list);
        
        // Get RUN and FORECAST dimensions
        let dimensions = state.catalog.get_model_dimensions(model).await.unwrap_or_default();
        model_dimensions.insert(model.clone(), dimensions);
    }
    
    let xml = build_wms_capabilities_xml(version, &models, &model_params, &model_dimensions);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(xml.into())
        .unwrap()
}

async fn wms_get_map(state: Arc<AppState>, params: WmsParams) -> Response {
    let layers = match &params.layers {
        Some(l) => l,
        None => {
            return wms_exception(
                "MissingParameterValue",
                "LAYERS is required",
                StatusCode::BAD_REQUEST,
            )
        }
    };
    let width = params.width.unwrap_or(256);
    let height = params.height.unwrap_or(256);
    let style = params.styles.as_deref().unwrap_or("default");
    let bbox = params.bbox.as_deref();
    let crs = params.crs.as_deref();
    let time = params.time.clone();

    info!(layer = %layers, style = %style, width = width, height = height, bbox = ?bbox, crs = ?crs, time = ?time, "GetMap request");
    
    // Try to render actual data, return error on failure
    match render_weather_data(&state, layers, style, width, height, bbox, crs, time.as_deref()).await {
        Ok(png_data) => {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(png_data.into())
                .unwrap()
        }
        Err(e) => {
            error!(error = %e, "Rendering failed");
            wms_exception(
                "NoApplicableCode",
                &format!("Rendering failed: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        }
    }
}

async fn wms_get_feature_info(state: Arc<AppState>, params: WmsParams) -> Response {
    use wms_protocol::{InfoFormat, FeatureInfoResponse};
    
    // Validate required parameters
    let query_layers = match &params.query_layers {
        Some(l) => l,
        None => {
            return wms_exception(
                "MissingParameterValue",
                "QUERY_LAYERS is required",
                StatusCode::BAD_REQUEST,
            )
        }
    };
    
    let bbox = match &params.bbox {
        Some(b) => b,
        None => {
            return wms_exception(
                "MissingParameterValue",
                "BBOX is required",
                StatusCode::BAD_REQUEST,
            )
        }
    };
    
    let width = params.width.unwrap_or(256);
    let height = params.height.unwrap_or(256);
    let crs = params.crs.as_deref().unwrap_or("EPSG:4326");
    
    let i = match params.i {
        Some(i) => i,
        None => {
            return wms_exception(
                "MissingParameterValue",
                "I (or X) parameter is required",
                StatusCode::BAD_REQUEST,
            )
        }
    };
    
    let j = match params.j {
        Some(j) => j,
        None => {
            return wms_exception(
                "MissingParameterValue",
                "J (or Y) parameter is required",
                StatusCode::BAD_REQUEST,
            )
        }
    };
    
    // Parse INFO_FORMAT
    let info_format = params
        .info_format
        .as_deref()
        .and_then(|f| InfoFormat::from_mime(f))
        .unwrap_or(InfoFormat::Html);
    
    // Parse BBOX
    let bbox_coords: Result<Vec<f64>, _> = bbox
        .split(',')
        .map(|s| s.trim().parse())
        .collect();
    
    let bbox_array = match bbox_coords {
        Ok(coords) if coords.len() == 4 => {
            [coords[0], coords[1], coords[2], coords[3]]
        }
        _ => {
            return wms_exception(
                "InvalidParameterValue",
                "BBOX must contain 4 coordinates",
                StatusCode::BAD_REQUEST,
            )
        }
    };
    
    info!(
        query_layers = %query_layers,
        bbox = ?bbox_array,
        width = width,
        height = height,
        i = i,
        j = j,
        crs = crs,
        info_format = ?info_format,
        "GetFeatureInfo request"
    );
    
    // Query each layer
    let layers: Vec<&str> = query_layers.split(',').map(|s| s.trim()).collect();
    let mut all_features = Vec::new();
    
    for layer in layers {
        match crate::rendering::query_point_value(
            &state.storage,
            &state.catalog,
            layer,
            bbox_array,
            width,
            height,
            i,
            j,
            crs,
        )
        .await
        {
            Ok(mut features) => {
                all_features.append(&mut features);
            }
            Err(e) => {
                error!(layer = %layer, error = %e, "Failed to query layer");
                // Continue with other layers instead of failing completely
            }
        }
    }
    
    let response = FeatureInfoResponse::new(all_features);
    
    // Format response based on INFO_FORMAT
    let (body, content_type) = match info_format {
        InfoFormat::Json => {
            match response.to_json() {
                Ok(json) => (json, "application/json"),
                Err(e) => {
                    return wms_exception(
                        "NoApplicableCode",
                        &format!("JSON encoding failed: {}", e),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                }
            }
        }
        InfoFormat::Html => {
            (response.to_html(), "text/html")
        }
        InfoFormat::Xml => {
            (response.to_xml(), "text/xml")
        }
        InfoFormat::Text => {
            (response.to_text(), "text/plain")
        }
    };
    
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body.into())
        .unwrap()
}

// ============================================================================
// WMTS Handlers
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct WmtsKvpParams {
    #[serde(rename = "SERVICE")]
    service: Option<String>,
    #[serde(rename = "REQUEST")]
    request: Option<String>,
    #[serde(rename = "LAYER")]
    layer: Option<String>,
    #[serde(rename = "STYLE")]
    style: Option<String>,
    #[serde(rename = "TILEMATRIXSET")]
    tile_matrix_set: Option<String>,
    #[serde(rename = "TILEMATRIX")]
    tile_matrix: Option<String>,
    #[serde(rename = "TILEROW")]
    tile_row: Option<u32>,
    #[serde(rename = "TILECOL")]
    tile_col: Option<u32>,
    #[serde(rename = "FORMAT")]
    format: Option<String>,
    #[serde(rename = "TIME")]
    time: Option<String>,
}

#[instrument(skip(state))]
pub async fn wmts_kvp_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<WmtsKvpParams>,
) -> Response {
    if params.service.as_deref() != Some("WMTS") {
        return wmts_exception(
            "InvalidParameterValue",
            "SERVICE must be WMTS",
            StatusCode::BAD_REQUEST,
        );
    }

    match params.request.as_deref() {
        Some("GetCapabilities") => wmts_get_capabilities(state).await,
        Some("GetTile") => {
            let layer = params.layer.clone().unwrap_or_default();
            let style = params
                .style
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let tile_matrix = params.tile_matrix.clone().unwrap_or_default();
            let tile_row = params.tile_row.unwrap_or(0);
            let tile_col = params.tile_col.unwrap_or(0);
            let z: u32 = tile_matrix.parse().unwrap_or(0);
            wmts_get_tile(state, &layer, &style, z, tile_col, tile_row).await
        }
        _ => wmts_exception(
            "MissingParameterValue",
            "REQUEST is required",
            StatusCode::BAD_REQUEST,
        ),
    }
}

#[instrument(skip(state))]
pub async fn wmts_rest_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(path): Path<String>,
) -> Response {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if parts.len() < 6 {
        return wmts_exception(
            "InvalidParameterValue",
            "Invalid path",
            StatusCode::BAD_REQUEST,
        );
    }
    // URL format: {layer}/{style}/{TileMatrixSet}/{z}/{x}/{y}.png
    // Leaflet sends tiles in XYZ convention where:
    //   z = zoom level (TileMatrix)
    //   x = column (longitude direction, 0 at left/west)
    //   y = row (latitude direction, 0 at top/north)
    let layer = parts[0];
    let style = parts[1];
    // parts[2] = TileMatrixSet (e.g., "WebMercatorQuad")
    let z: u32 = parts[3].parse().unwrap_or(0);
    let x: u32 = parts[4].parse().unwrap_or(0);  // Column (X)
    let last = parts[5];
    let (y_str, _) = last.rsplit_once('.').unwrap_or((last, "png"));
    let y: u32 = y_str.parse().unwrap_or(0);  // Row (Y)
    
    wmts_get_tile(state, layer, style, z, x, y).await
}

#[instrument(skip(state))]
pub async fn xyz_tile_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((layer, style, z, x, y)): Path<(String, String, u32, u32, String)>,
) -> Response {
    let (y_str, _) = y.rsplit_once('.').unwrap_or((&y, "png"));
    let y_val: u32 = y_str.parse().unwrap_or(0);
    wmts_get_tile(state, &layer, &style, z, x, y_val).await
}

async fn wmts_get_capabilities(state: Arc<AppState>) -> Response {
    let models = state.catalog.list_models().await.unwrap_or_default();
    
    // Get parameters and dimensions for each model
    let mut model_params = HashMap::new();
    let mut model_dimensions: HashMap<String, (Vec<String>, Vec<i32>)> = HashMap::new();
    for model in &models {
        let params_list = state.catalog.list_parameters(model).await.unwrap_or_default();
        model_params.insert(model.clone(), params_list);
        
        // Get RUN and FORECAST dimensions
        let dimensions = state.catalog.get_model_dimensions(model).await.unwrap_or_default();
        model_dimensions.insert(model.clone(), dimensions);
    }
    
    let xml = build_wmts_capabilities_xml(&models, &model_params, &model_dimensions);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(xml.into())
        .unwrap()
}

async fn wmts_get_tile(
    state: Arc<AppState>,
    layer: &str,
    style: &str,
    z: u32,
    x: u32,
    y: u32,
) -> Response {
    info!(layer = %layer, style = %style, z = z, x = x, y = y, "GetTile request");
    
    // Get tile bounding box using Web Mercator tile matrix set
    let tms = web_mercator_tile_matrix_set();
    let coord = TileCoord::new(z, x, y);
    
    let bbox = match tms.tile_bbox(&coord) {
        Some(bbox) => bbox,
        None => return wmts_exception("TileOutOfRange", "Invalid tile", StatusCode::BAD_REQUEST),
    };
    
    // Convert Web Mercator bbox to WGS84 (lat/lon) for GRIB data
    // GRIB data is in geographic coordinates (EPSG:4326)
    let latlon_bbox = wms_common::tile::tile_to_latlon_bounds(&coord);
    
    // Format bbox as [min_lon, min_lat, max_lon, max_lat]
    let bbox_array = [
        latlon_bbox.min_x as f32,
        latlon_bbox.min_y as f32,
        latlon_bbox.max_x as f32,
        latlon_bbox.max_y as f32,
    ];
    
    info!(
        z = z,
        x = x, 
        y = y,
        min_lon = bbox_array[0],
        min_lat = bbox_array[1],
        max_lon = bbox_array[2],
        max_lat = bbox_array[3],
        "Tile bbox"
    );
    
    // Parse layer name (format: "model_parameter")
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return wmts_exception(
            "InvalidParameterValue",
            "Invalid layer format",
            StatusCode::BAD_REQUEST,
        );
    }
    
    let model = parts[0];
    let parameter = parts[1..].join("_");
    
    // Check if this is a wind barbs composite layer
    let result = if parameter == "WIND_BARBS" {
        crate::rendering::render_wind_barbs_tile(
            &state.storage,
            &state.catalog,
            model,
            Some(coord),  // Pass tile coordinate for expanded rendering
            256,  // tile width
            256,  // tile height
            bbox_array,
        )
        .await
    } else if style == "isolines" {
        // Render isolines (contours) for this parameter
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            format!("{}/temperature_isolines.json", style_config_dir)
        } else {
            // Default to temperature isolines for now
            format!("{}/temperature_isolines.json", style_config_dir)
        };
        
        crate::rendering::render_isolines_tile(
            &state.storage,
            &state.catalog,
            model,
            &parameter,
            Some(coord),  // Pass tile coordinate for expanded rendering
            256,  // tile width
            256,  // tile height
            bbox_array,
            &style_file,
        )
        .await
    } else {
        // Render the tile with spatial subsetting
        crate::rendering::render_weather_data(
            &state.storage,
            &state.catalog,
            model,
            &parameter,
            None, // forecast_hour (use default/latest)
            256,  // tile width
            256,  // tile height
            Some(bbox_array),
        )
        .await
    };
    
    match result
    {
        Ok(png_data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/png")
            .header(header::CACHE_CONTROL, "max-age=3600")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(png_data.into())
            .unwrap(),
        Err(e) => {
            error!(error = %e, "WMTS tile rendering failed");
            wmts_exception(
                "NoApplicableCode",
                &format!("Rendering failed: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        }
    }
}

// ============================================================================
// Health
// ============================================================================

pub async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}
pub async fn ready_handler(Extension(state): Extension<Arc<AppState>>) -> impl IntoResponse {
    match state.catalog.list_models().await {
        Ok(_) => (StatusCode::OK, "Ready"),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "Not ready"),
    }
}
pub async fn metrics_handler() -> impl IntoResponse {
    (StatusCode::OK, "# metrics\n")
}

// ============================================================================
// Rendering
// ============================================================================

/// Convert Web Mercator (EPSG:3857) coordinates to WGS84 (EPSG:4326)
fn mercator_to_wgs84(x: f64, y: f64) -> (f64, f64) {
    let lon = (x / 20037508.34) * 180.0;
    let lat = (y / 20037508.34) * 180.0;
    let lat = 180.0 / std::f64::consts::PI * (2.0 * (lat * std::f64::consts::PI / 180.0).exp().atan() - std::f64::consts::PI / 2.0);
    (lon, lat)
}

async fn render_weather_data(
    state: &Arc<AppState>,
    layer: &str,
    style: &str,
    width: u32,
    height: u32,
    bbox: Option<&str>,
    crs: Option<&str>,
    time: Option<&str>,
) -> Result<Vec<u8>, String> {
    // Parse layer name (format: "model_parameter" or "model_WIND_BARBS")
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return Err("Invalid layer format".to_string());
    }

    let model = parts[0];
    let parameter = parts[1..].join("_");
    
    // Check if this is a wind barbs composite layer
    if parameter == "WIND_BARBS" {
        // Handle wind barbs specially - it combines UGRD and VGRD
        let parsed_bbox = bbox.and_then(|b| {
            let coords: Vec<f64> = b.split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            
            if coords.len() == 4 {
                let crs_str = crs.unwrap_or("EPSG:4326");
                let (min_lon, min_lat, max_lon, max_lat) = if crs_str.contains("3857") {
                    let (min_lon, min_lat) = mercator_to_wgs84(coords[0], coords[1]);
                    let (max_lon, max_lat) = mercator_to_wgs84(coords[2], coords[3]);
                    (min_lon, min_lat, max_lon, max_lat)
                } else {
                    (coords[0], coords[1], coords[2], coords[3])
                };
                
                Some([min_lon as f32, min_lat as f32, max_lon as f32, max_lat as f32])
            } else {
                None
            }
        });
        
        return crate::rendering::render_wind_barbs_layer(
            &state.storage,
            &state.catalog,
            model,
            width,
            height,
            parsed_bbox,
            None, // Use default barb spacing
        )
        .await;
    }

    // Parse BBOX parameter (format depends on CRS)
    let parsed_bbox = bbox.and_then(|b| {
        let coords: Vec<f64> = b.split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        
        if coords.len() == 4 {
            // Check CRS and convert if needed
            let crs_str = crs.unwrap_or("EPSG:4326");
            let (min_lon, min_lat, max_lon, max_lat) = if crs_str.contains("3857") {
                // Web Mercator - convert to WGS84
                let (min_lon, min_lat) = mercator_to_wgs84(coords[0], coords[1]);
                let (max_lon, max_lat) = mercator_to_wgs84(coords[2], coords[3]);
                (min_lon, min_lat, max_lon, max_lat)
            } else {
                // Assume WGS84/EPSG:4326
                (coords[0], coords[1], coords[2], coords[3])
            };
            
            Some([min_lon as f32, min_lat as f32, max_lon as f32, max_lat as f32])
        } else {
            None
        }
    });

    // Parse TIME parameter (format: "H" for forecast hour)
    let forecast_hour: Option<u32> = time.and_then(|t| t.parse().ok());
    
    info!(forecast_hour = ?forecast_hour, bbox = ?parsed_bbox, style = style, "Parsed WMS parameters");

    // Check if isolines style is requested
    if style == "isolines" {
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            format!("{}/temperature_isolines.json", style_config_dir)
        } else {
            // Default to temperature isolines for now
            format!("{}/temperature_isolines.json", style_config_dir)
        };
        
        // For WMS, we don't have tile coordinates, so pass None
        return crate::rendering::render_isolines_tile(
            &state.storage,
            &state.catalog,
            model,
            &parameter,
            None,  // No tile coordinate - render full bbox
            width,
            height,
            parsed_bbox.unwrap_or([-180.0, -90.0, 180.0, 90.0]),
            &style_file,
        )
        .await;
    }

    // Use shared rendering logic
    crate::rendering::render_weather_data(
        &state.storage,
        &state.catalog,
        model,
        &parameter,
        forecast_hour,
        width,
        height,
        parsed_bbox,
    )
    .await
}

// ============================================================================
// API: Forecast Times and Parameters
// ============================================================================

/// Response for available forecast times
#[derive(Debug, Serialize)]
pub struct ForecastTimesResponse {
    pub model: String,
    pub parameter: String,
    pub reference_time: String,
    pub forecast_hours: Vec<u32>,
}

/// Response for available parameters
#[derive(Debug, Serialize)]
pub struct ParametersResponse {
    pub model: String,
    pub parameters: Vec<String>,
}

/// Get available forecast hours for a parameter
#[instrument(skip(state))]
pub async fn forecast_times_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((model, parameter)): Path<(String, String)>,
) -> impl IntoResponse {
    // Query all forecast hours for this model/parameter combination
    match state.catalog.find_datasets(&storage::catalog::DatasetQuery {
        model: Some(model.clone()),
        parameter: Some(parameter.clone()),
        level: None,
        time_range: None,
        bbox: None,
    }).await {
        Ok(entries) => {
            // Collect unique forecast hours
            let mut hours: Vec<u32> = entries.iter().map(|e| e.forecast_hour).collect();
            hours.sort_unstable();
            hours.dedup();
            
            // Get reference time from first entry
            let reference_time = entries.first()
                .map(|e| e.reference_time.to_rfc3339())
                .unwrap_or_else(|| "unknown".to_string());
            
            let response = ForecastTimesResponse {
                model,
                parameter,
                reference_time,
                forecast_hours: hours,
            };
            (StatusCode::OK, Json(response))
        }
        Err(_) => {
            let response = ForecastTimesResponse {
                model,
                parameter,
                reference_time: "unknown".to_string(),
                forecast_hours: vec![],
            };
            (StatusCode::OK, Json(response))
        }
    }
}

/// Get available parameters for a model
#[instrument(skip(state))]
pub async fn parameters_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(model): Path<String>,
) -> impl IntoResponse {
    match state.catalog.list_parameters(&model).await {
        Ok(parameters) => {
            let response = ParametersResponse {
                model,
                parameters,
            };
            (StatusCode::OK, Json(response))
        }
        Err(_) => {
            let response = ParametersResponse {
                model,
                parameters: vec![],
            };
            (StatusCode::OK, Json(response))
        }
    }
}

// ============================================================================
// Ingestion Events API
// ============================================================================

#[derive(Debug, Serialize)]
pub struct IngestionEvent {
    pub model: String,
    pub parameter: String,
    pub level: String,
    pub reference_time: String,
    pub forecast_hour: u32,
    pub file_size: u64,
}

#[instrument(skip(state))]
pub async fn ingestion_events_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<Vec<IngestionEvent>> {
    // Get recent ingestions from the last 60 minutes
    match state.catalog.get_recent_ingestions(60).await {
        Ok(entries) => {
            let events = entries
                .into_iter()
                .map(|entry| IngestionEvent {
                    model: entry.model,
                    parameter: entry.parameter,
                    level: entry.level,
                    reference_time: entry.reference_time.to_rfc3339(),
                    forecast_hour: entry.forecast_hour,
                    file_size: entry.file_size,
                })
                .collect();
            Json(events)
        }
        Err(_) => Json(Vec::new()),
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn wms_exception(code: &str, msg: &str, status: StatusCode) -> Response {
    let xml = format!(
        r#"<?xml version="1.0"?><ServiceExceptionReport><ServiceException code="{}">{}</ServiceException></ServiceExceptionReport>"#,
        code, msg
    );
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/xml")
        .body(xml.into())
        .unwrap()
}

fn wmts_exception(code: &str, msg: &str, status: StatusCode) -> Response {
    let xml = format!(
        r#"<?xml version="1.0"?><ows:ExceptionReport xmlns:ows="http://www.opengis.net/ows/1.1"><ows:Exception exceptionCode="{}"><ows:ExceptionText>{}</ows:ExceptionText></ows:Exception></ows:ExceptionReport>"#,
        code, msg
    );
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/xml")
        .body(xml.into())
        .unwrap()
}

fn build_wms_capabilities_xml(
    version: &str,
    models: &[String],
    model_params: &HashMap<String, Vec<String>>,
    model_dimensions: &HashMap<String, (Vec<String>, Vec<i32>)>,
) -> String {
    let empty_params = Vec::new();
    let empty_dims = (Vec::new(), Vec::new());
    
    let layers: String = models
        .iter()
        .map(|model| {
            let params = model_params.get(model).unwrap_or(&empty_params);
            let (runs, forecasts) = model_dimensions.get(model).unwrap_or(&empty_dims);
            
            // Build dimension elements
            let run_values = if runs.is_empty() { "latest".to_string() } else { runs.join(",") };
            let run_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
            let forecast_values = if forecasts.is_empty() { "0".to_string() } else { forecasts.iter().map(|h| h.to_string()).collect::<Vec<_>>().join(",") };
            let forecast_default = forecasts.first().unwrap_or(&0);
            
            let dimensions = format!(
                r#"<Dimension name="RUN" units="ISO8601" default="{}">{}</Dimension><Dimension name="FORECAST" units="hours" default="{}">{}</Dimension>"#,
                run_default, run_values, forecast_default, forecast_values
            );
            
            let param_layers = params
                .iter()
                .map(|p| {
                    // Add styles and dimensions to each layer
                    let styles = if p.contains("TMP") || p.contains("TEMP") {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>temperature</Name><Title>Temperature Gradient</Title></Style><Style><Name>isolines</Name><Title>Temperature Isolines</Title></Style>"
                    } else if p.contains("WIND") || p.contains("GUST") {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>wind</Name><Title>Wind Speed</Title></Style>"
                    } else if p.contains("PRES") || p.contains("PRMSL") {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>atmospheric</Name><Title>Atmospheric Pressure</Title></Style>"
                    } else if p.contains("RH") || p.contains("HUMID") || p.contains("PRECIP") {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>precipitation</Name><Title>Precipitation</Title></Style>"
                    } else {
                        "<Style><Name>default</Name><Title>Default</Title></Style>"
                    };
                    
                    format!(
                        r#"<Layer queryable="1"><Name>{}_{}</Name><Title>{} - {}</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>-180</westBoundLongitude><eastBoundLongitude>180</eastBoundLongitude><southBoundLatitude>-90</southBoundLatitude><northBoundLatitude>90</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="-180" miny="-90" maxx="180" maxy="90"/>{}{}</Layer>"#,
                        model, p, model.to_uppercase(), p, styles, dimensions
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            
             // Add composite wind barbs layer if we have both UGRD and VGRD
             let wind_barbs_layer = if params.contains(&"UGRD".to_string()) && params.contains(&"VGRD".to_string()) {
                 format!(
                     r#"<Layer queryable="1"><Name>{}_WIND_BARBS</Name><Title>{} - Wind Barbs</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>-180</westBoundLongitude><eastBoundLongitude>180</eastBoundLongitude><southBoundLatitude>-90</southBoundLatitude><northBoundLatitude>90</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="-180" miny="-90" maxx="180" maxy="90"/><Style><Name>default</Name><Title>Default Barbs</Title></Style>{}</Layer>"#,
                     model, model.to_uppercase(), dimensions
                 )
             } else {
                 String::new()
             };

             format!(
                 r#"<Layer><Name>{}</Name><Title>{}</Title>{}{}</Layer>"#,
                 model,
                 model.to_uppercase(),
                 param_layers,
                 wind_barbs_layer
             )
         })
         .collect();
     format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<WMS_Capabilities version="{}" xmlns="http://www.opengis.net/wms" xmlns:xlink="http://www.w3.org/1999/xlink">
  <Service>
    <Name>WMS</Name>
    <Title>Weather WMS Service</Title>
    <Abstract>Web Map Service for weather model data</Abstract>
    <OnlineResource xlink:href="http://localhost:8080/wms"/>
  </Service>
  <Capability>
    <Request>
      <GetCapabilities>
        <Format>text/xml</Format>
        <DCPType>
          <HTTP>
            <Get><OnlineResource xlink:href="http://localhost:8080/wms?"/></Get>
          </HTTP>
        </DCPType>
      </GetCapabilities>
      <GetMap>
        <Format>image/png</Format>
        <DCPType>
          <HTTP>
            <Get><OnlineResource xlink:href="http://localhost:8080/wms?"/></Get>
          </HTTP>
        </DCPType>
      </GetMap>
      <GetFeatureInfo>
        <Format>text/html</Format>
        <Format>application/json</Format>
        <Format>text/xml</Format>
        <Format>text/plain</Format>
        <DCPType>
          <HTTP>
            <Get><OnlineResource xlink:href="http://localhost:8080/wms?"/></Get>
          </HTTP>
        </DCPType>
      </GetFeatureInfo>
    </Request>
    <Exception>
      <Format>XML</Format>
    </Exception>
    <Layer>
      <Title>Weather Data</Title>
      <CRS>EPSG:4326</CRS>
      <CRS>EPSG:3857</CRS>
      {}
    </Layer>
  </Capability>
</WMS_Capabilities>"#,
        version, layers
    )
}

fn build_wmts_capabilities_xml(
    models: &[String], 
    model_params: &HashMap<String, Vec<String>>,
    model_dimensions: &HashMap<String, (Vec<String>, Vec<i32>)>,
) -> String {
    let empty_params = Vec::new();
    let empty_dims = (Vec::new(), Vec::new());
    
    // Build layer definitions for each model/parameter combination
    let mut all_layers: Vec<String> = models
        .iter()
        .flat_map(|model| {
            let params = model_params.get(model).unwrap_or(&empty_params);
            let (runs, forecasts) = model_dimensions.get(model).unwrap_or(&empty_dims);
            
            // Build dimension values
            let run_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
            let run_values: Vec<String> = if runs.is_empty() { 
                vec!["latest".to_string()] 
            } else { 
                runs.clone() 
            };
            let forecast_default = forecasts.first().unwrap_or(&0);
            let forecast_values: Vec<i32> = if forecasts.is_empty() { 
                vec![0] 
            } else { 
                forecasts.clone() 
            };
            
            params.iter().map(move |param| {
                let layer_id = format!("{}_{}", model, param);
                let layer_title = format!("{} - {}", model.to_uppercase(), param);
                
                // Determine available styles based on parameter type
                let styles = if param.contains("TMP") || param.contains("TEMP") {
                    r#"      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Temperature Gradient</ows:Title>
        <ows:Identifier>temperature</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Temperature Isolines</ows:Title>
        <ows:Identifier>isolines</ows:Identifier>
      </Style>"#
                } else if param.contains("WIND") || param.contains("GUST") {
                    r#"      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Wind Speed</ows:Title>
        <ows:Identifier>wind</ows:Identifier>
      </Style>"#
                } else if param.contains("PRES") || param.contains("PRMSL") {
                    r#"      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Atmospheric Pressure</ows:Title>
        <ows:Identifier>atmospheric</ows:Identifier>
      </Style>"#
                } else if param.contains("RH") || param.contains("HUMID") || param.contains("PRECIP") {
                    r#"      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Precipitation</ows:Title>
        <ows:Identifier>precipitation</ows:Identifier>
      </Style>"#
                } else {
                    r#"      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>"#
                };
                
                // Build dimension elements for RUN and FORECAST
                let run_values_xml: String = run_values.iter()
                    .map(|v| format!("        <Value>{}</Value>", v))
                    .collect::<Vec<_>>()
                    .join("\n");
                let forecast_values_xml: String = forecast_values.iter()
                    .map(|v| format!("        <Value>{}</Value>", v))
                    .collect::<Vec<_>>()
                    .join("\n");
                
                format!(
                    r#"    <Layer>
      <ows:Title>{}</ows:Title>
      <ows:Identifier>{}</ows:Identifier>
      <ows:WGS84BoundingBox>
        <ows:LowerCorner>-180.0 -90.0</ows:LowerCorner>
        <ows:UpperCorner>180.0 90.0</ows:UpperCorner>
      </ows:WGS84BoundingBox>
{}
      <Format>image/png</Format>
      <TileMatrixSetLink>
        <TileMatrixSet>WebMercatorQuad</TileMatrixSet>
      </TileMatrixSetLink>
      <Dimension>
        <ows:Identifier>RUN</ows:Identifier>
        <Default>{}</Default>
{}
      </Dimension>
      <Dimension>
        <ows:Identifier>FORECAST</ows:Identifier>
        <Default>{}</Default>
{}
      </Dimension>
      <ResourceURL format="image/png" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png"/>
    </Layer>"#,
                    layer_title, layer_id, styles, 
                    run_default, run_values_xml,
                    forecast_default, forecast_values_xml,
                    layer_id
                )
            })
        })
        .collect::<Vec<_>>();
    
    // Add composite WIND_BARBS layers for each model that has both UGRD and VGRD
    for model in models {
        let params = model_params.get(model).unwrap_or(&empty_params);
        let (runs, forecasts) = model_dimensions.get(model).unwrap_or(&empty_dims);
        let has_ugrd = params.iter().any(|p| p == "UGRD");
        let has_vgrd = params.iter().any(|p| p == "VGRD");
        
        if has_ugrd && has_vgrd {
            let layer_id = format!("{}_WIND_BARBS", model);
            let layer_title = format!("{} - Wind Barbs", model.to_uppercase());
            
            let run_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
            let run_values_xml: String = if runs.is_empty() {
                "        <Value>latest</Value>".to_string()
            } else {
                runs.iter()
                    .map(|v| format!("        <Value>{}</Value>", v))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let forecast_default = forecasts.first().unwrap_or(&0);
            let forecast_values_xml: String = if forecasts.is_empty() {
                "        <Value>0</Value>".to_string()
            } else {
                forecasts.iter()
                    .map(|v| format!("        <Value>{}</Value>", v))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            
            let wind_barbs_layer = format!(
                r#"    <Layer>
      <ows:Title>{}</ows:Title>
      <ows:Identifier>{}</ows:Identifier>
      <ows:WGS84BoundingBox>
        <ows:LowerCorner>-180.0 -90.0</ows:LowerCorner>
        <ows:UpperCorner>180.0 90.0</ows:UpperCorner>
      </ows:WGS84BoundingBox>
      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>
      <Format>image/png</Format>
      <TileMatrixSetLink>
        <TileMatrixSet>WebMercatorQuad</TileMatrixSet>
      </TileMatrixSetLink>
      <Dimension>
        <ows:Identifier>RUN</ows:Identifier>
        <Default>{}</Default>
{}
      </Dimension>
      <Dimension>
        <ows:Identifier>FORECAST</ows:Identifier>
        <Default>{}</Default>
{}
      </Dimension>
      <ResourceURL format="image/png" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png"/>
    </Layer>"#,
                layer_title, layer_id, 
                run_default, run_values_xml,
                forecast_default, forecast_values_xml,
                layer_id
            );
            
            all_layers.push(wind_barbs_layer);
        }
    }
    
    let layers = all_layers.join("\n");
    
    // Build TileMatrixSet for WebMercatorQuad (zoom levels 0-18)
    let tile_matrices: String = (0..=18)
        .map(|z| {
            let n = 2u32.pow(z);
            let scale = 559082264.0287178 / (n as f64);
            let max_extent = 20037508.342789244;
            
            format!(
                r#"      <TileMatrix>
        <ows:Identifier>{}</ows:Identifier>
        <ScaleDenominator>{}</ScaleDenominator>
        <TopLeftCorner>{} {}</TopLeftCorner>
        <TileWidth>256</TileWidth>
        <TileHeight>256</TileHeight>
        <MatrixWidth>{}</MatrixWidth>
        <MatrixHeight>{}</MatrixHeight>
      </TileMatrix>"#,
                z, scale, -max_extent, max_extent, n, n
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Capabilities xmlns="http://www.opengis.net/wmts/1.0"
    xmlns:ows="http://www.opengis.net/ows/1.1"
    xmlns:xlink="http://www.w3.org/1999/xlink"
    version="1.0.0">
  <ows:ServiceIdentification>
    <ows:Title>Weather WMTS Service</ows:Title>
    <ows:Abstract>WMTS service for weather model data</ows:Abstract>
    <ows:ServiceType>OGC WMTS</ows:ServiceType>
    <ows:ServiceTypeVersion>1.0.0</ows:ServiceTypeVersion>
  </ows:ServiceIdentification>
  <ows:ServiceProvider>
    <ows:ProviderName>Weather WMS</ows:ProviderName>
  </ows:ServiceProvider>
  <ows:OperationsMetadata>
    <ows:Operation name="GetCapabilities">
      <ows:DCP>
        <ows:HTTP>
          <ows:Get xlink:href="http://localhost:8080/wmts?">
            <ows:Constraint name="GetEncoding">
              <ows:AllowedValues><ows:Value>KVP</ows:Value></ows:AllowedValues>
            </ows:Constraint>
          </ows:Get>
        </ows:HTTP>
      </ows:DCP>
    </ows:Operation>
    <ows:Operation name="GetTile">
      <ows:DCP>
        <ows:HTTP>
          <ows:Get xlink:href="http://localhost:8080/wmts?">
            <ows:Constraint name="GetEncoding">
              <ows:AllowedValues><ows:Value>KVP</ows:Value></ows:AllowedValues>
            </ows:Constraint>
          </ows:Get>
          <ows:Get xlink:href="http://localhost:8080/wmts/rest/">
            <ows:Constraint name="GetEncoding">
              <ows:AllowedValues><ows:Value>RESTful</ows:Value></ows:AllowedValues>
            </ows:Constraint>
          </ows:Get>
        </ows:HTTP>
      </ows:DCP>
    </ows:Operation>
  </ows:OperationsMetadata>
  <Contents>
{}
    <TileMatrixSet>
      <ows:Identifier>WebMercatorQuad</ows:Identifier>
      <ows:SupportedCRS>urn:ogc:def:crs:EPSG::3857</ows:SupportedCRS>
      <WellKnownScaleSet>http://www.opengis.net/def/wkss/OGC/1.0/GoogleMapsCompatible</WellKnownScaleSet>
{}
    </TileMatrixSet>
  </Contents>
</Capabilities>"#,
        layers, tile_matrices
    )
}

fn generate_placeholder_image(width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut pixels = vec![200u8; w * h * 4];
    for i in 0..pixels.len() / 4 {
        pixels[i * 4 + 3] = 255;
    }
    let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    write_chunk(&mut png, b"IHDR", &ihdr);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let idx = (y * w + x) * 4;
            raw.extend_from_slice(&pixels[idx..idx + 4]);
        }
    }
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&raw).unwrap();
    write_chunk(&mut png, b"IDAT", &enc.finish().unwrap());
    write_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_chunk(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32fast::hash(&[name.as_slice(), data].concat()).to_be_bytes());
}

// ============================================================================
// Validation API Handlers
// ============================================================================

/// GET /api/validation/status - Get current validation status
#[instrument(skip(state))]
pub async fn validation_status_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<crate::validation::ValidationStatus>, StatusCode> {
    info!("Validation status requested");
    
    // Get base URL from environment or use localhost
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    
    let status = crate::validation::run_validation(&base_url).await;
    
    Ok(Json(status))
}

/// GET /api/validation/run - Run validation and return results
#[instrument(skip(state))]
pub async fn validation_run_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<crate::validation::ValidationStatus>, StatusCode> {
    info!("Manual validation run requested");
    
    // Get base URL from environment or use localhost
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    
    let status = crate::validation::run_validation(&base_url).await;
    
    info!(
        wms_status = status.wms.status,
        wmts_status = status.wmts.status,
        overall_status = status.overall_status,
        "Validation completed"
    );
    
    Ok(Json(status))
}
