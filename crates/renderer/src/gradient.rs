//! Gradient/heatmap rendering for gridded weather data.

use std::cmp::min;

/// Subset global grid data to a geographic bounding box.
///
/// Assumes data is a global lat/lon grid with:
/// - Latitudes from 90°N to -90°S (top to bottom)
/// - Longitudes from 0° to 360° (or -180° to 180°)
/// - Data in row-major order
///
/// # Arguments
/// - `data`: Input global grid data (row-major order)
/// - `grid_width`: Full grid width (number of longitude points)
/// - `grid_height`: Full grid height (number of latitude points)
/// - `bbox`: [min_lon, min_lat, max_lon, max_lat] in degrees
///
/// # Returns
/// Subset grid data and new dimensions (width, height)
pub fn subset_grid(
    data: &[f32],
    grid_width: usize,
    grid_height: usize,
    bbox: &[f32; 4],
) -> (Vec<f32>, usize, usize) {
    let [min_lon, min_lat, max_lon, max_lat] = *bbox;
    
    // Convert geographic coordinates to grid indices
    // Assuming global grid: lat 90 to -90, lon 0 to 360 (or -180 to 180)
    
    // Normalize longitudes to 0-360 range
    let norm_min_lon = if min_lon < 0.0 { min_lon + 360.0 } else { min_lon };
    let norm_max_lon = if max_lon < 0.0 { max_lon + 360.0 } else { max_lon };
    
    // Calculate grid cell size (assuming pixel-as-point with centers at grid points)
    let lon_step = 360.0 / grid_width as f32;
    let lat_step = 180.0 / grid_height as f32;
    
    // Convert bbox to grid indices
    // GRIB grid: lat 90 to -90, lon 0 to 360
    // Grid points are at centers: lat[i] = 90 - i * lat_step, lon[j] = j * lon_step
    //
    // For continuous tile boundaries, we need to ensure adjacent tiles
    // extract overlapping boundary pixels from the source grid
    //
    // Strategy: Use floor() for min indices, ceil() for max indices
    // This ensures we capture all grid points that intersect the bbox
    let y_min = ((90.0 - max_lat) / lat_step).floor().max(0.0) as usize;
    let y_max = ((90.0 - min_lat) / lat_step).ceil().min(grid_height as f32) as usize;
    
    let x_min = (norm_min_lon / lon_step).floor().max(0.0) as usize;
    let x_max = (norm_max_lon / lon_step).ceil().min(grid_width as f32) as usize;
    
    // Ensure valid ranges
    let y_min = y_min.min(grid_height - 1);
    let y_max = y_max.min(grid_height).max(y_min + 1);
    let x_min = x_min.min(grid_width - 1);
    let x_max = x_max.min(grid_width).max(x_min + 1);
    
    let subset_width = x_max - x_min;
    let subset_height = y_max - y_min;
    
    // Sanity check
    if subset_width == 0 || subset_height == 0 || subset_width > grid_width || subset_height > grid_height {
        // Invalid subset, return full grid
        eprintln!("Warning: Invalid subset bounds, returning full grid. subset={}x{}, grid={}x{}", 
                  subset_width, subset_height, grid_width, grid_height);
        return (data.to_vec(), grid_width, grid_height);
    }
    
    // Extract subset
    let mut subset = Vec::with_capacity(subset_width * subset_height);
    
    for y in y_min..y_max {
        for x in x_min..x_max {
            let idx = y * grid_width + x;
            subset.push(data.get(idx).copied().unwrap_or(0.0));
        }
    }
    
    (subset, subset_width, subset_height)
}

/// Resample grid data to a different resolution using bilinear interpolation.
///
/// # Arguments
/// - `data`: Input grid data (row-major order)
/// - `src_width`: Source grid width
/// - `src_height`: Source grid height
/// - `dst_width`: Destination grid width
/// - `dst_height`: Destination grid height
///
/// # Returns
/// Resampled grid data at the requested resolution
pub fn resample_grid(
    data: &[f32],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
) -> Vec<f32> {
    if src_width == dst_width && src_height == dst_height {
        // No resampling needed
        return data.to_vec();
    }

    let mut output = vec![0.0f32; dst_width * dst_height];

    let x_ratio = (src_width - 1) as f32 / (dst_width - 1) as f32;
    let y_ratio = (src_height - 1) as f32 / (dst_height - 1) as f32;

    for y in 0..dst_height {
        for x in 0..dst_width {
            let src_x = x as f32 * x_ratio;
            let src_y = y as f32 * y_ratio;

            // Bilinear interpolation
            let x1 = src_x.floor() as usize;
            let y1 = src_y.floor() as usize;
            let x2 = (x1 + 1).min(src_width - 1);
            let y2 = (y1 + 1).min(src_height - 1);

            let dx = src_x - x1 as f32;
            let dy = src_y - y1 as f32;

            // Get the four surrounding values
            let v11 = data.get(y1 * src_width + x1).copied().unwrap_or(0.0);
            let v21 = data.get(y1 * src_width + x2).copied().unwrap_or(0.0);
            let v12 = data.get(y2 * src_width + x1).copied().unwrap_or(0.0);
            let v22 = data.get(y2 * src_width + x2).copied().unwrap_or(0.0);

            // Interpolate
            let v1 = v11 * (1.0 - dx) + v21 * dx;
            let v2 = v12 * (1.0 - dx) + v22 * dx;
            let value = v1 * (1.0 - dy) + v2 * dy;

            output[y * dst_width + x] = value;
        }
    }

    output
}

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

