//! Numbers rendering for displaying numeric values at grid points.

use image::{ImageBuffer, Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use rusttype::{Font, Scale};

// Re-export ColorStop for convenience
pub use crate::style::ColorStop;

/// Embedded font data - DejaVu Sans Mono (a clean, readable monospace font)
const FONT_DATA: &[u8] = include_bytes!("../assets/DejaVuSansMono.ttf");

/// Unit transformation for converting raw data values to display values.
/// Supports subtraction (K→C), division (Pa→hPa), and linear (scale + offset).
#[derive(Debug, Clone, Copy)]
pub enum UnitTransform {
    /// No transformation
    None,
    /// Subtract a value (e.g., K→C: subtract 273.15)
    Subtract(f32),
    /// Divide by a value (e.g., Pa→hPa: divide by 100)
    Divide(f32),
    /// Linear transform: value * scale + offset
    Linear { scale: f32, offset: f32 },
}

impl UnitTransform {
    /// Apply the transformation to a value
    pub fn apply(&self, value: f32) -> f32 {
        match self {
            Self::None => value,
            Self::Subtract(offset) => value - offset,
            Self::Divide(divisor) => value / divisor,
            Self::Linear { scale, offset } => value * scale + offset,
        }
    }
    
    /// Create from legacy Option<f32> format for backwards compatibility.
    /// Positive values are subtraction, negative values indicate division (abs value).
    pub fn from_legacy(offset: Option<f32>) -> Self {
        match offset {
            None => Self::None,
            Some(v) if v < 0.0 => Self::Divide(v.abs()),
            Some(v) => Self::Subtract(v),
        }
    }
}

impl Default for UnitTransform {
    fn default() -> Self {
        Self::None
    }
}

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
    /// DEPRECATED: Use unit_transform instead
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

/// Format a numeric value for display (1 decimal place)
pub fn format_value(value: f32) -> String {
    // Round to 1 decimal place - matches typical weather data precision
    // and allows values to match GetFeatureInfo queries
    let rounded = (value * 10.0).round() / 10.0;
    format!("{:.1}", rounded)
}

/// Format a numeric value with specified decimal places
pub fn format_value_with_decimals(value: f32, decimal_places: u32) -> String {
    match decimal_places {
        0 => format!("{:.0}", value.round()),
        1 => format!("{:.1}", (value * 10.0).round() / 10.0),
        2 => format!("{:.2}", (value * 100.0).round() / 100.0),
        _ => format!("{:.1}", value), // Default to 1 decimal
    }
}

/// Get RGB color for a value based on color stops
pub fn get_color_for_value(value: f32, stops: &[ColorStop]) -> Rgba<u8> {
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

/// Configuration for geographically-aligned numbers rendering
#[derive(Debug, Clone)]
pub struct AlignedNumbersConfig {
    /// Geographic spacing between numbers in degrees
    pub geo_spacing: f64,
    /// Font size for numbers
    pub font_size: f32,
    /// Color stops for value-to-color mapping
    pub color_stops: Vec<ColorStop>,
    /// Optional unit conversion offset (e.g., 273.15 for K to C)
    pub unit_conversion: Option<f32>,
    /// Bounding box for the render area [min_lon, min_lat, max_lon, max_lat]
    pub bbox: [f64; 4],
    /// Visible area (for filtering - only draw numbers in this region)
    /// This is the center tile bbox when using expanded rendering
    pub visible_bbox: Option<[f64; 4]>,
}

/// Render numeric values with geographically-aligned positions.
/// 
/// Numbers are placed at positions that snap to a global grid based on
/// geographic coordinates, ensuring consistent positioning across tiles.
///
/// # Arguments
/// * `grid` - 2D array of data values [lat][lon]
/// * `width` - Output image width in pixels
/// * `height` - Output image height in pixels  
/// * `config` - Aligned numbers rendering configuration
///
/// # Returns
/// RGBA image with numeric values drawn at geographically-aligned positions
pub fn render_numbers_aligned(
    grid: &[Vec<f32>],
    width: u32,
    height: u32,
    config: &AlignedNumbersConfig,
) -> RgbaImage {
    tracing::info!(
        geo_spacing = config.geo_spacing,
        bbox = ?config.bbox,
        visible_bbox = ?config.visible_bbox,
        "Rendering numbers with geographic alignment"
    );
    
    let mut img = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 0]));

    if grid.is_empty() || grid[0].is_empty() {
        return img;
    }

    // Load font
    let font = match Font::try_from_bytes(FONT_DATA) {
        Some(f) => f,
        None => {
            tracing::warn!("Failed to load font for numbers rendering");
            return img;
        }
    };

    let scale = Scale::uniform(config.font_size);

    let grid_height = grid.len();
    let grid_width = grid[0].len();

    let [min_lon, min_lat, max_lon, max_lat] = config.bbox;
    let lon_range = max_lon - min_lon;
    let lat_range = max_lat - min_lat;

    // Calculate the first aligned position (snap to geo_spacing grid)
    let spacing = config.geo_spacing;
    let first_lon = (min_lon / spacing).ceil() * spacing;
    let first_lat = (min_lat / spacing).ceil() * spacing;

    // Get visible area (if not specified, use full bbox)
    let [vis_min_lon, vis_min_lat, vis_max_lon, vis_max_lat] = config.visible_bbox.unwrap_or(config.bbox);

    // Iterate over aligned geographic positions
    let mut geo_lat = first_lat;
    while geo_lat <= max_lat {
        let mut geo_lon = first_lon;
        while geo_lon <= max_lon {
            // Check if this position is in the visible area (center tile)
            // Add small buffer for numbers that might extend into visible area
            let text_buffer_deg = spacing * 0.3; // Buffer for text width
            if geo_lon < vis_min_lon - text_buffer_deg || geo_lon > vis_max_lon + text_buffer_deg ||
               geo_lat < vis_min_lat - text_buffer_deg || geo_lat > vis_max_lat + text_buffer_deg {
                geo_lon += spacing;
                continue;
            }

            // Convert geographic to pixel coordinates
            let px = ((geo_lon - min_lon) / lon_range * width as f64) as i32;
            let py = ((max_lat - geo_lat) / lat_range * height as f64) as i32;

            // Skip if outside render area
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                geo_lon += spacing;
                continue;
            }

            // Convert geographic to grid coordinates for value lookup
            let grid_x = ((geo_lon - min_lon) / lon_range * grid_width as f64) as usize;
            let grid_y = ((max_lat - geo_lat) / lat_range * grid_height as f64) as usize;

            // Bounds check
            if grid_x >= grid_width || grid_y >= grid_height {
                geo_lon += spacing;
                continue;
            }

            // Get value with bilinear interpolation
            let value = sample_grid_bilinear(grid, grid_width, grid_height, 
                (geo_lon - min_lon) / lon_range * grid_width as f64,
                (max_lat - geo_lat) / lat_range * grid_height as f64);

            // Skip NaN values
            if value.is_nan() {
                geo_lon += spacing;
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

            // Get color for this value
            let color = get_color_for_value(value, &config.color_stops);

            // Draw background rectangle for better readability
            draw_text_background(&mut img, &text, px, py, config.font_size);

            // Draw the text
            draw_text_mut(&mut img, color, px, py, scale, &font, &text);

            geo_lon += spacing;
        }
        geo_lat += spacing;
    }

    img
}

