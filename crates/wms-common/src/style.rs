//! Style configuration for rendering weather data.
//!
//! This module defines a JSON-based configuration schema for defining
//! how weather parameters should be rendered (gradients, contours, etc.)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Root style configuration - can contain multiple named styles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleConfig {
    /// Version of the style schema
    #[serde(default = "default_version")]
    pub version: String,

    /// Named style definitions
    pub styles: HashMap<String, StyleDefinition>,
}

fn default_version() -> String {
    "1.0".to_string()
}

impl StyleConfig {
    /// Load style configuration from a JSON file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, StyleError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| StyleError::IoError(e.to_string()))?;
        Self::from_json(&content)
    }

    /// Parse style configuration from JSON string.
    pub fn from_json(json: &str) -> Result<Self, StyleError> {
        serde_json::from_str(json).map_err(|e| StyleError::ParseError(e.to_string()))
    }

    /// Get a style by name.
    pub fn get(&self, name: &str) -> Option<&StyleDefinition> {
        self.styles.get(name)
    }

    /// Validate all styles in the configuration.
    pub fn validate(&self) -> Result<(), StyleError> {
        for (name, style) in &self.styles {
            style
                .validate()
                .map_err(|e| StyleError::ValidationError(format!("{}: {}", name, e)))?;
        }
        Ok(())
    }
}

/// A complete style definition for a layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleDefinition {
    /// Human-readable name
    pub name: String,

    /// Description of the style
    #[serde(default)]
    pub description: String,

    /// The rendering type
    #[serde(flatten)]
    pub renderer: RendererConfig,

    /// Optional legend configuration
    #[serde(default)]
    pub legend: Option<LegendConfig>,

    /// Unit label for display
    #[serde(default)]
    pub units: Option<String>,

    /// Value transformation before rendering
    #[serde(default)]
    pub transform: Option<ValueTransform>,
}

impl StyleDefinition {
    pub fn validate(&self) -> Result<(), String> {
        self.renderer.validate()
    }
}

/// Configuration for different renderer types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RendererConfig {
    /// Continuous color gradient
    Gradient(GradientConfig),

    /// Discrete color classification
    Classified(ClassifiedConfig),

    /// Contour/isoline rendering
    Contour(ContourConfig),

    /// Wind barb rendering
    WindBarbs(WindBarbConfig),

    /// Wind arrow/vector rendering
    WindArrows(WindArrowConfig),

    /// Filled contours (like radar)
    FilledContour(FilledContourConfig),
}

impl RendererConfig {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            RendererConfig::Gradient(g) => g.validate(),
            RendererConfig::Classified(c) => c.validate(),
            RendererConfig::Contour(c) => c.validate(),
            RendererConfig::FilledContour(f) => f.validate(),
            _ => Ok(()),
        }
    }
}

/// Continuous gradient color mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradientConfig {
    /// Color stops defining the gradient
    pub stops: Vec<ColorStop>,

    /// How to interpolate between stops
    #[serde(default)]
    pub interpolation: Interpolation,

    /// How to handle values outside the defined range
    #[serde(default)]
    pub out_of_range: OutOfRangeBehavior,

    /// Optional value for transparent/no-data
    #[serde(default)]
    pub no_data_value: Option<f64>,

    /// Color for no-data pixels
    #[serde(default)]
    pub no_data_color: Option<Color>,
}

impl GradientConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.stops.len() < 2 {
            return Err("Gradient must have at least 2 color stops".to_string());
        }

        // Check stops are in ascending order
        for i in 1..self.stops.len() {
            if self.stops[i].value <= self.stops[i - 1].value {
                return Err("Color stops must be in ascending value order".to_string());
            }
        }

        Ok(())
    }

    /// Interpolate color for a given value.
    pub fn interpolate(&self, value: f64) -> Color {
        // Handle out of range
        if value < self.stops.first().unwrap().value {
            return match self.out_of_range {
                OutOfRangeBehavior::Clamp => self.stops.first().unwrap().color.clone(),
                OutOfRangeBehavior::Transparent => Color::transparent(),
                OutOfRangeBehavior::Extend => self.stops.first().unwrap().color.clone(),
            };
        }

        if value > self.stops.last().unwrap().value {
            return match self.out_of_range {
                OutOfRangeBehavior::Clamp => self.stops.last().unwrap().color.clone(),
                OutOfRangeBehavior::Transparent => Color::transparent(),
                OutOfRangeBehavior::Extend => self.stops.last().unwrap().color.clone(),
            };
        }

        // Find bracketing stops
        for i in 1..self.stops.len() {
            if value <= self.stops[i].value {
                let low = &self.stops[i - 1];
                let high = &self.stops[i];
                let t = (value - low.value) / (high.value - low.value);
                return low.color.lerp(&high.color, t, &self.interpolation);
            }
        }

        self.stops.last().unwrap().color.clone()
    }
}

