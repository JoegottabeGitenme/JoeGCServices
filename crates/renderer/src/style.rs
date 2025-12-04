//! Style configuration for weather data rendering.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Style configuration loaded from JSON
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StyleConfig {
    pub version: String,
    pub styles: HashMap<String, StyleDefinition>,
}

/// Value range for the style
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ValueRange {
    pub min: f32,
    pub max: f32,
}

/// A single style definition
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StyleDefinition {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub style_type: String,
    pub units: Option<String>,
    pub range: Option<ValueRange>,
    pub transform: Option<Transform>,
    #[serde(default)]
    pub stops: Vec<ColorStop>,
    pub interpolation: Option<String>,
    pub out_of_range: Option<String>,
    pub legend: Option<Legend>,
}

/// Color transformation
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Transform {
    #[serde(rename = "type")]
    pub transform_type: String,
}

/// Color stop for gradient
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ColorStop {
    pub value: f32,
    pub color: String,
    pub label: Option<String>,
}

/// Legend configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Legend {
    pub title: Option<String>,
    pub orientation: Option<String>,
    pub ticks: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl StyleConfig {
    /// Load style configuration from JSON string
    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }

    /// Load style configuration from file
    pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self::from_json(&content)?)
    }

    /// Get a specific style definition
    pub fn get_style(&self, name: &str) -> Option<&StyleDefinition> {
        self.styles.get(name)
    }
}

/// Parse hex color string to RGBA
/// Supports both 6-char RGB (#RRGGBB) and 8-char RGBA (#RRGGBBAA) formats
pub fn hex_to_rgba(hex: &str) -> Option<(u8, u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    
    match hex.len() {
        6 => {
            // RGB format - fully opaque
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r, g, b, 255))
        }
        8 => {
            // RGBA format
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some((r, g, b, a))
        }
        _ => None,
    }
}

/// Legacy function for backwards compatibility
pub fn hex_to_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    hex_to_rgba(hex).map(|(r, g, b, _)| (r, g, b))
}

/// Contour style configuration (nested format from JSON files)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContourStyleFile {
    pub version: Option<String>,
    pub metadata: Option<ContourMetadata>,
    pub styles: std::collections::HashMap<String, ContourStyleDefinition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContourMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// A single contour style definition within the file
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContourStyleDefinition {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub style_type: String,
    pub units: Option<String>,
    pub transform: Option<Transform>,
    pub contour: ContourOptions,
    pub legend: Option<Legend>,
}

/// Contour style configuration (flattened for rendering use)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContourStyle {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub style_type: String,
    pub units: Option<String>,
    pub contour: ContourOptions,
}

/// Contour rendering options
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContourOptions {
    pub levels: Option<Vec<f32>>,
    pub interval: Option<f32>,
    pub unit_conversion: Option<f32>,
    pub min_value: Option<f32>,
    pub max_value: Option<f32>,
    pub line_width: f32,
    #[serde(deserialize_with = "deserialize_color")]
    pub line_color: [u8; 4],
    pub smoothing_passes: Option<u32>,
    pub base: Option<f32>,
    pub major_interval: Option<u32>,
    pub major_line_width: Option<f32>,
    pub labels: Option<bool>,
    pub label_font_size: Option<f32>,
    /// Minimum spacing between labels along a contour (in pixels)
    pub label_spacing: Option<f32>,
    /// Special levels with custom styling (e.g., freezing level)
    pub special_levels: Option<Vec<SpecialLevel>>,
}

/// Special level with custom styling
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SpecialLevel {
    /// The value in display units (e.g., 0 for 0°C)
    pub value: f32,
    /// Custom line color for this level
    #[serde(default, deserialize_with = "deserialize_color_option")]
    pub line_color: Option<[u8; 4]>,
    /// Custom line width for this level
    pub line_width: Option<f32>,
    /// Custom label for this level (e.g., "Freezing")
    pub label: Option<String>,
}

