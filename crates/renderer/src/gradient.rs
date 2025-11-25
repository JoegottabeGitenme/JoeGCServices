//! Gradient/heatmap rendering for gridded weather data.

use std::cmp::min;

/// Color value in RGBA format
#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn transparent() -> Self {
        Self { r: 0, g: 0, b: 0, a: 0 }
    }
}

/// Temperature color scale (Celsius)
/// Maps temperature values to colors
pub fn temperature_color(temp_celsius: f32) -> Color {
    // Temperature scale (common weather visualization):
    // -50°C: Deep purple
    // -30°C: Blue
    // 0°C: Cyan
    // 10°C: Green
    // 20°C: Yellow
    // 30°C: Orange
    // 40°C: Red
    // 50°C: Dark red

    match temp_celsius {
        t if t < -50.0 => Color::new(25, 0, 76, 255),     // Deep purple
        t if t < -30.0 => interpolate_color(
            Color::new(25, 0, 76, 255),
            Color::new(0, 0, 255, 255),
            (t + 50.0) / 20.0,
        ),
        t if t < 0.0 => interpolate_color(
            Color::new(0, 0, 255, 255),
            Color::new(0, 255, 255, 255),
            (t + 30.0) / 30.0,
        ),
        t if t < 10.0 => interpolate_color(
            Color::new(0, 255, 255, 255),
            Color::new(0, 255, 0, 255),
            (t + 0.0) / 10.0,
        ),
        t if t < 20.0 => interpolate_color(
            Color::new(0, 255, 0, 255),
            Color::new(255, 255, 0, 255),
            (t - 10.0) / 10.0,
        ),
        t if t < 30.0 => interpolate_color(
            Color::new(255, 255, 0, 255),
            Color::new(255, 165, 0, 255),
            (t - 20.0) / 10.0,
        ),
        t if t < 40.0 => interpolate_color(
            Color::new(255, 165, 0, 255),
            Color::new(255, 0, 0, 255),
            (t - 30.0) / 10.0,
        ),
        t if t < 50.0 => interpolate_color(
            Color::new(255, 0, 0, 255),
            Color::new(139, 0, 0, 255),
            (t - 40.0) / 10.0,
        ),
        _ => Color::new(139, 0, 0, 255), // Dark red
    }
}

/// Linear color interpolation
fn interpolate_color(color1: Color, color2: Color, t: f32) -> Color {
    let t = t.max(0.0).min(1.0);
    let t_inv = 1.0 - t;

    Color::new(
        ((color1.r as f32 * t_inv) + (color2.r as f32 * t)) as u8,
        ((color1.g as f32 * t_inv) + (color2.g as f32 * t)) as u8,
        ((color1.b as f32 * t_inv) + (color2.b as f32 * t)) as u8,
        ((color1.a as f32 * t_inv) + (color2.a as f32 * t)) as u8,
    )
}

/// Render grid data as a gradient heatmap
/// 
/// # Arguments
/// - `data`: 2D grid of values (row-major order)
/// - `width`: Number of columns
/// - `height`: Number of rows
/// - `min_val`: Minimum value in the data (for scaling)
/// - `max_val`: Maximum value in the data (for scaling)
/// - `color_fn`: Function to convert a normalized value (0-1) to a color
///
/// # Returns
/// RGBA pixel data (4 bytes per pixel)
pub fn render_grid<F>(
    data: &[f32],
    width: usize,
    height: usize,
    min_val: f32,
    max_val: f32,
    color_fn: F,
) -> Vec<u8>
where
    F: Fn(f32) -> Color,
{
    let mut pixels = vec![0u8; width * height * 4];
    
    let range = max_val - min_val;
    let range = if range.abs() < 0.001 { 1.0 } else { range };

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if idx < data.len() {
                let value = data[idx];
                let normalized = (value - min_val) / range;
                let normalized = normalized.max(0.0).min(1.0);
                
                let color = color_fn(normalized);
                
                let pixel_idx = idx * 4;
                pixels[pixel_idx] = color.r;
                pixels[pixel_idx + 1] = color.g;
                pixels[pixel_idx + 2] = color.b;
                pixels[pixel_idx + 3] = color.a;
            }
        }
    }

    pixels
}

/// Render temperature grid data
pub fn render_temperature(
    data: &[f32],
    width: usize,
    height: usize,
    min_temp: f32,
    max_temp: f32,
) -> Vec<u8> {
    render_grid(data, width, height, min_temp, max_temp, |norm| {
        // Convert normalized value (0-1) back to temperature range
        let temp = min_temp + (max_temp - min_temp) * norm;
        temperature_color(temp)
    })
}
