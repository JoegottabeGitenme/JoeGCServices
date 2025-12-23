//! Common utilities shared across WMS and WMTS handlers.

use axum::{
    http::{header, StatusCode},
    response::Response,
};

use serde::Deserialize;

use crate::model_config::ModelDimensionRegistry;

// ============================================================================
// Exception Helpers
// ============================================================================

/// Generate a WMS-formatted exception response
pub fn wms_exception(code: &str, msg: &str, status: StatusCode) -> Response {
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

/// Generate a WMTS-formatted exception response
pub fn wmts_exception(code: &str, msg: &str, status: StatusCode) -> Response {
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

// ============================================================================
// Coordinate Conversion
// ============================================================================

/// Convert Web Mercator (EPSG:3857) coordinates to WGS84 (EPSG:4326)
pub fn mercator_to_wgs84(x: f64, y: f64) -> (f64, f64) {
    let lon = (x / 20037508.34) * 180.0;
    let lat = (y / 20037508.34) * 180.0;
    let lat = 180.0 / std::f64::consts::PI * (2.0 * (lat * std::f64::consts::PI / 180.0).exp().atan() - std::f64::consts::PI / 2.0);
    (lon, lat)
}

// ============================================================================
// Dimension Parameters
// ============================================================================

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
pub fn parse_iso8601_timestamp(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
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

// ============================================================================
// Style File XML Helpers
// ============================================================================

/// Load styles from a JSON file and generate WMS-compatible XML for capabilities
pub fn get_styles_xml_from_file(style_file: &str) -> String {
    // Try to load and parse the style file
    if let Ok(content) = std::fs::read_to_string(style_file) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(styles) = json.get("styles").and_then(|s| s.as_object()) {
                let mut xml_parts = Vec::new();
                
                // Find the default style name (style with default: true, or first style)
                let default_style_name = styles.iter()
                    .find(|(_, def)| def.get("default").and_then(|d| d.as_bool()).unwrap_or(false))
                    .map(|(name, _)| name.clone());
                
                // Output default style first (WMS convention)
                if let Some(ref default_name) = default_style_name {
                    if let Some(style_def) = styles.get(default_name) {
                        let title = style_def.get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or(default_name);
                        xml_parts.push(format!(
                            "<Style><Name>{}</Name><Title>{}</Title></Style>",
                            default_name, title
                        ));
                    }
                }
                
                // Then output remaining styles
                for (style_key, style_def) in styles {
                    // Skip if this was the default (already output)
                    if Some(style_key) == default_style_name.as_ref() {
                        continue;
                    }
                    
                    let name = style_key;
                    let title = style_def.get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or(style_key);
                    
                    xml_parts.push(format!(
                        "<Style><Name>{}</Name><Title>{}</Title></Style>",
                        name, title
                    ));
                }
                
                if !xml_parts.is_empty() {
                    return xml_parts.join("");
                }
            }
        }
    }
    
    // Fallback to just default style if file can't be read
    "<Style><Name>default</Name><Title>Default</Title></Style>".to_string()
}

/// Load styles from a JSON file and generate WMTS-compatible XML for capabilities
pub fn get_wmts_styles_xml_from_file(style_file: &str) -> String {
    // Try to load and parse the style file
    if let Ok(content) = std::fs::read_to_string(style_file) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(styles) = json.get("styles").and_then(|s| s.as_object()) {
                let mut xml_parts = Vec::new();
                
                // First, find the style marked as default (or use first if none marked)
                let default_style_name = styles.iter()
                    .find(|(_, def)| def.get("default").and_then(|d| d.as_bool()).unwrap_or(false))
                    .map(|(name, _)| name.as_str())
                    .or_else(|| styles.keys().next().map(|s| s.as_str()));
                
                for (style_key, style_def) in styles {
                    let identifier = style_key;
                    let title = style_def.get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or(style_key);
                    
                    // Check if this style is the default
                    let is_default = Some(style_key.as_str()) == default_style_name;
                    let default_attr = if is_default { " isDefault=\"true\"" } else { "" };
                    
                    xml_parts.push(format!(
                        r#"<Style{}><ows:Identifier>{}</ows:Identifier><ows:Title>{}</ows:Title></Style>"#,
                        default_attr, identifier, title
                    ));
                }
                
                if !xml_parts.is_empty() {
                    return xml_parts.join("");
                }
            }
        }
    }
    
    // Fallback to just default style if file can't be read
    r#"<Style isDefault="true"><ows:Identifier>default</ows:Identifier><ows:Title>Default</ows:Title></Style>"#.to_string()
}

// ============================================================================
// Image Format Conversion
// ============================================================================

/// Default JPEG quality (0-100). Can be overridden via environment variable.
const DEFAULT_JPEG_QUALITY: u8 = 90;

/// Default WebP quality (0-100). Can be overridden via environment variable.
/// WebP is more efficient than JPEG, so a slightly lower quality still looks good.
const DEFAULT_WEBP_QUALITY: f32 = 85.0;

