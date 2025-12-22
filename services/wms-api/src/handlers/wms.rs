//! WMS (Web Map Service) request handlers.
//!
//! This module handles WMS 1.3.0 protocol requests:
//! - GetCapabilities: Returns service metadata and available layers
//! - GetMap: Renders weather data as map images  
//! - GetFeatureInfo: Returns data values at a specific point

use axum::{
    extract::{Extension, Query},
    http::{header, StatusCode},
    response::Response,
};
use serde::Deserialize;
use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, instrument, error};

use crate::state::AppState;
use crate::layer_config::LayerConfigRegistry;
use crate::model_config::ModelDimensionRegistry;
use storage::ParameterAvailability;
use super::common::{
    wms_exception, mercator_to_wgs84, DimensionParams,
    get_styles_xml_from_file,
};

// ============================================================================
// WMS Error Types (OGC Exception Codes)
// ============================================================================

/// Supported CRS codes for WMS requests
const SUPPORTED_CRS: &[&str] = &["EPSG:4326", "EPSG:3857", "CRS:84"];

/// Supported output formats for GetMap
const SUPPORTED_FORMATS: &[&str] = &["image/png", "image/jpeg", "image/gif"];

/// WMS rendering errors with OGC-compliant exception codes
#[derive(Debug)]
pub enum WmsError {
    /// Layer name format is invalid (LayerNotDefined)
    LayerNotDefined(String),
    /// Style does not exist for the layer (StyleNotDefined)
    StyleNotDefined(String),
    /// CRS is not supported (InvalidCRS)
    InvalidCRS(String),
    /// Format is not supported (InvalidFormat)
    InvalidFormat(String),
    /// BBOX is invalid (InvalidParameterValue)
    InvalidBBox(String),
    /// No data available for the requested layer/dimension combination (MissingDimensionValue)
    MissingData(String),
    /// Internal rendering error (NoApplicableCode)
    RenderingError(String),
}

