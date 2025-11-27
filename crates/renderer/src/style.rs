//! Style configuration for weather data rendering.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Style configuration loaded from JSON
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StyleConfig {
    pub version: String,
    pub styles: HashMap<String, StyleDefinition>,
}

/// A single style definition
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StyleDefinition {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub style_type: String,
    pub units: Option<String>,
    pub transform: Option<Transform>,
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

/// Parse hex color string to RGB
pub fn hex_to_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    
    Some((r, g, b))
}

/// Contour style configuration
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
    pub line_color: [u8; 4],
    pub smoothing_passes: Option<u32>,
}

impl ContourStyle {
    /// Load contour style from JSON file
    pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }
    
    /// Generate contour levels from the configuration
    pub fn generate_levels(&self, data_min: f32, data_max: f32) -> Vec<f32> {
        use crate::contour::generate_contour_levels;
        
        // If levels are explicitly specified, use them
        if let Some(ref levels) = self.contour.levels {
            return levels.clone();
        }
        
        // Otherwise generate from interval
        if let Some(interval) = self.contour.interval {
            let min = self.contour.min_value.unwrap_or(data_min);
            let max = self.contour.max_value.unwrap_or(data_max);
            
            // Apply unit conversion if specified (e.g., Kelvin to Celsius)
            let min_converted = if let Some(conversion) = self.contour.unit_conversion {
                min + conversion
            } else {
                min
            };
            let max_converted = if let Some(conversion) = self.contour.unit_conversion {
                max + conversion
            } else {
                max
            };
            
            // Generate levels in converted units
            let levels = generate_contour_levels(min_converted, max_converted, interval);
            
            // Convert back to original units
            if let Some(conversion) = self.contour.unit_conversion {
                levels.into_iter().map(|l| l - conversion).collect()
            } else {
                levels
            }
        } else {
            vec![]
        }
    }
}

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
    
    // Convert hex colors to RGB
    let colors: Vec<Option<(u8, u8, u8)>> = stops.iter().map(|s| hex_to_rgb(&s.color)).collect();
    let values: Vec<f32> = stops.iter().map(|s| s.value).collect();
    
    // Find data range
    let (min_val, max_val) = data.iter().fold(
        (f32::INFINITY, f32::NEG_INFINITY),
        |(min, max), &val| (min.min(val), max.max(val)),
    );
    
    // Render each pixel
    for (idx, &value) in data.iter().enumerate() {
        if idx >= width * height {
            break;
        }
        
        // Handle NaN as transparent (for data outside geographic bounds)
        if value.is_nan() {
            let pixel_offset = idx * 4;
            pixels[pixel_offset] = 0;     // R
            pixels[pixel_offset + 1] = 0; // G
            pixels[pixel_offset + 2] = 0; // B
            pixels[pixel_offset + 3] = 0; // A (transparent)
            continue;
        }
        
        // Normalize value to 0-1 range
        let range = max_val - min_val;
        let normalized = if range.abs() < 0.001 {
            0.5
        } else {
            ((value - min_val) / range).clamp(0.0, 1.0)
        };
        
        // Find the two surrounding color stops
        let mut low_idx = 0;
        let mut high_idx = values.len() - 1;
        
        for (i, &stop_val) in values.iter().enumerate() {
            if stop_val <= normalized * (values[values.len() - 1] - values[0]) + values[0] {
                low_idx = i;
            }
            if stop_val >= normalized * (values[values.len() - 1] - values[0]) + values[0] {
                high_idx = i;
                break;
            }
        }
        
        // Interpolate color
        let (r, g, b, a) = if low_idx == high_idx {
            match colors[low_idx] {
                Some((r, g, b)) => (r, g, b, 255),
                None => (200, 200, 200, 255),
            }
        } else {
            let low_val = values[low_idx];
            let high_val = values[high_idx];
            let t = if (high_val - low_val).abs() < 0.001 {
                0.0
            } else {
                ((normalized * (values[values.len() - 1] - values[0]) + values[0] - low_val) / (high_val - low_val))
                    .clamp(0.0, 1.0)
            };
            
            match (colors[low_idx], colors[high_idx]) {
                (Some((r1, g1, b1)), Some((r2, g2, b2))) => {
                    let r = (r1 as f32 * (1.0 - t) + r2 as f32 * t) as u8;
                    let g = (g1 as f32 * (1.0 - t) + g2 as f32 * t) as u8;
                    let b = (b1 as f32 * (1.0 - t) + b2 as f32 * t) as u8;
                    (r, g, b, 255)
                }
                _ => (200, 200, 200, 255),
            }
        };
        
        let pixel_idx = idx * 4;
        pixels[pixel_idx] = r;
        pixels[pixel_idx + 1] = g;
        pixels[pixel_idx + 2] = b;
        pixels[pixel_idx + 3] = a;
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