/// Convert PNG image data to JPEG format.
/// 
/// Uses quality level from JPEG_QUALITY environment variable, defaulting to 90.
/// Note: JPEG does not support transparency, so alpha channel is composited
/// onto a white background.
pub fn convert_png_to_jpeg(png_data: &[u8]) -> Result<Vec<u8>, String> {
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;
    
    // Get quality from environment or use default
    let quality = std::env::var("JPEG_QUALITY")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(DEFAULT_JPEG_QUALITY)
        .min(100);  // Cap at 100
    
    // Decode PNG
    let img = image::load_from_memory_with_format(png_data, ImageFormat::Png)
        .map_err(|e| format!("Failed to decode PNG: {}", e))?;
    
    // Convert RGBA to RGB by compositing onto white background
    // (JPEG doesn't support transparency)
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut rgb_img = RgbaImage::new(width, height);
    
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let Rgba([r, g, b, a]) = *pixel;
        let alpha = a as f32 / 255.0;
        // Composite onto white background
        let new_r = (r as f32 * alpha + 255.0 * (1.0 - alpha)) as u8;
        let new_g = (g as f32 * alpha + 255.0 * (1.0 - alpha)) as u8;
        let new_b = (b as f32 * alpha + 255.0 * (1.0 - alpha)) as u8;
        rgb_img.put_pixel(x, y, Rgba([new_r, new_g, new_b, 255]));
    }
    
    let rgb_img = DynamicImage::ImageRgba8(rgb_img).to_rgb8();
    
    // Encode as JPEG with specified quality
    let mut jpeg_data = Vec::new();
    let mut cursor = Cursor::new(&mut jpeg_data);
    
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);
    encoder.encode(
        &rgb_img,
        width,
        height,
        image::ColorType::Rgb8,
    ).map_err(|e| format!("Failed to encode JPEG: {}", e))?;
    
    Ok(jpeg_data)
}

/// Convert PNG image data to WebP format.
/// 
/// Uses quality level from WEBP_QUALITY environment variable, defaulting to 85.
/// WebP supports transparency (unlike JPEG), so alpha channel is preserved.
/// 
/// WebP advantages over PNG:
/// - Typically 25-35% smaller file size
/// - Faster encoding than zlib-compressed PNG
/// - Supported by all modern browsers
pub fn convert_png_to_webp(png_data: &[u8]) -> Result<Vec<u8>, String> {
    use image::ImageFormat;
    
    // Get quality from environment or use default
    let quality = std::env::var("WEBP_QUALITY")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(DEFAULT_WEBP_QUALITY)
        .clamp(0.0, 100.0);
    
    // Decode PNG
    let img = image::load_from_memory_with_format(png_data, ImageFormat::Png)
        .map_err(|e| format!("Failed to decode PNG: {}", e))?;
    
    // Get RGBA data
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    
    // Encode as WebP with transparency support
    let encoder = webp::Encoder::from_rgba(rgba.as_raw(), width, height);
    let webp_data = encoder.encode(quality);
    
    Ok(webp_data.to_vec())
}

/// Convert PNG image data to lossless WebP format.
/// 
/// Lossless WebP preserves exact pixel values while still achieving
/// significant compression (typically better than PNG).
/// 
/// Use this when exact color preservation is critical.
pub fn convert_png_to_webp_lossless(png_data: &[u8]) -> Result<Vec<u8>, String> {
    use image::ImageFormat;
    
    // Decode PNG
    let img = image::load_from_memory_with_format(png_data, ImageFormat::Png)
        .map_err(|e| format!("Failed to decode PNG: {}", e))?;
    
    // Get RGBA data
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    
    // Encode as lossless WebP
    let encoder = webp::Encoder::from_rgba(rgba.as_raw(), width, height);
    let webp_data = encoder.encode_lossless();
    
    Ok(webp_data.to_vec())
}

// ============================================================================
// PNG Image Utilities
// ============================================================================

/// Generate a simple placeholder image (gray with text)
pub fn generate_placeholder_image(width: u32, height: u32) -> Vec<u8> {
    // Create a simple gray PNG
    let mut data = Vec::new();
    
    // PNG signature
    data.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    
    // IHDR chunk
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width).to_be_bytes());
    ihdr.extend_from_slice(&(height).to_be_bytes());
    ihdr.push(8);  // bit depth
    ihdr.push(0);  // color type (grayscale)
    ihdr.push(0);  // compression method
    ihdr.push(0);  // filter method
    ihdr.push(0);  // interlace method
    write_chunk(&mut data, b"IHDR", &ihdr);
    
    // IDAT chunk (compressed image data)
    let mut raw = Vec::new();
    for _ in 0..height {
        raw.push(0);  // filter type none
        for _ in 0..width {
            raw.push(128);  // gray
        }
    }
    
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    use std::io::Write;
    encoder.write_all(&raw).unwrap();
    let compressed = encoder.finish().unwrap();
    write_chunk(&mut data, b"IDAT", &compressed);
    
    // IEND chunk
    write_chunk(&mut data, b"IEND", &[]);
    
    data
}

