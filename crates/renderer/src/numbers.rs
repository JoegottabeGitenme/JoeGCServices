//! Numbers rendering for displaying numeric values at grid points.

use image::{ImageBuffer, Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use rusttype::{Font, Scale};
use crate::style::ColorStop;

/// Embedded font data - DejaVu Sans Mono (a clean, readable monospace font)
const FONT_DATA: &[u8] = include_bytes!("../assets/DejaVuSansMono.ttf");

/// Configuration for numbers rendering
#[derive(Debug, Clone)]
pub struct NumbersConfig {
    /// Target spacing between numbers in pixels
    pub spacing: u32,
    /// Font size for numbers
    pub font_size: f32,
    /// Color stops for value-to-color mapping
    pub color_stops: Vec<ColorStop>,
    /// Optional unit conversion offset (e.g., 273.15 for K to C)
    pub unit_conversion: Option<f32>,
}

impl Default for NumbersConfig {
    fn default() -> Self {
        Self {
            spacing: 60,
            font_size: 12.0,
            color_stops: Vec::new(),
            unit_conversion: None,
        }
    }
}

/// Render numeric values from a grid onto an image
///
/// # Arguments
/// * `grid` - 2D array of data values [lat][lon]
/// * `width` - Output image width in pixels
/// * `height` - Output image height in pixels
/// * `config` - Numbers rendering configuration
///
/// # Returns
/// RGBA image with numeric values drawn at sampled grid points
pub fn render_numbers(
    grid: &[Vec<f32>],
    width: u32,
    height: u32,
    config: &NumbersConfig,
) -> RgbaImage {
    let mut img = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 0]));

    if grid.is_empty() || grid[0].is_empty() {
        return img;
    }

    // Load font
    let font = match Font::try_from_bytes(FONT_DATA) {
        Some(f) => f,
        None => {
            // If font loading fails, return empty image
            tracing::warn!("Failed to load font for numbers rendering");
            return img;
        }
    };

    let scale = Scale::uniform(config.font_size);

    let grid_height = grid.len();
    let grid_width = grid[0].len();

    // Calculate sampling step based on desired pixel spacing
    let (sample_step_y, sample_step_x) = calculate_grid_sampling(
        grid_height,
        grid_width,
        width,
        height,
        config.spacing,
    );

    // Draw numbers at sampled grid points
    let mut y_grid = 0;
    while y_grid < grid_height {
        let mut x_grid = 0;
        while x_grid < grid_width {
            let value = grid[y_grid][x_grid];

            // Skip NaN values
            if value.is_nan() {
                x_grid += sample_step_x;
                continue;
            }

            // Apply unit conversion if configured
            let display_value = if let Some(offset) = config.unit_conversion {
                value - offset
            } else {
                value
            };

            // Format the value
            let text = format_value(display_value);

            // Calculate pixel position
            let px = ((x_grid as f32 / grid_width as f32) * width as f32) as i32;
            let py = ((y_grid as f32 / grid_height as f32) * height as f32) as i32;

            // Get color for this value
            let color = get_color_for_value(value, &config.color_stops);

            // Draw background rectangle for better readability
            draw_text_background(&mut img, &text, px, py, config.font_size);

            // Draw the text
            if px >= 0 && py >= 0 && (px as u32) < width && (py as u32) < height {
                draw_text_mut(&mut img, color, px, py, scale, &font, &text);
            }

            x_grid += sample_step_x;
        }
        y_grid += sample_step_y;
    }

    img
}

/// Draw a semi-transparent white background behind text for readability
fn draw_text_background(img: &mut RgbaImage, text: &str, x: i32, y: i32, font_size: f32) {
    let bg_color = Rgba([255, 255, 255, 220]); // Semi-transparent white
    let padding = 2;
    
    // Estimate text dimensions (roughly)
    let char_width = (font_size * 0.6) as i32;
    let text_width = (text.len() as i32) * char_width;
    let text_height = font_size as i32;

    for dy in -padding..(text_height + padding) {
        for dx in -padding..(text_width + padding) {
            let px = x + dx;
            let py = y + dy;
            if px >= 0 && px < img.width() as i32 && py >= 0 && py < img.height() as i32 {
                img.put_pixel(px as u32, py as u32, bg_color);
            }
        }
    }
}