/// A color stop in a gradient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorStop {
    /// The data value at this stop
    pub value: f64,

    /// The color at this stop
    pub color: Color,

    /// Optional label for legend
    #[serde(default)]
    pub label: Option<String>,
}

/// Color representation supporting multiple formats.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Color {
    /// Hex string: "#RRGGBB" or "#RRGGBBAA"
    Hex(String),

    /// RGB array: [r, g, b] or [r, g, b, a]
    Array(Vec<u8>),

    /// Named color
    Named(String),

    /// Explicit RGBA
    Rgba { r: u8, g: u8, b: u8, a: u8 },
}

impl Color {
    pub fn transparent() -> Self {
        Color::Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }
    }

    /// Convert to RGBA tuple.
    pub fn to_rgba(&self) -> (u8, u8, u8, u8) {
        match self {
            Color::Hex(s) => parse_hex_color(s),
            Color::Array(arr) => {
                let r = arr.first().copied().unwrap_or(0);
                let g = arr.get(1).copied().unwrap_or(0);
                let b = arr.get(2).copied().unwrap_or(0);
                let a = arr.get(3).copied().unwrap_or(255);
                (r, g, b, a)
            }
            Color::Named(name) => named_color(name),
            Color::Rgba { r, g, b, a } => (*r, *g, *b, *a),
        }
    }

    /// Linear interpolation between two colors.
    pub fn lerp(&self, other: &Color, t: f64, interp: &Interpolation) -> Color {
        let (r1, g1, b1, a1) = self.to_rgba();
        let (r2, g2, b2, a2) = other.to_rgba();

        let t = t.clamp(0.0, 1.0);

        let lerp_u8 = |a: u8, b: u8, t: f64| -> u8 {
            ((a as f64) * (1.0 - t) + (b as f64) * t).round() as u8
        };

        match interp {
            Interpolation::Linear => Color::Rgba {
                r: lerp_u8(r1, r2, t),
                g: lerp_u8(g1, g2, t),
                b: lerp_u8(b1, b2, t),
                a: lerp_u8(a1, a2, t),
            },
            Interpolation::Step => {
                if t < 0.5 {
                    self.clone()
                } else {
                    other.clone()
                }
            }
        }
    }
}

fn parse_hex_color(s: &str) -> (u8, u8, u8, u8) {
    let s = s.trim_start_matches('#');
    let len = s.len();

    if len == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
        (r, g, b, 255)
    } else if len == 8 {
        let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
        let a = u8::from_str_radix(&s[6..8], 16).unwrap_or(255);
        (r, g, b, a)
    } else {
        (0, 0, 0, 255)
    }
}

fn named_color(name: &str) -> (u8, u8, u8, u8) {
    match name.to_lowercase().as_str() {
        "transparent" => (0, 0, 0, 0),
        "black" => (0, 0, 0, 255),
        "white" => (255, 255, 255, 255),
        "red" => (255, 0, 0, 255),
        "green" => (0, 255, 0, 255),
        "blue" => (0, 0, 255, 255),
        "yellow" => (255, 255, 0, 255),
        "cyan" => (0, 255, 255, 255),
        "magenta" => (255, 0, 255, 255),
        "orange" => (255, 165, 0, 255),
        "purple" => (128, 0, 128, 255),
        "gray" | "grey" => (128, 128, 128, 255),
        _ => (0, 0, 0, 255),
    }
}

/// Interpolation method between color stops.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    #[default]
    Linear,
    Step,
}

/// Behavior for values outside the gradient range.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutOfRangeBehavior {
    #[default]
    Clamp,
    Transparent,
    Extend,
}

