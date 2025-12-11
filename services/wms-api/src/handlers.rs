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

use crate::model_config::ModelDimensionRegistry;
use crate::state::AppState;

// ============================================================================
// WMS Handlers
// ============================================================================

#[allow(dead_code)]
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
    // Dimension parameters:
    // - TIME: For observation layers (GOES, MRMS) - ISO8601 timestamp
    // - RUN: For forecast models (GFS, HRRR) - ISO8601 model run time
    // - FORECAST: For forecast models - forecast hour offset from RUN
    #[serde(rename = "TIME", alias = "time")]
    time: Option<String>,
    #[serde(rename = "RUN", alias = "run")]
    run: Option<String>,
    #[serde(rename = "FORECAST", alias = "forecast")]
    forecast: Option<String>,
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
    let mut model_bboxes = HashMap::new();
    
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
        
        // Get bounding box for the model
        if let Ok(bbox) = state.catalog.get_model_bbox(model).await {
            model_bboxes.insert(model.clone(), bbox);
        }
    }
    
    let xml = build_wms_capabilities_xml(version, &models, &model_params, &model_dimensions, &param_levels, &model_bboxes, &state.model_dimensions);
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
    
    // Build dimension parameters from request
    let dimensions = DimensionParams {
        time: params.time.clone(),
        run: params.run.clone(),
        forecast: params.forecast.clone(),
        elevation: params.elevation.clone(),
    };

    info!(layer = %layers, style = %style, width = width, height = height, bbox = ?bbox, crs = ?crs, 
          time = ?dimensions.time, run = ?dimensions.run, forecast = ?dimensions.forecast, 
          elevation = ?dimensions.elevation, "GetMap request");
    
    // Record bbox for heatmap visualization (parse and convert to WGS84 if needed)
    // WMS requests don't use tile caching, so always record as miss
    if let Some(bbox_str) = bbox {
        let coords: Vec<f64> = bbox_str.split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if coords.len() == 4 {
            let crs_str = crs.unwrap_or("EPSG:4326");
            let bbox_array = if crs_str.contains("3857") {
                let (min_lon, min_lat) = mercator_to_wgs84(coords[0], coords[1]);
                let (max_lon, max_lat) = mercator_to_wgs84(coords[2], coords[3]);
                [min_lon as f32, min_lat as f32, max_lon as f32, max_lat as f32]
            } else {
                [coords[0] as f32, coords[1] as f32, coords[2] as f32, coords[3] as f32]
            };
            state.metrics.record_tile_request_location(&bbox_array, crate::metrics::TileCacheStatus::Miss);
        }
    }
    
    // Time the rendering
    let timer = Timer::start();
    
    // Try to render actual data, return error on failure
    match render_weather_data(&state, layers, style, width, height, bbox, crs, &dimensions).await {
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
        .and_then(InfoFormat::from_mime)
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
            &state.grib_cache,
            &state.catalog,
            &state.metrics,
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
#[allow(dead_code)]
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
    // Dimension parameters:
    // - TIME: For observation layers (GOES, MRMS) - ISO8601 timestamp
    // - RUN: For forecast models (GFS, HRRR) - ISO8601 model run time
    // - FORECAST: For forecast models - forecast hour offset from RUN
    #[serde(rename = "TIME")]
    time: Option<String>,
    #[serde(rename = "RUN")]
    run: Option<String>,
    #[serde(rename = "FORECAST")]
    forecast: Option<String>,
    #[serde(rename = "ELEVATION")]
    elevation: Option<String>,
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
            
            // Build dimension parameters
            let dimensions = DimensionParams {
                time: params.time.clone(),
                run: params.run.clone(),
                forecast: params.forecast.clone(),
                elevation: params.elevation.clone(),
            };
            
            // Parse model from layer name to determine dimension interpretation
            let model = layer.split('_').next().unwrap_or("");
            let (forecast_hour, observation_time, _reference_time) = dimensions.parse_for_layer(model, &state.model_dimensions);
            
            wmts_get_tile(state, &layer, &style, z, tile_col, tile_row, forecast_hour, observation_time, dimensions.elevation.as_deref()).await
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
    /// TIME dimension - for observation layers (GOES, MRMS) - ISO8601 datetime
    #[serde(rename = "time", alias = "TIME")]
    pub time: Option<String>,
    /// RUN dimension - for forecast models (GFS, HRRR) - ISO8601 model run time
    #[serde(rename = "run", alias = "RUN")]
    pub run: Option<String>,
    /// FORECAST dimension - for forecast models - forecast hour offset from RUN
    #[serde(rename = "forecast", alias = "FORECAST")]
    pub forecast: Option<String>,
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
    
    // Build dimension parameters
    let dimensions = DimensionParams {
        time: params.time.clone(),
        run: params.run.clone(),
        forecast: params.forecast.clone(),
        elevation: params.elevation.clone(),
    };
    
    // Parse model from layer name to determine dimension interpretation
    let model = layer.split('_').next().unwrap_or("");
    let (forecast_hour, observation_time, _reference_time) = dimensions.parse_for_layer(model, &state.model_dimensions);
    
    wmts_get_tile(state, layer, style, z, x, y, forecast_hour, observation_time, dimensions.elevation.as_deref()).await
}

#[instrument(skip(state))]
pub async fn xyz_tile_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((layer, style, z, x, y)): Path<(String, String, u32, u32, String)>,
    Query(params): Query<WmtsDimensionParams>,
) -> Response {
    let (y_str, _) = y.rsplit_once('.').unwrap_or((&y, "png"));
    let y_val: u32 = y_str.parse().unwrap_or(0);
    
    // Build dimension parameters
    let dimensions = DimensionParams {
        time: params.time.clone(),
        run: params.run.clone(),
        forecast: params.forecast.clone(),
        elevation: params.elevation.clone(),
    };
    
    // Parse model from layer name to determine dimension interpretation
    let model = layer.split('_').next().unwrap_or("");
    let (forecast_hour, observation_time, _reference_time) = dimensions.parse_for_layer(model, &state.model_dimensions);
    
    wmts_get_tile(state, &layer, &style, z, x, y_val, forecast_hour, observation_time, dimensions.elevation.as_deref()).await
}

