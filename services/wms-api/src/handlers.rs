//! HTTP request handlers for WMS and WMTS.

use axum::{
    extract::{Extension, Path, Query},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::sync::Arc;
use tracing::{info, instrument};

use storage::CacheKey;
use wms_common::{tile::web_mercator_tile_matrix_set, BoundingBox, CrsCode, TileCoord};

use crate::state::AppState;

// ============================================================================
// WMS Handlers
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct WmsParams {
    #[serde(rename = "SERVICE")]
    service: Option<String>,
    #[serde(rename = "REQUEST")]
    request: Option<String>,
    #[serde(rename = "VERSION")]
    version: Option<String>,
    #[serde(rename = "LAYERS")]
    layers: Option<String>,
    #[serde(rename = "STYLES")]
    styles: Option<String>,
    #[serde(rename = "CRS", alias = "SRS")]
    crs: Option<String>,
    #[serde(rename = "BBOX")]
    bbox: Option<String>,
    #[serde(rename = "WIDTH")]
    width: Option<u32>,
    #[serde(rename = "HEIGHT")]
    height: Option<u32>,
    #[serde(rename = "FORMAT")]
    format: Option<String>,
    #[serde(rename = "TIME")]
    time: Option<String>,
    #[serde(rename = "TRANSPARENT")]
    transparent: Option<String>,
}

#[instrument(skip(state))]
pub async fn wms_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<WmsParams>,
) -> Response {
    if params.service.as_deref() != Some("WMS") {
        return wms_exception(
            "InvalidParameterValue",
            "SERVICE must be WMS",
            StatusCode::BAD_REQUEST,
        );
    }

    match params.request.as_deref() {
        Some("GetCapabilities") => wms_get_capabilities(state, params).await,
        Some("GetMap") => wms_get_map(state, params).await,
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
    let xml = build_wms_capabilities_xml(version, &models);
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
    let placeholder = generate_placeholder_image(width, height);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .body(placeholder.into())
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

fn build_wms_capabilities_xml(version: &str, models: &[String]) -> String {
    let layers: String = models
        .iter()
        .map(|m| {
            format!(
                r#"<Layer><Name>{}</Name><Title>{}</Title></Layer>"#,
                m,
                m.to_uppercase()
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
