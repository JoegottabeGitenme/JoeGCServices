//! Style configuration for weather data rendering.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Pre-computed palette for fast indexed PNG rendering.
///
/// Instead of extracting the palette at PNG encoding time (expensive),
/// we pre-compute all possible colors from the style definition at load time.
/// This gives us both fast encoding AND small file sizes.
#[derive(Debug, Clone)]
pub struct PrecomputedPalette {
    /// All unique colors in the palette.
    /// Index 0 is always transparent (0, 0, 0, 0) for NaN/missing values.
    pub colors: Vec<(u8, u8, u8, u8)>,
    
    /// Lookup table: maps quantized value (0-4095) to palette index.
    /// Using 4096 entries (12 bits) for good precision while staying small.
    /// Value outside range maps to index 0 (transparent) or clamped color.
    pub value_to_index: Vec<u8>,
    
    /// The value range this palette covers
    pub min_value: f32,
    pub max_value: f32,
}

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
    /// Marks this style as the default for the file
    #[serde(default)]
    pub default: bool,
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
    /// Scale factor for linear transform (value * scale)
    #[serde(default)]
    pub scale: Option<f32>,
    /// Offset for linear transform (value * scale + offset)
    #[serde(default)]
    pub offset: Option<f32>,
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
    
    /// Get the default style (the one with `default: true`)
    /// Falls back to looking for a style named "default" for backwards compatibility
    pub fn get_default_style(&self) -> Option<(&String, &StyleDefinition)> {
        // First, look for a style with default: true
        for (name, style) in &self.styles {
            if style.default {
                return Some((name, style));
            }
        }
        // Fallback: look for style named "default" (backwards compat)
        self.styles.get_key_value("default")
            .or_else(|| self.styles.iter().next())
    }
    
    /// Get the default style name
    pub fn default_style_name(&self) -> Option<&str> {
        self.get_default_style().map(|(name, _)| name.as_str())
    }
}

/// Number of entries in the value-to-index lookup table.
/// 4096 (12 bits) provides good precision while keeping memory small (4KB).
const PALETTE_LUT_SIZE: usize = 4096;

impl StyleDefinition {
    /// Pre-compute the palette from style color stops.
    ///
    /// This pre-interpolates all colors at load time so rendering can
    /// output palette indices directly without color interpolation.
    ///
    /// Returns None if the style has no stops or invalid configuration.
    pub fn compute_palette(&self) -> Option<PrecomputedPalette> {
        if self.stops.is_empty() {
            return None;
        }
        
        // Sort stops by value
        let mut stops = self.stops.clone();
        stops.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
        
        // Parse colors
        let parsed_colors: Vec<(f32, (u8, u8, u8, u8))> = stops
            .iter()
            .filter_map(|s| hex_to_rgba(&s.color).map(|c| (s.value, c)))
            .collect();
        
        if parsed_colors.is_empty() {
            return None;
        }
        
        // Determine value range
        let min_value = self.range.as_ref().map(|r| r.min).unwrap_or(parsed_colors[0].0);
        let max_value = self.range.as_ref().map(|r| r.max).unwrap_or(parsed_colors[parsed_colors.len() - 1].0);
        let range = max_value - min_value;
        
        if range <= 0.0 {
            return None;
        }
        
        // Build palette with exactly 255 colors (index 0 reserved for transparent)
        // This ensures we have even coverage of the entire color range
        let mut colors: Vec<(u8, u8, u8, u8)> = Vec::with_capacity(256);
        
        // Index 0 is always transparent (for NaN/missing)
        colors.push((0, 0, 0, 0));
        
        // Generate 255 evenly-spaced colors across the value range
        for i in 0..255 {
            let t = i as f32 / 254.0;  // 0.0 to 1.0
            let value = min_value + t * range;
            let color = interpolate_color_at_value(value, &parsed_colors);
            colors.push(color);
        }
        
        // Build LUT: map each of 4096 entries to the nearest palette index
        let mut value_to_index = vec![0u8; PALETTE_LUT_SIZE];
        for i in 0..PALETTE_LUT_SIZE {
            // Map LUT index to palette index (1-255, skipping 0 which is transparent)
            // LUT 0 -> palette 1, LUT 4095 -> palette 255
            let palette_idx = 1 + (i * 254 / (PALETTE_LUT_SIZE - 1));
            value_to_index[i] = palette_idx as u8;
        }
        
        Some(PrecomputedPalette {
            colors,
            value_to_index,
            min_value,
            max_value,
        })
    }
}

