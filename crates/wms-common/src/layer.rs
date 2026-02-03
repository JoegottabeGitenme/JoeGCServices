//! Layer definitions and metadata for WMS services.

use crate::{BoundingBox, CrsCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Unique identifier for a layer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LayerId(pub String);

impl LayerId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Parse a compound layer ID like "gfs:temperature_2m"
    pub fn parse(s: &str) -> (Option<&str>, &str) {
        match s.split_once(':') {
            Some((model, param)) => (Some(model), param),
            None => (None, s),
        }
    }
}

impl std::fmt::Display for LayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A WMS layer definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    /// Unique layer identifier
    pub id: LayerId,

    /// Human-readable title for GetCapabilities
    pub title: String,

    /// Optional description/abstract
    pub description: Option<String>,

    /// Supported coordinate reference systems
    pub supported_crs: Vec<CrsCode>,

    /// Geographic bounding box (always in EPSG:4326)
    pub geographic_bbox: BoundingBox,

    /// Available styles for this layer
    pub styles: Vec<LayerStyle>,

    /// Time dimension info (if applicable)
    pub time_dimension: Option<TimeDimension>,

    /// Elevation dimension info (if applicable)
    pub elevation_dimension: Option<ElevationDimension>,

    /// Data source metadata
    pub metadata: LayerMetadata,
}

/// Style definition for a layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerStyle {
    /// Style name (used in GetMap requests)
    pub name: String,

    /// Human-readable title
    pub title: String,

    /// Style configuration
    pub config: StyleConfig,
}

/// Style rendering configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StyleConfig {
    /// Continuous gradient color mapping
    Gradient {
        /// Color stops: [(value, r, g, b, a), ...]
        color_stops: Vec<ColorStop>,
        /// Units for the legend
        units: String,
    },

    /// Discrete classification
    Classified {
        /// Classification breaks and colors
        classes: Vec<ClassBreak>,
    },

    /// Contour/isoline rendering
    Contour {
        /// Contour interval
        interval: f64,
        /// Line color
        color: [u8; 4],
        /// Line width in pixels
        line_width: f32,
        /// Whether to label contours
        labels: bool,
    },

    /// Wind barb rendering
    WindBarbs {
        /// Spacing between barbs in pixels
        spacing: u32,
        /// Barb color
        color: [u8; 4],
    },

    /// Wind arrow rendering
    WindArrows {
        /// Spacing between arrows in pixels
        spacing: u32,
        /// Color ramp for speed
        color_stops: Vec<ColorStop>,
    },
}

/// A color stop in a gradient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorStop {
    pub value: f64,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// A classification break.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassBreak {
    pub min: f64,
    pub max: f64,
    pub color: [u8; 4],
    pub label: String,
}

/// Time dimension configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeDimension {
    /// Default time value
    pub default: TimeDefault,

    /// Available times (ISO 8601 format)
    /// Can be explicit list or interval notation
    pub extent: TimeExtent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimeDefault {
    /// Use the most recent available time
    Current,
    /// Use a specific time
    Fixed(DateTime<Utc>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimeExtent {
    /// Explicit list of available times
    List(Vec<DateTime<Utc>>),

    /// Interval notation: start/end/resolution
    Interval {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        resolution: chrono::Duration,
    },
}

/// Elevation dimension configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevationDimension {
    /// Default elevation
    pub default: f64,

    /// Available elevations
    pub values: Vec<f64>,

    /// Units (e.g., "hPa", "m")
    pub units: String,
}

/// Metadata about the layer's data source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerMetadata {
    /// Data model/source (e.g., "GFS", "HRRR", "NAM")
    pub model: String,

    /// Parameter name in the source data
    pub parameter: String,

    /// Level/surface description
    pub level: String,

    /// Native grid specification
    pub native_grid: Option<String>,

    /// Update frequency
    pub update_frequency: Option<String>,

    /// Attribution/credits
    pub attribution: Option<String>,
}

impl Layer {
    /// Check if this layer supports a given CRS.
    pub fn supports_crs(&self, crs: &CrsCode) -> bool {
        self.supported_crs.contains(crs)
    }

    /// Get the default style for this layer.
    pub fn default_style(&self) -> Option<&LayerStyle> {
        self.styles.first()
    }

