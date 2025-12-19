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
use super::common::{
    wms_exception, mercator_to_wgs84, DimensionParams, 
    get_styles_xml_from_file,
};

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
    let models = state.catalog.list_models().await.unwrap_or_default();
    
    // Get parameters and dimensions for each model
    let mut model_params = HashMap::new();
    let mut model_dimensions: HashMap<String, (Vec<String>, Vec<i32>)> = HashMap::new();
    let mut param_levels: HashMap<String, Vec<String>> = HashMap::new();
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
    
    let layer_configs = state.layer_configs.read().await;
    let xml = build_wms_capabilities_xml(
        version, &models, &model_params, &model_dimensions, 
        &param_levels, &model_bboxes, &state.model_dimensions, &layer_configs
    );
    
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
            error!(
                layer = %layers,
                style = %style,
                width = width,
                height = height,
                bbox = ?bbox,
                crs = ?crs,
                time = ?dimensions.time,
                run = ?dimensions.run,
                forecast = ?dimensions.forecast,
                elevation = ?dimensions.elevation,
                error = %e,
                "WMS GetMap rendering failed"
            );
            wms_exception(
                "NoApplicableCode",
                &format!("Rendering failed: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
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
            &state.grib_cache,
            &state.catalog,
            &state.metrics,
            Some(&state.grid_processor_factory),
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
) -> Result<Vec<u8>, String> {
    // Parse layer name (format: "model_parameter" or "model_WIND_BARBS")
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return Err("Invalid layer format".to_string());
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
            &state.grib_cache,
            &state.catalog,
            Some(&state.grid_processor_factory),
            model,
            width,
            height,
            parsed_bbox,
            None,
            forecast_hour,
        )
        .await;
    }

    // Parse BBOX parameter
    let parsed_bbox = bbox.and_then(|b| parse_bbox(b, crs));
    
    info!(forecast_hour = ?forecast_hour, observation_time = ?observation_time, level = ?level, bbox = ?parsed_bbox, style = style, "Parsed WMS parameters");

    // Check CRS for projection
    let crs_str = crs.unwrap_or("EPSG:4326");
    let use_mercator = crs_str.contains("3857");
    
    if style == "isolines" {
        if state.model_dimensions.is_observation(model) {
            return Err(format!(
                "Isolines style is not supported for {} layers.",
                model.to_uppercase()
            ));
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
            parsed_bbox.unwrap_or([-180.0, -90.0, 180.0, 90.0]),
            &style_file,
            "isolines",
            forecast_hour,
            level.as_deref(),
            use_mercator,
        )
        .await;
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
        .await;
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
}

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
// WMS Capabilities XML Builder
// ============================================================================

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
