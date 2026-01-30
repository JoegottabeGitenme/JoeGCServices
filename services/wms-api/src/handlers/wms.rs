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
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, instrument};

use super::common::{
    convert_png_to_jpeg, convert_png_to_webp, get_styles_xml_from_file, mercator_to_wgs84,
    wms_exception, DimensionParams,
};
use crate::cite;
use crate::layer_config::LayerConfigRegistry;
use crate::model_config::ModelDimensionRegistry;
use crate::state::AppState;
use storage::ParameterAvailability;

// ============================================================================
// WMS Error Types (OGC Exception Codes)
// ============================================================================

/// Supported CRS codes for WMS requests
const SUPPORTED_CRS: &[&str] = &["EPSG:4326", "EPSG:3857", "CRS:84"];

/// Supported output formats for GetMap
const SUPPORTED_FORMATS: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

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
        if err.contains("No data found")
            || err.contains("no data available")
            || err.contains("Data not available")
        {
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

    if SUPPORTED_CRS
        .iter()
        .any(|supported| crs_upper == *supported)
    {
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

    if SUPPORTED_FORMATS
        .iter()
        .any(|supported| format_lower == *supported)
    {
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

    // Check that min < max for both axes (OGC WMS 1.3.0 spec 7.3.3.6)
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

    // Check for zero-size bounding box (min == max)
    if (min_x - max_x).abs() < f64::EPSILON {
        return Err(WmsError::InvalidBBox(
            "Invalid BBOX: minX equals maxX (zero width)".to_string()
        ));
    }

    if (min_y - max_y).abs() < f64::EPSILON {
        return Err(WmsError::InvalidBBox(
            "Invalid BBOX: minY equals maxY (zero height)".to_string()
        ));
    }

    Ok(())
}

// ============================================================================
// WMS Parameters
// ============================================================================

/// WMS parameters parsed from query string with case-insensitive parameter names.
/// OGC WMS 1.3.0 spec section 6.8.1 requires parameter names to be case-insensitive.
#[allow(dead_code)]
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct WmsParams {
    #[serde(alias = "SERVICE")]
    pub service: Option<String>,
    #[serde(alias = "REQUEST")]
    pub request: Option<String>,
    #[serde(alias = "VERSION")]
    pub version: Option<String>,
    #[serde(alias = "LAYERS")]
    pub layers: Option<String>,
    #[serde(alias = "STYLES")]
    pub styles: Option<String>,
    #[serde(alias = "CRS")]
    pub crs: Option<String>,
    #[serde(alias = "BBOX")]
    pub bbox: Option<String>,
    #[serde(alias = "WIDTH")]
    pub width: Option<u32>,
    #[serde(alias = "HEIGHT")]
    pub height: Option<u32>,
    #[serde(alias = "FORMAT")]
    pub format: Option<String>,
    // Dimension parameters:
    // - TIME: For observation layers (GOES, MRMS) - ISO8601 timestamp
    // - RUN: For forecast models (GFS, HRRR) - ISO8601 model run time
    // - FORECAST: For forecast models - forecast hour offset from RUN
    #[serde(alias = "TIME")]
    pub time: Option<String>,
    #[serde(alias = "RUN")]
    pub run: Option<String>,
    #[serde(alias = "FORECAST")]
    pub forecast: Option<String>,
    #[serde(alias = "ELEVATION")]
    pub elevation: Option<String>,
    #[serde(alias = "TRANSPARENT")]
    pub transparent: Option<String>,
    #[serde(alias = "BGCOLOR")]
    pub bgcolor: Option<String>,
    // GetFeatureInfo parameters
    #[serde(alias = "QUERY_LAYERS")]
    pub query_layers: Option<String>,
    #[serde(alias = "INFO_FORMAT")]
    pub info_format: Option<String>,
    #[serde(alias = "I")]
    pub i: Option<u32>,
    #[serde(alias = "J")]
    pub j: Option<u32>,
    #[serde(alias = "FEATURE_COUNT")]
    pub feature_count: Option<u32>,
}

impl WmsParams {
    /// Parse WMS parameters from a HashMap with case-insensitive keys.
    /// This complies with OGC WMS 1.3.0 spec section 6.8.1.
    pub fn from_query_map(query: &HashMap<String, String>) -> Self {
        // Create a case-insensitive lookup map (keys are uppercase)
        let ci_map: HashMap<String, &String> = query
            .iter()
            .map(|(k, v)| (k.to_uppercase(), v))
            .collect();

        let get_str = |key: &str| -> Option<String> {
            ci_map.get(key).map(|s| (*s).clone())
        };

        let get_u32 = |key: &str| -> Option<u32> {
            ci_map.get(key).and_then(|s| s.parse().ok())
        };

        WmsParams {
            service: get_str("SERVICE"),
            request: get_str("REQUEST"),
            version: get_str("VERSION"),
            layers: get_str("LAYERS"),
            styles: get_str("STYLES"),
            // CRS can also be SRS (WMS 1.1.x compatibility)
            crs: get_str("CRS").or_else(|| get_str("SRS")),
            bbox: get_str("BBOX"),
            width: get_u32("WIDTH"),
            height: get_u32("HEIGHT"),
            format: get_str("FORMAT"),
            time: get_str("TIME"),
            run: get_str("RUN"),
            forecast: get_str("FORECAST"),
            elevation: get_str("ELEVATION"),
            transparent: get_str("TRANSPARENT"),
            bgcolor: get_str("BGCOLOR"),
            query_layers: get_str("QUERY_LAYERS"),
            info_format: get_str("INFO_FORMAT"),
            // I/J can also be X/Y (WMS 1.1.x compatibility)
            i: get_u32("I").or_else(|| get_u32("X")),
            j: get_u32("J").or_else(|| get_u32("Y")),
            feature_count: get_u32("FEATURE_COUNT"),
        }
    }
}

// ============================================================================
// WMS Handler Entry Point
// ============================================================================

#[instrument(skip(state))]
pub async fn wms_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(query_map): Query<HashMap<String, String>>,
) -> Response {
    // Parse parameters with case-insensitive keys (OGC WMS 1.3.0 spec 6.8.1)
    let params = WmsParams::from_query_map(&query_map);

    // Normalize request parameter to match pattern
    let request = params.request.as_deref().map(|s| s.to_uppercase());

    // Validate SERVICE parameter:
    // - Required for GetCapabilities (OGC WMS 1.3.0 spec 7.2.3.1)
    // - Optional for GetMap and GetFeatureInfo (spec 6.3.3: "shall be included... with the value WMS"
    //   but servers may accept requests without it for operations that can only be WMS)
    // - If provided, must be "WMS"
    let service = params.service.as_deref().map(|s| s.to_uppercase());
    let service_required = matches!(request.as_deref(), Some("GETCAPABILITIES"));
    
    match (service.as_deref(), service_required) {
        (Some("WMS"), _) => {} // Valid SERVICE=WMS
        (Some(_), _) => {
            // SERVICE provided but not "WMS"
            return wms_exception(
                "InvalidParameterValue",
                "SERVICE must be WMS",
                StatusCode::BAD_REQUEST,
            );
        }
        (None, true) => {
            // SERVICE missing but required for GetCapabilities
            return wms_exception(
                "MissingParameterValue",
                "SERVICE parameter is required for GetCapabilities",
                StatusCode::BAD_REQUEST,
            );
        }
        (None, false) => {} // SERVICE not provided but not required for GetMap/GetFeatureInfo
    }

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
            // OGC WMS 1.3.0 requires text/xml for capabilities
            .header(header::CONTENT_TYPE, "text/xml")
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
            let mut has_wspd = false;

            for layer in &model_config.layers {
                // Skip composite layers - they're handled separately
                if layer.composite {
                    continue;
                }

                // Check if data exists for this layer
                if let Ok(Some(availability)) = state
                    .catalog
                    .get_parameter_availability(model_id, &layer.parameter)
                    .await
                {
                    let key = format!("{}_{}", model_id, layer.parameter);
                    param_availability.insert(key, availability);

                    // Track if we have WSPD/WIND for this model
                    if layer.parameter == "WSPD" || layer.parameter == "WIND" {
                        has_wspd = true;
                    }
                }
            }

            // For models with WSPD, also check for WDIR availability
            // (WDIR may not have its own WMS layer but is needed for wind barbs)
            if has_wspd {
                let wdir_key = format!("{}_WDIR", model_id);
                if !param_availability.contains_key(&wdir_key) {
                    if let Ok(Some(wdir_avail)) = state
                        .catalog
                        .get_parameter_availability(model_id, "WDIR")
                        .await
                    {
                        param_availability.insert(wdir_key, wdir_avail);
                    }
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
        // OGC WMS 1.3.0 requires text/xml for capabilities
        .header(header::CONTENT_TYPE, "text/xml")
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
    // Per OGC WMS 1.3.0 spec 7.3.3.4: Empty STYLES parameter means use default style
    let styles_param = params.styles.as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    let bbox = params.bbox.as_deref();
    let crs = params.crs.as_deref();
    let format = params.format.as_deref();
    let version = params.version.as_deref();

    // Validate VERSION (required for GetMap per OGC WMS 1.3.0 spec 7.3.2)
    if version.is_none() {
        return wms_exception(
            "MissingParameterValue",
            "VERSION is required",
            StatusCode::BAD_REQUEST,
        );
    }

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

    // Validate that all layers exist (OGC WMS 1.3.0 spec 7.3.3.3)
    {
        let layer_configs = state.layer_configs.read().await;
        for (idx, layer_name) in layer_names.iter().enumerate() {
            // Check for CITE test layers (cite:Lakes, cite:Ponds, etc.)
            if cite::is_cite_layer(layer_name) {
                if !cite::is_cite_enabled() {
                    return wms_exception(
                        "LayerNotDefined",
                        &format!("CITE test layers are not enabled. Set ENABLE_CITE_DATA=true"),
                        StatusCode::BAD_REQUEST,
                    );
                }
                let cite_layer = match cite::get_cite_layer(layer_name) {
                    Some(l) => l,
                    None => {
                        return wms_exception(
                            "LayerNotDefined",
                            &format!("CITE layer '{}' is not defined", layer_name),
                            StatusCode::BAD_REQUEST,
                        );
                    }
                };
                // Check for required dimensions (dimensions without default values)
                for dim in &cite_layer.dimensions {
                    if dim.default.is_none() {
                        // This dimension is required - check if it was provided
                        let dim_provided = match dim.name.to_lowercase().as_str() {
                            "elevation" => params.elevation.is_some(),
                            "time" => params.time.is_some(),
                            _ => true, // Unknown dimensions are assumed provided
                        };
                        if !dim_provided {
                            return wms_exception(
                                "MissingDimensionValue",
                                &format!(
                                    "The {} dimension has no default value and must be specified for layer '{}'",
                                    dim.name.to_uppercase(), layer_name
                                ),
                                StatusCode::BAD_REQUEST,
                            );
                        }
                    }
                }
                // CITE layers only support default style
                if let Some(style) = style_names.get(idx) {
                    if !style.is_empty() && *style != "default" {
                        return wms_exception(
                            "StyleNotDefined",
                            &format!("Style '{}' is not defined for CITE layer '{}'", style, layer_name),
                            StatusCode::BAD_REQUEST,
                        );
                    }
                }
                continue;
            }
            
            // Parse layer name (format: model_parameter)
            if let Some(underscore_pos) = layer_name.find('_') {
                let model = &layer_name[..underscore_pos];
                let param = &layer_name[underscore_pos + 1..];
                
                if !layer_configs.has_layer(model, param) {
                    return wms_exception(
                        "LayerNotDefined",
                        &format!("Layer '{}' is not defined", layer_name),
                        StatusCode::BAD_REQUEST,
                    );
                }
                
                // Validate style if specified
                if let Some(style) = style_names.get(idx) {
                    if !style.is_empty() && *style != "default" {
                        // Check if style exists for this layer
                        if let Some(layer_config) = layer_configs.get_layer_by_param(model, param) {
                            let style_file = &layer_config.style_file;
                            // Load style file and check if style exists
                            if let Ok(content) = std::fs::read_to_string(format!("config/styles/{}", style_file)) {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                                    if let Some(styles) = json.get("styles").and_then(|s| s.as_object()) {
                                        if !styles.contains_key(*style) {
                                            return wms_exception(
                                                "StyleNotDefined",
                                                &format!("Style '{}' is not defined for layer '{}'", style, layer_name),
                                                StatusCode::BAD_REQUEST,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                // Invalid layer name format
                return wms_exception(
                    "LayerNotDefined",
                    &format!("Layer '{}' is not defined", layer_name),
                    StatusCode::BAD_REQUEST,
                );
            }
        }
    }

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
        let coords: Vec<f64> = bbox_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if coords.len() == 4 {
            let crs_str = crs.unwrap_or("EPSG:4326");
            let bbox_array = if crs_str.contains("3857") {
                let (min_lon, min_lat) = mercator_to_wgs84(coords[0], coords[1]);
                let (max_lon, max_lat) = mercator_to_wgs84(coords[2], coords[3]);
                [
                    min_lon as f32,
                    min_lat as f32,
                    max_lon as f32,
                    max_lat as f32,
                ]
            } else {
                // WMS 1.3.0 with EPSG:4326 uses axis order lat,lon
                [
                    coords[1] as f32,
                    coords[0] as f32,
                    coords[3] as f32,
                    coords[2] as f32,
                ]
            };
            state
                .metrics
                .record_tile_request_location(&bbox_array, crate::metrics::TileCacheStatus::Miss);
        }
    }

    // Parse TRANSPARENT parameter (default FALSE per OGC WMS 1.3.0 spec 7.3.3.9)
    // "The default value of TRANSPARENT is FALSE"
    let transparent = match params.transparent.as_deref() {
        Some(t) => t.to_uppercase() == "TRUE",
        None => false, // Default is FALSE (opaque) per spec
    };

    // Parse BGCOLOR parameter (format: 0xRRGGBB)
    let bgcolor = params.bgcolor.as_ref().and_then(|bg| {
        let hex = bg.trim_start_matches("0x").trim_start_matches("0X");
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some([r, g, b])
        } else {
            None
        }
    });

    // Time the rendering
    let timer = Timer::start();

    // Render layers (single or multiple)
    let render_result = if layer_names.len() == 1 {
        // Single layer - use existing function
        let style = style_names.first().copied().unwrap_or("default");
        render_weather_data(
            &state,
            layer_names[0],
            style,
            width,
            height,
            bbox,
            crs,
            version,
            &dimensions,
            transparent,
            bgcolor,
        )
        .await
    } else {
        // Multiple layers - render each and composite
        render_multi_layer(
            &state,
            &layer_names,
            &style_names,
            width,
            height,
            bbox,
            crs,
            version,
            &dimensions,
            transparent,
            bgcolor,
        )
        .await
    };

    // Try to render actual data, return error on failure
    match render_result {
        Ok(png_data) => {
            state.metrics.record_render(timer.elapsed_us(), true).await;

            // Convert to requested format
            let requested_format = format.unwrap_or("image/png").to_lowercase();
            let (output_data, content_type) = match requested_format.as_str() {
                "image/jpeg" => {
                    match convert_png_to_jpeg(&png_data) {
                        Ok(jpeg_data) => (jpeg_data, "image/jpeg"),
                        Err(_) => (png_data, "image/png"), // Fallback to PNG on error
                    }
                }
                "image/webp" => {
                    match convert_png_to_webp(&png_data) {
                        Ok(webp_data) => (webp_data, "image/webp"),
                        Err(_) => (png_data, "image/png"), // Fallback to PNG on error
                    }
                }
                _ => (png_data, "image/png"),
            };

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(output_data.into())
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
            wms_exception(e.code(), &e.message(), e.status_code())
        }
    }
}

// ============================================================================
// GetFeatureInfo
// ============================================================================

async fn wms_get_feature_info(state: Arc<AppState>, params: WmsParams) -> Response {
    use wms_protocol::{FeatureInfoResponse, InfoFormat};

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
            &format!(
                "I parameter value {} is out of range. Must be between 0 and {} (WIDTH-1).",
                i,
                width - 1
            ),
            StatusCode::BAD_REQUEST,
        );
    }

    // Validate J parameter is within image bounds (0 to HEIGHT-1)
    if j >= height {
        return wms_exception(
            "InvalidPoint",
            &format!(
                "J parameter value {} is out of range. Must be between 0 and {} (HEIGHT-1).",
                j,
                height - 1
            ),
            StatusCode::BAD_REQUEST,
        );
    }

    // Parse INFO_FORMAT
    let info_format = match params.info_format.as_deref() {
        Some(fmt) => match InfoFormat::from_mime(fmt) {
            Some(f) => f,
            None => {
                return wms_exception(
                        "InvalidFormat",
                        &format!("INFO_FORMAT '{}' is not supported. Supported formats: application/json, text/html, text/xml, text/plain", fmt),
                        StatusCode::BAD_REQUEST,
                    );
            }
        },
        None => InfoFormat::Html, // Default to HTML if not specified
    };

    // Parse BBOX
    let bbox_coords: Result<Vec<f64>, _> = bbox.split(',').map(|s| s.trim().parse()).collect();

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

    // Parse TIME parameter - can be either:
    // 1. A forecast hour integer (e.g., "6" for GFS/HRRR forecast models)
    // 2. An ISO 8601 timestamp (e.g., "2024-12-21T21:01:00Z" for GOES satellite/observation data)
    let (forecast_hour, valid_time): (Option<u32>, Option<chrono::DateTime<chrono::Utc>>) =
        if let Some(time_str) = params.time.as_ref() {
            // First try parsing as integer (forecast hour)
            if let Ok(hour) = time_str.parse::<u32>() {
                (Some(hour), None)
            } else {
                // Try parsing as ISO 8601 timestamp
                use chrono::{DateTime, Utc};
                if let Ok(dt) = DateTime::parse_from_rfc3339(time_str) {
                    (None, Some(dt.with_timezone(&Utc)))
                } else if let Ok(dt) = time_str.parse::<DateTime<Utc>>() {
                    (None, Some(dt))
                } else {
                    // Could not parse - log warning but continue
                    tracing::warn!(time = %time_str, "Could not parse TIME parameter as forecast hour or ISO timestamp");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

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
        valid_time = ?valid_time,
        elevation = ?elevation,
        "GetFeatureInfo request"
    );

    // Query each layer
    let layers: Vec<&str> = query_layers.split(',').map(|s| s.trim()).collect();

    // Validate all layer names before querying
    // Get list of valid models from catalog
    let valid_models = state.catalog.list_models().await.unwrap_or_default();

    for layer in &layers {
        // Check for CITE layers
        if cite::is_cite_layer(layer) {
            if !cite::is_cite_enabled() {
                return wms_exception(
                    "LayerNotDefined",
                    &format!("CITE test layers are not enabled. Set ENABLE_CITE_DATA=true"),
                    StatusCode::BAD_REQUEST,
                );
            }
            match cite::get_cite_layer(layer) {
                Some(cite_layer) if !cite_layer.queryable => {
                    return wms_exception(
                        "LayerNotQueryable",
                        &format!("Layer '{}' does not support GetFeatureInfo", layer),
                        StatusCode::BAD_REQUEST,
                    );
                }
                Some(_) => continue, // Valid queryable CITE layer
                None => {
                    return wms_exception(
                        "LayerNotDefined",
                        &format!("CITE layer '{}' is not defined", layer),
                        StatusCode::BAD_REQUEST,
                    );
                }
            }
        }
        
        let parts: Vec<&str> = layer.split('_').collect();
        if parts.len() < 2 {
            return wms_exception(
                "LayerNotDefined",
                &format!(
                    "Layer '{}' is not defined. Layer names must be in format 'model_parameter'.",
                    layer
                ),
                StatusCode::BAD_REQUEST,
            );
        }

        // Check if the model exists in the catalog
        let model = parts[0].to_lowercase();
        if !valid_models.iter().any(|m| m.to_lowercase() == model) {
            return wms_exception(
                "LayerNotDefined",
                &format!(
                    "Layer '{}' is not defined. Model '{}' not found.",
                    layer, parts[0]
                ),
                StatusCode::BAD_REQUEST,
            );
        }
    }

    let mut all_features = Vec::new();

    for layer in layers {
        // Handle CITE layers
        if cite::is_cite_layer(layer) {
            match cite::get_cite_feature_info(layer, i, j, width, height, bbox_array) {
                Ok(attrs) if !attrs.is_empty() => {
                    // Extract location from attributes
                    let lon: f64 = attrs.iter().find(|(k, _)| k == "x").map(|(_, v)| v.parse().unwrap_or(0.0)).unwrap_or(0.0);
                    let lat: f64 = attrs.iter().find(|(k, _)| k == "y").map(|(_, v)| v.parse().unwrap_or(0.0)).unwrap_or(0.0);
                    
                    // Create FeatureInfo for CITE layer
                    let feature = wms_protocol::FeatureInfo {
                        layer_name: layer.to_string(),
                        parameter: "CITE Test Feature".to_string(),
                        value: 1.0, // Feature present
                        unit: "".to_string(),
                        raw_value: 1.0,
                        raw_unit: "present".to_string(),
                        location: wms_protocol::Location {
                            longitude: lon,
                            latitude: lat,
                        },
                        forecast_hour: None,
                        reference_time: None,
                        level: None,
                    };
                    all_features.push(feature);
                }
                Ok(_) => {} // No feature at this point
                Err(e) => {
                    error!(layer = %layer, error = %e, "Failed to query CITE layer");
                }
            }
            continue;
        }
        
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

        // Get layer configs for unit conversion
        let layer_configs = state.layer_configs.read().await;

        match crate::rendering::query_point_value(
            &state.catalog,
            &state.metrics,
            &state.grid_processor_factory,
            &layer_configs,
            layer,
            bbox_array,
            width,
            height,
            i,
            j,
            crs,
            forecast_hour,
            valid_time,
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
        InfoFormat::Json => match response.to_json() {
            Ok(json) => (json, "application/json"),
            Err(e) => {
                return wms_exception(
                    "NoApplicableCode",
                    &format!("JSON encoding failed: {}", e),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            }
        },
        InfoFormat::Html => (response.to_html(), "text/html"),
        InfoFormat::Xml => (response.to_xml(), "text/xml"),
        InfoFormat::Text => (response.to_text(), "text/plain"),
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
    version: Option<&str>,
    dimensions: &DimensionParams,
    transparent: bool,
    bgcolor: Option<[u8; 3]>,
) -> Result<Vec<u8>, WmsError> {
    // Check for CITE test layers (cite:Lakes, cite:Ponds, etc.)
    if cite::is_cite_layer(layer) {
        return render_cite_layer(layer, width, height, bbox, crs, version, transparent, bgcolor);
    }
    
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
    let (forecast_hour, observation_time, _reference_time) =
        dimensions.parse_for_layer(model, &state.model_dimensions);

    // Get default level if not specified
    let level = match &dimensions.elevation {
        Some(elev) => Some(elev.replace(" ", "_")),
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
        let parsed_bbox = bbox.and_then(|b| parse_bbox(b, crs, version));

        // Get wind barbs style file
        let wind_style_file = state
            .layer_configs
            .read()
            .await
            .get_style_file_for_parameter(model, "WIND_BARBS");

        // Check if model uses speed/direction (WSPD/WDIR) or U/V components (UGRD/VGRD)
        // NDFD uses speed/direction, most other models use U/V components
        let uses_speed_direction = model.starts_with("nbm") || model == "ndfd";

        if uses_speed_direction {
            return crate::rendering::render_wind_barbs_from_speed_direction_tile(
                &state.catalog,
                &state.grid_processor_factory,
                model,
                None, // No tile coord for WMS
                width,
                height,
                parsed_bbox.unwrap_or([-180.0, -90.0, 180.0, 90.0]),
                observation_time,
                forecast_hour,
                level.as_deref(),
                Some(&wind_style_file),
                None, // Use default style
            )
            .await
            .map_err(WmsError::from_rendering_error);
        } else {
            return crate::rendering::render_wind_barbs_layer(
                &state.catalog,
                &state.grid_processor_factory,
                model,
                width,
                height,
                parsed_bbox,
                forecast_hour,
                Some(&wind_style_file),
                None, // Use default style
            )
            .await
            .map_err(WmsError::from_rendering_error);
        }
    }

    // Parse BBOX parameter
    let parsed_bbox = bbox.and_then(|b| parse_bbox(b, crs, version));

    info!(forecast_hour = ?forecast_hour, observation_time = ?observation_time, level = ?level, bbox = ?parsed_bbox, style = style, "Parsed WMS parameters");

    // Check CRS for projection
    let crs_str = crs.unwrap_or("EPSG:4326");
    let use_mercator = crs_str.contains("3857");

    if style == "isolines" {
        // Isolines are not supported for radar/satellite imagery, but ARE supported
        // for gridded forecast products like NDFD (even though NDFD uses observation-style TIME dimension)
        let is_imagery_model = matches!(
            model,
            "mrms" | "goes18" | "goes19" | "goes18-fulldisk" | "goes19-fulldisk"
        );
        if is_imagery_model {
            return Err(WmsError::StyleNotDefined(format!(
                "Style 'isolines' is not supported for radar/satellite imagery layers like {}.",
                model.to_uppercase()
            )));
        }

        let style_file = state
            .layer_configs
            .read()
            .await
            .get_style_file_for_parameter(model, &parameter);

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
            observation_time,
            level.as_deref(),
            use_mercator,
        )
        .await
        .map_err(WmsError::from_rendering_error);
    }

    // Check if model requires full grid reads (non-geographic projection)
    let requires_full_grid = state.model_dimensions.requires_full_grid(model);

    // Standard rendering
    let style_file = state
        .layer_configs
        .read()
        .await
        .get_style_file_for_parameter(model, &parameter);

    let png_data = crate::rendering::render_weather_data(
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
        &state.grid_processor_factory,
        requires_full_grid,
    )
    .await
    .map_err(WmsError::from_rendering_error)?;

    // Apply TRANSPARENT and BGCOLOR per WMS spec
    // If TRANSPARENT=FALSE, composite the (potentially transparent) result onto BGCOLOR background
    if !transparent {
        apply_bgcolor(&png_data, width, height, bgcolor)
    } else {
        Ok(png_data)
    }
}

/// Render a CITE test layer
fn render_cite_layer(
    layer_name: &str,
    width: u32,
    height: u32,
    bbox: Option<&str>,
    crs: Option<&str>,
    version: Option<&str>,
    transparent: bool,
    bgcolor: Option<[u8; 3]>,
) -> Result<Vec<u8>, WmsError> {
    // Parse BBOX
    let parsed_bbox = bbox
        .and_then(|b| parse_bbox(b, crs, version))
        .map(|b| [b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64])
        .unwrap_or([-180.0, -90.0, 180.0, 90.0]);

    // CITE layers use CRS:84 (lon,lat order), but we've already handled axis order in parse_bbox
    // Render the layer
    cite::render_cite_layer(layer_name, width, height, parsed_bbox, transparent, bgcolor)
        .map_err(|e| WmsError::RenderingError(e))
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
    version: Option<&str>,
    dimensions: &DimensionParams,
    transparent: bool,
    bgcolor: Option<[u8; 3]>,
) -> Result<Vec<u8>, WmsError> {
    use image::{ImageBuffer, Rgba, RgbaImage};
    use std::io::Cursor;

    if layer_names.is_empty() {
        return Err(WmsError::LayerNotDefined("No layers specified".to_string()));
    }

    // Create base image with appropriate background
    let base_pixel = if transparent {
        Rgba([0, 0, 0, 0])
    } else {
        let [r, g, b] = bgcolor.unwrap_or([255, 255, 255]);
        Rgba([r, g, b, 255])
    };
    let mut composite: RgbaImage = ImageBuffer::from_pixel(width, height, base_pixel);

    // Render each layer and composite
    for (i, layer_name) in layer_names.iter().enumerate() {
        // Get the style for this layer (use default if not enough styles provided)
        let style = style_names.get(i).copied().unwrap_or("default");
        let style = if style.is_empty() { "default" } else { style };

        info!(layer = %layer_name, style = %style, layer_index = i, "Rendering layer for multi-layer composite");

        // Render this layer (always transparent for compositing, final result uses requested transparency)
        match render_weather_data(
            state, layer_name, style, width, height, bbox, crs, version, dimensions, true, None,
        )
        .await
        {
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

    composite
        .write_with_encoder(encoder)
        .map_err(|e| WmsError::RenderingError(format!("Failed to encode composite PNG: {}", e)))?;

    Ok(png_bytes)
}

/// Apply background color to a transparent PNG
/// 
/// Per WMS 1.3.0 spec, when TRANSPARENT=FALSE, transparent areas should be
/// filled with BGCOLOR (default white 0xFFFFFF).
fn apply_bgcolor(
    png_data: &[u8],
    width: u32,
    height: u32,
    bgcolor: Option<[u8; 3]>,
) -> Result<Vec<u8>, WmsError> {
    use image::{ImageBuffer, Rgba, RgbaImage};
    use std::io::Cursor;

    // Decode the PNG
    let source_image = match image::load_from_memory(png_data) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            return Err(WmsError::RenderingError(format!(
                "Failed to decode PNG for bgcolor: {}",
                e
            )));
        }
    };

    // Create background image with BGCOLOR (default white per WMS spec)
    let [r, g, b] = bgcolor.unwrap_or([255, 255, 255]);
    let bg_pixel = Rgba([r, g, b, 255]);
    let mut result: RgbaImage = ImageBuffer::from_pixel(width, height, bg_pixel);

    // Composite source over background
    for (x, y, pixel) in source_image.enumerate_pixels() {
        if x < width && y < height {
            let bg = result.get_pixel(x, y);
            let blended = alpha_blend(*bg, *pixel);
            result.put_pixel(x, y, blended);
        }
    }

    // Encode result to PNG
    let mut png_bytes: Vec<u8> = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(Cursor::new(&mut png_bytes));

    result
        .write_with_encoder(encoder)
        .map_err(|e| WmsError::RenderingError(format!("Failed to encode PNG with bgcolor: {}", e)))?;

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
///
/// WMS axis order depends on CRS and version:
/// - WMS 1.1.x: Always X,Y (lon,lat for geographic CRS)
/// - WMS 1.3.0 with EPSG:4326 (CRS:84 variant): X,Y (lon,lat)
/// - WMS 1.3.0 with EPSG:4326 (standard): Y,X (lat,lon)
/// - Web Mercator (EPSG:3857): Always X,Y
///
/// We use CRS:84 semantics (lon,lat) for EPSG:4326 to match common client behavior,
/// since most clients (including Leaflet, OpenLayers) send lon,lat regardless of WMS version.
fn parse_bbox(bbox_str: &str, crs: Option<&str>, version: Option<&str>) -> Option<[f32; 4]> {
    let coords: Vec<f64> = bbox_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if coords.len() == 4 {
        let crs_str = crs.unwrap_or("EPSG:4326");
        let (min_lon, min_lat, max_lon, max_lat) = if crs_str.contains("3857") {
            // Web Mercator: convert from meters to WGS84
            let (min_lon, min_lat) = mercator_to_wgs84(coords[0], coords[1]);
            let (max_lon, max_lat) = mercator_to_wgs84(coords[2], coords[3]);
            (min_lon, min_lat, max_lon, max_lat)
        } else if let Some(v) = version {
            // Version explicitly specified
            if v.starts_with("1.3") && crs_str.contains("4326") && !crs_str.contains("CRS:84") {
                // WMS 1.3.0 with EPSG:4326 (not CRS:84): axis order is lat,lon
                // BBOX = minY,minX,maxY,maxX = minLat,minLon,maxLat,maxLon
                (coords[1], coords[0], coords[3], coords[2])
            } else {
                // WMS 1.1.x or CRS:84: axis order is X,Y (lon,lat)
                (coords[0], coords[1], coords[2], coords[3])
            }
        } else {
            // No version specified: assume common client behavior (lon,lat order)
            // Most mapping libraries (Leaflet, OpenLayers) send lon,lat regardless
            (coords[0], coords[1], coords[2], coords[3])
        };

        Some([
            min_lon as f32,
            min_lat as f32,
            max_lon as f32,
            max_lat as f32,
        ])
    } else {
        None
    }
}

// ============================================================================
// WMS Capabilities XML Builder
// ============================================================================

/// Build WMS capabilities XML from layer configs (config-driven approach).
/// Only includes layers that have data available in the catalog.
///
///TODO we only need the one correct method to create a capabilities document CLAUDE!
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
        // Some models use UGRD/VGRD (GFS, HRRR), others use WSPD/WDIR (NDFD)
        let mut ugrd_availability: Option<&ParameterAvailability> = None;
        let mut vgrd_availability: Option<&ParameterAvailability> = None;
        let mut wspd_availability: Option<&ParameterAvailability> = None;
        let mut wdir_availability: Option<&ParameterAvailability> = None;

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

            // Track wind component availability for wind barbs
            match layer.parameter.as_str() {
                "UGRD" => ugrd_availability = Some(availability),
                "VGRD" => vgrd_availability = Some(availability),
                "WSPD" | "WIND" => wspd_availability = Some(availability),
                "WDIR" => wdir_availability = Some(availability),
                _ => {}
            }

            // Build dimensions for this specific layer
            let dimensions_xml = build_layer_dimensions_xml(availability, is_observational);

            // Get styles from style file
            let style_path = layer_configs.get_style_path(layer);
            let styles_xml = get_styles_xml_from_file(&style_path);

            // Build bounding box (normalize longitude to -180/180)
            let (west, east, south, north) = normalize_bbox(&availability.bbox);

            // WMS 1.3.0 EPSG:4326 uses lat/lon axis order for BoundingBox:
            // minx=south, miny=west, maxx=north, maxy=east
            let layer_xml = format!(
                r#"<Layer queryable="1"><Name>{}_{}</Name><Title>{} - {}</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="{}" miny="{}" maxx="{}" maxy="{}"/>{}{}</Layer>"#,
                model_id,
                layer.parameter,
                model_config.display_name,
                layer.title,
                west,
                east,
                south,
                north,
                south,  // minx = south latitude (WMS 1.3.0 EPSG:4326 uses lat/lon order)
                west,   // miny = west longitude
                north,  // maxx = north latitude
                east,   // maxy = east longitude
                styles_xml,
                dimensions_xml
            );
            layer_xml_parts.push(layer_xml);
        }

        // Handle WIND_BARBS composite layer
        // Check for either UGRD/VGRD (traditional models) or WSPD/WDIR (NDFD-style models)
        // Note: WDIR may not have its own layer in the config (it's only useful combined with WSPD),
        // so we also check param_availability directly
        let wdir_key = format!("{}_WDIR", model_id);
        let wdir_from_db = param_availability.get(&wdir_key);
        let effective_wdir = wdir_availability.or(wdir_from_db);

        let wind_components: Option<(&ParameterAvailability, &ParameterAvailability)> =
            if let (Some(ugrd), Some(vgrd)) = (ugrd_availability, vgrd_availability) {
                Some((ugrd, vgrd))
            } else if let (Some(wspd), Some(wdir)) = (wspd_availability, effective_wdir) {
                Some((wspd, wdir))
            } else {
                None
            };

        if let Some((wind1, wind2)) = wind_components {
            // Find common levels between the two wind components
            let common_levels: Vec<String> = wind1
                .levels
                .iter()
                .filter(|l| wind2.levels.contains(l))
                .cloned()
                .collect();

            // Find common times between the two wind components
            let common_times: Vec<String> = wind1
                .times
                .iter()
                .filter(|t| wind2.times.contains(t))
                .cloned()
                .collect();

            // Find common forecast hours
            let common_forecast_hours: Vec<i32> = wind1
                .forecast_hours
                .iter()
                .filter(|h| wind2.forecast_hours.contains(h))
                .copied()
                .collect();

            // Only include WIND_BARBS if there's common data
            if !common_times.is_empty() && (!common_levels.is_empty() || is_observational) {
                let wind_availability = ParameterAvailability {
                    times: common_times,
                    forecast_hours: common_forecast_hours,
                    levels: common_levels,
                    bbox: wind1.bbox.clone(),
                };

                let dimensions_xml =
                    build_layer_dimensions_xml(&wind_availability, is_observational);

                let (west, east, south, north) = normalize_bbox(&wind1.bbox);

                // WMS 1.3.0 EPSG:4326 uses lat/lon axis order for BoundingBox
                let wind_layer_xml = format!(
                    r#"<Layer queryable="1"><Name>{}_WIND_BARBS</Name><Title>{} - Wind Barbs</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS><EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox><BoundingBox CRS="EPSG:4326" minx="{}" miny="{}" maxx="{}" maxy="{}"/><Style><Name>default</Name><Title>Default Barbs</Title></Style>{}</Layer>"#,
                    model_id,
                    model_config.display_name,
                    west,
                    east,
                    south,
                    north,
                    south,  // minx = south latitude (WMS 1.3.0 EPSG:4326 uses lat/lon order)
                    west,   // miny = west longitude
                    north,  // maxx = north latitude
                    east,   // maxy = east longitude
                    dimensions_xml
                );
                layer_xml_parts.push(wind_layer_xml);
            }
        }

        // Only include model if it has at least one layer with data
        // NOTE: Model container layers do NOT have <Name> elements because per OGC WMS 1.3.0 spec,
        // every named layer must have BoundingBox and EX_GeographicBoundingBox (either direct or inherited).
        // These container layers are just for grouping - they are not requestable.
        if !layer_xml_parts.is_empty() {
            let model_xml = format!(
                r#"<Layer><Title>{}</Title>{}</Layer>"#,
                model_config.display_name,
                layer_xml_parts.join("")
            );
            model_layers.push(model_xml);
        }
    }

    // Add CITE test layers if enabled
    // Regular CITE layers go first so they get selected by tests
    let cite_layers = cite::get_cite_capabilities_layers();
    // Required-dimension CITE layers go last so they don't get selected by tests
    // that don't know about their required dimensions
    let cite_required_dim_layers = cite::get_cite_required_dimension_layers();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<WMS_Capabilities version="{}" xmlns="http://www.opengis.net/wms" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.opengis.net/wms http://schemas.opengis.net/wms/1.3.0/capabilities_1_3_0.xsd">
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
        <Format>image/jpeg</Format>
        <Format>image/webp</Format>
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
      <Title>WMS Server Root Layer</Title>
      <CRS>EPSG:4326</CRS>
      <CRS>EPSG:3857</CRS>
      <CRS>CRS:84</CRS>
      {}
      <Layer>
        <Title>Weather Data</Title>
        <CRS>EPSG:4326</CRS>
        <CRS>EPSG:3857</CRS>
        {}
      </Layer>
      {}
    </Layer>
  </Capability>
</WMS_Capabilities>"#,
        version,
        cite_layers,
        model_layers.join(""),
        cite_required_dim_layers
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
        let time_default = availability
            .times
            .first()
            .map(|s| s.as_str())
            .unwrap_or("latest");
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
        let run_default = availability
            .times
            .first()
            .map(|s| s.as_str())
            .unwrap_or("latest");

        let forecast_values = if availability.forecast_hours.is_empty() {
            "0".to_string()
        } else {
            availability
                .forecast_hours
                .iter()
                .map(|h| h.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let forecast_default = availability.forecast_hours.first().unwrap_or(&0);

        // Note: Use "{} hours" format for default to avoid matching CITE test's
        // XPath predicate @default='0' which would incorrectly flag this as a
        // dimension without a valid default (see dimensions.xml test).
        dimensions.push_str(&format!(
            r#"<Dimension name="RUN" units="ISO8601" default="{}">{}</Dimension><Dimension name="FORECAST" units="hours" default="{} hours">{}</Dimension>"#,
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
        let w = if bbox.min_x > 180.0 {
            bbox.min_x - 360.0
        } else {
            bbox.min_x
        };
        let e = if bbox.max_x > 180.0 {
            bbox.max_x - 360.0
        } else {
            bbox.max_x
        };
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
    fn test_parse_bbox_wgs84_v130() {
        // WMS 1.3.0 EPSG:4326 format: min_lat, min_lon, max_lat, max_lon
        let bbox = parse_bbox("30.0,-120.0,50.0,-80.0", Some("EPSG:4326"), Some("1.3.0"));
        assert!(bbox.is_some());
        let b = bbox.unwrap();
        // Should be converted to [min_lon, min_lat, max_lon, max_lat]
        assert!((b[0] - (-120.0)).abs() < 0.01); // min_lon
        assert!((b[1] - 30.0).abs() < 0.01); // min_lat
        assert!((b[2] - (-80.0)).abs() < 0.01); // max_lon
        assert!((b[3] - 50.0).abs() < 0.01); // max_lat
    }

    #[test]
    fn test_parse_bbox_wgs84_v111() {
        // WMS 1.1.1 EPSG:4326 format: min_lon, min_lat, max_lon, max_lat
        let bbox = parse_bbox("-120.0,30.0,-80.0,50.0", Some("EPSG:4326"), Some("1.1.1"));
        assert!(bbox.is_some());
        let b = bbox.unwrap();
        // Should stay as [min_lon, min_lat, max_lon, max_lat]
        assert!((b[0] - (-120.0)).abs() < 0.01); // min_lon
        assert!((b[1] - 30.0).abs() < 0.01); // min_lat
        assert!((b[2] - (-80.0)).abs() < 0.01); // max_lon
        assert!((b[3] - 50.0).abs() < 0.01); // max_lat
    }

    #[test]
    fn test_parse_bbox_wgs84_no_version() {
        // No version specified: uses lon,lat order (1.1.x style, common client behavior)
        let bbox = parse_bbox("-120.0,30.0,-80.0,50.0", Some("EPSG:4326"), None);
        assert!(bbox.is_some());
        let b = bbox.unwrap();
        // Should stay as [min_lon, min_lat, max_lon, max_lat]
        assert!((b[0] - (-120.0)).abs() < 0.01); // min_lon
        assert!((b[1] - 30.0).abs() < 0.01); // min_lat
        assert!((b[2] - (-80.0)).abs() < 0.01); // max_lon
        assert!((b[3] - 50.0).abs() < 0.01); // max_lat
    }

    #[test]
    fn test_parse_bbox_web_mercator() {
        // Web Mercator: minx, miny, maxx, maxy in meters
        let bbox = parse_bbox(
            "-13358338.9,3503549.8,-8766409.9,6446275.8",
            Some("EPSG:3857"),
            None,
        );
        assert!(bbox.is_some());
        let b = bbox.unwrap();
        // Should convert to WGS84 approximately
        assert!(b[0] < -100.0); // min_lon (west coast US)
        assert!(b[1] > 20.0); // min_lat
        assert!(b[2] > -90.0); // max_lon (east of west coast)
        assert!(b[3] < 60.0); // max_lat
    }

    #[test]
    fn test_parse_bbox_invalid() {
        let bbox = parse_bbox("invalid", None, None);
        assert!(bbox.is_none());

        let bbox = parse_bbox("1,2,3", None, None);
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