/// Sample grid value using bilinear interpolation
fn sample_grid_bilinear(grid: &[Vec<f32>], width: usize, height: usize, x: f64, y: f64) -> f32 {
    let x1 = x.floor() as usize;
    let y1 = y.floor() as usize;
    let x2 = (x1 + 1).min(width - 1);
    let y2 = (y1 + 1).min(height - 1);

    if x1 >= width || y1 >= height {
        return f32::NAN;
    }

    let dx = (x - x1 as f64) as f32;
    let dy = (y - y1 as f64) as f32;

    let v11 = grid[y1][x1];
    let v21 = if x2 < width { grid[y1][x2] } else { v11 };
    let v12 = if y2 < height { grid[y2][x1] } else { v11 };
    let v22 = if x2 < width && y2 < height { grid[y2][x2] } else { v11 };

    // Skip if any corner is NaN
    if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
        return f32::NAN;
    }

    let v1 = v11 * (1.0 - dx) + v21 * dx;
    let v2 = v12 * (1.0 - dx) + v22 * dx;
    v1 * (1.0 - dy) + v2 * dy
}

/// Configuration for rendering numbers at exact source grid points
#[derive(Debug, Clone)]
pub struct GridPointNumbersConfig {
    /// Font size for numbers
    pub font_size: f32,
    /// Color stops for value-to-color mapping
    pub color_stops: Vec<ColorStop>,
    /// Optional unit conversion offset (e.g., 273.15 for K to C)
    /// DEPRECATED: Use unit_transform instead for new code
    pub unit_conversion: Option<f32>,
    /// Unit transformation to apply to values before display
    pub unit_transform: UnitTransform,
    /// Render bounding box [min_lon, min_lat, max_lon, max_lat] in -180 to 180 range
    pub render_bbox: [f64; 4],
    /// Original source grid bounding box [min_lon, min_lat, max_lon, max_lat]
    /// May be in 0-360 format for some datasets (like GFS)
    pub source_bbox: [f64; 4],
    /// Original source grid dimensions (width, height)
    pub source_dims: (usize, usize),
    /// Visible area (for filtering - only draw numbers in this region)
    /// This is the center tile bbox when using expanded rendering
    pub visible_bbox: Option<[f64; 4]>,
    /// Minimum pixel spacing between numbers (to avoid overcrowding)
    pub min_pixel_spacing: u32,
    /// Whether the source grid uses 0-360 longitude range
    pub source_uses_360: bool,
}