/// Pack RGBA into u32 for hashing
#[inline]
#[allow(dead_code)]
fn pack_color_u32(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

/// Interpolate color at a specific value given sorted color stops
fn interpolate_color_at_value(value: f32, stops: &[(f32, (u8, u8, u8, u8))]) -> (u8, u8, u8, u8) {
    if stops.is_empty() {
        return (0, 0, 0, 0);
    }
    
    // Below first stop
    if value <= stops[0].0 {
        return stops[0].1;
    }
    
    // Above last stop
    if value >= stops[stops.len() - 1].0 {
        return stops[stops.len() - 1].1;
    }
    
    // Find surrounding stops
    for i in 0..stops.len() - 1 {
        let (low_val, low_color) = stops[i];
        let (high_val, high_color) = stops[i + 1];
        
        if value >= low_val && value <= high_val {
            let range = high_val - low_val;
            if range.abs() < 0.0001 {
                return low_color;
            }
            
            let t = (value - low_val) / range;
            return (
                (low_color.0 as f32 * (1.0 - t) + high_color.0 as f32 * t) as u8,
                (low_color.1 as f32 * (1.0 - t) + high_color.1 as f32 * t) as u8,
                (low_color.2 as f32 * (1.0 - t) + high_color.2 as f32 * t) as u8,
                (low_color.3 as f32 * (1.0 - t) + high_color.3 as f32 * t) as u8,
            );
        }
    }
    
    stops[stops.len() - 1].1
}

/// Find the closest color in the palette (fallback when palette is full)
#[allow(dead_code)]
fn find_closest_color_index(palette: &[(u8, u8, u8, u8)], target: (u8, u8, u8, u8)) -> u8 {
    let mut best_idx = 0u8;
    let mut best_dist = u32::MAX;
    
    for (i, &(r, g, b, a)) in palette.iter().enumerate().skip(1) {
        // Simple color distance (could use LAB for better results)
        let dr = (r as i32 - target.0 as i32).abs() as u32;
        let dg = (g as i32 - target.1 as i32).abs() as u32;
        let db = (b as i32 - target.2 as i32).abs() as u32;
        let da = (a as i32 - target.3 as i32).abs() as u32;
        let dist = dr * dr + dg * dg + db * db + da * da;
        
        if dist < best_dist {
            best_dist = dist;
            best_idx = i as u8;
        }
    }
    
    best_idx
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
    /// Marks this style as the default for the file
    #[serde(default)]
    pub default: bool,
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
    pub transform: Option<Transform>,
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
    /// Load the default contour style from JSON file
    /// Looks for a style with `default: true`, falls back to "default" name
    pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let json: serde_json::Value = serde_json::from_str(&content)?;
        
        // Find the default style name
        if let Some(styles) = json.get("styles").and_then(|s| s.as_object()) {
            // First look for a style with default: true
            for (name, style) in styles {
                if style.get("default").and_then(|d| d.as_bool()).unwrap_or(false) {
                    return Self::from_file_with_style(path, name);
                }
            }
            // Fallback to "default" name for backwards compat
            if styles.contains_key("default") {
                return Self::from_file_with_style(path, "default");
            }
            // Last resort: use first style
            if let Some(name) = styles.keys().next() {
                return Self::from_file_with_style(path, name);
            }
        }
        
        // Fall back to flat format (direct ContourStyle)
        Ok(serde_json::from_str(&content)?)
    }
    
    /// Load a specific style variant from JSON file
    /// Supports mixed-type style files (gradient + contour + numbers in same file)
    pub fn from_file_with_style(path: &str, style_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        
        // Parse as generic JSON first to handle mixed-type style files
        let json: serde_json::Value = serde_json::from_str(&content)?;
        
        // Try to get the specific style from styles map
        if let Some(styles) = json.get("styles").and_then(|s| s.as_object()) {
            if let Some(style_value) = styles.get(style_name) {
                // Parse just this one style as ContourStyleDefinition
                let style_def: ContourStyleDefinition = serde_json::from_value(style_value.clone())
                    .map_err(|e| format!("Failed to parse style '{}': {}", style_name, e))?;
                
                return Ok(ContourStyle {
                    name: style_def.name.clone(),
                    title: Some(style_def.name.clone()),
                    description: style_def.description.clone(),
                    style_type: style_def.style_type.clone(),
                    units: style_def.units.clone(),
                    transform: style_def.transform.clone(),
                    contour: style_def.contour.clone(),
                });
            } else {
                return Err(format!("Style '{}' not found in {}", style_name, path).into());
            }
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

/// Apply a unit transform to a single value
pub fn apply_transform(value: f32, transform: Option<&Transform>) -> f32 {
    match transform {
        Some(t) => {
            let transform_type = t.transform_type.to_lowercase();
            if transform_type == "pa_to_hpa" {
                value / 100.0
            } else if transform_type == "k_to_c" || transform_type == "kelvin_to_celsius" {
                value - 273.15
            } else if transform_type == "m_to_km" {
                value / 1000.0
            } else if transform_type == "mps_to_knots" {
                value * 1.94384
            } else if transform_type == "linear" {
                // Linear transform: value * scale + offset
                let scale = t.scale.unwrap_or(1.0);
                let offset = t.offset.unwrap_or(0.0);
                value * scale + offset
            } else {
                value
            }
        }
        None => value,
    }
}

/// Apply style-based color mapping to data
///
/// # Performance
/// Uses rayon for parallel row processing. Each row is processed independently
/// across multiple CPU cores.
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
    
    // Get the transform from the style
    let transform = style.transform.clone();
    
    // Use range from style if specified, otherwise use stop boundaries
    let min_range = style.range.as_ref().map(|r| r.min).unwrap_or(stop_values[0]);
    let max_range = style.range.as_ref().map(|r| r.max).unwrap_or(stop_values[stop_values.len() - 1]);
    
    // Get out_of_range behavior: "clamp" (default), "transparent", or "extend"
    let out_of_range_transparent = style.out_of_range.as_deref() == Some("transparent");
    
    let row_bytes = width * 4;
    
    // Process rows in parallel
    pixels
        .par_chunks_mut(row_bytes)
        .enumerate()
        .for_each(|(y, row)| {
            let data_row_start = y * width;
            
            for x in 0..width {
                let data_idx = data_row_start + x;
                let pixel_idx = x * 4;
                
                if data_idx >= data.len() {
                    break;
                }
                
                let raw_value = data[data_idx];
                
                // Handle NaN and common missing values as transparent
                if raw_value.is_nan() || raw_value <= MISSING_VALUE_THRESHOLD {
                    row[pixel_idx] = 0;
                    row[pixel_idx + 1] = 0;
                    row[pixel_idx + 2] = 0;
                    row[pixel_idx + 3] = 0;
                    continue;
                }
                
                // Apply transform to convert to display units
                let value = apply_transform(raw_value, transform.as_ref());
                
                // Handle out-of-range values
                if value < min_range {
                    if out_of_range_transparent {
                        row[pixel_idx] = 0;
                        row[pixel_idx + 1] = 0;
                        row[pixel_idx + 2] = 0;
                        row[pixel_idx + 3] = 0;
                    } else if let Some((r, g, b, a)) = colors[0] {
                        row[pixel_idx] = r;
                        row[pixel_idx + 1] = g;
                        row[pixel_idx + 2] = b;
                        row[pixel_idx + 3] = a;
                    }
                    continue;
                }
                
                if value > max_range {
                    if out_of_range_transparent {
                        row[pixel_idx] = 0;
                        row[pixel_idx + 1] = 0;
                        row[pixel_idx + 2] = 0;
                        row[pixel_idx + 3] = 0;
                    } else if let Some((r, g, b, a)) = colors[colors.len() - 1] {
                        row[pixel_idx] = r;
                        row[pixel_idx + 1] = g;
                        row[pixel_idx + 2] = b;
                        row[pixel_idx + 3] = a;
                    }
                    continue;
                }
                
                // Find the two surrounding color stops
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
                
                row[pixel_idx] = r;
                row[pixel_idx + 1] = g;
                row[pixel_idx + 2] = b;
                row[pixel_idx + 3] = a;
            }
        });
    
    pixels
}

/// Apply style-based color mapping, outputting palette indices directly.
///
/// This is much faster than `apply_style_gradient()` + palette extraction because:
/// 1. Outputs 1 byte per pixel instead of 4
/// 2. Uses simple LUT lookup instead of color interpolation
/// 3. Palette is pre-computed at style load time
///
/// # Arguments
/// * `data` - Input weather data values
/// * `width` - Image width
/// * `height` - Image height  
/// * `palette` - Pre-computed palette from `StyleDefinition::compute_palette()`
/// * `style` - Style definition (for transform)
///
/// # Returns
/// Vec of palette indices (1 byte per pixel), ready for indexed PNG encoding
pub fn apply_style_gradient_indexed(
    data: &[f32],
    width: usize,
    height: usize,
    palette: &PrecomputedPalette,
    style: &StyleDefinition,
) -> Vec<u8> {
    let mut indices = vec![0u8; width * height];
    
    let transform = style.transform.as_ref();
    let min_value = palette.min_value;
    let max_value = palette.max_value;
    let range = max_value - min_value;
    let lut_max = (PALETTE_LUT_SIZE - 1) as f32;
    
    // Get out_of_range behavior
    let out_of_range_transparent = style.out_of_range.as_deref() == Some("transparent");
    
    // Index for below-range values (first color after transparent, or 0 if transparent)
    let below_range_idx = if out_of_range_transparent { 0 } else { palette.value_to_index[0] };
    // Index for above-range values (last color, or 0 if transparent)  
    let above_range_idx = if out_of_range_transparent { 0 } else { palette.value_to_index[PALETTE_LUT_SIZE - 1] };
    
    // Process rows in parallel
    indices
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            let data_row_start = y * width;
            
            for x in 0..width {
                let data_idx = data_row_start + x;
                
                if data_idx >= data.len() {
                    break;
                }
                
                let raw_value = data[data_idx];
                
                // Handle NaN and missing values -> transparent (index 0)
                if raw_value.is_nan() || raw_value <= MISSING_VALUE_THRESHOLD {
                    row[x] = 0;
                    continue;
                }
                
                // Apply transform
                let value = apply_transform(raw_value, transform);
                
                // Handle out-of-range
                if value < min_value {
                    row[x] = below_range_idx;
                    continue;
                }
                
                if value > max_value {
                    row[x] = above_range_idx;
                    continue;
                }
                
                // Normalize to LUT index and lookup
                let t = (value - min_value) / range;
                let lut_idx = (t * lut_max) as usize;
                row[x] = palette.value_to_index[lut_idx.min(PALETTE_LUT_SIZE - 1)];
            }
        });
    
    indices
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
    
    #[test]
    fn test_linear_transform() {
        let json = r##"{
            "version": "1.0",
            "styles": {
                "gradient": {
                    "default": true,
                    "name": "Test",
                    "type": "gradient",
                    "transform": {
                        "type": "linear",
                        "scale": 0.0167,
                        "offset": 0
                    },
                    "stops": [
                        {"value": 0, "color": "#000000"},
                        {"value": 100, "color": "#FFFFFF"}
                    ]
                }
            }
        }"##;
        
        let config = StyleConfig::from_json(json).unwrap();
        let style = config.get_style("gradient").unwrap();
        
        // Verify transform is parsed
        assert!(style.transform.is_some());
        let t = style.transform.as_ref().unwrap();
        assert_eq!(t.transform_type, "linear");
        assert_eq!(t.scale, Some(0.0167));
        assert_eq!(t.offset, Some(0.0));
        
        // Test transform application
        let raw_value: f32 = 35500.0;
        let transformed = apply_transform(raw_value, style.transform.as_ref());
        let expected = 35500.0 * 0.0167;
        assert!((transformed - expected).abs() < 0.01, "Expected {}, got {}", expected, transformed);
    }
    
    #[test]
    fn test_precomputed_palette_matches_rgba() {
        // Create a simple gradient style
        let json = r##"{
            "version": "1.0",
            "styles": {
                "gradient": {
                    "default": true,
                    "name": "Test Gradient",
                    "type": "gradient",
                    "range": {"min": 0.0, "max": 100.0},
                    "stops": [
                        {"value": 0, "color": "#0000FF"},
                        {"value": 50, "color": "#00FF00"},
                        {"value": 100, "color": "#FF0000"}
                    ]
                }
            }
        }"##;
        
        let config = StyleConfig::from_json(json).unwrap();
        let style = config.get_style("gradient").unwrap();
        let palette = style.compute_palette().expect("Should compute palette");
        
        println!("Palette range: {} to {}", palette.min_value, palette.max_value);
        println!("Palette colors: {}", palette.colors.len());
        
        // Debug: print some LUT entries
        println!("\nLUT entries (value -> lut_idx -> palette_idx -> color):");
        for i in [0, 1024, 2048, 3072, 4095] {
            let value = palette.min_value + (i as f32 / 4095.0) * (palette.max_value - palette.min_value);
            let idx = palette.value_to_index[i];
            let color = palette.colors[idx as usize];
            println!("  {} -> {} -> {} -> {:?}", value, i, idx, color);
        }
        
        // Test with specific values
        let test_values = vec![0.0f32, 25.0, 50.0, 75.0, 100.0];
        
        for &val in &test_values {
            // Get RGBA color
            let rgba = apply_style_gradient(&[val], 1, 1, style);
            let rgba_color = (rgba[0], rgba[1], rgba[2], rgba[3]);
            
            // Get indexed color - compute manually to debug
            let t = (val - palette.min_value) / (palette.max_value - palette.min_value);
            let lut_idx = (t * 4095.0) as usize;
            let palette_idx = palette.value_to_index[lut_idx.min(4095)];
            let indexed_color = palette.colors[palette_idx as usize];
            
            println!("Value {}: t={:.3}, lut={}, RGBA={:?}, Indexed idx={} color={:?}", 
                val, t, lut_idx, rgba_color, palette_idx, indexed_color);
            
            // Colors should be similar (may not be exact due to quantization)
            let r_diff = (rgba_color.0 as i32 - indexed_color.0 as i32).abs();
            let g_diff = (rgba_color.1 as i32 - indexed_color.1 as i32).abs();
            let b_diff = (rgba_color.2 as i32 - indexed_color.2 as i32).abs();
            
            assert!(r_diff <= 5, "Red mismatch at {}: {} vs {}", val, rgba_color.0, indexed_color.0);
            assert!(g_diff <= 5, "Green mismatch at {}: {} vs {}", val, rgba_color.1, indexed_color.1);
            assert!(b_diff <= 5, "Blue mismatch at {}: {} vs {}", val, rgba_color.2, indexed_color.2);
        }
    }
}