/// Custom deserializer for optional color
fn deserialize_color_option<'de, D>(deserializer: D) -> Result<Option<[u8; 4]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    
    struct ColorOptionVisitor;
    
    impl<'de> Visitor<'de> for ColorOptionVisitor {
        type Value = Option<[u8; 4]>;
        
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a hex color string like '#FF0000', an RGBA array [255, 0, 0, 255], or null")
        }
        
        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
        
        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
        
        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let hex = value.trim_start_matches('#');
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16).map_err(de::Error::custom)?;
                let g = u8::from_str_radix(&hex[2..4], 16).map_err(de::Error::custom)?;
                let b = u8::from_str_radix(&hex[4..6], 16).map_err(de::Error::custom)?;
                Ok(Some([r, g, b, 255]))
            } else if hex.len() == 8 {
                let r = u8::from_str_radix(&hex[0..2], 16).map_err(de::Error::custom)?;
                let g = u8::from_str_radix(&hex[2..4], 16).map_err(de::Error::custom)?;
                let b = u8::from_str_radix(&hex[4..6], 16).map_err(de::Error::custom)?;
                let a = u8::from_str_radix(&hex[6..8], 16).map_err(de::Error::custom)?;
                Ok(Some([r, g, b, a]))
            } else {
                Err(de::Error::custom(format!("invalid hex color: {}", value)))
            }
        }
        
        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            let r = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(0, &self))?;
            let g = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
            let b = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
            let a = seq.next_element()?.unwrap_or(255u8);
            Ok(Some([r, g, b, a]))
        }
    }
    
    deserializer.deserialize_any(ColorOptionVisitor)
}

/// Custom deserializer for color that handles both hex strings and arrays
fn deserialize_color<'de, D>(deserializer: D) -> Result<[u8; 4], D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    
    struct ColorVisitor;
    
    impl<'de> Visitor<'de> for ColorVisitor {
        type Value = [u8; 4];
        
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a hex color string like '#FF0000' or an RGBA array [255, 0, 0, 255]")
        }
        
        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let hex = value.trim_start_matches('#');
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16).map_err(de::Error::custom)?;
                let g = u8::from_str_radix(&hex[2..4], 16).map_err(de::Error::custom)?;
                let b = u8::from_str_radix(&hex[4..6], 16).map_err(de::Error::custom)?;
                Ok([r, g, b, 255])
            } else if hex.len() == 8 {
                let r = u8::from_str_radix(&hex[0..2], 16).map_err(de::Error::custom)?;
                let g = u8::from_str_radix(&hex[2..4], 16).map_err(de::Error::custom)?;
                let b = u8::from_str_radix(&hex[4..6], 16).map_err(de::Error::custom)?;
                let a = u8::from_str_radix(&hex[6..8], 16).map_err(de::Error::custom)?;
                Ok([r, g, b, a])
            } else {
                Err(de::Error::custom(format!("invalid hex color: {}", value)))
            }
        }
        
        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            let r = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(0, &self))?;
            let g = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
            let b = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
            let a = seq.next_element()?.unwrap_or(255u8);
            Ok([r, g, b, a])
        }
    }
    
    deserializer.deserialize_any(ColorVisitor)
}

impl ContourStyle {
    /// Load contour style from JSON file
    /// Supports both nested format (with styles.default) and flat format
    pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Self::from_file_with_style(path, "default")
    }
    
    /// Load a specific style variant from JSON file
    pub fn from_file_with_style(path: &str, style_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        
        // First try to parse as nested format (with styles map)
        if let Ok(file) = serde_json::from_str::<ContourStyleFile>(&content) {
            let style_def = file.styles.get(style_name)
                .ok_or_else(|| format!("Style '{}' not found in {}", style_name, path))?;
            
            return Ok(ContourStyle {
                name: style_def.name.clone(),
                title: Some(style_def.name.clone()),
                description: style_def.description.clone(),
                style_type: style_def.style_type.clone(),
                units: style_def.units.clone(),
                contour: style_def.contour.clone(),
            });
        }
        
        // Fall back to flat format (direct ContourStyle)
        Ok(serde_json::from_str(&content)?)
    }
    
    /// Generate contour levels from the configuration
    pub fn generate_levels(&self, data_min: f32, data_max: f32) -> Vec<f32> {
        // If levels are explicitly specified, use them
        if let Some(ref levels) = self.contour.levels {
            return levels.clone();
        }
        
        // Otherwise generate from interval
        if let Some(interval) = self.contour.interval {
            let min = self.contour.min_value.unwrap_or(data_min);
            let max = self.contour.max_value.unwrap_or(data_max);
            let base = self.contour.base.unwrap_or(0.0);
            
            // Generate levels in display units (e.g., Celsius) starting from base
            // This ensures levels align with nice values like 0°C
            let mut levels = Vec::new();
            
            // Start from the base and go down to min
            let mut level = base;
            while level >= min {
                if level >= min && level <= max {
                    levels.push(level);
                }
                level -= interval;
            }
            
            // Go up from base to max
            level = base + interval;
            while level <= max {
                if level >= min && level <= max {
                    levels.push(level);
                }
                level += interval;
            }
            
            // Sort levels
            levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            
            // Convert to data units if unit_conversion is specified
            if let Some(conversion) = self.contour.unit_conversion {
                levels.iter().map(|&l| l + conversion).collect()
            } else {
                levels
            }
        } else {
            vec![]
        }
    }
}