/// Discrete classification configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifiedConfig {
    /// Classification breaks
    pub classes: Vec<ClassBreak>,

    /// Color for values not in any class
    #[serde(default)]
    pub default_color: Option<Color>,
}

impl ClassifiedConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.classes.is_empty() {
            return Err("Classification must have at least 1 class".to_string());
        }
        Ok(())
    }

    /// Find color for a value.
    pub fn classify(&self, value: f64) -> Option<&Color> {
        for class in &self.classes {
            let in_min = class.min.map(|m| value >= m).unwrap_or(true);
            let in_max = class.max.map(|m| value < m).unwrap_or(true);
            if in_min && in_max {
                return Some(&class.color);
            }
        }
        self.default_color.as_ref()
    }
}

/// A classification break/range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassBreak {
    /// Minimum value (inclusive), None for negative infinity
    #[serde(default)]
    pub min: Option<f64>,

    /// Maximum value (exclusive), None for positive infinity
    #[serde(default)]
    pub max: Option<f64>,

    /// Color for this class
    pub color: Color,

    /// Label for legend
    #[serde(default)]
    pub label: Option<String>,
}

/// Contour line configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContourConfig {
    /// Contour interval
    pub interval: f64,

    /// Optional base value (contours at base + n*interval)
    #[serde(default)]
    pub base: f64,

    /// Line color
    pub color: Color,

    /// Line width in pixels
    #[serde(default = "default_line_width")]
    pub line_width: f32,

    /// Major contour interval (thicker lines)
    #[serde(default)]
    pub major_interval: Option<f64>,

    /// Major line width
    #[serde(default)]
    pub major_line_width: Option<f32>,

    /// Whether to label contours
    #[serde(default)]
    pub labels: bool,

    /// Label font size
    #[serde(default = "default_font_size")]
    pub label_font_size: f32,

    /// Minimum/maximum values to contour
    #[serde(default)]
    pub min_value: Option<f64>,
    #[serde(default)]
    pub max_value: Option<f64>,
}

impl ContourConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.interval <= 0.0 {
            return Err("Contour interval must be positive".to_string());
        }
        Ok(())
    }
}

fn default_line_width() -> f32 {
    1.0
}
fn default_font_size() -> f32 {
    10.0
}

/// Filled contour configuration (like radar imagery).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilledContourConfig {
    /// Levels and their colors
    pub levels: Vec<ContourLevel>,

    /// Color for values below lowest level
    #[serde(default)]
    pub below_color: Option<Color>,

    /// Color for values above highest level
    #[serde(default)]
    pub above_color: Option<Color>,
}

impl FilledContourConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.levels.len() < 2 {
            return Err("Filled contours need at least 2 levels".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContourLevel {
    pub value: f64,
    pub color: Color,
    #[serde(default)]
    pub label: Option<String>,
}

/// Wind barb configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindBarbConfig {
    /// Spacing between barbs in pixels
    #[serde(default = "default_barb_spacing")]
    pub spacing: u32,

    /// Barb color
    pub color: Color,

    /// Barb size in pixels
    #[serde(default = "default_barb_size")]
    pub size: f32,

    /// Whether speed is in knots (true) or m/s (false)
    #[serde(default = "default_true")]
    pub knots: bool,

    /// Calm threshold (below this, show calm circle)
    #[serde(default = "default_calm_threshold")]
    pub calm_threshold: f64,
}

fn default_barb_spacing() -> u32 {
    50
}
fn default_barb_size() -> f32 {
    20.0
}
fn default_true() -> bool {
    true
}
fn default_calm_threshold() -> f64 {
    2.5
}

/// Wind arrow/vector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindArrowConfig {
    /// Spacing between arrows in pixels
    #[serde(default = "default_barb_spacing")]
    pub spacing: u32,

    /// Optional color ramp based on speed
    #[serde(default)]
    pub color_by_speed: Option<GradientConfig>,

    /// Fixed color (if not using speed-based coloring)
    #[serde(default)]
    pub color: Option<Color>,

    /// Arrow scale factor
    #[serde(default = "default_arrow_scale")]
    pub scale: f32,

    /// Minimum arrow length
    #[serde(default)]
    pub min_length: Option<f32>,

    /// Maximum arrow length
    #[serde(default)]
    pub max_length: Option<f32>,
}

