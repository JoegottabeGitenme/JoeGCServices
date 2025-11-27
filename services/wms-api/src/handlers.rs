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
use tracing::{info, instrument, error, debug};

use storage::CacheKey;
use wms_common::{tile::web_mercator_tile_matrix_set, BoundingBox, CrsCode, TileCoord};

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
    #[serde(rename = "ELEVATION", alias = "elevation")]
    elevation: Option<String>,
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
    let mut param_levels: HashMap<String, Vec<String>> = HashMap::new();  // model_param -> levels
    
    for model in &models {
        let params_list = state.catalog.list_parameters(model).await.unwrap_or_default();
        
        // Get levels for each parameter
        for param in &params_list {
            let levels = state.catalog.get_available_levels(model, param).await.unwrap_or_default();
            let key = format!("{}_{}", model, param);
            param_levels.insert(key, levels);
        }
        
        model_params.insert(model.clone(), params_list);
        
        // Get RUN and FORECAST dimensions
        let dimensions = state.catalog.get_model_dimensions(model).await.unwrap_or_default();
        model_dimensions.insert(model.clone(), dimensions);
    }
    
    let xml = build_wms_capabilities_xml(version, &models, &model_params, &model_dimensions, &param_levels);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(xml.into())
        .unwrap()
}

async fn wms_get_map(state: Arc<AppState>, params: WmsParams) -> Response {
    use crate::metrics::Timer;
    
    // Record WMS request
    state.metrics.record_wms_request();
    
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
    let elevation = params.elevation.clone();

    info!(layer = %layers, style = %style, width = width, height = height, bbox = ?bbox, crs = ?crs, time = ?time, elevation = ?elevation, "GetMap request");
    
    // Time the rendering
    let timer = Timer::start();
    
    // Try to render actual data, return error on failure
    match render_weather_data(&state, layers, style, width, height, bbox, crs, time.as_deref(), elevation.as_deref()).await {
        Ok(png_data) => {
            state.metrics.record_render(timer.elapsed_us(), true).await;
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(png_data.into())
                .unwrap()
        }
        Err(e) => {
            state.metrics.record_render(timer.elapsed_us(), false).await;
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
    
    // Parse TIME parameter (forecast hour)
    let forecast_hour: Option<u32> = params.time.as_ref().and_then(|t| t.parse().ok());
    
    // Parse ELEVATION parameter (level string, e.g., "500 mb", "2 m above ground")
    let elevation = params.elevation.clone();
    
    info!(
        query_layers = %query_layers,
        bbox = ?bbox_array,
        width = width,
        height = height,
        i = i,
        j = j,
        crs = crs,
        info_format = ?info_format,
        forecast_hour = ?forecast_hour,
        elevation = ?elevation,
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
            forecast_hour,
            elevation.as_deref(),
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
            wmts_get_tile(state, &layer, &style, z, tile_col, tile_row, None, None).await
        }
        _ => wmts_exception(
            "MissingParameterValue",
            "REQUEST is required",
            StatusCode::BAD_REQUEST,
        ),
    }
}

/// Query parameters for WMTS RESTful tile requests (dimensions)
#[derive(Debug, Deserialize, Default)]
pub struct WmtsDimensionParams {
    /// TIME dimension - forecast hour (e.g., "3", "6", "12") or ISO8601 datetime
    #[serde(rename = "time", alias = "TIME")]
    pub time: Option<String>,
    /// ELEVATION dimension - vertical level (e.g., "500 mb", "2 m above ground")
    #[serde(rename = "elevation", alias = "ELEVATION")]
    pub elevation: Option<String>,
}

#[instrument(skip(state))]
pub async fn wmts_rest_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(path): Path<String>,
    Query(params): Query<WmtsDimensionParams>,
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
    
    // Parse TIME parameter as forecast hour
    let forecast_hour: Option<u32> = params.time.as_ref().and_then(|t| t.parse().ok());
    let elevation = params.elevation.clone();
    
    wmts_get_tile(state, layer, style, z, x, y, forecast_hour, elevation.as_deref()).await
}