    /// Find a style by name.
    pub fn get_style(&self, name: &str) -> Option<&LayerStyle> {
        self.styles.iter().find(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // LayerId Tests
    // =========================================================================

    #[test]
    fn test_layer_id_new() {
        let id = LayerId::new("gfs:temperature_2m");
        assert_eq!(id.0, "gfs:temperature_2m");
    }

    #[test]
    fn test_layer_id_display() {
        let id = LayerId::new("hrrr:wind_speed");
        assert_eq!(format!("{}", id), "hrrr:wind_speed");
    }

    #[test]
    fn test_layer_id_parse_compound() {
        let (model, param) = LayerId::parse("gfs:temperature_2m");
        assert_eq!(model, Some("gfs"));
        assert_eq!(param, "temperature_2m");
    }

    #[test]
    fn test_layer_id_parse_compound_with_colons() {
        // Only splits on first colon
        let (model, param) = LayerId::parse("model:param:extra");
        assert_eq!(model, Some("model"));
        assert_eq!(param, "param:extra");
    }

    #[test]
    fn test_layer_id_parse_simple() {
        let (model, param) = LayerId::parse("temperature_2m");
        assert_eq!(model, None);
        assert_eq!(param, "temperature_2m");
    }

    #[test]
    fn test_layer_id_parse_empty() {
        let (model, param) = LayerId::parse("");
        assert_eq!(model, None);
        assert_eq!(param, "");
    }

    #[test]
    fn test_layer_id_equality() {
        let id1 = LayerId::new("gfs:temp");
        let id2 = LayerId::new("gfs:temp");
        let id3 = LayerId::new("hrrr:temp");
        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    // =========================================================================
    // Layer Tests
    // =========================================================================

    /// Helper to create a test layer
    fn create_test_layer(styles: Vec<LayerStyle>, supported_crs: Vec<CrsCode>) -> Layer {
        Layer {
            id: LayerId::new("test:layer"),
            title: "Test Layer".to_string(),
            description: Some("A test layer".to_string()),
            supported_crs,
            geographic_bbox: BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
            styles,
            time_dimension: None,
            elevation_dimension: None,
            metadata: LayerMetadata {
                model: "TEST".to_string(),
                parameter: "test_param".to_string(),
                level: "surface".to_string(),
                native_grid: None,
                update_frequency: None,
                attribution: None,
            },
        }
    }

    /// Helper to create a test style
    fn create_test_style(name: &str) -> LayerStyle {
        LayerStyle {
            name: name.to_string(),
            title: format!("{} Style", name),
            config: StyleConfig::Gradient {
                color_stops: vec![
                    ColorStop {
                        value: 0.0,
                        r: 0,
                        g: 0,
                        b: 255,
                        a: 255,
                    },
                    ColorStop {
                        value: 100.0,
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                ],
                units: "units".to_string(),
            },
        }
    }

    #[test]
    fn test_layer_supports_crs_found() {
        let layer = create_test_layer(vec![], vec![CrsCode::Epsg4326, CrsCode::Epsg3857]);
        assert!(layer.supports_crs(&CrsCode::Epsg4326));
        assert!(layer.supports_crs(&CrsCode::Epsg3857));
    }

    #[test]
    fn test_layer_supports_crs_not_found() {
        let layer = create_test_layer(vec![], vec![CrsCode::Epsg4326]);
        assert!(!layer.supports_crs(&CrsCode::Epsg3857));
        assert!(!layer.supports_crs(&CrsCode::Epsg5070));
    }

    #[test]
    fn test_layer_supports_crs_empty() {
        let layer = create_test_layer(vec![], vec![]);
        assert!(!layer.supports_crs(&CrsCode::Epsg4326));
    }

    #[test]
    fn test_layer_default_style_with_styles() {
        let styles = vec![create_test_style("default"), create_test_style("alternate")];
        let layer = create_test_layer(styles, vec![]);

        let default = layer.default_style().unwrap();
        assert_eq!(default.name, "default");
    }

    #[test]
    fn test_layer_default_style_empty() {
        let layer = create_test_layer(vec![], vec![]);
        assert!(layer.default_style().is_none());
    }

    #[test]
    fn test_layer_get_style_found() {
        let styles = vec![
            create_test_style("style_a"),
            create_test_style("style_b"),
            create_test_style("style_c"),
        ];
        let layer = create_test_layer(styles, vec![]);

        let style = layer.get_style("style_b").unwrap();
        assert_eq!(style.name, "style_b");
    }

    #[test]
    fn test_layer_get_style_not_found() {
        let styles = vec![create_test_style("existing")];
        let layer = create_test_layer(styles, vec![]);

        assert!(layer.get_style("nonexistent").is_none());
    }

    #[test]
    fn test_layer_get_style_empty() {
        let layer = create_test_layer(vec![], vec![]);
        assert!(layer.get_style("any").is_none());
    }
}
