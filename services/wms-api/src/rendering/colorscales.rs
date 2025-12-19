//! Color scale rendering for weather data.
//!
//! This module provides color mapping functions that load styles from JSON configuration
//! files in `config/styles/`. The JSON files are the **single source of truth** for all
//! color scales - there are no fallback gradients in production.
//!
//! Style files are determined by the layer configuration in `config/layers/*.yaml`.
//! Each layer defines its `style_file` which maps to a file in `config/styles/`.
//!
//! ## Error Handling
//!
//! If a style file cannot be loaded or doesn't contain the requested style, an error
//! is returned. This enforces that all layers must have properly configured styles.

use renderer::style::{StyleConfig, apply_style_gradient};
use renderer::gradient;

// ============================================================================
// Color conversion utilities
// ============================================================================

/// Convert HSV to RGB (simplified version)
///
/// # Arguments
/// * `h` - Hue in degrees (0-360)
/// * `s` - Saturation (0-1)
/// * `v` - Value/brightness (0-1)
///
/// # Returns
/// RGB tuple as (u8, u8, u8)
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

// ============================================================================
// Style-based rendering
// ============================================================================

/// Render data using a style loaded from the given style file path.
///
/// This is the primary rendering function. The style file path should come from
/// the layer configuration (via `LayerConfigRegistry.get_style_file_for_parameter()`).
///
/// # Arguments
/// * `data` - Grid data values to render
/// * `style_file_path` - Full path to the style JSON file
/// * `style_name` - Optional specific style name; if None, uses the default style
/// * `width` - Output image width in pixels
/// * `height` - Output image height in pixels
///
/// # Returns
/// RGBA pixel data as Vec<u8> (length = width * height * 4), or an error if the style
/// cannot be loaded or is invalid.
///
/// # Errors
/// Returns an error if:
/// - The style file cannot be loaded
/// - The requested style name is not found in the file
/// - The style type is not suitable for gradient rendering
pub fn render_with_style_file(
    data: &[f32],
    style_file_path: &str,
    style_name: Option<&str>,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, String> {
    let config = StyleConfig::from_file(style_file_path)
        .map_err(|e| format!("Failed to load style file '{}': {}", style_file_path, e))?;
    
    // Get requested style or default style
    let style = if let Some(name) = style_name {
        config.get_style(name).ok_or_else(|| {
            format!(
                "Style '{}' not found in style file '{}'. Available styles: {:?}",
                name,
                style_file_path,
                config.styles.keys().collect::<Vec<_>>()
            )
        })?
    } else {
        config.get_default_style().map(|(_, s)| s).ok_or_else(|| {
            format!(
                "No default style defined in style file '{}'. Available styles: {:?}",
                style_file_path,
                config.styles.keys().collect::<Vec<_>>()
            )
        })?
    };
    
    // Check if the style type is "gradient" or similar that we can render
    if style.style_type == "gradient" || style.style_type == "filled_contour" {
        Ok(apply_style_gradient(data, width, height, style))
    } else {
        Err(format!(
            "Style type '{}' in '{}' is not suitable for gradient rendering. Expected 'gradient' or 'filled_contour'.",
            style.style_type,
            style_file_path
        ))
    }
}

/// Render data using a fallback gradient.
///
/// This is used as a last resort when:
/// - No style file is provided
/// - The style file cannot be loaded
/// - The style file doesn't have a suitable style
///
/// # Arguments
/// * `data` - Grid data values to render
/// * `parameter` - Parameter name (for logging/diagnostics only)
/// * `min_val` - Minimum data value
/// * `max_val` - Maximum data value
/// * `width` - Output image width in pixels
/// * `height` - Output image height in pixels
///
/// # Returns
/// RGBA pixel data as Vec<u8> (length = width * height * 4)
#[allow(dead_code)] // Used in tests
pub fn render_by_parameter(
    data: &[f32],
    _parameter: &str,
    min_val: f32,
    max_val: f32,
    width: usize,
    height: usize,
) -> Vec<u8> {
    // Note: We no longer do parameter-to-style mapping here.
    // The style file should come from the layer configuration.
    // This function now just renders with a generic fallback gradient.
    render_fallback_gradient(data, width, height, min_val, max_val)
}

/// Render data with a simple blue-red gradient (fallback when no style is configured).
fn render_fallback_gradient(
    data: &[f32],
    width: usize,
    height: usize,
    min_val: f32,
    max_val: f32,
) -> Vec<u8> {
    renderer::gradient::render_grid(
        data,
        width,
        height,
        min_val,
        max_val,
        |norm| {
            // Generic blue-red gradient (cold to hot)
            let hue = (1.0 - norm) * 240.0; // Blue (240°) to Red (0°)
            let rgb = hsv_to_rgb(hue, 1.0, 1.0);
            gradient::Color::new(rgb.0, rgb.1, rgb.2, 255)
        },
    )
}