#[instrument(skip(state))]
pub async fn xyz_tile_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((layer, style, z, x, y)): Path<(String, String, u32, u32, String)>,
    Query(params): Query<WmtsDimensionParams>,
) -> Response {
    let (y_str, _) = y.rsplit_once('.').unwrap_or((&y, "png"));
    let y_val: u32 = y_str.parse().unwrap_or(0);
    
    // Parse TIME parameter as forecast hour
    let forecast_hour: Option<u32> = params.time.as_ref().and_then(|t| t.parse().ok());
    let elevation = params.elevation.clone();
    
    wmts_get_tile(state, &layer, &style, z, x, y_val, forecast_hour, elevation.as_deref()).await
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
    forecast_hour: Option<u32>,
    elevation: Option<&str>,
) -> Response {
    use crate::metrics::Timer;
    
    // Record WMTS request
    state.metrics.record_wmts_request();
    let timer = Timer::start();
    
    info!(layer = %layer, style = %style, z = z, x = x, y = y, forecast_hour = ?forecast_hour, elevation = ?elevation, "GetTile request");
    
    // Build cache key for this tile, including time and elevation for uniqueness
    let time_key = forecast_hour.map(|h| format!("t{}", h));
    let elevation_key = elevation.map(|e| e.replace(' ', "_"));
    let dimension_suffix = match (&time_key, &elevation_key) {
        (Some(t), Some(e)) => Some(format!("{}_{}", t, e)),
        (Some(t), None) => Some(t.clone()),
        (None, Some(e)) => Some(e.clone()),
        (None, None) => None,
    };
    
    let cache_key = CacheKey::new(
        layer,
        style,
        CrsCode::Epsg3857,  // WMTS always uses Web Mercator
        BoundingBox::new(x as f64, y as f64, z as f64, 0.0),  // Use tile coords for key
        256,
        256,
        dimension_suffix,
        "png",
    );
    
    // Check Redis cache first
    {
        let mut cache = state.cache.lock().await;
        if let Ok(Some(cached_data)) = cache.get(&cache_key).await {
            state.metrics.record_cache_hit().await;
            info!(layer = %layer, z = z, x = x, y = y, "Cache hit");
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::CACHE_CONTROL, "max-age=3600")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header("X-Cache", "HIT")
                .body(cached_data.to_vec().into())
                .unwrap();
        }
        state.metrics.record_cache_miss().await;
    }
    
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
        crate::rendering::render_wind_barbs_tile_with_level(
            &state.storage,
            &state.catalog,
            model,
            Some(coord),  // Pass tile coordinate for expanded rendering
            256,  // tile width
            256,  // tile height
            bbox_array,
            forecast_hour,
            elevation,
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
        
        crate::rendering::render_isolines_tile_with_level(
            &state.storage,
            &state.catalog,
            model,
            &parameter,
            Some(coord),  // Pass tile coordinate for expanded rendering
            256,  // tile width
            256,  // tile height
            bbox_array,
            &style_file,
            forecast_hour,
            elevation,
            true,  // WMTS tiles are always in Web Mercator
        )
        .await
    } else {
        // Render the tile with spatial subsetting and optional time/level
        crate::rendering::render_weather_data_with_level(
            &state.storage,
            &state.catalog,
            model,
            &parameter,
            forecast_hour,
            elevation,
            256,  // tile width
            256,  // tile height
            Some(bbox_array),
            None,  // style_name
            true,  // use_mercator for WMTS
        )
        .await
    };
    
    match result
    {
        Ok(png_data) => {
            state.metrics.record_render(timer.elapsed_us(), true).await;
            
            // Store in Redis cache (async, don't wait)
            let cache_data = png_data.clone();
            let state_clone = state.clone();
            let cache_key_clone = cache_key.clone();
            tokio::spawn(async move {
                let mut cache = state_clone.cache.lock().await;
                if let Err(e) = cache.set(&cache_key_clone, &cache_data, None).await {
                    error!(error = %e, "Failed to cache tile");
                }
            });
            
            // Prefetch neighboring tiles in background (Google tile strategy)
            // Only prefetch at zoom levels 3-12 to avoid excessive requests
            if z >= 3 && z <= 12 {
                spawn_tile_prefetch(state.clone(), layer.to_string(), style.to_string(), coord);
            }
            
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::CACHE_CONTROL, "max-age=3600")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header("X-Cache", "MISS")
                .body(png_data.into())
                .unwrap()
        }
        Err(e) => {
            state.metrics.record_render(timer.elapsed_us(), false).await;
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

/// JSON metrics endpoint for the web UI
pub async fn api_metrics_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let snapshot = state.metrics.snapshot().await;
    let system = crate::metrics::SystemStats::read();
    
    // Get Redis cache stats
    let cache_stats = {
        let mut cache = state.cache.lock().await;
        match cache.stats().await {
            Ok(stats) => serde_json::json!({
                "connected": true,
                "key_count": stats.key_count,
                "memory_used": stats.memory_used
            }),
            Err(_) => serde_json::json!({
                "connected": false,
                "key_count": 0,
                "memory_used": 0
            })
        }
    };
    
    // Combine metrics, system, and cache stats
    let combined = serde_json::json!({
        "metrics": snapshot,
        "system": system,
        "cache": cache_stats
    });
    
    Json(combined)
}