impl WmsError {
    /// Get the OGC exception code for this error
    pub fn code(&self) -> &'static str {
        match self {
            WmsError::LayerNotDefined(_) => "LayerNotDefined",
            WmsError::StyleNotDefined(_) => "StyleNotDefined",
            WmsError::InvalidCRS(_) => "InvalidCRS",
            WmsError::InvalidFormat(_) => "InvalidFormat",
            WmsError::InvalidBBox(_) => "InvalidParameterValue",
            WmsError::MissingData(_) => "MissingDimensionValue",
            WmsError::RenderingError(_) => "NoApplicableCode",
        }
    }

    /// Get the error message
    pub fn message(&self) -> String {
        match self {
            WmsError::LayerNotDefined(msg) => msg.clone(),
            WmsError::StyleNotDefined(msg) => msg.clone(),
            WmsError::InvalidCRS(msg) => msg.clone(),
            WmsError::InvalidFormat(msg) => msg.clone(),
            WmsError::InvalidBBox(msg) => msg.clone(),
            WmsError::MissingData(msg) => msg.clone(),
            WmsError::RenderingError(msg) => format!("Rendering failed: {}", msg),
        }
    }

    /// Get the HTTP status code for this error
    pub fn status_code(&self) -> StatusCode {
        match self {
            WmsError::LayerNotDefined(_) => StatusCode::BAD_REQUEST,
            WmsError::StyleNotDefined(_) => StatusCode::BAD_REQUEST,
            WmsError::InvalidCRS(_) => StatusCode::BAD_REQUEST,
            WmsError::InvalidFormat(_) => StatusCode::BAD_REQUEST,
            WmsError::InvalidBBox(_) => StatusCode::BAD_REQUEST,
            WmsError::MissingData(_) => StatusCode::NOT_FOUND,
            WmsError::RenderingError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Convert a rendering error string to the appropriate WmsError type
    /// by detecting patterns in the error message
    pub fn from_rendering_error(err: String) -> Self {
        // Detect style-related errors
        if err.contains("Style '") && err.contains("' not found") {
            return WmsError::StyleNotDefined(err);
        }
        if err.contains("style") && err.contains("not supported") {
            return WmsError::StyleNotDefined(err);
        }
        // Detect layer-related errors
        if err.contains("layer") && (err.contains("not found") || err.contains("not defined")) {
            return WmsError::LayerNotDefined(err);
        }
        // Detect missing data errors (no data available for the requested dimensions)
        if err.contains("No data found") || err.contains("no data available") || err.contains("Data not available") {
            return WmsError::MissingData(err);
        }
        // Default to rendering error
        WmsError::RenderingError(err)
    }
}

/// Validate that the CRS is supported
fn validate_crs(crs: Option<&str>) -> Result<(), WmsError> {
    let crs_str = crs.unwrap_or("EPSG:4326");
    let crs_upper = crs_str.to_uppercase();

    if SUPPORTED_CRS.iter().any(|supported| crs_upper == *supported) {
        Ok(())
    } else {
        Err(WmsError::InvalidCRS(format!(
            "CRS '{}' is not supported. Supported CRS: {}",
            crs_str,
            SUPPORTED_CRS.join(", ")
        )))
    }
}

/// Validate that the output format is supported
fn validate_format(format: Option<&str>) -> Result<(), WmsError> {
    let format_str = format.unwrap_or("image/png");
    let format_lower = format_str.to_lowercase();

    if SUPPORTED_FORMATS.iter().any(|supported| format_lower == *supported) {
        Ok(())
    } else {
        Err(WmsError::InvalidFormat(format!(
            "Format '{}' is not supported. Supported formats: {}",
            format_str,
            SUPPORTED_FORMATS.join(", ")
        )))
    }
}

/// Validate that the BBOX is properly formed
/// For WMS 1.3.0 with EPSG:4326, BBOX is minLat,minLon,maxLat,maxLon
/// For EPSG:3857, BBOX is minX,minY,maxX,maxY
fn validate_bbox(bbox: Option<&str>, crs: Option<&str>) -> Result<(), WmsError> {
    let bbox_str = match bbox {
        Some(b) => b,
        None => return Ok(()), // BBOX is optional in some contexts
    };
    
    let coords: Vec<f64> = bbox_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    
    if coords.len() != 4 {
        return Err(WmsError::InvalidBBox(format!(
            "BBOX must contain exactly 4 comma-separated values, got {}",
            coords.len()
        )));
    }
    
    let crs_str = crs.unwrap_or("EPSG:4326");
    let is_geographic = !crs_str.contains("3857");
    
    // For EPSG:4326 (WMS 1.3.0): BBOX is minLat,minLon,maxLat,maxLon
    // For EPSG:3857: BBOX is minX,minY,maxX,maxY
    let (min_x, min_y, max_x, max_y) = if is_geographic {
        // EPSG:4326: coords are [minLat, minLon, maxLat, maxLon]
        (coords[1], coords[0], coords[3], coords[2]) // Convert to minLon,minLat,maxLon,maxLat
    } else {
        (coords[0], coords[1], coords[2], coords[3])
    };
    
    // Check that min < max for both axes
    if min_x > max_x {
        return Err(WmsError::InvalidBBox(format!(
            "Invalid BBOX: minX ({}) is greater than maxX ({})",
            min_x, max_x
        )));
    }
    
    if min_y > max_y {
        return Err(WmsError::InvalidBBox(format!(
            "Invalid BBOX: minY ({}) is greater than maxY ({})",
            min_y, max_y
        )));
    }
    
    Ok(())
}

// ============================================================================
// WMS Parameters
// ============================================================================

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct WmsParams {
    #[serde(rename = "SERVICE", alias = "service")]
    pub service: Option<String>,
    #[serde(rename = "REQUEST", alias = "request")]
    pub request: Option<String>,
    #[serde(rename = "VERSION", alias = "version")]
    pub version: Option<String>,
    #[serde(rename = "LAYERS", alias = "layers")]
    pub layers: Option<String>,
    #[serde(rename = "STYLES", alias = "styles")]
    pub styles: Option<String>,
    #[serde(rename = "CRS", alias = "SRS", alias = "crs", alias = "srs")]
    pub crs: Option<String>,
    #[serde(rename = "BBOX", alias = "bbox")]
    pub bbox: Option<String>,
    #[serde(rename = "WIDTH", alias = "width")]
    pub width: Option<u32>,
    #[serde(rename = "HEIGHT", alias = "height")]
    pub height: Option<u32>,
    #[serde(rename = "FORMAT", alias = "format")]
    pub format: Option<String>,
    // Dimension parameters:
    // - TIME: For observation layers (GOES, MRMS) - ISO8601 timestamp
    // - RUN: For forecast models (GFS, HRRR) - ISO8601 model run time
    // - FORECAST: For forecast models - forecast hour offset from RUN
    #[serde(rename = "TIME", alias = "time")]
    pub time: Option<String>,
    #[serde(rename = "RUN", alias = "run")]
    pub run: Option<String>,
    #[serde(rename = "FORECAST", alias = "forecast")]
    pub forecast: Option<String>,
    #[serde(rename = "ELEVATION", alias = "elevation")]
    pub elevation: Option<String>,
    #[serde(rename = "TRANSPARENT", alias = "transparent")]
    pub transparent: Option<String>,
    // GetFeatureInfo parameters
    #[serde(rename = "QUERY_LAYERS", alias = "query_layers")]
    pub query_layers: Option<String>,
    #[serde(rename = "INFO_FORMAT", alias = "info_format")]
    pub info_format: Option<String>,
    #[serde(rename = "I", alias = "i", alias = "X", alias = "x")]
    pub i: Option<u32>,
    #[serde(rename = "J", alias = "j", alias = "Y", alias = "y")]
    pub j: Option<u32>,
    #[serde(rename = "FEATURE_COUNT", alias = "feature_count")]
    pub feature_count: Option<u32>,
}

// ============================================================================
// WMS Handler Entry Point
// ============================================================================

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

// ============================================================================
// GetCapabilities
// ============================================================================

async fn wms_get_capabilities(state: Arc<AppState>, params: WmsParams) -> Response {
    let version = params.version.as_deref().unwrap_or("1.3.0");
    
    // Check cache first
    if let Some(cached_xml) = state.capabilities_cache.get_wms().await {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/xml")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(cached_xml.into())
            .unwrap();
    }

    // Build capabilities from layer configs (config-driven approach)
    // Only include layers that have data in the catalog
    let layer_configs = state.layer_configs.read().await;
    
    // Collect availability data for each configured layer
    let mut param_availability: HashMap<String, storage::ParameterAvailability> = HashMap::new();
    
    for model_id in layer_configs.models() {
        if let Some(model_config) = layer_configs.get_model(model_id) {
            for layer in &model_config.layers {
                // Skip composite layers - they're handled separately
                if layer.composite {
                    continue;
                }
                
                // Check if data exists for this layer
                if let Ok(Some(availability)) = state.catalog
                    .get_parameter_availability(model_id, &layer.parameter)
                    .await
                {
                    let key = format!("{}_{}", model_id, layer.parameter);
                    param_availability.insert(key, availability);
                }
            }
        }
    }

    let xml = build_wms_capabilities_xml_v2(
        version,
        &layer_configs,
        &param_availability,
        &state.model_dimensions,
    );

    // Cache the result
    state.capabilities_cache.set_wms(xml.clone()).await;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(xml.into())
        .unwrap()
}

// ============================================================================
// GetMap
// ============================================================================

async fn wms_get_map(state: Arc<AppState>, params: WmsParams) -> Response {
    use crate::metrics::Timer;

    // Record WMS request
    state.metrics.record_wms_request();

    let layers_param = match &params.layers {
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
    let styles_param = params.styles.as_deref().unwrap_or("default");
    let bbox = params.bbox.as_deref();
    let crs = params.crs.as_deref();
    let format = params.format.as_deref();

    // Validate CRS
    if let Err(e) = validate_crs(crs) {
        return wms_exception(e.code(), &e.message(), e.status_code());
    }

    // Validate FORMAT
    if let Err(e) = validate_format(format) {
        return wms_exception(e.code(), &e.message(), e.status_code());
    }

    // Validate BBOX
    if let Err(e) = validate_bbox(bbox, crs) {
        return wms_exception(e.code(), &e.message(), e.status_code());
    }

    // Parse multiple layers and styles
    let layer_names: Vec<&str> = layers_param.split(',').map(|s| s.trim()).collect();
    let style_names: Vec<&str> = styles_param.split(',').map(|s| s.trim()).collect();
    
    // Build dimension parameters from request
    let dimensions = DimensionParams {
        time: params.time.clone(),
        run: params.run.clone(),
        forecast: params.forecast.clone(),
        elevation: params.elevation.clone(),
    };

    info!(layers = %layers_param, styles = %styles_param, num_layers = layer_names.len(),
          width = width, height = height, bbox = ?bbox, crs = ?crs, 
          time = ?dimensions.time, run = ?dimensions.run, forecast = ?dimensions.forecast, 
          elevation = ?dimensions.elevation, "GetMap request");

    // Record bbox for heatmap visualization (parse and convert to WGS84 if needed)
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
                // WMS 1.3.0 with EPSG:4326 uses axis order lat,lon
                [coords[1] as f32, coords[0] as f32, coords[3] as f32, coords[2] as f32]
            };
            state.metrics.record_tile_request_location(&bbox_array, crate::metrics::TileCacheStatus::Miss);
        }
    }

    // Time the rendering
    let timer = Timer::start();

    // Render layers (single or multiple)
    let render_result = if layer_names.len() == 1 {
        // Single layer - use existing function
        let style = style_names.first().copied().unwrap_or("default");
        render_weather_data(&state, layer_names[0], style, width, height, bbox, crs, &dimensions).await
    } else {
        // Multiple layers - render each and composite
        render_multi_layer(&state, &layer_names, &style_names, width, height, bbox, crs, &dimensions).await
    };

    // Try to render actual data, return error on failure
    match render_result {
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
            error!(
                layers = %layers_param,
                styles = %styles_param,
                width = width,
                height = height,
                bbox = ?bbox,
                crs = ?crs,
                time = ?dimensions.time,
                run = ?dimensions.run,
                forecast = ?dimensions.forecast,
                elevation = ?dimensions.elevation,
                error = ?e,
                "WMS GetMap rendering failed"
            );
            wms_exception(
                e.code(),
                &e.message(),
                e.status_code(),
            )
        }
    }
}