async fn wmts_get_capabilities(state: Arc<AppState>) -> Response {
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
    
    let xml = build_wmts_capabilities_xml(&models, &model_params, &model_dimensions, &param_levels, &state.model_dimensions);
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
    observation_time: Option<chrono::DateTime<chrono::Utc>>,
    elevation: Option<&str>,
) -> Response {
    use crate::metrics::Timer;
    
    // Record WMTS request
    state.metrics.record_wmts_request();
    let timer = Timer::start();
    
    info!(layer = %layer, style = %style, z = z, x = x, y = y, forecast_hour = ?forecast_hour, observation_time = ?observation_time, elevation = ?elevation, "GetTile request");
    
    // Build cache key for this tile, including time and elevation for uniqueness
    let time_key = forecast_hour.map(|h| format!("t{}", h))
        .or_else(|| observation_time.map(|t| format!("obs{}", t.timestamp())));
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
        dimension_suffix.clone(),
        "png",
    );
    
    // Build string key for L1 cache
    let cache_key_str = format!(
        "{}:{}:EPSG:3857:{}_{}_{}:{}",
        layer,
        style,
        z,
        x,
        y,
        dimension_suffix.as_deref().unwrap_or("current")
    );
    
    // Get tile bounding box early so we can record heatmap for all paths
    let tms = web_mercator_tile_matrix_set();
    let coord = TileCoord::new(z, x, y);
    
    let _bbox = match tms.tile_bbox(&coord) {
        Some(bbox) => bbox,
        None => return wmts_exception("TileOutOfRange", "Invalid tile", StatusCode::BAD_REQUEST),
    };
    
    // Convert Web Mercator bbox to WGS84 (lat/lon) for GRIB data
    let latlon_bbox = wms_common::tile::tile_to_latlon_bounds(&coord);
    
    // Format bbox as [min_lon, min_lat, max_lon, max_lat]
    let bbox_array = [
        latlon_bbox.min_x as f32,
        latlon_bbox.min_y as f32,
        latlon_bbox.max_x as f32,
        latlon_bbox.max_y as f32,
    ];
    
    // Check L1 in-memory cache first (fastest) - if enabled
    if state.optimization_config.l1_cache_enabled {
        if let Some(tile_data) = state.tile_memory_cache.get(&cache_key_str).await {
            state.metrics.record_l1_cache_hit();
            state.metrics.record_tile_request_location(&bbox_array, crate::metrics::TileCacheStatus::L1Hit);
            debug!(layer = %layer, z = z, x = x, y = y, "L1 cache hit");
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::CACHE_CONTROL, "max-age=300")  // 5 min for L1 cached tiles
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header("X-Cache", "L1-HIT")
                .body(tile_data.to_vec().into())
                .unwrap();
        }
        
        // L1 cache miss - record it
        state.metrics.record_l1_cache_miss();
    }
    
    // Check L2 Redis cache
    {
        let mut cache = state.cache.lock().await;
        if let Ok(Some(cached_data)) = cache.get(&cache_key).await {
            state.metrics.record_cache_hit().await;
            state.metrics.record_tile_request_location(&bbox_array, crate::metrics::TileCacheStatus::L2Hit);
            info!(layer = %layer, z = z, x = x, y = y, "L2 cache hit");
            
            // Promote to L1 cache for future requests (if enabled)
            if state.optimization_config.l1_cache_enabled {
                let data_bytes = bytes::Bytes::from(cached_data.to_vec());
                state.tile_memory_cache.set(&cache_key_str, data_bytes.clone(), None).await;
            }
            
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::CACHE_CONTROL, "max-age=3600")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header("X-Cache", "L2-HIT")
                .body(cached_data.to_vec().into())
                .unwrap();
        }
        state.metrics.record_cache_miss().await;
    }
    
    // Cache miss - record for heatmap (will render below)
    state.metrics.record_tile_request_location(&bbox_array, crate::metrics::TileCacheStatus::Miss);
    
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
            &state.grib_cache,
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
        // Isolines are only supported for GRIB2 data (GFS, HRRR temperature)
        // Not supported for observation data (GOES, MRMS) which are satellite/radar imagery
        if state.model_dimensions.is_observation(model) {
            return wmts_exception(
                "StyleNotDefined",
                &format!("Isolines style is not supported for {} layers. Isolines are only available for temperature parameters (GFS, HRRR).", model.to_uppercase()),
                StatusCode::BAD_REQUEST,
            );
        }
        
        // Render isolines (contours) for this parameter
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            format!("{}/temperature_isolines.json", style_config_dir)
        } else {
            // Default to temperature isolines for now
            format!("{}/temperature_isolines.json", style_config_dir)
        };
        
        crate::rendering::render_isolines_tile_with_level(
            &state.grib_cache,
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
    } else if style == "numbers" {
        // Get appropriate style file for color mapping
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = if parameter.contains("CMI") {
            // GOES satellite data
            if parameter.contains("C01") || parameter.contains("C02") || parameter.contains("C03") {
                // Visible/near-IR bands
                format!("{}/goes_visible.json", style_config_dir)
            } else {
                // IR bands (C08-C16)
                format!("{}/goes_ir.json", style_config_dir)
            }
        } else if parameter.contains("TMP") || parameter.contains("TEMP") {
            format!("{}/temperature.json", style_config_dir)
        } else if parameter.contains("WIND") || parameter.contains("GUST") {
            format!("{}/wind.json", style_config_dir)
        } else if parameter.contains("PRES") || parameter.contains("PRMSL") {
            format!("{}/atmospheric.json", style_config_dir)
        } else if parameter.contains("PRECIP_RATE") {
            format!("{}/precip_rate.json", style_config_dir)
        } else if parameter.contains("QPE") || parameter.contains("PRECIP") {
            format!("{}/precipitation.json", style_config_dir)
        } else if parameter.contains("REFL") {
            format!("{}/reflectivity.json", style_config_dir)
        } else {
            // Default to temperature for generic parameters
            format!("{}/temperature.json", style_config_dir)
        };
        
        crate::rendering::render_numbers_tile(
            &state.grib_cache,
            state.grid_cache_if_enabled(),
            &state.catalog,
            &state.metrics,
            model,
            &parameter,
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
        // Supports both forecast hour (for GFS, HRRR) and observation time (for MRMS, GOES)
        // Use LUT for fast GOES rendering when available
        crate::rendering::render_weather_data_with_lut(
            &state.grib_cache,
            state.grid_cache_if_enabled(),
            &state.catalog,
            &state.metrics,
            model,
            &parameter,
            forecast_hour,
            observation_time,
            elevation,
            256,  // tile width
            256,  // tile height
            Some(bbox_array),
            None,  // style_name
            true,  // use_mercator for WMTS
            Some((z, x, y)),  // tile coords for LUT lookup
            Some(&state.projection_luts),  // projection LUT cache
        )
        .await
    };
    
    match result
    {
        Ok(png_data) => {
            // Classify layer type and record metrics
            let layer_type = crate::metrics::LayerType::from_layer_and_style(layer, style);
            state.metrics.record_render_with_type(timer.elapsed_us(), true, layer_type).await;
            
            // Store in L1 cache immediately (synchronous - it's in-memory) - if enabled
            if state.optimization_config.l1_cache_enabled {
                let png_bytes = bytes::Bytes::from(png_data.clone());
                state.tile_memory_cache.set(&cache_key_str, png_bytes, None).await;
            }
            
            // Store in L2 Redis cache (async, don't wait)
            let cache_data = png_data.clone();
            let state_clone = state.clone();
            let cache_key_clone = cache_key.clone();
            tokio::spawn(async move {
                let mut cache = state_clone.cache.lock().await;
                if let Err(e) = cache.set(&cache_key_clone, &cache_data, None).await {
                    error!(error = %e, "Failed to cache tile in Redis");
                }
            });
            
            // Prefetch neighboring tiles in background (Google tile strategy) - if enabled
            if state.optimization_config.prefetch_enabled 
               && z >= state.optimization_config.prefetch_min_zoom 
               && z <= state.optimization_config.prefetch_max_zoom 
            {
                spawn_tile_prefetch(
                    state.clone(),
                    layer.to_string(),
                    style.to_string(),
                    coord,
                    state.prefetch_rings,
                );
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
/// Prometheus metrics endpoint
pub async fn metrics_handler(
    Extension(state): Extension<Arc<AppState>>,
    Extension(prometheus): Extension<metrics_exporter_prometheus::PrometheusHandle>,
) -> impl IntoResponse {
    // Update GRIB cache metrics before rendering
    let grib_stats = state.grib_cache.stats().await;
    let grib_size = state.grib_cache.len().await;
    let grib_capacity = state.grib_cache.capacity();
    
    state.metrics.record_grib_cache_stats(
        grib_stats.hits,
        grib_stats.misses,
        grib_stats.evictions,
        grib_size,
        grib_capacity,
    );
    
    // Update grid data cache metrics
    let grid_stats = state.grid_cache.stats().await;
    state.metrics.record_grid_cache_stats(&grid_stats);
    
    // Update L1 tile memory cache metrics
    let l1_stats = state.tile_memory_cache.stats();
    state.metrics.record_tile_memory_cache_stats(&l1_stats);
    
    // Update container resource metrics
    let container_stats = read_container_stats();
    if let Some(memory_used) = container_stats["memory"]["used_bytes"].as_u64() {
        let memory_total = container_stats["memory"]["total_bytes"].as_u64().unwrap_or(0);
        let memory_percent = container_stats["memory"]["percent_used"].as_f64().unwrap_or(0.0);
        let process_rss = container_stats["process"]["rss_bytes"].as_u64().unwrap_or(0);
        let cpu_load_1m = container_stats["cpu"]["load_1m"].as_f64().unwrap_or(0.0);
        let cpu_load_5m = container_stats["cpu"]["load_5m"].as_f64().unwrap_or(0.0);
        let cpu_load_15m = container_stats["cpu"]["load_15m"].as_f64().unwrap_or(0.0);
        let cpu_count = container_stats["cpu"]["count"].as_u64().unwrap_or(1) as usize;
        
        state.metrics.record_container_stats(
            memory_used,
            memory_total,
            memory_percent,
            process_rss,
            cpu_load_1m,
            cpu_load_5m,
            cpu_load_15m,
            cpu_count,
        );
    }
    
    let metrics = prometheus.render();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        metrics
    )
}

/// JSON metrics endpoint for the web UI
pub async fn api_metrics_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let snapshot = state.metrics.snapshot().await;
    let system = crate::metrics::SystemStats::read();
    
    // Get L1 tile memory cache stats
    let l1_stats = state.tile_memory_cache.stats();
    let l1_cache_stats = serde_json::json!({
        "hits": l1_stats.hits.load(std::sync::atomic::Ordering::Relaxed),
        "misses": l1_stats.misses.load(std::sync::atomic::Ordering::Relaxed),
        "hit_rate": l1_stats.hit_rate(),
        "evictions": l1_stats.evictions.load(std::sync::atomic::Ordering::Relaxed),
        "expired": l1_stats.expired.load(std::sync::atomic::Ordering::Relaxed),
        "size_bytes": l1_stats.size_bytes.load(std::sync::atomic::Ordering::Relaxed),
    });
    
    // Get Redis cache stats (L2)
    let l2_cache_stats = {
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
    
    // Get grid cache stats
    let grid_stats = state.grid_cache.stats().await;
    let grid_cache_stats = serde_json::json!({
        "hits": grid_stats.hits,
        "misses": grid_stats.misses,
        "evictions": grid_stats.evictions,
        "entries": grid_stats.entries,
        "capacity": grid_stats.capacity,
        "memory_bytes": grid_stats.memory_bytes,
        "memory_mb": grid_stats.memory_mb(),
        "hit_rate": grid_stats.hit_rate(),
        "utilization": grid_stats.utilization(),
    });
    
    // Combine metrics, system, and cache stats
    let combined = serde_json::json!({
        "metrics": snapshot,
        "system": system,
        "l1_cache": l1_cache_stats,
        "l2_cache": l2_cache_stats,
        "grid_cache": grid_cache_stats
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

/// Container/pod resource stats endpoint
/// Reads from /proc to get memory and CPU info, also checks cgroup limits
pub async fn container_stats_handler() -> impl IntoResponse {
    let stats = read_container_stats();
    Json(stats)
}

/// Tile request heatmap endpoint - returns geographic distribution of tile requests
pub async fn tile_heatmap_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let snapshot = state.metrics.get_tile_heatmap().await;
    Json(snapshot)
}

/// Clear the tile request heatmap
pub async fn tile_heatmap_clear_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.clear_tile_heatmap().await;
    Json(serde_json::json!({"status": "cleared"}))
}

/// Read container resource statistics from /proc and cgroup filesystems
fn read_container_stats() -> serde_json::Value {
    // Memory stats from /proc/meminfo
    let (mem_total, mem_available, mem_used) = read_proc_meminfo();
    
    // CPU stats
    let cpu_count = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);
    
    // Load average from /proc/loadavg
    let load_avg = read_load_average();
    
    // Cgroup memory limits (for containerized environments)
    let (cgroup_limit, cgroup_usage) = read_cgroup_memory();
    
    // Process stats from /proc/self
    let (proc_rss, proc_vms) = read_proc_self_status();
    
    // Determine if we're in a container
    let in_container = std::path::Path::new("/.dockerenv").exists() 
        || std::env::var("KUBERNETES_SERVICE_HOST").is_ok();
    
    // Calculate effective memory limit and usage
    let effective_limit = if cgroup_limit > 0 { cgroup_limit } else { mem_total };
    let effective_used = if cgroup_usage > 0 { cgroup_usage } else { mem_used };
    let memory_percent = if effective_limit > 0 {
        (effective_used as f64 / effective_limit as f64 * 100.0).round()
    } else {
        0.0
    };
    
    serde_json::json!({
        "container": {
            "in_container": in_container,
            "hostname": std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string()),
        },
        "memory": {
            "total_bytes": effective_limit,
            "used_bytes": effective_used,
            "available_bytes": effective_limit.saturating_sub(effective_used),
            "percent_used": memory_percent,
            "cgroup_limit_bytes": cgroup_limit,
            "cgroup_usage_bytes": cgroup_usage,
            "host_total_bytes": mem_total,
            "host_available_bytes": mem_available,
        },
        "process": {
            "rss_bytes": proc_rss,
            "vms_bytes": proc_vms,
        },
        "cpu": {
            "count": cpu_count,
            "load_1m": load_avg.0,
            "load_5m": load_avg.1,
            "load_15m": load_avg.2,
        }
    })
}

/// Read memory info from /proc/meminfo
fn read_proc_meminfo() -> (u64, u64, u64) {
    let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total: u64 = 0;
    let mut available: u64 = 0;
    
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total = parse_kb_value(line) * 1024;
        } else if line.starts_with("MemAvailable:") {
            available = parse_kb_value(line) * 1024;
        }
    }
    
    let used = total.saturating_sub(available);
    (total, available, used)
}

/// Parse a value in KB from /proc/meminfo format
fn parse_kb_value(line: &str) -> u64 {
    line.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Read load average from /proc/loadavg
fn read_load_average() -> (f64, f64, f64) {
    let content = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let parts: Vec<&str> = content.split_whitespace().collect();
    
    let load_1 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let load_5 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let load_15 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    
    (load_1, load_5, load_15)
}

/// Read cgroup memory limits (works for cgroup v1 and v2)
fn read_cgroup_memory() -> (u64, u64) {
    // Try cgroup v2 first
    if let Ok(limit) = std::fs::read_to_string("/sys/fs/cgroup/memory.max") {
        let limit = limit.trim();
        let limit_bytes = if limit == "max" {
            0 // No limit
        } else {
            limit.parse().unwrap_or(0)
        };
        
        let usage = std::fs::read_to_string("/sys/fs/cgroup/memory.current")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        
        return (limit_bytes, usage);
    }
    
    // Try cgroup v1
    if let Ok(limit) = std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes") {
        let limit_bytes: u64 = limit.trim().parse().unwrap_or(0);
        // Check for "unlimited" (very large value)
        let limit_bytes = if limit_bytes > 1 << 62 { 0 } else { limit_bytes };
        
        let usage = std::fs::read_to_string("/sys/fs/cgroup/memory/memory.usage_in_bytes")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        
        return (limit_bytes, usage);
    }
    
    (0, 0)
}