/// Calculate grid sampling step to achieve target pixel spacing
fn calculate_grid_sampling(
    grid_height: usize,
    grid_width: usize,
    img_width: u32,
    img_height: u32,
    target_spacing: u32,
) -> (usize, usize) {
    // Calculate pixels per grid cell
    let pixels_per_cell_x = img_width as f32 / grid_width as f32;
    let pixels_per_cell_y = img_height as f32 / grid_height as f32;

    // Calculate how many grid cells we need to skip to achieve target spacing
    let step_x = (target_spacing as f32 / pixels_per_cell_x).ceil() as usize;
    let step_y = (target_spacing as f32 / pixels_per_cell_y).ceil() as usize;

    // Ensure at least 1
    (step_y.max(1), step_x.max(1))
}

/// Format a numeric value for display
fn format_value(value: f32) -> String {
    // Round to 1 decimal place for cleaner display
    let rounded = (value * 10.0).round() / 10.0;
    
    // Check if it's effectively an integer
    if (rounded - rounded.round()).abs() < 0.05 {
        format!("{:.0}", rounded)
    } else {
        // One decimal place
        format!("{:.1}", rounded)
    }
}

/// Get RGB color for a value based on color stops
fn get_color_for_value(value: f32, stops: &[ColorStop]) -> Rgba<u8> {
    if stops.is_empty() {
        return Rgba([0, 0, 0, 255]); // Black default for better readability on white bg
    }

    // Find the two stops this value falls between
    let mut lower_stop = &stops[0];
    let mut upper_stop = &stops[0];

    for i in 0..stops.len() {
        if value >= stops[i].value {
            lower_stop = &stops[i];
        }
        if i < stops.len() - 1 && value <= stops[i + 1].value {
            upper_stop = &stops[i + 1];
            break;
        }
    }

    // If value is beyond range, use the boundary color
    if value <= stops[0].value {
        return parse_color(&stops[0].color);
    }
    if value >= stops[stops.len() - 1].value {
        return parse_color(&stops[stops.len() - 1].color);
    }

    // Interpolate between colors
    let lower_color = parse_color(&lower_stop.color);
    let upper_color = parse_color(&upper_stop.color);

    if lower_stop.value == upper_stop.value {
        return lower_color;
    }

    let t = (value - lower_stop.value) / (upper_stop.value - lower_stop.value);
    let t = t.clamp(0.0, 1.0);

    // Darken interpolated colors for better readability
    let r = (lower_color[0] as f32 + t * (upper_color[0] as f32 - lower_color[0] as f32)) as u8;
    let g = (lower_color[1] as f32 + t * (upper_color[1] as f32 - lower_color[1] as f32)) as u8;
    let b = (lower_color[2] as f32 + t * (upper_color[2] as f32 - lower_color[2] as f32)) as u8;
    
    // Darken the color slightly for better contrast on white background
    let darken = 0.7;
    Rgba([
        (r as f32 * darken) as u8,
        (g as f32 * darken) as u8,
        (b as f32 * darken) as u8,
        255,
    ])
}

/// Parse hex color string to RGBA
fn parse_color(hex: &str) -> Rgba<u8> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return Rgba([0, 0, 0, 255]);
    }

    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);

    Rgba([r, g, b, 255])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_value() {
        assert_eq!(format_value(273.0), "273");
        assert_eq!(format_value(273.5), "274"); // rounds to nearest
        assert_eq!(format_value(273.15), "273");
        assert_eq!(format_value(0.0), "0");
        assert_eq!(format_value(-5.5), "-6"); // rounds
    }

    #[test]
    fn test_calculate_grid_sampling() {
        // 100x100 grid, 256x256 image, want 60px spacing
        let (step_y, step_x) = calculate_grid_sampling(100, 100, 256, 256, 60);
        // Each grid cell is ~2.56 pixels, so we need ~24 cells to get 60 pixels
        assert!(step_x >= 23 && step_x <= 25);
        assert!(step_y >= 23 && step_y <= 25);
    }

    #[test]
    fn test_render_numbers_empty_grid() {
        let grid: Vec<Vec<f32>> = vec![];
        let config = NumbersConfig::default();
        let img = render_numbers(&grid, 256, 256, &config);
        assert_eq!(img.width(), 256);
        assert_eq!(img.height(), 256);
    }
}
