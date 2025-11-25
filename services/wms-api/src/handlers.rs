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
use tracing::{info, instrument};

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
    
    // Get parameters for each model
    let mut model_params = HashMap::new();
    for model in &models {
        let params_list = state.catalog.list_parameters(model).await.unwrap_or_default();
        model_params.insert(model.clone(), params_list);
    }
    
    let xml = build_wms_capabilities_xml(version, &models, &model_params);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml")
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

    info!(layer = %layers, style = %style, width = width, height = height, "GetMap request");
    
    // Try to render actual data, fall back to placeholder on error
    match render_weather_data(&state, layers, width, height).await {
        Ok(png_data) => {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .body(png_data.into())
                .unwrap()
        }
        Err(e) => {
            info!(error = %e, "Rendering failed, using placeholder");
            let placeholder = generate_placeholder_image(width, height);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                .body(placeholder.into())
                .unwrap()
        }
    }
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
    #[serde(rename = "TILEMATRIX")]
    tile_matrix: Option<String>,
    #[serde(rename = "TILEROW")]
    tile_row: Option<u32>,
    #[serde(rename = "TILECOL")]
    tile_col: Option<u32>,
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
    let layer = parts[0];
    let style = parts[1];
    let tile_matrix = parts[3];
    let tile_row: u32 = parts[4].parse().unwrap_or(0);
    let last = parts[5];
    let (tile_col_str, _) = last.rsplit_once('.').unwrap_or((last, "png"));
    let tile_col: u32 = tile_col_str.parse().unwrap_or(0);
    let z: u32 = tile_matrix.parse().unwrap_or(0);
    wmts_get_tile(state, layer, style, z, tile_col, tile_row).await
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
    let xml = build_wmts_capabilities_xml(&models);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml")
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
    let tms = web_mercator_tile_matrix_set();
    let coord = TileCoord::new(z, x, y);
    if tms.tile_bbox(&coord).is_none() {
        return wmts_exception("TileOutOfRange", "Invalid tile", StatusCode::BAD_REQUEST);
    }
    let placeholder = generate_placeholder_image(256, 256);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CACHE_CONTROL, "max-age=3600")
        .body(placeholder.into())
        .unwrap()
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

async fn render_weather_data(
    state: &Arc<AppState>,
    layer: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    // Parse layer name (format: "model_parameter")
    let parts: Vec<&str> = layer.split('_').collect();
    if parts.len() < 2 {
        return Err("Invalid layer format".to_string());
    }

    let model = parts[0];
    let parameter = parts[1..].join("_");

    // Get latest dataset for this parameter
    let entry = state
        .catalog
        .get_latest(model, &parameter)
        .await
        .map_err(|e| format!("Catalog query failed: {}", e))?
        .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?;

    // Load GRIB2 file from storage
    let grib_data = state
        .storage
        .get(&entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load GRIB2 file: {}", e))?;

    // Parse GRIB2 and find matching message
    let mut reader = grib2_parser::Grib2Reader::new(grib_data);
    let mut message = None;

    while let Some(msg) = reader
        .next_message()
        .map_err(|e| format!("GRIB2 parse error: {}", e))?
    {
        // Match by parameter and level
        if msg.parameter() == &parameter[..] {
            message = Some(msg);
            break;
        }
    }

    let msg = message.ok_or_else(|| format!("Parameter {} not found in GRIB2", parameter))?;

    // Unpack grid data
    let grid_data = msg
        .unpack_data()
        .map_err(|e| format!("Unpacking failed: {}", e))?;

    let (grid_height, grid_width) = msg.grid_dims();
    let grid_width = grid_width as usize;
    let grid_height = grid_height as usize;

    info!(
        "Grid dimensions: {}x{}, data points: {}",
        grid_width,
        grid_height,
        grid_data.len()
    );

    if grid_data.len() != grid_width * grid_height {
        return Err(format!(
            "Grid data size mismatch: {} vs {}x{}",
            grid_data.len(),
            grid_width,
            grid_height
        ));
    }

    // Find data min/max for scaling
    let (min_val, max_val) = grid_data
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &val| {
            (min.min(val), max.max(val))
        });

    info!("Data range: {} to {}", min_val, max_val);
    info!("Sample values: {:?}", &grid_data[0..10.min(grid_data.len())]);

    // For now, render the full grid resolution
    // TODO: Implement proper resampling to match WMS request dimensions
    let rendered_width = width as usize;
    let rendered_height = height as usize;

    // Render based on parameter type
    let rgba_data = if parameter.contains("TMP") || parameter.contains("TEMP") {
        // Temperature in Kelvin, convert to Celsius for rendering
        let celsius_data: Vec<f32> = grid_data.iter().map(|k| k - 273.15).collect();
        let min_c = min_val - 273.15;
        let max_c = max_val - 273.15;
        
        info!(
            "Temperature range: {:.2}°C to {:.2}°C",
            min_c, max_c
        );
        
        renderer::gradient::render_temperature(&celsius_data, grid_width, grid_height, min_c, max_c)
    } else {
        // Generic gradient rendering
        renderer::gradient::render_grid(
            &grid_data,
            grid_width,
            grid_height,
            min_val,
            max_val,
            |norm| {
                // Generic blue-red gradient
                let hue = (1.0 - norm) * 240.0; // Blue to red
                let rgb = hsv_to_rgb(hue, 1.0, 1.0);
                gradient::Color::new(rgb.0, rgb.1, rgb.2, 255)
            },
        )
    };

    info!(
        "Rendered PNG from {}x{} grid, output {}x{}",
        grid_width, grid_height, rendered_width, rendered_height
    );

    // Convert to PNG 
    let png = renderer::png::create_png(&rgba_data, grid_width, grid_height)
        .map_err(|e| format!("PNG encoding failed: {}", e))?;

    Ok(png)
}

/// Convert HSV to RGB (simplified version)
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
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
) -> String {
    let empty_params = Vec::new();
    let layers: String = models
        .iter()
        .map(|model| {
            let params = model_params.get(model).unwrap_or(&empty_params);
            let param_layers = params
                .iter()
                .map(|p| {
                    format!(
                        r#"<Layer><Name>{}_{}</Name><Title>{} - {}</Title></Layer>"#,
                        model, p, model.to_uppercase(), p
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            
            format!(
                r#"<Layer><Name>{}</Name><Title>{}</Title>{}</Layer>"#,
                model,
                model.to_uppercase(),
                param_layers
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0"?><WMS_Capabilities version="{}"><Service><Title>Weather WMS</Title></Service><Capability><Layer>{}</Layer></Capability></WMS_Capabilities>"#,
        version, layers
    )
}

fn build_wmts_capabilities_xml(models: &[String]) -> String {
    let layers: String = models
        .iter()
        .map(|m| {
            format!(
                r#"<Layer><ows:Identifier>{}</ows:Identifier><ows:Title>{}</ows:Title></Layer>"#,
                m,
                m.to_uppercase()
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0"?><Capabilities xmlns="http://www.opengis.net/wmts/1.0" xmlns:ows="http://www.opengis.net/ows/1.1"><ows:ServiceIdentification><ows:Title>Weather WMTS</ows:Title></ows:ServiceIdentification><Contents>{}</Contents></Capabilities>"#,
        layers
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