/// Common missing value markers for weather data
const MISSING_VALUE_THRESHOLD: f32 = -90.0;

/// Apply style-based color mapping to data
pub fn apply_style_gradient(
    data: &[f32],
    width: usize,
    height: usize,
    style: &StyleDefinition,
) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height * 4];
    
    // Extract color stops and sort by value
    let mut stops = style.stops.clone();
    stops.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
    
    if stops.is_empty() {
        return pixels;
    }
    
    // Convert hex colors to RGBA (supports 6-char RGB and 8-char RGBA)
    let colors: Vec<Option<(u8, u8, u8, u8)>> = stops.iter().map(|s| hex_to_rgba(&s.color)).collect();
    let stop_values: Vec<f32> = stops.iter().map(|s| s.value).collect();
    
    let min_stop = stop_values[0];
    let max_stop = stop_values[stop_values.len() - 1];
    
    // Render each pixel
    for (idx, &value) in data.iter().enumerate() {
        if idx >= width * height {
            break;
        }
        
        let pixel_offset = idx * 4;
        
        // Handle NaN and common missing values as transparent
        // MRMS uses -99 and -999, other datasets may use different markers
        if value.is_nan() || value <= MISSING_VALUE_THRESHOLD {
            pixels[pixel_offset] = 0;     // R
            pixels[pixel_offset + 1] = 0; // G
            pixels[pixel_offset + 2] = 0; // B
            pixels[pixel_offset + 3] = 0; // A (transparent)
            continue;
        }
        
        // Map value directly to color stops (no normalization needed)
        // Find the two surrounding color stops based on actual data value
        let mut low_idx = 0;
        let mut high_idx = stop_values.len() - 1;
        
        for (i, &stop_val) in stop_values.iter().enumerate() {
            if stop_val <= value {
                low_idx = i;
            }
            if stop_val >= value && i > 0 {
                high_idx = i;
                break;
            }
        }
        
        // Handle out-of-range values
        if value <= min_stop {
            // Below minimum - use first stop's color
            if let Some((r, g, b, a)) = colors[0] {
                pixels[pixel_offset] = r;
                pixels[pixel_offset + 1] = g;
                pixels[pixel_offset + 2] = b;
                pixels[pixel_offset + 3] = a;
            }
            continue;
        }
        
        if value >= max_stop {
            // Above maximum - use last stop's color
            if let Some((r, g, b, a)) = colors[colors.len() - 1] {
                pixels[pixel_offset] = r;
                pixels[pixel_offset + 1] = g;
                pixels[pixel_offset + 2] = b;
                pixels[pixel_offset + 3] = a;
            }
            continue;
        }
        
        // Interpolate color between two stops
        let (r, g, b, a) = if low_idx == high_idx {
            match colors[low_idx] {
                Some((r, g, b, a)) => (r, g, b, a),
                None => (200, 200, 200, 255),
            }
        } else {
            let low_val = stop_values[low_idx];
            let high_val = stop_values[high_idx];
            let t = if (high_val - low_val).abs() < 0.001 {
                0.0
            } else {
                ((value - low_val) / (high_val - low_val)).clamp(0.0, 1.0)
            };
            
            match (colors[low_idx], colors[high_idx]) {
                (Some((r1, g1, b1, a1)), Some((r2, g2, b2, a2))) => {
                    let r = (r1 as f32 * (1.0 - t) + r2 as f32 * t) as u8;
                    let g = (g1 as f32 * (1.0 - t) + g2 as f32 * t) as u8;
                    let b = (b1 as f32 * (1.0 - t) + b2 as f32 * t) as u8;
                    let a = (a1 as f32 * (1.0 - t) + a2 as f32 * t) as u8;
                    (r, g, b, a)
                }
                _ => (200, 200, 200, 255),
            }
        };
        
        pixels[pixel_offset] = r;
        pixels[pixel_offset + 1] = g;
        pixels[pixel_offset + 2] = b;
        pixels[pixel_offset + 3] = a;
    }
    
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_to_rgb() {
        assert_eq!(hex_to_rgb("#FF0000"), Some((255, 0, 0)));
        assert_eq!(hex_to_rgb("#00FF00"), Some((0, 255, 0)));
        assert_eq!(hex_to_rgb("#0000FF"), Some((0, 0, 255)));
        assert_eq!(hex_to_rgb("FF0000"), Some((255, 0, 0)));
        assert_eq!(hex_to_rgb("#GGGGGG"), None);
    }
}