fn default_arrow_scale() -> f32 {
    1.0
}

/// Value transformation before rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValueTransform {
    /// Convert units: value * scale + offset
    Linear { scale: f64, offset: f64 },

    /// Kelvin to Celsius
    KelvinToCelsius,

    /// Kelvin to Fahrenheit
    KelvinToFahrenheit,

    /// Meters per second to knots
    MpsToKnots,

    /// Pascals to hectopascals
    PaToHpa,

    /// Logarithmic (for dBZ, etc): value = 10 * log10(data)
    Log10 { scale: f64 },
}

impl ValueTransform {
    pub fn apply(&self, value: f64) -> f64 {
        match self {
            ValueTransform::Linear { scale, offset } => value * scale + offset,
            ValueTransform::KelvinToCelsius => value - 273.15,
            ValueTransform::KelvinToFahrenheit => (value - 273.15) * 9.0 / 5.0 + 32.0,
            ValueTransform::MpsToKnots => value * 1.94384,
            ValueTransform::PaToHpa => value / 100.0,
            ValueTransform::Log10 { scale } => scale * value.log10(),
        }
    }
}

/// Legend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegendConfig {
    /// Legend title
    #[serde(default)]
    pub title: Option<String>,

    /// Orientation
    #[serde(default)]
    pub orientation: LegendOrientation,

    /// Number of tick marks
    #[serde(default = "default_ticks")]
    pub ticks: u32,

    /// Width in pixels
    #[serde(default = "default_legend_width")]
    pub width: u32,

    /// Height in pixels
    #[serde(default = "default_legend_height")]
    pub height: u32,
}

fn default_ticks() -> u32 {
    5
}
fn default_legend_width() -> u32 {
    300
}
fn default_legend_height() -> u32 {
    30
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegendOrientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Style-related errors.
#[derive(Debug, thiserror::Error)]
pub enum StyleError {
    #[error("IO error: {0}")]
    IoError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_temperature_style() {
        let json = r##"{"version":"1.0","styles":{"temperature_c":{"name":"Temperature (Celsius)","description":"Standard temperature gradient","type":"gradient","units":"C","transform":{"type":"kelvin_to_celsius"},"stops":[{"value":-40,"color":"#9013FE","label":"-40C"},{"value":-20,"color":"#0000FF","label":"-20C"},{"value":0,"color":"#00FFFF","label":"0C"},{"value":10,"color":"#00FF00","label":"10C"},{"value":20,"color":"#FFFF00","label":"20C"},{"value":30,"color":"#FF8000","label":"30C"},{"value":40,"color":"#FF0000","label":"40C"}],"interpolation":"linear","out_of_range":"clamp"}}}"##;

        let config = StyleConfig::from_json(json).unwrap();
        config.validate().unwrap();

        let style = config.get("temperature_c").unwrap();
        assert_eq!(style.name, "Temperature (Celsius)");

        if let RendererConfig::Gradient(g) = &style.renderer {
            assert_eq!(g.stops.len(), 7);

            // Test interpolation
            let color = g.interpolate(15.0);
            let (_r, green, _b, _a) = color.to_rgba();
            assert!(green > 200); // Should be yellowish-green
        } else {
            panic!("Expected gradient config");
        }
    }

    #[test]
    fn test_parse_classified_style() {
        let json = r##"{"version":"1.0","styles":{"precipitation_type":{"name":"Precipitation Type","type":"classified","classes":[{"min":0,"max":1,"color":"transparent","label":"None"},{"min":1,"max":2,"color":"#00FF00","label":"Rain"},{"min":2,"max":3,"color":"#00FFFF","label":"FreezingRain"},{"min":3,"max":4,"color":"#FFFFFF","label":"Snow"}]}}}"##;

        let config = StyleConfig::from_json(json).unwrap();
        config.validate().unwrap();
    }

    #[test]
    fn test_color_parsing() {
        let hex = Color::Hex("#FF5500".to_string());
        assert_eq!(hex.to_rgba(), (255, 85, 0, 255));

        let arr = Color::Array(vec![100, 150, 200]);
        assert_eq!(arr.to_rgba(), (100, 150, 200, 255));

        let named = Color::Named("red".to_string());
        assert_eq!(named.to_rgba(), (255, 0, 0, 255));
    }
}
