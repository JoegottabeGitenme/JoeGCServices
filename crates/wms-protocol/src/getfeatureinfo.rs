//! WMS GetFeatureInfo handling
//!
//! Implements OGC WMS 1.3.0 GetFeatureInfo operation for querying
//! data values at specific map points.
//!
// TODO ask claude about if this is duplicate, consider this to be the place to handle custom getFeatureInfo rendering

use serde::{Deserialize, Serialize};

/// GetFeatureInfo request parameters
#[derive(Debug, Clone, Deserialize)]
pub struct GetFeatureInfoRequest {
    /// Layers to display (same as GetMap)
    pub layers: Vec<String>,
    /// Layers to query for information
    pub query_layers: Vec<String>,
    /// Coordinate reference system
    pub crs: String,
    /// Bounding box [min_lon, min_lat, max_lon, max_lat]
    pub bbox: [f64; 4],
    /// Map width in pixels
    pub width: u32,
    /// Map height in pixels
    pub height: u32,
    /// Pixel column (X coordinate, 0-based from left)
    pub i: u32,
    /// Pixel row (Y coordinate, 0-based from top)
    pub j: u32,
    /// Response format
    pub info_format: InfoFormat,
    /// Maximum number of features to return
    pub feature_count: Option<u32>,
}

/// Supported GetFeatureInfo response formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
pub enum InfoFormat {
    /// application/json - Machine-readable JSON
    #[serde(rename = "application/json")]
    Json,
    /// text/html - Human-readable HTML for popups
    #[serde(rename = "text/html")]
    #[default]
    Html,
    /// text/xml - OGC-compliant XML
    #[serde(rename = "text/xml")]
    Xml,
    /// text/plain - Simple text format
    #[serde(rename = "text/plain")]
    Text,
}

impl InfoFormat {
    /// Parse from MIME type string
    pub fn from_mime(mime: &str) -> Option<Self> {
        match mime.to_lowercase().as_str() {
            "application/json" => Some(InfoFormat::Json),
            "text/html" => Some(InfoFormat::Html),
            "text/xml" => Some(InfoFormat::Xml),
            "text/plain" => Some(InfoFormat::Text),
            _ => None,
        }
    }

    /// Get MIME type string
    pub fn to_mime(&self) -> &'static str {
        match self {
            InfoFormat::Json => "application/json",
            InfoFormat::Html => "text/html",
            InfoFormat::Xml => "text/xml",
            InfoFormat::Text => "text/plain",
        }
    }
}

/// Feature information for a single layer at a point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureInfo {
    /// Layer name (e.g., "gfs_TMP")
    pub layer_name: String,
    /// Parameter display name (e.g., "Temperature")
    pub parameter: String,
    /// Converted value for display
    pub value: f64,
    /// Display unit (e.g., "°C", "hPa")
    pub unit: String,
    /// Raw value from GRIB
    pub raw_value: f64,
    /// Raw unit from GRIB (e.g., "K", "Pa")
    pub raw_unit: String,
    /// Query location [longitude, latitude]
    pub location: Location,
    /// Forecast hour
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forecast_hour: Option<u32>,
    /// Reference time (run time)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_time: Option<String>,
    /// Vertical level/elevation (e.g., "500 mb", "2 m above ground")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
}

/// Geographic location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub longitude: f64,
    pub latitude: f64,
}

/// GetFeatureInfo response container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureInfoResponse {
    /// Response type identifier
    #[serde(rename = "type")]
    pub response_type: String,
    /// List of feature information
    pub features: Vec<FeatureInfo>,
}

impl FeatureInfoResponse {
    /// Create new response with features
    pub fn new(features: Vec<FeatureInfo>) -> Self {
        Self {
            response_type: "FeatureInfoResponse".to_string(),
            features,
        }
    }

    /// Format as JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Format as HTML for popup display
    pub fn to_html(&self) -> String {
        let mut html = String::from("<div class=\"feature-info\">\n");

        for feature in &self.features {
            html.push_str(&format!("  <h4>{}</h4>\n", feature.layer_name));
            html.push_str("  <table>\n");
            html.push_str(&format!(
                "    <tr><td>Parameter:</td><td class=\"value\">{}</td></tr>\n",
                feature.parameter
            ));
            html.push_str(&format!(
                "    <tr><td>Value:</td><td class=\"value\">{:.2} {}</td></tr>\n",
                feature.value, feature.unit
            ));
            if let Some(ref level) = feature.level {
                html.push_str(&format!(
                    "    <tr><td>Level:</td><td class=\"value\">{}</td></tr>\n",
                    level
                ));
            }
            html.push_str(&format!(
                "    <tr><td>Location:</td><td class=\"value\">{:.3}°, {:.3}°</td></tr>\n",
                feature.location.latitude, feature.location.longitude
            ));
            if let Some(hour) = feature.forecast_hour {
                html.push_str(&format!(
                    "    <tr><td>Forecast:</td><td class=\"value\">+{} hours</td></tr>\n",
                    hour
                ));
            }
            html.push_str("  </table>\n");
        }

        html.push_str("</div>");
        html
    }