/// Render numeric values at exact source grid point locations.
/// 
/// This shows the exact data values at each grid point, which is useful for
/// debugging and validation. Numbers are placed at the geographic locations
/// corresponding to the original data grid points.
///
/// # Arguments
/// * `grid` - 2D array of resampled data values [y][x] matching render dimensions
/// * `width` - Output image width in pixels
/// * `height` - Output image height in pixels  
/// * `config` - Grid point numbers rendering configuration
///
/// # Returns
/// RGBA image with numeric values drawn at source grid point locations
pub fn render_numbers_at_grid_points(
    grid: &[Vec<f32>],
    width: u32,
    height: u32,
    config: &GridPointNumbersConfig,
) -> RgbaImage {
    let mut img = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 0]));

    if grid.is_empty() || grid[0].is_empty() {
        return img;
    }

    // Load font
    let font = match Font::try_from_bytes(FONT_DATA) {
        Some(f) => f,
        None => {
            tracing::warn!("Failed to load font for numbers rendering");
            return img;
        }
    };

    let scale = Scale::uniform(config.font_size);

    let [render_min_lon, render_min_lat, render_max_lon, render_max_lat] = config.render_bbox;
    let render_lon_range = render_max_lon - render_min_lon;
    let render_lat_range = render_max_lat - render_min_lat;

    let [source_min_lon, source_min_lat, source_max_lon, source_max_lat] = config.source_bbox;
    let (source_width, source_height) = config.source_dims;
    
    // Calculate source grid resolution
    let source_lon_step = (source_max_lon - source_min_lon) / (source_width - 1) as f64;
    let source_lat_step = (source_max_lat - source_min_lat) / (source_height - 1) as f64;

    // Get visible area (if not specified, use full render bbox)
    let [vis_min_lon, vis_min_lat, vis_max_lon, vis_max_lat] = config.visible_bbox.unwrap_or(config.render_bbox);

    // Helper to convert longitude from -180/180 to 0-360 format if needed
    let to_source_lon = |lon: f64| -> f64 {
        if config.source_uses_360 && lon < 0.0 {
            lon + 360.0
        } else {
            lon
        }
    };
    
    // Helper to convert longitude from 0-360 to -180/180 format for display
    let from_source_lon = |lon: f64| -> f64 {
        if config.source_uses_360 && lon > 180.0 {
            lon - 360.0
        } else {
            lon
        }
    };

    // Convert render bbox to source coordinates for finding grid indices
    let render_min_lon_src = to_source_lon(render_min_lon);
    let render_max_lon_src = to_source_lon(render_max_lon);

    // Calculate step to skip grid points if they would be too close together
    let pixels_per_source_lon = (width as f64 / render_lon_range) * source_lon_step;
    let pixels_per_source_lat = (height as f64 / render_lat_range) * source_lat_step;
    
    let step_x = ((config.min_pixel_spacing as f64 / pixels_per_source_lon).ceil() as usize).max(1);
    let step_y = ((config.min_pixel_spacing as f64 / pixels_per_source_lat).ceil() as usize).max(1);

    tracing::info!(
        source_dims = ?config.source_dims,
        source_lon_step = source_lon_step,
        source_lat_step = source_lat_step,
        step_x = step_x,
        step_y = step_y,
        source_uses_360 = config.source_uses_360,
        visible_bbox = ?config.visible_bbox,
        "Rendering numbers at source grid points"
    );

    // Find source grid indices that fall within the render bbox (in source coordinates)
    let start_i = ((render_min_lon_src - source_min_lon) / source_lon_step).floor() as i64;
    let end_i = ((render_max_lon_src - source_min_lon) / source_lon_step).ceil() as i64;
    let start_j = ((render_min_lat - source_min_lat) / source_lat_step).floor() as i64;
    let end_j = ((render_max_lat - source_min_lat) / source_lat_step).ceil() as i64;

    // Align to step boundaries for consistent placement across tiles
    let start_i = (start_i / step_x as i64) * step_x as i64;
    let start_j = (start_j / step_y as i64) * step_y as i64;

    let grid_height = grid.len();
    let grid_width = grid[0].len();

    // Iterate over source grid points
    let mut j = start_j;
    while j <= end_j {
        if j < 0 || j >= source_height as i64 {
            j += step_y as i64;
            continue;
        }

        let mut i = start_i;
        while i <= end_i {
            if i < 0 || i >= source_width as i64 {
                i += step_x as i64;
                continue;
            }

            // Calculate geographic position of this grid point (in source coordinates)
            let geo_lon_src = source_min_lon + (i as f64 * source_lon_step);
            let geo_lat = source_min_lat + (j as f64 * source_lat_step);
            
            // Convert to display coordinates (-180 to 180)
            let geo_lon = from_source_lon(geo_lon_src);

            // Check if this position is in the visible area (center tile)
            // Use a small buffer for text that might extend into visible area
            let text_buffer_lon = source_lon_step * 0.5;
            let text_buffer_lat = source_lat_step * 0.5;
            if geo_lon < vis_min_lon - text_buffer_lon || geo_lon > vis_max_lon + text_buffer_lon ||
               geo_lat < vis_min_lat - text_buffer_lat || geo_lat > vis_max_lat + text_buffer_lat {
                i += step_x as i64;
                continue;
            }

            // Convert geographic to pixel coordinates in the render image
            let px = ((geo_lon - render_min_lon) / render_lon_range * width as f64) as i32;
            let py = ((render_max_lat - geo_lat) / render_lat_range * height as f64) as i32;

            // Skip if outside render area
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                i += step_x as i64;
                continue;
            }

            // Sample value from the resampled grid at this pixel location
            // We use the resampled grid coordinates that correspond to this geographic position
            let grid_x = ((geo_lon - render_min_lon) / render_lon_range * grid_width as f64) as usize;
            let grid_y = ((render_max_lat - geo_lat) / render_lat_range * grid_height as f64) as usize;

            if grid_x >= grid_width || grid_y >= grid_height {
                i += step_x as i64;
                continue;
            }

            let value = grid[grid_y][grid_x];

            // Skip NaN values
            if value.is_nan() {
                i += step_x as i64;
                continue;
            }

            // Apply unit transformation if configured
            // Prefer new unit_transform field, fall back to legacy unit_conversion
            let display_value = match config.unit_transform {
                UnitTransform::None => {
                    // Fall back to legacy conversion if present
                    if let Some(offset) = config.unit_conversion {
                        value - offset
                    } else {
                        value
                    }
                }
                ref transform => transform.apply(value),
            };

            // Format the value
            let text = format_value(display_value);

            // Get color for this value (use transformed value for color mapping)
            let color = get_color_for_value(display_value, &config.color_stops);

            // Calculate text dimensions for centering
            let char_width = (config.font_size * 0.6) as i32;
            let text_width = (text.len() as i32) * char_width;
            let text_height = config.font_size as i32;
            
            // Center the text on the grid point
            let centered_px = px - text_width / 2;
            let centered_py = py - text_height / 2;

            // Draw background rectangle for better readability (centered)
            draw_text_background(&mut img, &text, centered_px, centered_py, config.font_size);

            // Draw the text (centered)
            draw_text_mut(&mut img, color, centered_px, centered_py, scale, &font, &text);

            i += step_x as i64;
        }
        j += step_y as i64;
    }

    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_value() {
        // format_value outputs 1 decimal place
        assert_eq!(format_value(273.0), "273.0");
        assert_eq!(format_value(273.5), "273.5");
        assert_eq!(format_value(273.15), "273.2"); // rounds to 1 decimal
        assert_eq!(format_value(0.0), "0.0");
        assert_eq!(format_value(-5.5), "-5.5");
        assert_eq!(format_value(-5.55), "-5.6"); // rounds
    }

    #[test]
    fn test_calculate_grid_sampling() {
        // 100x100 grid, 256x256 image, want 60px spacing
        let (step_y, step_x) = calculate_grid_sampling(100, 100, 256, 256, 60);
        // Each grid cell is ~2.56 pixels, so we need ~24 cells to get 60 pixels
        assert!((23..=25).contains(&step_x));
        assert!((23..=25).contains(&step_y));
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