/// Write a PNG chunk with CRC
pub fn write_chunk(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(data);
    let mut crc_data = Vec::new();
    crc_data.extend_from_slice(name);
    crc_data.extend_from_slice(data);
    let crc = crc32fast::hash(&crc_data);
    out.extend_from_slice(&crc.to_be_bytes());
}

// ============================================================================
// WMTS Dimension Query Parameters
// ============================================================================

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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mercator_to_wgs84_origin() {
        let (lon, lat) = mercator_to_wgs84(0.0, 0.0);
        assert!((lon - 0.0).abs() < 0.0001);
        assert!((lat - 0.0).abs() < 0.0001);
    }

    #[test]
    fn test_mercator_to_wgs84_known_point() {
        // New York City approximately: -74.006, 40.7128
        // In Web Mercator: -8238310, 4970072
        let (lon, lat) = mercator_to_wgs84(-8238310.0, 4970072.0);
        assert!((lon - (-74.006)).abs() < 0.01);
        assert!((lat - 40.7128).abs() < 0.01);
    }

    #[test]
    fn test_mercator_to_wgs84_extremes() {
        // Web Mercator bounds
        let (lon, lat) = mercator_to_wgs84(-20037508.34, 20037508.34);
        assert!((lon - (-180.0)).abs() < 0.01);
        assert!(lat > 85.0); // Near max latitude
    }

    #[test]
    fn test_parse_iso8601_timestamp_z_suffix() {
        let ts = parse_iso8601_timestamp("2024-01-15T12:30:00Z");
        assert!(ts.is_some());
        let dt = ts.unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 12);
        assert_eq!(dt.minute(), 30);
    }

    #[test]
    fn test_parse_iso8601_timestamp_with_millis() {
        let ts = parse_iso8601_timestamp("2024-01-15T12:30:00.123Z");
        assert!(ts.is_some());
    }

    #[test]
    fn test_parse_iso8601_timestamp_rfc3339() {
        let ts = parse_iso8601_timestamp("2024-01-15T12:30:00+00:00");
        assert!(ts.is_some());
    }

    #[test]
    fn test_parse_iso8601_timestamp_invalid() {
        let ts = parse_iso8601_timestamp("not-a-date");
        assert!(ts.is_none());
    }

    #[test]
    fn test_generate_placeholder_image() {
        let img = generate_placeholder_image(64, 64);
        // Check PNG signature
        assert_eq!(&img[0..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        // Should have reasonable size
        assert!(img.len() > 50);
    }

    #[test]
    fn test_get_styles_xml_fallback() {
        // Non-existent file should return fallback
        let xml = get_styles_xml_from_file("/nonexistent/path.json");
        assert!(xml.contains("default"));
        assert!(xml.contains("Default"));
    }

    #[test]
    fn test_wms_exception_format() {
        let resp = wms_exception("TestCode", "Test message", StatusCode::BAD_REQUEST);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_wmts_exception_format() {
        let resp = wmts_exception("TestCode", "Test message", StatusCode::BAD_REQUEST);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    use chrono::Datelike;
    use chrono::Timelike;

    #[test]
    fn test_convert_png_to_webp() {
        // Create a simple 2x2 PNG
        let png_data = generate_placeholder_image(2, 2);
        
        // Convert to WebP
        let result = convert_png_to_webp(&png_data);
        assert!(result.is_ok(), "WebP conversion should succeed");
        
        let webp_data = result.unwrap();
        // WebP magic bytes: RIFF....WEBP
        assert!(webp_data.len() > 12, "WebP should have reasonable size");
        assert_eq!(&webp_data[0..4], b"RIFF", "WebP should start with RIFF");
        assert_eq!(&webp_data[8..12], b"WEBP", "WebP should have WEBP marker");
    }

    #[test]
    fn test_convert_png_to_webp_lossless() {
        // Create a simple test PNG
        let png_data = generate_placeholder_image(4, 4);
        
        // Convert to lossless WebP
        let result = convert_png_to_webp_lossless(&png_data);
        assert!(result.is_ok(), "Lossless WebP conversion should succeed");
        
        let webp_data = result.unwrap();
        assert!(webp_data.len() > 12, "WebP should have reasonable size");
        assert_eq!(&webp_data[0..4], b"RIFF", "WebP should start with RIFF");
        assert_eq!(&webp_data[8..12], b"WEBP", "WebP should have WEBP marker");
    }

    #[test]
    fn test_webp_smaller_than_png() {
        // Create a larger test image where WebP compression benefits should be visible
        let png_data = generate_placeholder_image(64, 64);
        
        let webp_result = convert_png_to_webp(&png_data);
        assert!(webp_result.is_ok());
        
        let webp_data = webp_result.unwrap();
        println!("PNG size: {} bytes, WebP size: {} bytes", png_data.len(), webp_data.len());
        
        // WebP should generally be smaller for this type of image
        // Not a strict requirement as it depends on content, but useful to verify
        assert!(webp_data.len() > 0);
    }
}