/// Storage stats endpoint - returns MinIO bucket statistics
pub async fn storage_stats_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    match state.storage.stats().await {
        Ok(stats) => {
            let json = serde_json::json!({
                "total_size": stats.total_size,
                "object_count": stats.object_count,
                "bucket": stats.bucket
            });
            (StatusCode::OK, Json(json))
        }
        Err(e) => {
            let json = serde_json::json!({
                "error": e.to_string()
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json))
        }
    }
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
    elevation: Option<&str>,
) -> Result<Vec<u8>, String> {
    // Parse layer name (format: "model_parameter" or "model_WIND_BARBS")
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return Err("Invalid layer format".to_string());
    }

    let model = parts[0];
    let parameter = parts[1..].join("_");
    
    // Parse TIME parameter early (supports both forecast hour like "3" and ISO8601 datetime)
    let forecast_hour: Option<u32> = time.and_then(|t| {
        // First try to parse as integer (forecast hour)
        if let Ok(hour) = t.parse::<u32>() {
            Some(hour)
        } else {
            // Try to parse as ISO8601 datetime and find matching dataset
            // For now, we'll just support forecast hour format
            // ISO8601 support would require querying catalog for nearest time
            None
        }
    });
    
    // ELEVATION parameter is the level string (e.g., "500 mb", "2 m above ground")
    let level = elevation.map(|s| s.to_string());
    
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
            forecast_hour,
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
    
    info!(forecast_hour = ?forecast_hour, level = ?level, bbox = ?parsed_bbox, style = style, "Parsed WMS parameters");

    // Check if isolines style is requested
    // Check if CRS is Web Mercator - use Mercator projection for resampling
    let crs_str = crs.unwrap_or("EPSG:4326");
    let use_mercator = crs_str.contains("3857");
    
    if style == "isolines" {
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            format!("{}/temperature_isolines.json", style_config_dir)
        } else {
            // Default to temperature isolines for now
            format!("{}/temperature_isolines.json", style_config_dir)
        };
        
        // For WMS, we don't have tile coordinates, so pass None
        return crate::rendering::render_isolines_tile_with_level(
            &state.storage,
            &state.catalog,
            model,
            &parameter,
            None,  // No tile coordinate - render full bbox
            width,
            height,
            parsed_bbox.unwrap_or([-180.0, -90.0, 180.0, 90.0]),
            &style_file,
            forecast_hour,
            level.as_deref(),
            use_mercator,
        )
        .await;
    }

    // Use shared rendering logic with Mercator projection support and level
    crate::rendering::render_weather_data_with_level(
        &state.storage,
        &state.catalog,
        model,
        &parameter,
        forecast_hour,
        level.as_deref(),
        width,
        height,
        parsed_bbox,
        Some(style),
        use_mercator,
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

/// Get human-readable name for a GRIB parameter code
fn get_parameter_name(param: &str) -> String {
    match param {
        "PRMSL" => "Mean Sea Level Pressure".to_string(),
        "TMP" => "Temperature".to_string(),
        "RH" => "Relative Humidity".to_string(),
        "UGRD" => "U-Component Wind".to_string(),
        "VGRD" => "V-Component Wind".to_string(),
        "WIND_BARBS" => "Wind Barbs".to_string(),
        "APCP" => "Accumulated Precipitation".to_string(),
        "TCDC" => "Total Cloud Cover".to_string(),
        "GUST" => "Wind Gust Speed".to_string(),
        // GRIB2 Product 1 (Meteorological) parameters
        "P1_22" => "Cloud Mixing Ratio".to_string(),
        "P1_23" => "Ice Mixing Ratio".to_string(),
        "P1_24" => "Rain Mixing Ratio".to_string(),
        // Default: return the code itself
        _ => param.to_string(),
    }
}

fn build_wms_capabilities_xml(
    version: &str,
    models: &[String],
    model_params: &HashMap<String, Vec<String>>,
    model_dimensions: &HashMap<String, (Vec<String>, Vec<i32>)>,
    param_levels: &HashMap<String, Vec<String>>,
) -> String {
    let empty_params = Vec::new();
    let empty_dims = (Vec::new(), Vec::new());
    let empty_levels = Vec::new();
    
    let layers: String = models
        .iter()
        .map(|model| {
            let params = model_params.get(model).unwrap_or(&empty_params);
            let (runs, forecasts) = model_dimensions.get(model).unwrap_or(&empty_dims);
            
            // Build base dimension elements (RUN and FORECAST)
            let run_values = if runs.is_empty() { "latest".to_string() } else { runs.join(",") };
            let run_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
            let forecast_values = if forecasts.is_empty() { "0".to_string() } else { forecasts.iter().map(|h| h.to_string()).collect::<Vec<_>>().join(",") };
            // Default to latest (highest) forecast hour to match get_latest() behavior
            let forecast_default = forecasts.last().unwrap_or(&0);
            
            let base_dimensions = format!(
                r#"<Dimension name="RUN" units="ISO8601" default="{}">{}</Dimension><Dimension name="FORECAST" units="hours" default="{}">{}</Dimension>"#,
                run_default, run_values, forecast_default, forecast_values
            );
            
            let param_layers = params
                .iter()
                .map(|p| {
                    // Get levels for this parameter
                    let key = format!("{}_{}", model, p);
                    let levels = param_levels.get(&key).unwrap_or(&empty_levels);
                    
                    // Build ELEVATION dimension if there are multiple levels
                    let elevation_dim = if levels.len() > 1 {
                        // Sort levels for display (pressure levels should be sorted numerically)
                        let mut sorted_levels = levels.clone();
                        sorted_levels.sort_by(|a, b| {
                            // Try to parse as pressure level for proper sorting
                            let a_val = a.replace(" mb", "").parse::<i32>().unwrap_or(9999);
                            let b_val = b.replace(" mb", "").parse::<i32>().unwrap_or(9999);
                            b_val.cmp(&a_val)  // Descending (1000 mb first)
                        });
                        let level_values = sorted_levels.join(",");
                        let default_level = sorted_levels.first().map(|s| s.as_str()).unwrap_or("");
                        format!(
                            r#"<Dimension name="ELEVATION" units="" default="{}">{}</Dimension>"#,
                            default_level, level_values
                        )
                    } else {
                        String::new()
                    };
                    
                    let all_dimensions = format!("{}{}", base_dimensions, elevation_dim);
                    
                    // Add styles to each layer
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
                        model, p, model.to_uppercase(), get_parameter_name(p), styles, all_dimensions
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            
             // Add composite wind barbs layer if we have both UGRD and VGRD
             // Get UGRD levels for wind barbs (same as VGRD)
             let ugrd_key = format!("{}_UGRD", model);
             let wind_levels = param_levels.get(&ugrd_key).unwrap_or(&empty_levels);
             let wind_elevation_dim = if wind_levels.len() > 1 {
                 let mut sorted_levels = wind_levels.clone();
                 sorted_levels.sort_by(|a, b| {
                     let a_val = a.replace(" mb", "").parse::<i32>().unwrap_or(9999);
                     let b_val = b.replace(" mb", "").parse::<i32>().unwrap_or(9999);
                     b_val.cmp(&a_val)
                 });
                 let level_values = sorted_levels.join(",");
                 let default_level = sorted_levels.first().map(|s| s.as_str()).unwrap_or("");
                 format!(
                     r#"<Dimension name="ELEVATION" units="" default="{}">{}</Dimension>"#,
                     default_level, level_values
                 )
             } else {
                 String::new()
             };
             
             let wind_barbs_layer = if params.contains(&"UGRD".to_string()) && params.contains(&"VGRD".to_string()) {
                 format!(
                     r#"<Layer queryable="1"><Name>{}_WIND_BARBS</Name><Title>{} - Wind Barbs</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>-180</westBoundLongitude><eastBoundLongitude>180</eastBoundLongitude><southBoundLatitude>-90</southBoundLatitude><northBoundLatitude>90</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="-180" miny="-90" maxx="180" maxy="90"/><Style><Name>default</Name><Title>Default Barbs</Title></Style>{}{}</Layer>"#,
                     model, model.to_uppercase(), base_dimensions, wind_elevation_dim
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
            let (_runs, forecasts) = model_dimensions.get(model).unwrap_or(&empty_dims);
            
            // Build dimension values for time (forecast hour)
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
                
                // Build dimension elements for time (forecast hour)
                // Using "time" as dimension identifier to match query parameter
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
        <ows:Identifier>time</ows:Identifier>
        <ows:UOM>hours</ows:UOM>
        <Default>{}</Default>
{}
      </Dimension>
      <ResourceURL format="image/png" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png"/>
    </Layer>"#,
                    layer_title, layer_id, styles, 
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
        <ows:Identifier>time</ows:Identifier>
        <ows:UOM>hours</ows:UOM>
        <Default>{}</Default>
{}
      </Dimension>
      <ResourceURL format="image/png" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png"/>
    </Layer>"#,
                layer_title, layer_id, 
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

// ============================================================================
// Tile Prefetching
// ============================================================================

/// Get the 8 neighboring tiles around a center tile (Google tile strategy).
/// Returns tiles at the same zoom level that surround the requested tile.
fn get_neighboring_tiles(center: &TileCoord) -> Vec<TileCoord> {
    let z = center.z;
    let max_tile = 2u32.pow(z) - 1;
    
    let mut neighbors = Vec::with_capacity(8);
    
    // All 8 directions: N, NE, E, SE, S, SW, W, NW
    let offsets: [(i32, i32); 8] = [
        (0, -1),   // N
        (1, -1),   // NE
        (1, 0),    // E
        (1, 1),    // SE
        (0, 1),    // S
        (-1, 1),   // SW
        (-1, 0),   // W
        (-1, -1),  // NW
    ];
    
    for (dx, dy) in offsets {
        let new_x = center.x as i32 + dx;
        let new_y = center.y as i32 + dy;
        
        // Only include valid tile coordinates
        if new_x >= 0 && new_x <= max_tile as i32 && new_y >= 0 && new_y <= max_tile as i32 {
            neighbors.push(TileCoord::new(z, new_x as u32, new_y as u32));
        }
    }
    
    neighbors
}

/// Spawn background tasks to prefetch neighboring tiles.
/// This improves perceived performance when panning the map.
fn spawn_tile_prefetch(
    state: Arc<AppState>,
    layer: String,
    style: String,
    center: TileCoord,
) {
    let neighbors = get_neighboring_tiles(&center);
    
    for neighbor in neighbors {
        let state = state.clone();
        let layer = layer.clone();
        let style = style.clone();
        
        tokio::spawn(async move {
            prefetch_single_tile(state, &layer, &style, neighbor).await;
        });
    }
}

/// Prefetch a single tile if not already cached.
async fn prefetch_single_tile(
    state: Arc<AppState>,
    layer: &str,
    style: &str,
    coord: TileCoord,
) {
    // Build cache key
    let cache_key = CacheKey::new(
        layer,
        style,
        CrsCode::Epsg3857,
        BoundingBox::new(coord.x as f64, coord.y as f64, coord.z as f64, 0.0),
        256,
        256,
        None,
        "png",
    );
    
    // Check if already cached
    {
        let mut cache = state.cache.lock().await;
        if let Ok(Some(_)) = cache.get(&cache_key).await {
            debug!(z = coord.z, x = coord.x, y = coord.y, "Prefetch: already cached");
            return;
        }
    }
    
    debug!(layer = %layer, z = coord.z, x = coord.x, y = coord.y, "Prefetching tile");
    
    // Get tile bounding box
    let latlon_bbox = wms_common::tile::tile_to_latlon_bounds(&coord);
    let bbox_array = [
        latlon_bbox.min_x as f32,
        latlon_bbox.min_y as f32,
        latlon_bbox.max_x as f32,
        latlon_bbox.max_y as f32,
    ];
    
    // Parse layer name
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return;
    }
    
    let model = parts[0];
    let parameter = parts[1..].join("_");
    
    // Render the tile
    let result = if parameter == "WIND_BARBS" {
        crate::rendering::render_wind_barbs_tile(
            &state.storage,
            &state.catalog,
            model,
            Some(coord),
            256,
            256,
            bbox_array,
            None,
        )
        .await
    } else if style == "isolines" {
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            format!("{}/temperature_isolines.json", style_config_dir)
        } else {
            format!("{}/temperature_isolines.json", style_config_dir)
        };
        
        crate::rendering::render_isolines_tile(
            &state.storage,
            &state.catalog,
            model,
            &parameter,
            Some(coord),
            256,
            256,
            bbox_array,
            &style_file,
            None,
            true,
        )
        .await
    } else {
        crate::rendering::render_weather_data(
            &state.storage,
            &state.catalog,
            model,
            &parameter,
            None,
            256,
            256,
            Some(bbox_array),
        )
        .await
    };
    
    // Cache the result if successful
    if let Ok(png_data) = result {
        let mut cache = state.cache.lock().await;
        if let Err(e) = cache.set(&cache_key, &png_data, None).await {
            debug!(error = %e, "Failed to cache prefetched tile");
        } else {
            debug!(z = coord.z, x = coord.x, y = coord.y, "Prefetched and cached tile");
        }
    }
}

