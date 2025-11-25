//! Layer definitions and metadata for WMS services.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::{BoundingBox, CrsCode};

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