/// Read process memory from /proc/self/status
fn read_proc_self_status() -> (u64, u64) {
    let content = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut rss: u64 = 0;
    let mut vms: u64 = 0;
    
    for line in content.lines() {
        if line.starts_with("VmRSS:") {
            rss = parse_kb_value(line) * 1024;
        } else if line.starts_with("VmSize:") {
            vms = parse_kb_value(line) * 1024;
        }
    }
    
    (rss, vms)
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

/// Dimension parameters for WMS/WMTS requests
/// 
/// For observation layers (GOES, MRMS): use `time` (ISO8601 timestamp)
/// For forecast models (GFS, HRRR): use `run` (ISO8601) + `forecast` (hours)
#[derive(Debug, Clone, Default)]
pub struct DimensionParams {
    /// TIME dimension - for observation layers (ISO8601 timestamp)
    pub time: Option<String>,
    /// RUN dimension - for forecast models (ISO8601 model run time)
    pub run: Option<String>,
    /// FORECAST dimension - for forecast models (hours from run time)
    pub forecast: Option<String>,
    /// ELEVATION dimension - vertical level (e.g., "500 mb", "2 m above ground")
    pub elevation: Option<String>,
}

impl DimensionParams {
    /// Parse dimensions based on layer type (observation vs forecast model)
    /// Uses the model dimension registry to determine dimension type.
    /// Returns (forecast_hour, observation_time, reference_time) tuple
    pub fn parse_for_layer(&self, model: &str, registry: &ModelDimensionRegistry) -> (Option<u32>, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>) {
        let is_observational = registry.is_observation(model);
        
        if is_observational {
            // Observation layers use TIME dimension (ISO8601)
            let observation_time = self.time.as_ref().and_then(|t| {
                parse_iso8601_timestamp(t)
            });
            (None, observation_time, None)
        } else {
            // Forecast models use RUN + FORECAST dimensions
            let reference_time = self.run.as_ref().and_then(|r| {
                if r == "latest" {
                    None  // Will use latest run
                } else {
                    parse_iso8601_timestamp(r)
                }
            });
            
            let forecast_hour = self.forecast.as_ref().and_then(|f| {
                f.parse::<u32>().ok()
            });
            
            (forecast_hour, None, reference_time)
        }
    }
}

/// Parse an ISO8601 timestamp string
fn parse_iso8601_timestamp(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    // Try various ISO8601 formats
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ") {
        return Some(dt.and_utc());
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ") {
        return Some(dt.and_utc());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    None
}

async fn render_weather_data(
    state: &Arc<AppState>,
    layer: &str,
    style: &str,
    width: u32,
    height: u32,
    bbox: Option<&str>,
    crs: Option<&str>,
    dimensions: &DimensionParams,
) -> Result<Vec<u8>, String> {
    // Parse layer name (format: "model_parameter" or "model_WIND_BARBS")
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return Err("Invalid layer format".to_string());
    }

    let model = parts[0];
    let parameter = parts[1..].join("_");
    
    // Parse dimensions based on layer type (using config-driven dimension registry)
    let (forecast_hour, observation_time, _reference_time) = dimensions.parse_for_layer(model, &state.model_dimensions);
    
    // ELEVATION parameter is the level string (e.g., "500 mb", "2 m above ground")
    let level = dimensions.elevation.clone();
    
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
            &state.grib_cache,
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
    
    info!(forecast_hour = ?forecast_hour, observation_time = ?observation_time, level = ?level, bbox = ?parsed_bbox, style = style, "Parsed WMS parameters");

    // Check if isolines style is requested
    // Check if CRS is Web Mercator - use Mercator projection for resampling
    let crs_str = crs.unwrap_or("EPSG:4326");
    let use_mercator = crs_str.contains("3857");
    
    if style == "isolines" {
        // Isolines are only supported for GRIB2 data (GFS, HRRR temperature)
        // Not supported for observation data (GOES, MRMS) which are satellite/radar imagery
        if state.model_dimensions.is_observation(model) {
            return Err(format!(
                "Isolines style is not supported for {} layers. Isolines are only available for temperature parameters (GFS, HRRR).",
                model.to_uppercase()
            ));
        }
        
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            format!("{}/temperature_isolines.json", style_config_dir)
        } else {
            // Default to temperature isolines for now
            format!("{}/temperature_isolines.json", style_config_dir)
        };
        
        // For WMS, we don't have tile coordinates, so pass None
        return crate::rendering::render_isolines_tile_with_level(
            &state.grib_cache,
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
    
    if style == "numbers" {
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = if parameter.contains("CMI") {
            // GOES satellite data
            if parameter.contains("C01") || parameter.contains("C02") || parameter.contains("C03") {
                // Visible/near-IR bands
                format!("{}/goes_visible.json", style_config_dir)
            } else {
                // IR bands (C08-C16)
                format!("{}/goes_ir.json", style_config_dir)
            }
        } else if parameter.contains("TMP") || parameter.contains("TEMP") {
            format!("{}/temperature.json", style_config_dir)
        } else if parameter.contains("WIND") || parameter.contains("GUST") {
            format!("{}/wind.json", style_config_dir)
        } else if parameter.contains("PRES") || parameter.contains("PRMSL") {
            format!("{}/atmospheric.json", style_config_dir)
        } else if parameter.contains("PRECIP_RATE") {
            format!("{}/precip_rate.json", style_config_dir)
        } else if parameter.contains("QPE") || parameter.contains("PRECIP") {
            format!("{}/precipitation.json", style_config_dir)
        } else if parameter.contains("REFL") {
            format!("{}/reflectivity.json", style_config_dir)
        } else {
            // Default to temperature for generic parameters
            format!("{}/temperature.json", style_config_dir)
        };
        
        return crate::rendering::render_numbers_tile(
            &state.grib_cache,
            state.grid_cache_if_enabled(),
            &state.catalog,
            &state.metrics,
            model,
            &parameter,
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

    // Use shared rendering logic with support for observation time
    crate::rendering::render_weather_data_with_time(
        &state.grib_cache,
        state.grid_cache_if_enabled(),
        &state.catalog,
        &state.metrics,
        model,
        &parameter,
        forecast_hour,
        observation_time,
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

/// Get human-readable display name for a model/data source
fn get_model_display_name(model: &str) -> String {
    match model {
        "goes16" => "GOES-16 East".to_string(),
        "goes18" => "GOES-18 West".to_string(),
        "gfs" => "GFS".to_string(),
        "hrrr" => "HRRR".to_string(),
        "mrms" => "MRMS".to_string(),
        _ => model.to_uppercase(),
    }
}

/// Get human-readable name for a GRIB parameter code
fn get_parameter_name(param: &str) -> String {
    match param {
        // Core surface parameters
        "PRMSL" => "Mean Sea Level Pressure".to_string(),
        "TMP" => "Temperature".to_string(),
        "DPT" => "Dew Point Temperature".to_string(),
        "RH" => "Relative Humidity".to_string(),
        "UGRD" => "U-Component Wind".to_string(),
        "VGRD" => "V-Component Wind".to_string(),
        "WIND_BARBS" => "Wind Barbs".to_string(),
        "GUST" => "Wind Gust Speed".to_string(),
        "HGT" => "Geopotential Height".to_string(),
        
        // Precipitation parameters
        "APCP" => "Total Precipitation".to_string(),
        "PWAT" => "Precipitable Water".to_string(),
        
        // Convective/stability parameters
        "CAPE" => "Convective Available Potential Energy".to_string(),
        "CIN" => "Convective Inhibition".to_string(),
        
        // Cloud parameters
        "TCDC" => "Total Cloud Cover".to_string(),
        "LCDC" => "Low Cloud Cover".to_string(),
        "MCDC" => "Middle Cloud Cover".to_string(),
        "HCDC" => "High Cloud Cover".to_string(),
        
        // Visibility
        "VIS" => "Visibility".to_string(),
        
        // Radar/reflectivity (HRRR)
        "REFC" => "Composite Reflectivity".to_string(),
        "RETOP" => "Echo Top Height".to_string(),
        
        // Severe weather (HRRR)
        "MXUPHL" => "Max Updraft Helicity".to_string(),
        "LTNG" => "Lightning Threat".to_string(),
        "HLCY" => "Storm-Relative Helicity".to_string(),
        
        // GRIB2 Product 1 (Meteorological) parameters
        "P1_22" => "Cloud Mixing Ratio".to_string(),
        "P1_23" => "Ice Mixing Ratio".to_string(),
        "P1_24" => "Rain Mixing Ratio".to_string(),
        
        // MRMS parameters
        "REFL" => "Radar Reflectivity".to_string(),
        "PRECIP_RATE" => "Precipitation Rate".to_string(),
        "QPE" => "Quantitative Precipitation Estimate".to_string(),
        "QPE_01H" => "1-Hour Precipitation".to_string(),
        "QPE_03H" => "3-Hour Precipitation".to_string(),
        "QPE_06H" => "6-Hour Precipitation".to_string(),
        "QPE_24H" => "24-Hour Precipitation".to_string(),
        
        // GOES parameters (ABI bands) - User-friendly titles with band info
        "IR" => "Infrared Imagery".to_string(),
        "WV" => "Water Vapor".to_string(),
        "CMI" => "Cloud and Moisture Imagery".to_string(),
        "CMI_C01" => "Visible Blue - Band 1 (0.47µm)".to_string(),
        "CMI_C02" => "Visible Red - Band 2 (0.64µm)".to_string(),
        "CMI_C03" => "Veggie - Band 3 (0.86µm)".to_string(),
        "CMI_C04" => "Cirrus - Band 4 (1.37µm)".to_string(),
        "CMI_C05" => "Snow/Ice - Band 5 (1.6µm)".to_string(),
        "CMI_C06" => "Cloud Particle Size - Band 6 (2.2µm)".to_string(),
        "CMI_C07" => "Shortwave Window IR - Band 7 (3.9µm)".to_string(),
        "CMI_C08" => "Upper-Level Water Vapor - Band 8 (6.2µm)".to_string(),
        "CMI_C09" => "Mid-Level Water Vapor - Band 9 (6.9µm)".to_string(),
        "CMI_C10" => "Lower-Level Water Vapor - Band 10 (7.3µm)".to_string(),
        "CMI_C11" => "Cloud-Top Phase - Band 11 (8.4µm)".to_string(),
        "CMI_C12" => "Ozone - Band 12 (9.6µm)".to_string(),
        "CMI_C13" => "Clean Longwave IR - Band 13 (10.3µm)".to_string(),
        "CMI_C14" => "Longwave IR - Band 14 (11.2µm)".to_string(),
        "CMI_C15" => "Dirty Longwave IR - Band 15 (12.3µm)".to_string(),
        "CMI_C16" => "CO2 Longwave IR - Band 16 (13.3µm)".to_string(),
        
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
    model_bboxes: &HashMap<String, wms_common::BoundingBox>,
    dimension_registry: &ModelDimensionRegistry,
) -> String {
    let empty_params = Vec::new();
    let empty_dims = (Vec::new(), Vec::new());
    let empty_levels = Vec::new();
    
    let layers: String = models
        .iter()
        .map(|model| {
            let params = model_params.get(model).unwrap_or(&empty_params);
            let (runs, forecasts) = model_dimensions.get(model).unwrap_or(&empty_dims);
            
            // Get bbox for this model, default to global if not found
            let bbox = model_bboxes.get(model).cloned().unwrap_or_else(|| {
                wms_common::BoundingBox::new(0.0, -90.0, 360.0, 90.0)
            });
            
            // Normalize longitude to -180/180 for WMS
            // For global data (0-360), convert to -180/180
            let (west, east) = if bbox.min_x == 0.0 && bbox.max_x == 360.0 {
                (-180.0, 180.0)
            } else {
                // For regional data, convert >180 to negative
                let w = if bbox.min_x > 180.0 { bbox.min_x - 360.0 } else { bbox.min_x };
                let e = if bbox.max_x > 180.0 { bbox.max_x - 360.0 } else { bbox.max_x };
                (w, e)
            };
            let south = bbox.min_y;
            let north = bbox.max_y;
            
            // Check if this is an observational model using the config-driven registry
            // Observational data uses TIME dimension with ISO8601 timestamps
            // Forecast models use RUN (model initialization time) + FORECAST (hours ahead) dimensions
            let is_observational = dimension_registry.is_observation(model);
            
            // Build time-related dimensions based on data type
            let base_dimensions = if is_observational {
                // For observational data: TIME dimension with available observation timestamps
                // The runs list contains the observation times (stored as reference_time with forecast_hour=0)
                let time_values = if runs.is_empty() { "latest".to_string() } else { runs.join(",") };
                let time_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
                format!(
                    r#"<Dimension name="TIME" units="ISO8601" default="{}">{}</Dimension>"#,
                    time_default, time_values
                )
            } else {
                // For forecast models: RUN dimension (model run time) + FORECAST dimension (hours ahead)
                // RUN: ISO8601 timestamps of available model runs (defaults to latest)
                let run_values = if runs.is_empty() { "latest".to_string() } else { runs.join(",") };
                let run_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
                
                // FORECAST: Available forecast hours (defaults to first/earliest)
                let forecast_values = if forecasts.is_empty() { 
                    "0".to_string() 
                } else { 
                    forecasts.iter()
                        .map(|h| h.to_string())
                        .collect::<Vec<_>>()
                        .join(",") 
                };
                let forecast_default = forecasts.first().unwrap_or(&0);
                
                format!(
                    r#"<Dimension name="RUN" units="ISO8601" default="{}">{}</Dimension><Dimension name="FORECAST" units="hours" default="{}">{}</Dimension>"#,
                    run_default, run_values,
                    forecast_default, forecast_values
                )
            };
            
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
                    
                    // Add styles to each layer based on parameter type
                    let styles = if p.contains("TMP") || p.contains("TEMP") || p == "DPT" {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>temperature</Name><Title>Temperature Gradient</Title></Style><Style><Name>isolines</Name><Title>Temperature Isolines</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p.contains("WIND") || p.contains("GUST") {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>wind</Name><Title>Wind Speed</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p.contains("PRES") || p.contains("PRMSL") {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>atmospheric</Name><Title>Atmospheric Pressure</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p == "RH" || p.contains("HUMID") || p == "PWAT" {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>humidity</Name><Title>Humidity</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p == "CAPE" || p == "CIN" {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>cape</Name><Title>Convective Energy</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p.contains("TCDC") || p.contains("LCDC") || p.contains("MCDC") || p.contains("HCDC") {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>cloud</Name><Title>Cloud Cover</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p == "VIS" {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>visibility</Name><Title>Visibility</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p == "LTNG" {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>lightning</Name><Title>Lightning Threat</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p == "MXUPHL" || p == "HLCY" {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>helicity</Name><Title>Storm Helicity</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p == "REFC" || p.contains("REFL") || p == "RETOP" {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>reflectivity</Name><Title>Radar Reflectivity</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p.contains("PRECIP_RATE") {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>precip_rate</Name><Title>Precipitation Rate</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else if p == "APCP" || p.contains("QPE") || p.contains("PRECIP") {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>precipitation</Name><Title>Precipitation</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    } else {
                        "<Style><Name>default</Name><Title>Default</Title></Style><Style><Name>numbers</Name><Title>Numeric Values</Title></Style>"
                    };
                    
                    format!(
                        r#"<Layer queryable="1"><Name>{}_{}</Name><Title>{} - {}</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="{}" miny="{}" maxx="{}" maxy="{}"/>{}{}</Layer>"#,
                        model, p, get_model_display_name(model), get_parameter_name(p),
                        west, east, south, north,
                        west, south, east, north,
                        styles, all_dimensions
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
                     r#"<Layer queryable="1"><Name>{}_WIND_BARBS</Name><Title>{} - Wind Barbs</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="{}" miny="{}" maxx="{}" maxy="{}"/><Style><Name>default</Name><Title>Default Barbs</Title></Style>{}{}</Layer>"#,
                     model, get_model_display_name(model),
                     west, east, south, north,
                     west, south, east, north,
                     base_dimensions, wind_elevation_dim
                 )
             } else {
                 String::new()
             };

             format!(
                 r#"<Layer><Name>{}</Name><Title>{}</Title>{}{}</Layer>"#,
                 model,
                 get_model_display_name(model),
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
    param_levels: &HashMap<String, Vec<String>>,
    dimension_registry: &ModelDimensionRegistry,
) -> String {
    let empty_params = Vec::new();
    let empty_dims = (Vec::new(), Vec::new());
    let empty_levels = Vec::new();
    
    // Build layer definitions for each model/parameter combination
    let mut all_layers: Vec<String> = models
        .iter()
        .flat_map(|model| {
            let params = model_params.get(model).unwrap_or(&empty_params);
            let (runs, forecasts) = model_dimensions.get(model).unwrap_or(&empty_dims);
            
            // Check if this is an observational model using the config-driven registry
            let is_observational = dimension_registry.is_observation(model);
            
            // Build time-related dimension XML based on data type
            let time_dimensions_xml = if is_observational {
                // For observational data: TIME dimension with ISO8601 timestamps
                let time_values_xml: String = if runs.is_empty() {
                    "        <Value>latest</Value>".to_string()
                } else {
                    runs.iter()
                        .map(|v| format!("        <Value>{}</Value>", v))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let time_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
                
                format!(
                    r#"      <Dimension>
        <ows:Identifier>time</ows:Identifier>
        <ows:UOM>ISO8601</ows:UOM>
        <Default>{}</Default>
{}
      </Dimension>"#,
                    time_default, time_values_xml
                )
            } else {
                // For forecast models: RUN dimension + FORECAST dimension
                let run_values_xml: String = if runs.is_empty() {
                    "        <Value>latest</Value>".to_string()
                } else {
                    runs.iter()
                        .map(|v| format!("        <Value>{}</Value>", v))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let run_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
                
                let forecast_values_xml: String = if forecasts.is_empty() {
                    "        <Value>0</Value>".to_string()
                } else {
                    forecasts.iter()
                        .map(|v| format!("        <Value>{}</Value>", v))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let forecast_default = forecasts.first().unwrap_or(&0);
                
                format!(
                    r#"      <Dimension>
        <ows:Identifier>run</ows:Identifier>
        <ows:UOM>ISO8601</ows:UOM>
        <Default>{}</Default>
{}
      </Dimension>
      <Dimension>
        <ows:Identifier>forecast</ows:Identifier>
        <ows:UOM>hours</ows:UOM>
        <Default>{}</Default>
{}
      </Dimension>"#,
                    run_default, run_values_xml,
                    forecast_default, forecast_values_xml
                )
            };
            
            let model_clone = model.clone();
            let time_dimensions_xml_clone = time_dimensions_xml.clone();
            let empty_levels_clone = empty_levels.clone();
            params.iter().map(move |param| {
                let layer_id = format!("{}_{}", model_clone, param);
                let layer_title = format!("{} - {}", get_model_display_name(&model_clone), get_parameter_name(param));
                
                // Get levels for this parameter and build ELEVATION dimension if available
                let param_key = format!("{}_{}", model_clone, param);
                let levels = param_levels.get(&param_key).unwrap_or(&empty_levels_clone);
                
                let elevation_dim = if levels.len() > 1 {
                    // Sort levels for display (pressure levels should be sorted numerically)
                    let mut sorted_levels = levels.clone();
                    sorted_levels.sort_by(|a, b| {
                        let a_val = a.replace(" mb", "").parse::<i32>().unwrap_or(9999);
                        let b_val = b.replace(" mb", "").parse::<i32>().unwrap_or(9999);
                        b_val.cmp(&a_val)  // Descending (1000 mb first)
                    });
                    let level_values_xml: String = sorted_levels.iter()
                        .map(|v| format!("        <Value>{}</Value>", v))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let default_level = sorted_levels.first().map(|s| s.as_str()).unwrap_or("");
                    
                    format!(
                        r#"
      <Dimension>
        <ows:Identifier>elevation</ows:Identifier>
        <ows:UOM></ows:UOM>
        <Default>{}</Default>
{}
      </Dimension>"#,
                        default_level, level_values_xml
                    )
                } else {
                    String::new()
                };
                
                // Combine time dimensions with elevation dimension
                let all_dimensions = format!("{}{}", time_dimensions_xml_clone, elevation_dim);
                
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
      </Style>
      <Style>
        <ows:Title>Numeric Values</ows:Title>
        <ows:Identifier>numbers</ows:Identifier>
      </Style>"#
                } else if param.contains("WIND") || param.contains("GUST") {
                    r#"      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Wind Speed</ows:Title>
        <ows:Identifier>wind</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Numeric Values</ows:Title>
        <ows:Identifier>numbers</ows:Identifier>
      </Style>"#
                } else if param.contains("PRES") || param.contains("PRMSL") {
                    r#"      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Atmospheric Pressure</ows:Title>
        <ows:Identifier>atmospheric</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Numeric Values</ows:Title>
        <ows:Identifier>numbers</ows:Identifier>
      </Style>"#
                } else if param.contains("RH") || param.contains("HUMID") || param.contains("PRECIP") {
                    r#"      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Precipitation</ows:Title>
        <ows:Identifier>precipitation</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Numeric Values</ows:Title>
        <ows:Identifier>numbers</ows:Identifier>
      </Style>"#
                } else {
                    r#"      <Style isDefault="true">
        <ows:Title>Default</ows:Title>
        <ows:Identifier>default</ows:Identifier>
      </Style>
      <Style>
        <ows:Title>Numeric Values</ows:Title>
        <ows:Identifier>numbers</ows:Identifier>
      </Style>"#
                };
                
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
{}
      <ResourceURL format="image/png" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png"/>
    </Layer>"#,
                    layer_title, layer_id, styles, 
                    all_dimensions,
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
            let layer_title = format!("{} - Wind Barbs", get_model_display_name(model));
            
            // Wind barbs are only for forecast models (they need UGRD/VGRD from GFS/HRRR)
            // Build RUN + FORECAST dimensions
            let run_values_xml: String = if runs.is_empty() {
                "        <Value>latest</Value>".to_string()
            } else {
                runs.iter()
                    .map(|v| format!("        <Value>{}</Value>", v))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let run_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
            
            let forecast_values_xml: String = if forecasts.is_empty() {
                "        <Value>0</Value>".to_string()
            } else {
                forecasts.iter()
                    .map(|v| format!("        <Value>{}</Value>", v))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let forecast_default = forecasts.first().unwrap_or(&0);
            
            // Get ELEVATION dimension from UGRD (same levels as VGRD)
            let ugrd_key = format!("{}_UGRD", model);
            let wind_levels = param_levels.get(&ugrd_key).unwrap_or(&empty_levels);
            let elevation_dim = if wind_levels.len() > 1 {
                let mut sorted_levels = wind_levels.clone();
                sorted_levels.sort_by(|a, b| {
                    let a_val = a.replace(" mb", "").parse::<i32>().unwrap_or(9999);
                    let b_val = b.replace(" mb", "").parse::<i32>().unwrap_or(9999);
                    b_val.cmp(&a_val)
                });
                let level_values_xml: String = sorted_levels.iter()
                    .map(|v| format!("        <Value>{}</Value>", v))
                    .collect::<Vec<_>>()
                    .join("\n");
                let default_level = sorted_levels.first().map(|s| s.as_str()).unwrap_or("");
                
                format!(
                    r#"
      <Dimension>
        <ows:Identifier>elevation</ows:Identifier>
        <ows:UOM></ows:UOM>
        <Default>{}</Default>
{}
      </Dimension>"#,
                    default_level, level_values_xml
                )
            } else {
                String::new()
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
        <ows:Identifier>run</ows:Identifier>
        <ows:UOM>ISO8601</ows:UOM>
        <Default>{}</Default>
{}
      </Dimension>
      <Dimension>
        <ows:Identifier>forecast</ows:Identifier>
        <ows:UOM>hours</ows:UOM>
        <Default>{}</Default>
{}
      </Dimension>{}
      <ResourceURL format="image/png" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png"/>
    </Layer>"#,
                layer_title, layer_id, 
                run_default, run_values_xml,
                forecast_default, forecast_values_xml,
                elevation_dim,
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
#[instrument(skip(_state))]
pub async fn validation_status_handler(
    Extension(_state): Extension<Arc<AppState>>,
) -> Result<Json<crate::validation::ValidationStatus>, StatusCode> {
    info!("Validation status requested");
    
    // Get base URL from environment or use localhost
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    
    let status = crate::validation::run_validation(&base_url).await;
    
    Ok(Json(status))
}

/// GET /api/validation/run - Run validation and return results
#[instrument(skip(_state))]
pub async fn validation_run_handler(
    Extension(_state): Extension<Arc<AppState>>,
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

/// Run startup-style validation to test data ingestion and rendering.
/// This validates each available model's data by rendering test tiles.
#[instrument(skip(state))]
pub async fn startup_validation_run_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    use crate::startup_validation::{StartupValidator, StartupValidationConfig};
    
    info!("Startup validation run requested");
    
    let config = StartupValidationConfig::from_env();
    let validator = StartupValidator::new(state, config);
    let summary = validator.validate().await;
    
    info!(
        total = summary.total_tests,
        passed = summary.passed,
        failed = summary.failed,
        models = ?summary.models_available,
        "Startup validation completed"
    );
    
    Json(summary)
}

// ============================================================================
// Tile Prefetching
// ============================================================================

/// Get the 8 neighboring tiles around a center tile (Google tile strategy).
/// Returns tiles at the same zoom level that surround the requested tile.
/// Get tiles within N rings around a center tile.
/// Ring 0: just the center (1 tile)
/// Ring 1: 8 tiles immediately surrounding center
/// Ring 2: 16 tiles surrounding ring 1 (total 24 tiles for rings=2)
/// Ring N: 8*N tiles surrounding ring N-1
fn get_tiles_in_rings(center: &TileCoord, rings: u32) -> Vec<TileCoord> {
    let z = center.z;
    let max_tile = 2u32.pow(z) - 1;
    let cx = center.x as i32;
    let cy = center.y as i32;
    
    // Estimate capacity: sum of tiles in all rings (excluding center)
    // Ring 1: 8, Ring 2: 16, Ring 3: 24, etc.
    let capacity = if rings == 0 { 0 } else { (rings * (rings + 1) * 4) as usize };
    let mut tiles = Vec::with_capacity(capacity);
    
    // Iterate through each ring
    for ring in 1..=rings {
        let r = ring as i32;
        
        // For each ring, we walk around the perimeter
        // Start at top-left corner of the ring and walk clockwise
        
        // Top edge (moving right)
        for dx in -r..=r {
            let x = cx + dx;
            let y = cy - r;
            if x >= 0 && x <= max_tile as i32 && y >= 0 && y <= max_tile as i32 {
                tiles.push(TileCoord::new(z, x as u32, y as u32));
            }
        }
        
        // Right edge (moving down) - skip top-right corner (already added)
        for dy in -r+1..=r {
            let x = cx + r;
            let y = cy + dy;
            if x >= 0 && x <= max_tile as i32 && y >= 0 && y <= max_tile as i32 {
                tiles.push(TileCoord::new(z, x as u32, y as u32));
            }
        }
        
        // Bottom edge (moving left) - skip bottom-right corner
        for dx in (-r+1..=r).rev() {
            let x = cx + dx;
            let y = cy + r;
            if x >= 0 && x <= max_tile as i32 && y >= 0 && y <= max_tile as i32 {
                tiles.push(TileCoord::new(z, x as u32, y as u32));
            }
        }
        
        // Left edge (moving up) - skip bottom-left and top-left corners
        for dy in (-r+1..r).rev() {
            let x = cx - r;
            let y = cy + dy;
            if x >= 0 && x <= max_tile as i32 && y >= 0 && y <= max_tile as i32 {
                tiles.push(TileCoord::new(z, x as u32, y as u32));
            }
        }
    }
    
    tiles
}

/// Get immediate neighboring tiles (1 ring = 8 tiles).
/// This is a convenience wrapper around get_tiles_in_rings for backward compatibility.
#[allow(dead_code)]
fn get_neighboring_tiles(center: &TileCoord) -> Vec<TileCoord> {
    get_tiles_in_rings(center, 1)
}

/// Spawn background tasks to prefetch neighboring tiles.
/// This improves perceived performance when panning the map.
/// 
/// # Arguments
/// * `rings` - Number of rings to prefetch (1 = 8 tiles, 2 = 24 tiles, 3 = 48 tiles)
fn spawn_tile_prefetch(
    state: Arc<AppState>,
    layer: String,
    style: String,
    center: TileCoord,
    rings: u32,
) {
    let neighbors = get_tiles_in_rings(&center, rings);
    
    debug!(
        layer = %layer,
        z = center.z,
        x = center.x,
        y = center.y,
        rings = rings,
        tile_count = neighbors.len(),
        "Spawning prefetch tasks"
    );
    
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
            &state.grib_cache,
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
            &state.grib_cache,
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
    } else if style == "numbers" {
        let style_config_dir = std::env::var("STYLE_CONFIG_DIR").unwrap_or_else(|_| "./config/styles".to_string());
        let style_file = if parameter.contains("TMP") || parameter.contains("TEMP") {
            format!("{}/temperature.json", style_config_dir)
        } else if parameter.contains("WIND") || parameter.contains("GUST") {
            format!("{}/wind.json", style_config_dir)
        } else if parameter.contains("PRES") || parameter.contains("PRMSL") {
            format!("{}/atmospheric.json", style_config_dir)
        } else if parameter.contains("PRECIP_RATE") {
            format!("{}/precip_rate.json", style_config_dir)
        } else if parameter.contains("QPE") || parameter.contains("PRECIP") {
            format!("{}/precipitation.json", style_config_dir)
        } else if parameter.contains("REFL") {
            format!("{}/reflectivity.json", style_config_dir)
        } else {
            format!("{}/temperature.json", style_config_dir)
        };
        
        crate::rendering::render_numbers_tile(
            &state.grib_cache,
            state.grid_cache_if_enabled(),
            &state.catalog,
            &state.metrics,
            model,
            &parameter,
            256,
            256,
            bbox_array,
            &style_file,
            None,
            None,
            true,
        )
        .await
    } else {
        crate::rendering::render_weather_data(
            &state.grib_cache,
            &state.catalog,
            &state.metrics,
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

/// Clear all in-memory caches (L1 tile cache, GRIB cache, Grid cache)
/// POST /api/cache/clear
/// 
/// This endpoint is useful for:
/// - Benchmarking: ensures cold cache state between test runs
/// - Development: clearing stale cached data after code changes
/// - Testing: isolating cache behavior
/// 
/// Note: This does NOT clear the Redis L2 cache. Use reset_test_state.sh for full cache reset.
#[instrument(skip(state))]
pub async fn cache_clear_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    use serde_json::json;
    
    info!("Clearing all in-memory caches");
    
    // Get stats before clearing
    let l1_before = state.tile_memory_cache.len().await;
    let grib_before = state.grib_cache.len().await;
    let grid_before = state.grid_cache.len().await;
    
    // Clear all caches
    state.tile_memory_cache.clear().await;
    state.grib_cache.clear().await;
    state.grid_cache.clear().await;
    
    info!(
        l1_cleared = l1_before,
        grib_cleared = grib_before,
        grid_cleared = grid_before,
        "In-memory caches cleared"
    );
    
    Json(json!({
        "success": true,
        "cleared": {
            "l1_tile_cache": l1_before,
            "grib_cache": grib_before,
            "grid_cache": grid_before,
        },
        "message": "All in-memory caches cleared. Redis L2 cache was not affected."
    }))
}

/// Cache viewer - list all cached tiles
pub async fn cache_list_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let mut cache = state.cache.lock().await;
    
    match cache.keys("*").await {
        Ok(keys) => {
            let tiles: Vec<serde_json::Value> = keys.iter().map(|key| {
                // Parse the cache key to extract metadata
                // Key format: "wms:layer:style:crs:bbox_w_h:time:format"
                let parts: Vec<&str> = key.split(':').collect();
                
                serde_json::json!({
                    "key": key,
                    "layer": parts.get(1).unwrap_or(&"unknown"),
                    "style": parts.get(2).unwrap_or(&"unknown"),
                    "crs": parts.get(3).unwrap_or(&"unknown"),
                })
            }).collect();
            
            Json(serde_json::json!({
                "total": tiles.len(),
                "tiles": tiles
            }))
        }
        Err(e) => {
            Json(serde_json::json!({
                "error": e.to_string(),
                "total": 0,
                "tiles": []
            }))
        }
    }
}

/// Configuration endpoint - shows current optimization settings
pub async fn config_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    use serde_json::json;
    
    // L2 cache (Redis) is always enabled when connected
    let l2_cache_enabled = true;
    
    // Check projection LUT status
    let lut_goes16_loaded = state.projection_luts.goes16.is_some();
    let lut_goes18_loaded = state.projection_luts.goes18.is_some();
    let lut_memory_mb = state.projection_luts.memory_usage() as f64 / 1024.0 / 1024.0;
    
    Json(json!({
        "optimizations": {
            "l1_cache": {
                "enabled": state.optimization_config.l1_cache_enabled,
                "size": state.optimization_config.l1_cache_size,
                "ttl_secs": state.optimization_config.l1_cache_ttl_secs,
            },
            "l2_cache": {
                "enabled": l2_cache_enabled,
            },
            "grib_cache": {
                "enabled": state.optimization_config.grib_cache_enabled,
                "size": state.optimization_config.grib_cache_size,
            },
            "grid_cache": {
                "enabled": state.optimization_config.grid_cache_enabled,
                "size": state.optimization_config.grid_cache_size,
            },
            "prefetch": {
                "enabled": state.optimization_config.prefetch_enabled,
                "rings": state.optimization_config.prefetch_rings,
                "min_zoom": state.optimization_config.prefetch_min_zoom,
                "max_zoom": state.optimization_config.prefetch_max_zoom,
            },
            "cache_warming": {
                "enabled": state.optimization_config.cache_warming_enabled,
            },
            "projection_lut": {
                "enabled": state.optimization_config.projection_lut_enabled,
                "dir": state.optimization_config.projection_lut_dir,
                "goes16_loaded": lut_goes16_loaded,
                "goes18_loaded": lut_goes18_loaded,
                "memory_mb": lut_memory_mb,
            },
            "memory_pressure": {
                "enabled": state.optimization_config.memory_pressure_enabled,
                "limit_mb": state.optimization_config.memory_limit_mb,
                "threshold": state.optimization_config.memory_pressure_threshold,
                "target": state.optimization_config.memory_pressure_target,
            }
        },
        "version": env!("CARGO_PKG_VERSION"),
        "runtime": {
            "prefetch_rings": state.prefetch_rings,
        }
    }))
}

/// Load test results endpoint - serves historical test results
pub async fn loadtest_results_handler() -> impl IntoResponse {
    use serde_json::json;
    use std::fs;
    
    // Read JSONL file from validation/load-test/results/
    let results_dir = std::env::var("LOAD_TEST_RESULTS_DIR")
        .unwrap_or_else(|_| "./validation/load-test/results".to_string());
    
    let jsonl_path = format!("{}/runs.jsonl", results_dir);
    
    info!("Looking for load test results at: {}", jsonl_path);
    
    #[derive(Serialize, Deserialize, Clone)]
    struct TestRun {
        timestamp: String,
        scenario_name: String,
        duration_secs: f64,
        total_requests: u64,
        successful_requests: u64,
        failed_requests: u64,
        requests_per_second: f64,
        latency_p50: f64,
        latency_p90: f64,
        latency_p95: f64,
        latency_p99: f64,
        latency_min: f64,
        latency_max: f64,
        latency_avg: f64,
        cache_hit_rate: f64,
        bytes_per_second: f64,
        #[serde(default)]
        layers: Vec<String>,
        #[serde(default)]
        concurrency: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        system_config: Option<SystemConfigInfo>,
    }
    
    #[derive(Serialize, Deserialize, Clone)]
    struct SystemConfigInfo {
        l1_cache_enabled: bool,
        l1_cache_size: usize,
        l1_cache_ttl_secs: u64,
        #[serde(default)]
        l2_cache_enabled: bool,
        grib_cache_enabled: bool,
        grib_cache_size: usize,
        #[serde(default)]
        grid_cache_enabled: bool,
        #[serde(default)]
        grid_cache_size: usize,
        prefetch_enabled: bool,
        prefetch_rings: u32,
        prefetch_min_zoom: u32,
        prefetch_max_zoom: u32,
        cache_warming_enabled: bool,
    }
    
    let mut runs = Vec::new();
    
    if let Ok(content) = fs::read_to_string(&jsonl_path) {
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            
            // Parse each line as JSON
            if let Ok(run) = serde_json::from_str::<TestRun>(line) {
                runs.push(run);
            }
        }
    } else {
        info!("No JSONL file found at {}, returning empty results", jsonl_path);
    }
    
    Json(json!({
        "count": runs.len(),
        "runs": runs,
    }))
}

/// Load test request log files endpoint - lists available JSONL files
pub async fn loadtest_files_handler() -> impl IntoResponse {
    use serde_json::json;
    use std::fs;
    
    let results_dir = std::env::var("LOAD_TEST_RESULTS_DIR")
        .unwrap_or_else(|_| "./validation/load-test/results".to_string());
    
    let mut files = Vec::new();
    
    if let Ok(entries) = fs::read_dir(&results_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                // Include any JSONL file except runs.jsonl (which is the summary file)
                if filename.ends_with(".jsonl") && filename != "runs.jsonl" {
                    if let Ok(metadata) = entry.metadata() {
                        let size_bytes = metadata.len();
                        let size_str = if size_bytes > 1_000_000 {
                            format!("{:.1}MB", size_bytes as f64 / 1_000_000.0)
                        } else if size_bytes > 1_000 {
                            format!("{:.1}KB", size_bytes as f64 / 1_000.0)
                        } else {
                            format!("{}B", size_bytes)
                        };
                        
                        files.push(json!({
                            "name": filename,
                            "path": format!("/api/loadtest/file/{}", filename),
                            "size": size_str,
                            "bytes": size_bytes,
                        }));
                    }
                }
            }
        }
    }
    
    // Sort by modification time (newest first)
    files.sort_by(|a, b| {
        let a_name = a["name"].as_str().unwrap_or("");
        let b_name = b["name"].as_str().unwrap_or("");
        // Extract timestamp portion (last 15 chars before .jsonl: YYYYMMDD_HHMMSS)
        let a_ts = if a_name.len() > 21 { &a_name[a_name.len()-21..a_name.len()-6] } else { a_name };
        let b_ts = if b_name.len() > 21 { &b_name[b_name.len()-21..b_name.len()-6] } else { b_name };
        b_ts.cmp(a_ts)
    });
    
    Json(files)
}

/// Serve a specific load test request log file
pub async fn loadtest_file_handler(
    Path(filename): Path<String>,
) -> impl IntoResponse {
    use std::fs;
    
    // Security: only allow JSONL files (not runs.jsonl which is summary data)
    if !filename.ends_with(".jsonl") || filename == "runs.jsonl" {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid filename".to_string(),
        ).into_response();
    }
    
    // Prevent path traversal
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return (
            StatusCode::BAD_REQUEST,
            "Invalid filename".to_string(),
        ).into_response();
    }
    
    let results_dir = std::env::var("LOAD_TEST_RESULTS_DIR")
        .unwrap_or_else(|_| "./validation/load-test/results".to_string());
    
    let file_path = format!("{}/{}", results_dir, filename);
    
    match fs::read_to_string(&file_path) {
        Ok(content) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/x-ndjson")],
            content,
        ).into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            "File not found".to_string(),
        ).into_response(),
    }
}

/// Criterion microbenchmark results API endpoint
/// Reads benchmark data from target/criterion directory
pub async fn criterion_benchmarks_handler() -> impl IntoResponse {
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    
    #[allow(dead_code)]
    #[derive(Serialize)]
    struct CriterionEstimate {
        point_estimate: f64,
        lower_bound: f64,
        upper_bound: f64,
        unit: String,
    }
    
    #[derive(Serialize)]
    struct CriterionBenchmark {
        group_id: String,
        function_id: String,
        full_id: String,
        mean_ns: f64,
        mean_ms: f64,
        median_ns: f64,
        median_ms: f64,
        std_dev_ns: f64,
        throughput: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        change_pct: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        change_direction: Option<String>,
    }
    
    #[derive(Serialize)]
    struct CriterionGroup {
        name: String,
        benchmarks: Vec<CriterionBenchmark>,
    }
    
    let criterion_dir = std::env::var("CRITERION_DIR")
        .unwrap_or_else(|_| "./target/criterion".to_string());
    
    let criterion_path = PathBuf::from(&criterion_dir);
    
    if !criterion_path.exists() {
        return Json(json!({
            "error": "Criterion results directory not found",
            "path": criterion_dir,
            "groups": []
        }));
    }
    
    let mut groups: Vec<CriterionGroup> = Vec::new();
    
    // Walk through criterion directory to find benchmark results
    if let Ok(entries) = fs::read_dir(&criterion_path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            
            let group_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            
            // Skip the 'report' directory
            if group_name == "report" {
                continue;
            }
            
            let mut benchmarks: Vec<CriterionBenchmark> = Vec::new();
            
            // Look for benchmark subdirectories or direct estimates
            fn collect_benchmarks(
                dir: &PathBuf, 
                group_id: &str, 
                prefix: &str,
                benchmarks: &mut Vec<CriterionBenchmark>
            ) {
                if let Ok(sub_entries) = fs::read_dir(dir) {
                    for sub_entry in sub_entries.filter_map(|e| e.ok()) {
                        let sub_path = sub_entry.path();
                        if !sub_path.is_dir() {
                            continue;
                        }
                        
                        let sub_name = sub_path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown");
                        
                        // Skip special directories
                        if sub_name == "report" || sub_name == "base" || sub_name == "change" {
                            continue;
                        }
                        
                        // Check if this directory has a 'new' subdirectory with estimates
                        let new_estimates = sub_path.join("new").join("estimates.json");
                        let benchmark_json = sub_path.join("new").join("benchmark.json");
                        
                        if new_estimates.exists() {
                            // Parse estimates
                            if let Ok(est_content) = fs::read_to_string(&new_estimates) {
                                if let Ok(estimates) = serde_json::from_str::<serde_json::Value>(&est_content) {
                                    let mean_ns = estimates["mean"]["point_estimate"].as_f64().unwrap_or(0.0);
                                    let median_ns = estimates["median"]["point_estimate"].as_f64().unwrap_or(0.0);
                                    let std_dev_ns = estimates["std_dev"]["point_estimate"].as_f64().unwrap_or(0.0);
                                    
                                    // Try to get throughput from benchmark.json
                                    let throughput = if benchmark_json.exists() {
                                        fs::read_to_string(&benchmark_json).ok()
                                            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                                            .and_then(|b| b["throughput"]["Elements"].as_u64())
                                    } else {
                                        None
                                    };
                                    
                                    // Check for change data (comparison with baseline)
                                    let change_path = sub_path.join("change").join("estimates.json");
                                    let (change_pct, change_direction) = if change_path.exists() {
                                        fs::read_to_string(&change_path).ok()
                                            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                                            .map(|change| {
                                                let pct = change["mean"]["point_estimate"].as_f64().unwrap_or(0.0) * 100.0;
                                                let direction = if pct < -5.0 {
                                                    "improved".to_string()
                                                } else if pct > 5.0 {
                                                    "regressed".to_string()
                                                } else {
                                                    "unchanged".to_string()
                                                };
                                                (Some(pct), Some(direction))
                                            })
                                            .unwrap_or((None, None))
                                    } else {
                                        (None, None)
                                    };
                                    
                                    let function_id = if prefix.is_empty() {
                                        sub_name.to_string()
                                    } else {
                                        format!("{}/{}", prefix, sub_name)
                                    };
                                    
                                    benchmarks.push(CriterionBenchmark {
                                        group_id: group_id.to_string(),
                                        function_id: function_id.clone(),
                                        full_id: format!("{}/{}", group_id, function_id),
                                        mean_ns,
                                        mean_ms: mean_ns / 1_000_000.0,
                                        median_ns,
                                        median_ms: median_ns / 1_000_000.0,
                                        std_dev_ns,
                                        throughput,
                                        change_pct,
                                        change_direction,
                                    });
                                }
                            }
                        } else {
                            // Recurse into subdirectory
                            let new_prefix = if prefix.is_empty() {
                                sub_name.to_string()
                            } else {
                                format!("{}/{}", prefix, sub_name)
                            };
                            collect_benchmarks(&sub_path, group_id, &new_prefix, benchmarks);
                        }
                    }
                }
            }
            
            collect_benchmarks(&path, &group_name, "", &mut benchmarks);
            
            if !benchmarks.is_empty() {
                // Sort benchmarks by full_id for consistent ordering
                benchmarks.sort_by(|a, b| a.full_id.cmp(&b.full_id));
                
                groups.push(CriterionGroup {
                    name: group_name,
                    benchmarks,
                });
            }
        }
    }
    
    // Sort groups by name
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    
    // Count total benchmarks
    let total_benchmarks: usize = groups.iter().map(|g| g.benchmarks.len()).sum();
    
    Json(json!({
        "criterion_dir": criterion_dir,
        "total_groups": groups.len(),
        "total_benchmarks": total_benchmarks,
        "groups": groups
    }))
}

/// Benchmark results API endpoint - serves runs.jsonl with git metadata
/// Used by web/benchmarks.html to compare benchmark results by commit
pub async fn benchmarks_handler() -> impl IntoResponse {
    use serde_json::json;
    use std::fs;
    
    // Read JSONL file from validation/load-test/results/
    let results_dir = std::env::var("LOAD_TEST_RESULTS_DIR")
        .unwrap_or_else(|_| "./validation/load-test/results".to_string());
    
    let jsonl_path = format!("{}/runs.jsonl", results_dir);
    
    info!("Loading benchmark results from: {}", jsonl_path);
    
    /// Git repository information captured at test time
    #[derive(Serialize, Deserialize, Clone)]
    struct GitInfo {
        commit_hash: String,
        commit_short: String,
        branch: String,
        commit_message: String,
        commit_author: String,
        commit_date: String,
        is_dirty: bool,
    }
    
    /// System configuration captured at test time
    #[derive(Serialize, Deserialize, Clone)]
    struct SystemConfigInfo {
        l1_cache_enabled: bool,
        l1_cache_size: usize,
        l1_cache_ttl_secs: u64,
        #[serde(default)]
        l2_cache_enabled: bool,
        grib_cache_enabled: bool,
        grib_cache_size: usize,
        #[serde(default)]
        grid_cache_enabled: bool,
        #[serde(default)]
        grid_cache_size: usize,
        prefetch_enabled: bool,
        prefetch_rings: u32,
        prefetch_min_zoom: u32,
        prefetch_max_zoom: u32,
        cache_warming_enabled: bool,
    }
    
    #[derive(Serialize, Deserialize, Clone)]
    struct BenchmarkRun {
        timestamp: String,
        scenario_name: String,
        #[serde(default)]
        config_name: String,
        duration_secs: f64,
        total_requests: u64,
        successful_requests: u64,
        failed_requests: u64,
        requests_per_second: f64,
        latency_p50: f64,
        #[serde(default)]
        latency_p75: f64,
        latency_p90: f64,
        latency_p95: f64,
        latency_p99: f64,
        latency_min: f64,
        latency_max: f64,
        latency_avg: f64,
        cache_hit_rate: f64,
        bytes_per_second: f64,
        #[serde(default)]
        tiles_per_second: f64,
        #[serde(default)]
        layers: Vec<String>,
        #[serde(default)]
        concurrency: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        system_config: Option<SystemConfigInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        git_info: Option<GitInfo>,
    }
    
    let mut runs = Vec::new();
    
    if let Ok(content) = fs::read_to_string(&jsonl_path) {
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            
            // Parse each line as JSON
            if let Ok(run) = serde_json::from_str::<BenchmarkRun>(line) {
                runs.push(run);
            } else {
                debug!("Failed to parse benchmark line: {}", line);
            }
        }
    } else {
        info!("No runs.jsonl file found at {}, returning empty results", jsonl_path);
    }
    
    // Sort by timestamp descending (newest first)
    runs.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    
    Json(json!({
        "count": runs.len(),
        "runs": runs,
    }))
}

/// Load test dashboard HTML page with comparison features
pub async fn loadtest_dashboard_handler() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Load Test Dashboard - Weather WMS</title>
    <script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            padding: 20px;
        }
        .container { max-width: 1600px; margin: 0 auto; }
        header {
            background: white;
            border-radius: 12px;
            padding: 24px;
            margin-bottom: 20px;
            box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
            display: flex;
            justify-content: space-between;
            align-items: center;
        }
        h1 { color: #2d3748; font-size: 28px; font-weight: 700; }
        .header-actions { display: flex; gap: 12px; align-items: center; }
        .back-link, .btn {
            color: #667eea;
            text-decoration: none;
            font-weight: 600;
            padding: 8px 16px;
            border-radius: 6px;
            transition: background 0.2s;
            border: none;
            cursor: pointer;
            font-size: 14px;
        }
        .back-link:hover, .btn:hover { background: #edf2f7; }
        .btn-primary { background: #667eea; color: white; }
        .btn-primary:hover { background: #5a67d8; }
        .summary-cards {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 16px;
            margin-bottom: 24px;
        }
        .card {
            background: white;
            border-radius: 12px;
            padding: 20px;
            box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
        }
        .card h3 {
            color: #718096;
            font-size: 12px;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            margin-bottom: 8px;
        }
        .stat { color: #2d3748; font-size: 28px; font-weight: 700; }
        .stat-label { color: #a0aec0; font-size: 11px; margin-top: 4px; }
        
        /* Comparison Section */
        .comparison-section {
            background: white;
            border-radius: 12px;
            padding: 24px;
            margin-bottom: 24px;
            box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
        }
        .comparison-section h2 {
            color: #2d3748;
            font-size: 18px;
            font-weight: 700;
            margin-bottom: 16px;
        }
        .comparison-controls {
            display: flex;
            gap: 16px;
            align-items: flex-end;
            flex-wrap: wrap;
            margin-bottom: 20px;
        }
        .control-group { flex: 1; min-width: 250px; }
        .control-group label {
            display: block;
            color: #4a5568;
            font-size: 13px;
            font-weight: 600;
            margin-bottom: 6px;
        }
        .control-group select {
            width: 100%;
            padding: 10px 12px;
            border: 1px solid #e2e8f0;
            border-radius: 6px;
            font-size: 14px;
            color: #2d3748;
            background: white;
        }
        .comparison-results { display: none; }
        .comparison-results.visible { display: block; }
        .comparison-grid {
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 20px;
            margin-bottom: 24px;
        }
        @media (max-width: 900px) { .comparison-grid { grid-template-columns: 1fr; } }
        .result-card {
            background: #f7fafc;
            border-radius: 8px;
            padding: 16px;
            border: 2px solid #e2e8f0;
        }
        .result-card.baseline { border-color: #4299e1; }
        .result-card.comparison { border-color: #48bb78; }
        .result-card-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 12px;
            padding-bottom: 12px;
            border-bottom: 1px solid #e2e8f0;
        }
        .result-card-title { font-weight: 700; color: #2d3748; }
        .result-badge {
            padding: 4px 10px;
            border-radius: 12px;
            font-size: 11px;
            font-weight: 600;
            text-transform: uppercase;
        }
        .result-badge.baseline { background: #bee3f8; color: #2b6cb0; }
        .result-badge.comparison { background: #c6f6d5; color: #276749; }
        .git-info {
            background: #edf2f7;
            border-radius: 6px;
            padding: 10px;
            margin-bottom: 12px;
            font-size: 12px;
        }
        .git-info .commit { font-family: monospace; color: #d69e2e; font-weight: 600; }
        .git-info .branch { color: #4299e1; }
        .git-info .dirty { color: #e53e3e; font-weight: 600; }
        .git-info .message { color: #718096; font-style: italic; margin-top: 4px; }
        .metrics-grid {
            display: grid;
            grid-template-columns: repeat(2, 1fr);
            gap: 8px;
        }
        .metric-item {
            background: white;
            padding: 10px;
            border-radius: 6px;
            border: 1px solid #e2e8f0;
        }
        .metric-label {
            font-size: 10px;
            color: #718096;
            text-transform: uppercase;
            margin-bottom: 4px;
        }
        .metric-value { font-size: 18px; font-weight: 700; color: #2d3748; }
        .metric-value.better { color: #48bb78; }
        .metric-value.worse { color: #e53e3e; }
        .metric-diff { font-size: 12px; margin-left: 6px; }
        .metric-diff.positive { color: #48bb78; }
        .metric-diff.negative { color: #e53e3e; }
        .chart-container {
            background: #f7fafc;
            border-radius: 8px;
            padding: 16px;
            margin-top: 20px;
        }
        .chart-container h3 { color: #4a5568; font-size: 14px; margin-bottom: 12px; }
        .chart-wrapper { height: 250px; }
        
        /* Table Section */
        .runs-table {
            background: white;
            border-radius: 12px;
            padding: 24px;
            box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1);
            overflow-x: auto;
        }
        .runs-table h2 {
            color: #2d3748;
            font-size: 18px;
            font-weight: 700;
            margin-bottom: 16px;
        }
        .filter-row {
            display: flex;
            gap: 12px;
            margin-bottom: 16px;
            flex-wrap: wrap;
        }
        .filter-row select {
            padding: 8px 12px;
            border: 1px solid #e2e8f0;
            border-radius: 6px;
            font-size: 13px;
            color: #2d3748;
        }
        table { width: 100%; border-collapse: collapse; }
        thead { background: #f7fafc; }
        th {
            text-align: left;
            padding: 10px 12px;
            color: #4a5568;
            font-weight: 600;
            font-size: 12px;
            border-bottom: 2px solid #e2e8f0;
        }
        td {
            padding: 10px 12px;
            color: #2d3748;
            border-bottom: 1px solid #e2e8f0;
            font-size: 13px;
        }
        tbody tr:hover { background: #f7fafc; }
        tbody tr.selected { background: #ebf8ff; }
        .metric { font-weight: 600; }
        .excellent { color: #48bb78; }
        .good { color: #4299e1; }
        .ok { color: #ed8936; }
        .slow { color: #f56565; }
        .badge {
            display: inline-block;
            padding: 2px 6px;
            border-radius: 4px;
            font-size: 10px;
            font-weight: 600;
            background: #e2e8f0;
            color: #4a5568;
            margin-right: 4px;
        }
        .badge.enabled { background: #48bb78; color: white; }
        .badge.disabled { background: #cbd5e0; color: #718096; }
        .commit-badge {
            font-family: monospace;
            font-size: 11px;
            background: #fef3c7;
            color: #92400e;
            padding: 2px 6px;
            border-radius: 4px;
        }
        .empty-state {
            text-align: center;
            padding: 60px 20px;
            color: #718096;
        }
        .empty-state h3 { font-size: 18px; margin-bottom: 8px; color: #4a5568; }
        .loading { text-align: center; padding: 40px; color: #718096; }
        @keyframes spin { to { transform: rotate(360deg); } }
        .spinner {
            display: inline-block;
            width: 24px;
            height: 24px;
            border: 3px solid rgba(0, 0, 0, 0.1);
            border-radius: 50%;
            border-top-color: #667eea;
            animation: spin 1s linear infinite;
        }
        .select-cell { width: 30px; }
        .select-cell input { cursor: pointer; }
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>Load Test Dashboard</h1>
            <div class="header-actions">
                <button class="btn" onclick="loadResults()">Refresh</button>
                <a href="/" class="back-link">Back to Map</a>
            </div>
        </header>
        
        <div class="summary-cards">
            <div class="card">
                <h3>Total Test Runs</h3>
                <div class="stat" id="total-runs">--</div>
                <div class="stat-label">Historical tests</div>
            </div>
            <div class="card">
                <h3>Best Requests/sec</h3>
                <div class="stat" id="best-rps">--</div>
                <div class="stat-label">Peak performance</div>
            </div>
            <div class="card">
                <h3>Best p99 Latency</h3>
                <div class="stat" id="best-p99">--</div>
                <div class="stat-label">Milliseconds</div>
            </div>
            <div class="card">
                <h3>Avg Cache Hit Rate</h3>
                <div class="stat" id="avg-cache">--</div>
                <div class="stat-label">Percentage</div>
            </div>
        </div>
        
        <!-- Comparison Section -->
        <div class="comparison-section">
            <h2>Compare Test Runs</h2>
            <div class="comparison-controls">
                <div class="control-group">
                    <label>Baseline Run</label>
                    <select id="baseline-select">
                        <option value="">Select baseline...</option>
                    </select>
                </div>
                <div class="control-group">
                    <label>Comparison Run</label>
                    <select id="comparison-select">
                        <option value="">Select run to compare...</option>
                    </select>
                </div>
                <button class="btn btn-primary" onclick="compareRuns()">Compare</button>
            </div>
            
            <div id="comparison-results" class="comparison-results">
                <div class="comparison-grid">
                    <div class="result-card baseline">
                        <div class="result-card-header">
                            <span class="result-card-title">Baseline</span>
                            <span class="result-badge baseline">Baseline</span>
                        </div>
                        <div id="baseline-content"></div>
                    </div>
                    <div class="result-card comparison">
                        <div class="result-card-header">
                            <span class="result-card-title">Comparison</span>
                            <span class="result-badge comparison">Comparison</span>
                        </div>
                        <div id="comparison-content"></div>
                    </div>
                </div>
                
                <div class="chart-container">
                    <h3>Latency Comparison (ms)</h3>
                    <div class="chart-wrapper">
                        <canvas id="latency-chart"></canvas>
                    </div>
                </div>
            </div>
        </div>
        
        <div class="runs-table">
            <h2>Test Run History</h2>
            <div class="filter-row">
                <select id="scenario-filter">
                    <option value="">All Scenarios</option>
                </select>
                <select id="branch-filter">
                    <option value="">All Branches</option>
                </select>
            </div>
            <div id="loading" class="loading">
                <div class="spinner"></div>
                <p style="margin-top: 16px;">Loading test results...</p>
            </div>
            <div id="table-container" style="display: none;">
                <table>
                    <thead>
                        <tr>
                            <th>Timestamp</th>
                            <th>Scenario</th>
                            <th>Commit</th>
                            <th>Concurrency</th>
                            <th>Req/sec</th>
                            <th>p50</th>
                            <th>p99</th>
                            <th>Cache Hit</th>
                            <th>Config</th>
                        </tr>
                    </thead>
                    <tbody id="runs-tbody"></tbody>
                </table>
            </div>
            <div id="empty-state" class="empty-state" style="display: none;">
                <h3>No test results found</h3>
                <p>Run a load test to see results here.</p>
                <p style="margin-top: 16px; font-family: monospace; font-size: 14px;">
                    ./scripts/run_load_test.sh
                </p>
            </div>
        </div>
    </div>
    
    <script>
        let allRuns = [];
        let latencyChart = null;
        
        async function loadResults() {
            try {
                const response = await fetch('/api/loadtest/results');
                const data = await response.json();
                
                document.getElementById('loading').style.display = 'none';
                allRuns = data.runs || [];
                
                if (allRuns.length === 0) {
                    document.getElementById('empty-state').style.display = 'block';
                    document.getElementById('total-runs').textContent = '0';
                    return;
                }
                
                document.getElementById('table-container').style.display = 'block';
                document.getElementById('total-runs').textContent = data.count;
                
                const bestRps = Math.max(...allRuns.map(r => r.requests_per_second));
                const bestP99 = Math.min(...allRuns.map(r => r.latency_p99));
                const avgCache = allRuns.reduce((sum, r) => sum + r.cache_hit_rate, 0) / allRuns.length;
                
                document.getElementById('best-rps').textContent = bestRps.toFixed(1);
                document.getElementById('best-p99').textContent = bestP99.toFixed(2) + 'ms';
                document.getElementById('avg-cache').textContent = avgCache.toFixed(1) + '%';
                
                populateSelects();
                populateFilters();
                renderTable(allRuns);
                
            } catch (error) {
                console.error('Failed to load results:', error);
                document.getElementById('loading').innerHTML = '<p style="color: #f56565;">Failed to load results</p>';
            }
        }
        
        function populateSelects() {
            const options = allRuns.map((run, idx) => {
                const date = new Date(run.timestamp).toLocaleString();
                const commit = run.git_info?.commit_short || 'unknown';
                const dirty = run.git_info?.is_dirty ? ' *' : '';
                return `<option value="${idx}">${run.scenario_name} - ${commit}${dirty} - ${date}</option>`;
            }).join('');
            
            document.getElementById('baseline-select').innerHTML = '<option value="">Select baseline...</option>' + options;
            document.getElementById('comparison-select').innerHTML = '<option value="">Select run to compare...</option>' + options;
        }
        
        function populateFilters() {
            const scenarios = [...new Set(allRuns.map(r => r.scenario_name))];
            const branches = [...new Set(allRuns.map(r => r.git_info?.branch).filter(Boolean))];
            
            document.getElementById('scenario-filter').innerHTML = '<option value="">All Scenarios</option>' +
                scenarios.map(s => `<option value="${s}">${s}</option>`).join('');
            
            document.getElementById('branch-filter').innerHTML = '<option value="">All Branches</option>' +
                branches.map(b => `<option value="${b}">${b}</option>`).join('');
        }
        
        document.getElementById('scenario-filter').addEventListener('change', filterRuns);
        document.getElementById('branch-filter').addEventListener('change', filterRuns);
        
        function filterRuns() {
            const scenario = document.getElementById('scenario-filter').value;
            const branch = document.getElementById('branch-filter').value;
            
            let filtered = allRuns;
            if (scenario) filtered = filtered.filter(r => r.scenario_name === scenario);
            if (branch) filtered = filtered.filter(r => r.git_info?.branch === branch);
            
            renderTable(filtered);
        }
        
        function renderTable(runs) {
            const tbody = document.getElementById('runs-tbody');
            tbody.innerHTML = runs.slice().reverse().map(run => {
                const date = new Date(run.timestamp).toLocaleString();
                const commit = run.git_info?.commit_short || '-';
                const dirty = run.git_info?.is_dirty ? ' <span style="color:#e53e3e">*</span>' : '';
                
                let optBadges = '';
                if (run.system_config) {
                    const sc = run.system_config;
                    if (sc.l1_cache_enabled) optBadges += '<span class="badge enabled">L1</span>';
                    if (sc.l2_cache_enabled) optBadges += '<span class="badge enabled">L2</span>';
                    if (sc.grib_cache_enabled) optBadges += '<span class="badge enabled">GRIB</span>';
                    if (sc.prefetch_enabled) optBadges += '<span class="badge enabled">PF(' + (sc.prefetch_rings || '?') + ')</span>';
                    if (sc.cache_warming_enabled) optBadges += '<span class="badge enabled">WARM</span>';
                } else {
                    optBadges = '<span class="badge">?</span>';
                }
                
                return `<tr>
                    <td>${date}</td>
                    <td>${run.scenario_name}</td>
                    <td><span class="commit-badge">${commit}</span>${dirty}</td>
                    <td>${run.concurrency || '-'}</td>
                    <td class="metric ${getRpsClass(run.requests_per_second)}">${run.requests_per_second.toFixed(1)}</td>
                    <td class="metric">${run.latency_p50.toFixed(2)}</td>
                    <td class="metric ${getLatencyClass(run.latency_p99)}">${run.latency_p99.toFixed(2)}</td>
                    <td class="metric ${getCacheClass(run.cache_hit_rate)}">${run.cache_hit_rate.toFixed(1)}%</td>
                    <td>${optBadges}</td>
                </tr>`;
            }).join('');
        }
        
        function compareRuns() {
            const baselineIdx = document.getElementById('baseline-select').value;
            const comparisonIdx = document.getElementById('comparison-select').value;
            
            if (baselineIdx === '' || comparisonIdx === '') {
                alert('Please select both baseline and comparison runs');
                return;
            }
            
            const baseline = allRuns[parseInt(baselineIdx)];
            const comparison = allRuns[parseInt(comparisonIdx)];
            
            document.getElementById('baseline-content').innerHTML = renderRunDetails(baseline);
            document.getElementById('comparison-content').innerHTML = renderRunDetails(comparison, baseline);
            
            renderLatencyChart(baseline, comparison);
            document.getElementById('comparison-results').classList.add('visible');
        }
        
        function renderRunDetails(run, baseline = null) {
            const gitHtml = run.git_info ? `
                <div class="git-info">
                    <span class="commit">${run.git_info.commit_short}</span>
                    on <span class="branch">${run.git_info.branch}</span>
                    ${run.git_info.is_dirty ? '<span class="dirty">(dirty)</span>' : ''}
                    <div class="message">"${run.git_info.commit_message || ''}"</div>
                </div>
            ` : '';
            
            return `
                ${gitHtml}
                <div class="metrics-grid">
                    ${renderMetric('Req/sec', run.requests_per_second, baseline?.requests_per_second, true)}
                    ${renderMetric('Total Req', run.total_requests, baseline?.total_requests)}
                    ${renderMetric('p50', run.latency_p50, baseline?.latency_p50, false, 'ms')}
                    ${renderMetric('p99', run.latency_p99, baseline?.latency_p99, false, 'ms')}
                    ${renderMetric('Cache Hit', run.cache_hit_rate, baseline?.cache_hit_rate, true, '%')}
                    ${renderMetric('Duration', run.duration_secs, null, false, 's')}
                </div>
            `;
        }
        
        function renderMetric(label, value, baselineValue, higherIsBetter = true, unit = '') {
            let diffHtml = '';
            let valueClass = '';
            
            if (baselineValue !== null && baselineValue !== undefined) {
                const diff = value - baselineValue;
                const pctDiff = baselineValue !== 0 ? (diff / baselineValue) * 100 : 0;
                const isPositive = higherIsBetter ? diff > 0 : diff < 0;
                
                if (Math.abs(pctDiff) > 1) {
                    diffHtml = `<span class="metric-diff ${isPositive ? 'positive' : 'negative'}">
                        ${diff > 0 ? '+' : ''}${pctDiff.toFixed(1)}%
                    </span>`;
                    valueClass = isPositive ? 'better' : 'worse';
                }
            }
            
            const displayValue = typeof value === 'number' ? 
                (value > 1000 ? value.toLocaleString() : value.toFixed(2)) : value;
            
            return `
                <div class="metric-item">
                    <div class="metric-label">${label}</div>
                    <div class="metric-value ${valueClass}">${displayValue}${unit}${diffHtml}</div>
                </div>
            `;
        }
        
        function renderLatencyChart(baseline, comparison) {
            const ctx = document.getElementById('latency-chart').getContext('2d');
            
            if (latencyChart) latencyChart.destroy();
            
            latencyChart = new Chart(ctx, {
                type: 'bar',
                data: {
                    labels: ['p50', 'p75', 'p90', 'p95', 'p99'],
                    datasets: [
                        {
                            label: 'Baseline',
                            data: [baseline.latency_p50, baseline.latency_p75 || 0, baseline.latency_p90, 
                                   baseline.latency_p95, baseline.latency_p99],
                            backgroundColor: 'rgba(66, 153, 225, 0.7)',
                            borderColor: '#4299e1',
                            borderWidth: 1
                        },
                        {
                            label: 'Comparison',
                            data: [comparison.latency_p50, comparison.latency_p75 || 0, comparison.latency_p90,
                                   comparison.latency_p95, comparison.latency_p99],
                            backgroundColor: 'rgba(72, 187, 120, 0.7)',
                            borderColor: '#48bb78',
                            borderWidth: 1
                        }
                    ]
                },
                options: {
                    responsive: true,
                    maintainAspectRatio: false,
                    plugins: { legend: { position: 'top' } },
                    scales: {
                        y: {
                            beginAtZero: true,
                            title: { display: true, text: 'Latency (ms)' }
                        }
                    }
                }
            });
        }
        
        function getRpsClass(rps) {
            if (rps >= 10000) return 'excellent';
            if (rps >= 5000) return 'good';
            if (rps >= 1000) return 'ok';
            return 'slow';
        }
        
        function getLatencyClass(ms) {
            if (ms <= 1) return 'excellent';
            if (ms <= 10) return 'good';
            if (ms <= 100) return 'ok';
            return 'slow';
        }
        
        function getCacheClass(percent) {
            if (percent >= 95) return 'excellent';
            if (percent >= 80) return 'good';
            if (percent >= 50) return 'ok';
            return 'slow';
        }
        
        loadResults();
        setInterval(loadResults, 60000);
    </script>
</body>
</html>
"#;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html
    )
}