/// Wind speed color scale (m/s)
/// Maps wind speed values to colors (0-20 m/s range)
pub fn wind_speed_color(speed_ms: f32) -> Color {
    // Wind speed scale:
    // 0 m/s: Calm (light gray)
    // 5 m/s: Light breeze (light cyan)
    // 10 m/s: Moderate (yellow)
    // 15 m/s: Fresh gale (orange)
    // 20+ m/s: Strong (dark red)
    
    match speed_ms {
        s if s < 0.0 => Color::new(200, 200, 200, 255),    // Calm
        s if s < 5.0 => interpolate_color(
            Color::new(200, 200, 200, 255),
            Color::new(0, 200, 255, 255),
            s / 5.0,
        ),
        s if s < 10.0 => interpolate_color(
            Color::new(0, 200, 255, 255),
            Color::new(255, 255, 0, 255),
            (s - 5.0) / 5.0,
        ),
        s if s < 15.0 => interpolate_color(
            Color::new(255, 255, 0, 255),
            Color::new(255, 165, 0, 255),
            (s - 10.0) / 5.0,
        ),
        s if s < 20.0 => interpolate_color(
            Color::new(255, 165, 0, 255),
            Color::new(139, 0, 0, 255),
            (s - 15.0) / 5.0,
        ),
        _ => Color::new(75, 0, 0, 255),                     // Dark red
    }
}

/// Pressure color scale (hPa)
/// Maps pressure values to colors (950-1050 hPa range)
pub fn pressure_color(pressure_hpa: f32) -> Color {
    // Pressure scale:
    // <970 hPa: Low (purple/blue) - stormy
    // 970-990 hPa: Below normal (blue)
    // 990-1010 hPa: Normal (green)
    // 1010-1030 hPa: Above normal (yellow)
    // >1030 hPa: High (red)
    
    match pressure_hpa {
        p if p < 970.0 => Color::new(75, 0, 130, 255),      // Indigo (stormy)
        p if p < 990.0 => interpolate_color(
            Color::new(75, 0, 130, 255),
            Color::new(0, 0, 255, 255),
            (p - 970.0) / 20.0,
        ),
        p if p < 1010.0 => interpolate_color(
            Color::new(0, 0, 255, 255),
            Color::new(0, 255, 0, 255),
            (p - 990.0) / 20.0,
        ),
        p if p < 1030.0 => interpolate_color(
            Color::new(0, 255, 0, 255),
            Color::new(255, 255, 0, 255),
            (p - 1010.0) / 20.0,
        ),
        p if p < 1050.0 => interpolate_color(
            Color::new(255, 255, 0, 255),
            Color::new(255, 0, 0, 255),
            (p - 1030.0) / 20.0,
        ),
        _ => Color::new(139, 0, 0, 255),                     // Dark red
    }
}

/// Humidity color scale (0-100%)
/// Maps relative humidity values to colors
pub fn humidity_color(humidity_percent: f32) -> Color {
    // Humidity scale:
    // 0%: Dry (tan)
    // 25%: Dry-ish (light yellow)
    // 50%: Moderate (yellow-green)
    // 75%: Wet (light blue)
    // 100%: Saturated (dark blue)
    
    match humidity_percent {
        h if h < 0.0 => Color::new(210, 180, 140, 255),     // Tan
        h if h < 25.0 => interpolate_color(
            Color::new(210, 180, 140, 255),
            Color::new(255, 255, 150, 255),
            h / 25.0,
        ),
        h if h < 50.0 => interpolate_color(
            Color::new(255, 255, 150, 255),
            Color::new(173, 255, 47, 255),
            (h - 25.0) / 25.0,
        ),
        h if h < 75.0 => interpolate_color(
            Color::new(173, 255, 47, 255),
            Color::new(100, 200, 255, 255),
            (h - 50.0) / 25.0,
        ),
        h if h <= 100.0 => interpolate_color(
            Color::new(100, 200, 255, 255),
            Color::new(25, 50, 200, 255),
            (h - 75.0) / 25.0,
        ),
        _ => Color::new(25, 50, 200, 255),                   // Dark blue
    }
}

/// Render wind speed grid data
pub fn render_wind_speed(
    data: &[f32],
    width: usize,
    height: usize,
    min_speed: f32,
    max_speed: f32,
) -> Vec<u8> {
    render_grid(data, width, height, min_speed, max_speed, |norm| {
        let speed = min_speed + (max_speed - min_speed) * norm;
        wind_speed_color(speed)
    })
}

/// Render pressure grid data
pub fn render_pressure(
    data: &[f32],
    width: usize,
    height: usize,
    min_pressure: f32,
    max_pressure: f32,
) -> Vec<u8> {
    render_grid(data, width, height, min_pressure, max_pressure, |norm| {
        let pressure = min_pressure + (max_pressure - min_pressure) * norm;
        pressure_color(pressure)
    })
}

/// Render humidity grid data
pub fn render_humidity(
    data: &[f32],
    width: usize,
    height: usize,
    min_humidity: f32,
    max_humidity: f32,
) -> Vec<u8> {
    render_grid(data, width, height, min_humidity, max_humidity, |norm| {
        let humidity = min_humidity + (max_humidity - min_humidity) * norm;
        humidity_color(humidity)
    })
}
