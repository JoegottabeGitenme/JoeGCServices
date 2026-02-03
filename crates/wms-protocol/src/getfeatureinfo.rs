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

    fn create_test_feature() -> FeatureInfo {
        FeatureInfo {
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
        }
    }

    #[test]
    fn test_pixel_to_geographic() {
        // Center of 256x256 map with bbox [-180, -90, 180, 90]
        let (lon, lat) = pixel_to_geographic(128, 128, 256, 256, [-180.0, -90.0, 180.0, 90.0]);
        assert!((lon - 0.0).abs() < 1.0);
        assert!((lat - 0.0).abs() < 1.0);
    }

    #[test]
    fn test_pixel_to_geographic_corners() {
        let bbox = [-180.0, -90.0, 180.0, 90.0];

        // Top-left corner (0, 0) - should be near (-180, 90)
        let (lon, lat) = pixel_to_geographic(0, 0, 256, 256, bbox);
        assert!(lon < -170.0, "Top-left lon should be near -180");
        assert!(lat > 80.0, "Top-left lat should be near 90");

        // Bottom-right corner (255, 255) - should be near (180, -90)
        let (lon, lat) = pixel_to_geographic(255, 255, 256, 256, bbox);
        assert!(lon > 170.0, "Bottom-right lon should be near 180");
        assert!(lat < -80.0, "Bottom-right lat should be near -90");
    }

    #[test]
    fn test_pixel_to_geographic_conus() {
        // Typical CONUS bbox
        let bbox = [-130.0, 20.0, -60.0, 55.0];
        let (lon, lat) = pixel_to_geographic(128, 128, 256, 256, bbox);
        assert!(lon > -100.0 && lon < -90.0, "Should be in central US");
        assert!(lat > 35.0 && lat < 40.0, "Should be in central US");
    }

    #[test]
    fn test_info_format_parsing() {
        assert_eq!(
            InfoFormat::from_mime("application/json"),
            Some(InfoFormat::Json)
        );
        assert_eq!(InfoFormat::from_mime("text/html"), Some(InfoFormat::Html));
        assert_eq!(InfoFormat::from_mime("TEXT/HTML"), Some(InfoFormat::Html));
        assert_eq!(InfoFormat::from_mime("text/xml"), Some(InfoFormat::Xml));
        assert_eq!(InfoFormat::from_mime("text/plain"), Some(InfoFormat::Text));
        assert_eq!(InfoFormat::from_mime("unknown"), None);
    }

    #[test]
    fn test_info_format_to_mime() {
        assert_eq!(InfoFormat::Json.to_mime(), "application/json");
        assert_eq!(InfoFormat::Html.to_mime(), "text/html");
        assert_eq!(InfoFormat::Xml.to_mime(), "text/xml");
        assert_eq!(InfoFormat::Text.to_mime(), "text/plain");
    }

    #[test]
    fn test_info_format_default() {
        let format: InfoFormat = Default::default();
        assert_eq!(format, InfoFormat::Html);
    }

    #[test]
    fn test_feature_info_response_json() {
        let response = FeatureInfoResponse::new(vec![create_test_feature()]);

        let json = response.to_json().unwrap();
        assert!(json.contains("FeatureInfoResponse"));
        assert!(json.contains("Temperature"));
        assert!(json.contains("500 mb"));
        assert!(json.contains("test_layer"));
    }

    #[test]
    fn test_feature_info_response_html() {
        let response = FeatureInfoResponse::new(vec![create_test_feature()]);

        let html = response.to_html();
        assert!(html.contains("<div class=\"feature-info\">"));
        assert!(html.contains("<h4>test_layer</h4>"));
        assert!(html.contains("Temperature"));
        assert!(html.contains("15.50"));
        assert!(html.contains("500 mb"));
        assert!(html.contains("+3 hours"));
    }

    #[test]
    fn test_feature_info_response_text() {
        let response = FeatureInfoResponse::new(vec![create_test_feature()]);

        let text = response.to_text();
        assert!(text.contains("Layer: test_layer"));
        assert!(text.contains("Parameter: Temperature"));
        assert!(text.contains("Value: 15.50 °C"));
        assert!(text.contains("Level: 500 mb"));
        assert!(text.contains("Forecast: +3 hours"));
    }

    #[test]
    fn test_feature_info_response_xml() {
        let response = FeatureInfoResponse::new(vec![create_test_feature()]);

        let xml = response.to_xml();
        assert!(xml.contains("<?xml version=\"1.0\""));
        assert!(xml.contains("<FeatureInfoResponse>"));
        assert!(xml.contains("<LayerName>test_layer</LayerName>"));
        assert!(xml.contains("<Parameter>Temperature</Parameter>"));
        assert!(xml.contains("<Value>15.50</Value>"));
        assert!(xml.contains("<Level>500 mb</Level>"));
        assert!(xml.contains("<ForecastHour>3</ForecastHour>"));
    }

    #[test]
    fn test_feature_info_response_multiple_features() {
        let mut feature2 = create_test_feature();
        feature2.layer_name = "second_layer".to_string();
        feature2.parameter = "Wind".to_string();

        let response = FeatureInfoResponse::new(vec![create_test_feature(), feature2]);

        // Test text format with separator
        let text = response.to_text();
        assert!(text.contains("---")); // Separator between features
        assert!(text.contains("test_layer"));
        assert!(text.contains("second_layer"));
    }

    #[test]
    fn test_feature_info_without_optional_fields() {
        let feature = FeatureInfo {
            layer_name: "test".to_string(),
            parameter: "Test".to_string(),
            value: 10.0,
            unit: "m/s".to_string(),
            raw_value: 10.0,
            raw_unit: "m/s".to_string(),
            location: Location {
                longitude: 0.0,
                latitude: 0.0,
            },
            forecast_hour: None,
            reference_time: None,
            level: None,
        };

        let response = FeatureInfoResponse::new(vec![feature]);

        // Should not contain optional fields
        let json = response.to_json().unwrap();
        assert!(!json.contains("forecast_hour"));
        assert!(!json.contains("level"));
    }

    #[test]
    fn test_mercator_to_wgs84_origin() {
        let (lon, lat) = mercator_to_wgs84(0.0, 0.0);
        assert!((lon - 0.0).abs() < 0.001);
        assert!((lat - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_mercator_to_wgs84_bounds() {
        // Test at Web Mercator bounds
        let (lon, _lat) = mercator_to_wgs84(20037508.34, 0.0);
        assert!((lon - 180.0).abs() < 0.01);

        let (lon, _lat) = mercator_to_wgs84(-20037508.34, 0.0);
        assert!((lon - (-180.0)).abs() < 0.01);
    }

    #[test]
    fn test_location_struct() {
        let loc = Location {
            longitude: -95.5,
            latitude: 40.0,
        };
        assert_eq!(loc.longitude, -95.5);
        assert_eq!(loc.latitude, 40.0);
    }
}