    /// Format as plain text
    pub fn to_text(&self) -> String {
        let mut text = String::new();

        for (i, feature) in self.features.iter().enumerate() {
            if i > 0 {
                text.push_str("\n---\n");
            }
            text.push_str(&format!("Layer: {}\n", feature.layer_name));
            text.push_str(&format!("Parameter: {}\n", feature.parameter));
            text.push_str(&format!("Value: {:.2} {}\n", feature.value, feature.unit));
            if let Some(ref level) = feature.level {
                text.push_str(&format!("Level: {}\n", level));
            }
            text.push_str(&format!(
                "Location: {:.3}°N, {:.3}°E\n",
                feature.location.latitude, feature.location.longitude
            ));
            if let Some(hour) = feature.forecast_hour {
                text.push_str(&format!("Forecast: +{} hours\n", hour));
            }
        }

        text
    }

    /// Format as XML
    pub fn to_xml(&self) -> String {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<FeatureInfoResponse>\n");

        for feature in &self.features {
            xml.push_str("  <FeatureInfo>\n");
            xml.push_str(&format!(
                "    <LayerName>{}</LayerName>\n",
                feature.layer_name
            ));
            xml.push_str(&format!(
                "    <Parameter>{}</Parameter>\n",
                feature.parameter
            ));
            xml.push_str(&format!("    <Value>{:.2}</Value>\n", feature.value));
            xml.push_str(&format!("    <Unit>{}</Unit>\n", feature.unit));
            if let Some(ref level) = feature.level {
                xml.push_str(&format!("    <Level>{}</Level>\n", level));
            }
            xml.push_str(&format!(
                "    <Location longitude=\"{:.3}\" latitude=\"{:.3}\"/>\n",
                feature.location.longitude, feature.location.latitude
            ));
            if let Some(hour) = feature.forecast_hour {
                xml.push_str(&format!("    <ForecastHour>{}</ForecastHour>\n", hour));
            }
            xml.push_str("  </FeatureInfo>\n");
        }

        xml.push_str("</FeatureInfoResponse>");
        xml
    }
}

/// Convert pixel coordinates to geographic coordinates
///
/// # Arguments
/// - `i`: Pixel column (0-based from left)
/// - `j`: Pixel row (0-based from top)
/// - `width`: Map width in pixels
/// - `height`: Map height in pixels
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat]
///
/// # Returns
/// (longitude, latitude)
pub fn pixel_to_geographic(i: u32, j: u32, width: u32, height: u32, bbox: [f64; 4]) -> (f64, f64) {
    let [min_lon, min_lat, max_lon, max_lat] = bbox;

    // Calculate pixel center position (0.5 offset for pixel center)
    let x_ratio = (i as f64 + 0.5) / width as f64;
    let y_ratio = (j as f64 + 0.5) / height as f64;

    // Convert to geographic coordinates
    let lon = min_lon + x_ratio * (max_lon - min_lon);
    let lat = max_lat - y_ratio * (max_lat - min_lat); // Y is inverted (top=max, bottom=min)

    (lon, lat)
}

/// Convert Web Mercator (EPSG:3857) coordinates to WGS84 (EPSG:4326)
pub fn mercator_to_wgs84(x: f64, y: f64) -> (f64, f64) {
    let lon = (x / 20037508.34) * 180.0;
    let lat = (y / 20037508.34) * 180.0;
    let lat = 180.0 / std::f64::consts::PI
        * (2.0 * (lat * std::f64::consts::PI / 180.0).exp().atan() - std::f64::consts::PI / 2.0);
    (lon, lat)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixel_to_geographic() {
        // Center of 256x256 map with bbox [-180, -90, 180, 90]
        let (lon, lat) = pixel_to_geographic(128, 128, 256, 256, [-180.0, -90.0, 180.0, 90.0]);
        assert!((lon - 0.0).abs() < 1.0);
        assert!((lat - 0.0).abs() < 1.0);
    }

    #[test]
    fn test_info_format_parsing() {
        assert_eq!(
            InfoFormat::from_mime("application/json"),
            Some(InfoFormat::Json)
        );
        assert_eq!(InfoFormat::from_mime("text/html"), Some(InfoFormat::Html));
        assert_eq!(InfoFormat::from_mime("TEXT/HTML"), Some(InfoFormat::Html));
    }

    #[test]
    fn test_feature_info_response_json() {
        let response = FeatureInfoResponse::new(vec![FeatureInfo {
            layer_name: "test_layer".to_string(),
            parameter: "Temperature".to_string(),
            value: 15.5,
            unit: "°C".to_string(),
            raw_value: 288.65,
            raw_unit: "K".to_string(),
            location: Location {
                longitude: -95.0,
                latitude: 40.0,
            },
            forecast_hour: Some(3),
            reference_time: Some("2025-11-26T12:00:00Z".to_string()),
            level: Some("500 mb".to_string()),
        }]);

        let json = response.to_json().unwrap();
        assert!(json.contains("FeatureInfoResponse"));
        assert!(json.contains("Temperature"));
        assert!(json.contains("500 mb"));
    }
}