// ============================================================================
// GetFeatureInfo
// ============================================================================

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

    // Validate CRS
    if let Err(e) = validate_crs(Some(crs)) {
        return wms_exception(e.code(), &e.message(), e.status_code());
    }

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

    // Validate I parameter is within image bounds (0 to WIDTH-1)
    if i >= width {
        return wms_exception(
            "InvalidPoint",
            &format!("I parameter value {} is out of range. Must be between 0 and {} (WIDTH-1).", i, width - 1),
            StatusCode::BAD_REQUEST,
        );
    }

    // Validate J parameter is within image bounds (0 to HEIGHT-1)
    if j >= height {
        return wms_exception(
            "InvalidPoint",
            &format!("J parameter value {} is out of range. Must be between 0 and {} (HEIGHT-1).", j, height - 1),
            StatusCode::BAD_REQUEST,
        );
    }

    // Parse INFO_FORMAT
    let info_format = match params.info_format.as_deref() {
        Some(fmt) => {
            match InfoFormat::from_mime(fmt) {
                Some(f) => f,
                None => {
                    return wms_exception(
                        "InvalidFormat",
                        &format!("INFO_FORMAT '{}' is not supported. Supported formats: application/json, text/html, text/xml, text/plain", fmt),
                        StatusCode::BAD_REQUEST,
                    );
                }
            }
        }
        None => InfoFormat::Html, // Default to HTML if not specified
    };

    // Parse BBOX
    let bbox_coords: Result<Vec<f64>, _> = bbox
        .split(',')
        .map(|s| s.trim().parse())
        .collect();

    let bbox_array = match bbox_coords {
        Ok(coords) if coords.len() == 4 => {
            if crs.contains("3857") {
                [coords[0], coords[1], coords[2], coords[3]]
            } else {
                // EPSG:4326 - input is [min_lat, min_lon, max_lat, max_lon]
                [coords[1], coords[0], coords[3], coords[2]]
            }
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

    // Parse ELEVATION parameter
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
    
    // Validate all layer names before querying
    // Get list of valid models from catalog
    let valid_models = state.catalog.list_models().await.unwrap_or_default();
    
    for layer in &layers {
        let parts: Vec<&str> = layer.split('_').collect();
        if parts.len() < 2 {
            return wms_exception(
                "LayerNotDefined",
                &format!("Layer '{}' is not defined. Layer names must be in format 'model_parameter'.", layer),
                StatusCode::BAD_REQUEST,
            );
        }
        
        // Check if the model exists in the catalog
        let model = parts[0].to_lowercase();
        if !valid_models.iter().any(|m| m.to_lowercase() == model) {
            return wms_exception(
                "LayerNotDefined",
                &format!("Layer '{}' is not defined. Model '{}' not found.", layer, parts[0]),
                StatusCode::BAD_REQUEST,
            );
        }
    }
    
    let mut all_features = Vec::new();

    for layer in layers {
        // Get effective elevation (use default if not specified)
        let effective_elevation: Option<String> = match &elevation {
            Some(elev) => Some(elev.clone()),
            None => {
                let parts: Vec<&str> = layer.split('_').collect();
                if parts.len() >= 2 {
                    let model = parts[0];
                    let parameter = parts[1..].join("_").to_uppercase();
                    let configs = state.layer_configs.read().await;
                    configs
                        .get_layer_by_param(model, &parameter)
                        .and_then(|l| l.default_level())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            }
        };

        match crate::rendering::query_point_value(
            &state.catalog,
            &state.metrics,
            &state.grid_processor_factory,
            layer,
            bbox_array,
            width,
            height,
            i,
            j,
            crs,
            forecast_hour,
            effective_elevation.as_deref(),
        )
            .await
        {
            Ok(mut features) => {
                all_features.append(&mut features);
            }
            Err(e) => {
                error!(layer = %layer, error = %e, "Failed to query layer");
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
// WMS Rendering
// ============================================================================

async fn render_weather_data(
    state: &Arc<AppState>,
    layer: &str,
    style: &str,
    width: u32,
    height: u32,
    bbox: Option<&str>,
    crs: Option<&str>,
    dimensions: &DimensionParams,
) -> Result<Vec<u8>, WmsError> {
    // Parse layer name (format: "model_parameter" or "model_WIND_BARBS")
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return Err(WmsError::LayerNotDefined(format!(
            "Layer '{}' is not defined.",
            layer
        )));
    }

    let model = parts[0];
    let parameter = parts[1..].join("_").to_uppercase();

    // Parse dimensions based on layer type
    let (forecast_hour, observation_time, _reference_time) = dimensions.parse_for_layer(model, &state.model_dimensions);

    // Get default level if not specified
    let level = match &dimensions.elevation {
        Some(elev) => Some(elev.clone()),
        None => {
            let configs = state.layer_configs.read().await;
            configs
                .get_layer_by_param(model, &parameter)
                .and_then(|l| l.default_level())
                .map(|s| s.to_string())
        }
    };

    // Check if this is a wind barbs composite layer
    if parameter == "WIND_BARBS" {
        let parsed_bbox = bbox.and_then(|b| parse_bbox(b, crs));

        return crate::rendering::render_wind_barbs_layer(
            &state.catalog,
            &state.grid_processor_factory,
            model,
            width,
            height,
            parsed_bbox,
            None,
            forecast_hour,
        )
            .await
            .map_err(WmsError::from_rendering_error);
    }

    // Parse BBOX parameter
    let parsed_bbox = bbox.and_then(|b| parse_bbox(b, crs));

    info!(forecast_hour = ?forecast_hour, observation_time = ?observation_time, level = ?level, bbox = ?parsed_bbox, style = style, "Parsed WMS parameters");

    // Check CRS for projection
    let crs_str = crs.unwrap_or("EPSG:4326");
    let use_mercator = crs_str.contains("3857");

    if style == "isolines" {
        if state.model_dimensions.is_observation(model) {
            return Err(WmsError::StyleNotDefined(format!(
                "Style 'isolines' is not supported for {} layers.",
                model.to_uppercase()
            )));
        }

        let style_file = state.layer_configs.read().await.get_style_file_for_parameter(model, &parameter);

        return crate::rendering::render_isolines_tile_with_level(
            &state.catalog,
            &state.grid_processor_factory,
            model,
            &parameter,
            None,
            width,
            height,
            parsed_bbox.unwrap_or([-180.0, -90.0, 180.0, 90.0]), // TODO don't hide an error behind this default?
            &style_file,
            "isolines",
            forecast_hour,
            level.as_deref(),
            use_mercator,
        )
            .await
            .map_err(WmsError::from_rendering_error);
    }

    if style == "numbers" {
        let style_file = state.layer_configs.read().await.get_style_file_for_parameter(model, &parameter);

        return crate::rendering::render_numbers_tile(
            &state.catalog,
            &state.metrics,
            &state.grid_processor_factory,
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
            .await
            .map_err(WmsError::from_rendering_error);
    }

    // Standard rendering
    let style_file = state.layer_configs.read().await.get_style_file_for_parameter(model, &parameter);

    crate::rendering::render_weather_data_with_lut(
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
        &style_file,
        Some(style),
        use_mercator,
        None,
        None,
        &state.grid_processor_factory,
    )
        .await
        .map_err(WmsError::from_rendering_error)
}

/// Render multiple layers and composite them together
/// Later layers are drawn on top of earlier layers using alpha blending
async fn render_multi_layer(
    state: &Arc<AppState>,
    layer_names: &[&str],
    style_names: &[&str],
    width: u32,
    height: u32,
    bbox: Option<&str>,
    crs: Option<&str>,
    dimensions: &DimensionParams,
) -> Result<Vec<u8>, WmsError> {
    use image::{ImageBuffer, Rgba, RgbaImage};
    use std::io::Cursor;
    
    if layer_names.is_empty() {
        return Err(WmsError::LayerNotDefined("No layers specified".to_string()));
    }
    
    // Create a transparent base image
    let mut composite: RgbaImage = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 0]));
    
    // Render each layer and composite
    for (i, layer_name) in layer_names.iter().enumerate() {
        // Get the style for this layer (use default if not enough styles provided)
        let style = style_names.get(i).copied().unwrap_or("default");
        let style = if style.is_empty() { "default" } else { style };
        
        info!(layer = %layer_name, style = %style, layer_index = i, "Rendering layer for multi-layer composite");
        
        // Render this layer
        match render_weather_data(state, layer_name, style, width, height, bbox, crs, dimensions).await {
            Ok(png_bytes) => {
                // Decode the PNG
                let layer_image = match image::load_from_memory(&png_bytes) {
                    Ok(img) => img.to_rgba8(),
                    Err(e) => {
                        error!(layer = %layer_name, error = %e, "Failed to decode layer PNG");
                        continue; // Skip this layer but continue with others
                    }
                };
                
                // Composite this layer on top using alpha blending
                for (x, y, pixel) in layer_image.enumerate_pixels() {
                    if x < width && y < height {
                        let base = composite.get_pixel(x, y);
                        let blended = alpha_blend(*base, *pixel);
                        composite.put_pixel(x, y, blended);
                    }
                }
            }
            Err(e) => {
                // Log the error but continue with other layers
                error!(layer = %layer_name, error = ?e, "Failed to render layer, skipping");
            }
        }
    }
    
    // Encode the composite image to PNG
    let mut png_bytes: Vec<u8> = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(Cursor::new(&mut png_bytes));
    
    composite.write_with_encoder(encoder)
        .map_err(|e| WmsError::RenderingError(format!("Failed to encode composite PNG: {}", e)))?;
    
    Ok(png_bytes)
}

/// Alpha blend two RGBA pixels (src over dst)
fn alpha_blend(dst: image::Rgba<u8>, src: image::Rgba<u8>) -> image::Rgba<u8> {
    let src_a = src[3] as f32 / 255.0;
    let dst_a = dst[3] as f32 / 255.0;
    
    // If source is fully transparent, return destination
    if src_a == 0.0 {
        return dst;
    }
    
    // If source is fully opaque, return source
    if src_a == 1.0 {
        return src;
    }
    
    // Standard "source over" alpha compositing
    let out_a = src_a + dst_a * (1.0 - src_a);
    
    if out_a == 0.0 {
        return image::Rgba([0, 0, 0, 0]);
    }
    
    let blend_channel = |src_c: u8, dst_c: u8| -> u8 {
        let src_c = src_c as f32 / 255.0;
        let dst_c = dst_c as f32 / 255.0;
        let out_c = (src_c * src_a + dst_c * dst_a * (1.0 - src_a)) / out_a;
        (out_c * 255.0).round() as u8
    };
    
    image::Rgba([
        blend_channel(src[0], dst[0]),
        blend_channel(src[1], dst[1]),
        blend_channel(src[2], dst[2]),
        (out_a * 255.0).round() as u8,
    ])
}

//TODO do we need to parse bbox for any arbitrary CRS?
/// Parse a BBOX string into [min_lon, min_lat, max_lon, max_lat]
fn parse_bbox(bbox_str: &str, crs: Option<&str>) -> Option<[f32; 4]> {
    let coords: Vec<f64> = bbox_str.split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if coords.len() == 4 {
        let crs_str = crs.unwrap_or("EPSG:4326");
        let (min_lon, min_lat, max_lon, max_lat) = if crs_str.contains("3857") {
            let (min_lon, min_lat) = mercator_to_wgs84(coords[0], coords[1]);
            let (max_lon, max_lat) = mercator_to_wgs84(coords[2], coords[3]);
            (min_lon, min_lat, max_lon, max_lat)
        } else {
            // WMS 1.3.0 with EPSG:4326 uses axis order lat,lon
            (coords[1], coords[0], coords[3], coords[2])
        };

        Some([min_lon as f32, min_lat as f32, max_lon as f32, max_lat as f32])
    } else {
        None
    }
}

// ============================================================================
// WMS Capabilities XML Builder (Legacy - kept for reference)
// ============================================================================

/// Legacy capabilities builder - catalog-driven approach.
/// Replaced by build_wms_capabilities_xml_v2 which is config-driven.
#[allow(dead_code)]
fn build_wms_capabilities_xml(
    version: &str,
    models: &[String],
    model_params: &HashMap<String, Vec<String>>,
    model_dimensions: &HashMap<String, (Vec<String>, Vec<i32>)>,
    param_levels: &HashMap<String, Vec<String>>,
    model_bboxes: &HashMap<String, wms_common::BoundingBox>,
    dimension_registry: &ModelDimensionRegistry,
    layer_configs: &LayerConfigRegistry,
) -> String {
    let empty_params = Vec::new();
    let empty_dims = (Vec::new(), Vec::new());
    let empty_levels = Vec::new();

    let layers: String = models
        .iter()
        .map(|model| {
            let params = model_params.get(model).unwrap_or(&empty_params);
            let (runs, forecasts) = model_dimensions.get(model).unwrap_or(&empty_dims);

            // Get bbox for this model
            let bbox = model_bboxes.get(model).cloned().unwrap_or_else(|| {
                wms_common::BoundingBox::new(0.0, -90.0, 360.0, 90.0)
            });

            // Normalize longitude to -180/180 for WMS
            let (west, east) = if bbox.min_x == 0.0 && bbox.max_x == 360.0 {
                (-180.0, 180.0)
            } else {
                let w = if bbox.min_x > 180.0 { bbox.min_x - 360.0 } else { bbox.min_x };
                let e = if bbox.max_x > 180.0 { bbox.max_x - 360.0 } else { bbox.max_x };
                (w, e)
            };
            let south = bbox.min_y;
            let north = bbox.max_y;

            let is_observational = dimension_registry.is_observation(model);

            // Build time dimensions
            let base_dimensions = if is_observational {
                let time_values = if runs.is_empty() { "latest".to_string() } else { runs.join(",") };
                let time_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
                format!(
                    r#"<Dimension name="TIME" units="ISO8601" default="{}">{}</Dimension>"#,
                    time_default, time_values
                )
            } else {
                let run_values = if runs.is_empty() { "latest".to_string() } else { runs.join(",") };
                let run_default = runs.first().map(|s| s.as_str()).unwrap_or("latest");
                let forecast_values = if forecasts.is_empty() {
                    "0".to_string()
                } else {
                    forecasts.iter().map(|h| h.to_string()).collect::<Vec<_>>().join(",")
                };
                let forecast_default = forecasts.first().unwrap_or(&0);

                format!(
                    r#"<Dimension name="RUN" units="ISO8601" default="{}">{}</Dimension><Dimension name="FORECAST" units="hours" default="{}">{}</Dimension>"#,
                    run_default, run_values, forecast_default, forecast_values
                )
            };

            let param_layers = params
                .iter()
                .map(|p| {
                    let key = format!("{}_{}", model, p);
                    let levels = param_levels.get(&key).unwrap_or(&empty_levels);

                    let elevation_dim = if levels.len() > 1 {
                        let mut sorted_levels = levels.clone();
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

                    let all_dimensions = format!("{}{}", base_dimensions, elevation_dim);
                    let style_file = layer_configs.get_style_file_for_parameter(model, p);
                    let styles = get_styles_xml_from_file(&style_file);
                    let model_name = layer_configs.get_model_display_name(model);
                    let param_name = layer_configs.get_parameter_display_name(model, p);

                    format!(
                        r#"<Layer queryable="1"><Name>{}_{}</Name><Title>{} - {}</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="{}" miny="{}" maxx="{}" maxy="{}"/>{}{}</Layer>"#,
                        model, p, model_name, param_name,
                        west, east, south, north,
                        west, south, east, north,
                        styles, all_dimensions
                    )
                })
                .collect::<Vec<_>>()
                .join("");

            // Add wind barbs layer
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

            let model_display = layer_configs.get_model_display_name(model);
            let wind_barbs_layer = if params.contains(&"UGRD".to_string()) && params.contains(&"VGRD".to_string()) {
                format!(
                    r#"<Layer queryable="1"><Name>{}_WIND_BARBS</Name><Title>{} - Wind Barbs</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="{}" miny="{}" maxx="{}" maxy="{}"/><Style><Name>default</Name><Title>Default Barbs</Title></Style>{}{}</Layer>"#,
                    model, model_display,
                    west, east, south, north,
                    west, south, east, north,
                    base_dimensions, wind_elevation_dim
                )
            } else {
                String::new()
            };

            format!(
                r#"<Layer><Name>{}</Name><Title>{}</Title>{}{}</Layer>"#,
                model, model_display, param_layers, wind_barbs_layer
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
        <DCPType><HTTP><Get><OnlineResource xlink:href="http://localhost:8080/wms?"/></Get></HTTP></DCPType>
      </GetCapabilities>
      <GetMap>
        <Format>image/png</Format>
        <DCPType><HTTP><Get><OnlineResource xlink:href="http://localhost:8080/wms?"/></Get></HTTP></DCPType>
      </GetMap>
      <GetFeatureInfo>
        <Format>text/html</Format>
        <Format>application/json</Format>
        <Format>text/xml</Format>
        <Format>text/plain</Format>
        <DCPType><HTTP><Get><OnlineResource xlink:href="http://localhost:8080/wms?"/></Get></HTTP></DCPType>
      </GetFeatureInfo>
    </Request>
    <Exception><Format>XML</Format></Exception>
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

/// Build WMS capabilities XML from layer configs (config-driven approach).
/// Only includes layers that have data available in the catalog.
fn build_wms_capabilities_xml_v2(
    version: &str,
    layer_configs: &LayerConfigRegistry,
    param_availability: &HashMap<String, ParameterAvailability>,
    dimension_registry: &ModelDimensionRegistry,
) -> String {
    let mut model_layers: Vec<String> = Vec::new();

    for model_id in layer_configs.models() {
        let Some(model_config) = layer_configs.get_model(model_id) else {
            continue;
        };

        let is_observational = dimension_registry.is_observation(model_id);
        let mut layer_xml_parts: Vec<String> = Vec::new();

        // Track availability for composite layer validation (e.g., WIND_BARBS)
        let mut ugrd_availability: Option<&ParameterAvailability> = None;
        let mut vgrd_availability: Option<&ParameterAvailability> = None;

        for layer in &model_config.layers {
            // Skip composite layers for now - handle them after regular layers
            if layer.composite {
                continue;
            }

            let key = format!("{}_{}", model_id, layer.parameter);
            let Some(availability) = param_availability.get(&key) else {
                // No data for this layer - skip it
                continue;
            };

            // Track UGRD/VGRD for wind barbs
            if layer.parameter == "UGRD" {
                ugrd_availability = Some(availability);
            } else if layer.parameter == "VGRD" {
                vgrd_availability = Some(availability);
            }

            // Build dimensions for this specific layer
            let dimensions_xml = build_layer_dimensions_xml(
                availability,
                is_observational,
            );

            // Get styles from style file
            let style_path = layer_configs.get_style_path(layer);
            let styles_xml = get_styles_xml_from_file(&style_path);

            // Build bounding box (normalize longitude to -180/180)
            let (west, east, south, north) = normalize_bbox(&availability.bbox);

            let layer_xml = format!(
                r#"<Layer queryable="1"><Name>{}_{}</Name><Title>{} - {}</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="{}" miny="{}" maxx="{}" maxy="{}"/>{}{}</Layer>"#,
                model_id, layer.parameter,
                model_config.display_name, layer.title,
                west, east, south, north,
                west, south, east, north,
                styles_xml, dimensions_xml
            );
            layer_xml_parts.push(layer_xml);
        }

        // Handle WIND_BARBS composite layer
        if let (Some(ugrd), Some(vgrd)) = (ugrd_availability, vgrd_availability) {
            // Find common levels between UGRD and VGRD
            let common_levels: Vec<String> = ugrd.levels.iter()
                .filter(|l| vgrd.levels.contains(l))
                .cloned()
                .collect();

            // Find common times between UGRD and VGRD
            let common_times: Vec<String> = ugrd.times.iter()
                .filter(|t| vgrd.times.contains(t))
                .cloned()
                .collect();

            // Find common forecast hours
            let common_forecast_hours: Vec<i32> = ugrd.forecast_hours.iter()
                .filter(|h| vgrd.forecast_hours.contains(h))
                .copied()
                .collect();

            // Only include WIND_BARBS if there's common data
            if !common_times.is_empty() && (!common_levels.is_empty() || is_observational) {
                let wind_availability = ParameterAvailability {
                    times: common_times,
                    forecast_hours: common_forecast_hours,
                    levels: common_levels,
                    bbox: ugrd.bbox.clone(), // Use UGRD bbox (they should be the same)
                };

                let dimensions_xml = build_layer_dimensions_xml(
                    &wind_availability,
                    is_observational,
                );

                let (west, east, south, north) = normalize_bbox(&ugrd.bbox);

                let wind_layer_xml = format!(
                    r#"<Layer queryable="1"><Name>{}_WIND_BARBS</Name><Title>{} - Wind Barbs</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="{}" miny="{}" maxx="{}" maxy="{}"/><Style><Name>default</Name><Title>Default Barbs</Title></Style>{}</Layer>"#,
                    model_id, model_config.display_name,
                    west, east, south, north,
                    west, south, east, north,
                    dimensions_xml
                );
                layer_xml_parts.push(wind_layer_xml);
            }
        }

        // Only include model if it has at least one layer with data
        if !layer_xml_parts.is_empty() {
            let model_xml = format!(
                r#"<Layer><Name>{}</Name><Title>{}</Title>{}</Layer>"#,
                model_id,
                model_config.display_name,
                layer_xml_parts.join("")
            );
            model_layers.push(model_xml);
        }
    }

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
        <DCPType><HTTP><Get><OnlineResource xlink:href="http://localhost:8080/wms?"/></Get></HTTP></DCPType>
      </GetCapabilities>
      <GetMap>
        <Format>image/png</Format>
        <DCPType><HTTP><Get><OnlineResource xlink:href="http://localhost:8080/wms?"/></Get></HTTP></DCPType>
      </GetMap>
      <GetFeatureInfo>
        <Format>text/html</Format>
        <Format>application/json</Format>
        <Format>text/xml</Format>
        <Format>text/plain</Format>
        <DCPType><HTTP><Get><OnlineResource xlink:href="http://localhost:8080/wms?"/></Get></HTTP></DCPType>
      </GetFeatureInfo>
    </Request>
    <Exception><Format>XML</Format></Exception>
    <Layer>
      <Title>Weather Data</Title>
      <CRS>EPSG:4326</CRS>
      <CRS>EPSG:3857</CRS>
      {}
    </Layer>
  </Capability>
</WMS_Capabilities>"#,
        version, model_layers.join("")
    )
}

/// Build dimension XML for a specific layer based on its actual data availability.
fn build_layer_dimensions_xml(
    availability: &ParameterAvailability,
    is_observational: bool,
) -> String {
    let mut dimensions = String::new();

    // Time/Run dimensions
    if is_observational {
        // Observation models use TIME dimension
        let time_values = if availability.times.is_empty() {
            "latest".to_string()
        } else {
            availability.times.join(",")
        };
        let time_default = availability.times.first().map(|s| s.as_str()).unwrap_or("latest");
        dimensions.push_str(&format!(
            r#"<Dimension name="TIME" units="ISO8601" default="{}">{}</Dimension>"#,
            time_default, time_values
        ));
    } else {
        // Forecast models use RUN + FORECAST dimensions
        let run_values = if availability.times.is_empty() {
            "latest".to_string()
        } else {
            availability.times.join(",")
        };
        let run_default = availability.times.first().map(|s| s.as_str()).unwrap_or("latest");
        
        let forecast_values = if availability.forecast_hours.is_empty() {
            "0".to_string()
        } else {
            availability.forecast_hours.iter().map(|h| h.to_string()).collect::<Vec<_>>().join(",")
        };
        let forecast_default = availability.forecast_hours.first().unwrap_or(&0);

        dimensions.push_str(&format!(
            r#"<Dimension name="RUN" units="ISO8601" default="{}">{}</Dimension><Dimension name="FORECAST" units="hours" default="{}">{}</Dimension>"#,
            run_default, run_values, forecast_default, forecast_values
        ));
    }

    // ELEVATION dimension (only if multiple levels)
    if availability.levels.len() > 1 {
        let mut sorted_levels = availability.levels.clone();
        sorted_levels.sort_by(|a, b| {
            // Sort pressure levels in descending order (1000 mb first)
            let a_val = a.replace(" mb", "").parse::<i32>().unwrap_or(9999);
            let b_val = b.replace(" mb", "").parse::<i32>().unwrap_or(9999);
            b_val.cmp(&a_val)
        });
        let level_values = sorted_levels.join(",");
        let default_level = sorted_levels.first().map(|s| s.as_str()).unwrap_or("");
        dimensions.push_str(&format!(
            r#"<Dimension name="ELEVATION" units="" default="{}">{}</Dimension>"#,
            default_level, level_values
        ));
    }

    dimensions
}

/// Normalize bounding box longitude to -180/180 for WMS.
fn normalize_bbox(bbox: &wms_common::BoundingBox) -> (f64, f64, f64, f64) {
    let (west, east) = if bbox.min_x == 0.0 && bbox.max_x == 360.0 {
        (-180.0, 180.0)
    } else {
        let w = if bbox.min_x > 180.0 { bbox.min_x - 360.0 } else { bbox.min_x };
        let e = if bbox.max_x > 180.0 { bbox.max_x - 360.0 } else { bbox.max_x };
        (w, e)
    };
    (west, east, bbox.min_y, bbox.max_y)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bbox_wgs84() {
        // WMS 1.3.0 EPSG:4326 format: min_lat, min_lon, max_lat, max_lon
        let bbox = parse_bbox("30.0,-120.0,50.0,-80.0", Some("EPSG:4326"));
        assert!(bbox.is_some());
        let b = bbox.unwrap();
        // Should be converted to [min_lon, min_lat, max_lon, max_lat]
        assert!((b[0] - (-120.0)).abs() < 0.01); // min_lon
        assert!((b[1] - 30.0).abs() < 0.01);     // min_lat
        assert!((b[2] - (-80.0)).abs() < 0.01);  // max_lon
        assert!((b[3] - 50.0).abs() < 0.01);     // max_lat
    }

    #[test]
    fn test_parse_bbox_web_mercator() {
        // Web Mercator: minx, miny, maxx, maxy in meters
        let bbox = parse_bbox("-13358338.9,3503549.8,-8766409.9,6446275.8", Some("EPSG:3857"));
        assert!(bbox.is_some());
        let b = bbox.unwrap();
        // Should convert to WGS84 approximately
        assert!(b[0] < -100.0); // min_lon (west coast US)
        assert!(b[1] > 20.0);   // min_lat
        assert!(b[2] > -90.0);  // max_lon (east of west coast)
        assert!(b[3] < 60.0);   // max_lat
    }

    #[test]
    fn test_parse_bbox_invalid() {
        let bbox = parse_bbox("invalid", None);
        assert!(bbox.is_none());

        let bbox = parse_bbox("1,2,3", None);
        assert!(bbox.is_none());
    }

    #[test]
    fn test_wms_params_default() {
        // Test that WmsParams can be deserialized with minimal data
        let json = r#"{"SERVICE": "WMS", "REQUEST": "GetCapabilities"}"#;
        let params: WmsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.service, Some("WMS".to_string()));
        assert_eq!(params.request, Some("GetCapabilities".to_string()));
        assert!(params.layers.is_none());
    }
}
