//! Gradient/heatmap rendering for gridded weather data.

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
    let norm_min_lon = if min_lon < 0.0 {
        min_lon + 360.0
    } else {
        min_lon
    };
    let norm_max_lon = if max_lon < 0.0 {
        max_lon + 360.0
    } else {
        max_lon
    };

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
    if subset_width == 0
        || subset_height == 0
        || subset_width > grid_width
        || subset_height > grid_height
    {
        // Invalid subset, return full grid
        eprintln!(
            "Warning: Invalid subset bounds, returning full grid. subset={}x{}, grid={}x{}",
            subset_width, subset_height, grid_width, grid_height
        );
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
///
/// # Performance
/// Uses rayon for parallel row processing when resampling is needed.
/// Buffer pooling reduces allocation overhead under high load.
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

    // Use buffer pool for the output buffer
    crate::buffer_pool::take_resample_buffer(dst_width, dst_height, |output| {
        resample_grid_into(data, src_width, src_height, dst_width, dst_height, output);
    })
}

/// Resample grid data into a pre-allocated buffer.
fn resample_grid_into(
    data: &[f32],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
    output: &mut [f32],
) {
    use rayon::prelude::*;

    let x_ratio = (src_width - 1) as f32 / (dst_width - 1) as f32;
    let y_ratio = (src_height - 1) as f32 / (dst_height - 1) as f32;

    // Process rows in parallel - each output row depends only on source data (read-only)
    output
        .par_chunks_mut(dst_width)
        .enumerate()
        .for_each(|(y, row)| {
            let src_y = y as f32 * y_ratio;
            let y1 = src_y.floor() as usize;
            let y2 = (y1 + 1).min(src_height - 1);
            let dy = src_y - y1 as f32;

            for x in 0..dst_width {
                let src_x = x as f32 * x_ratio;

                // Bilinear interpolation
                let x1 = src_x.floor() as usize;
                let x2 = (x1 + 1).min(src_width - 1);
                let dx = src_x - x1 as f32;

                // Get the four surrounding values
                let v11 = data.get(y1 * src_width + x1).copied().unwrap_or(0.0);
                let v21 = data.get(y1 * src_width + x2).copied().unwrap_or(0.0);
                let v12 = data.get(y2 * src_width + x1).copied().unwrap_or(0.0);
                let v22 = data.get(y2 * src_width + x2).copied().unwrap_or(0.0);

                // Interpolate
                let v1 = v11 * (1.0 - dx) + v21 * dx;
                let v2 = v12 * (1.0 - dx) + v22 * dx;
                let value = v1 * (1.0 - dy) + v2 * dy;

                row[x] = value;
            }
        });
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
        Self {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }
    }
}

/// Linear color interpolation
pub fn interpolate_color(color1: Color, color2: Color, t: f32) -> Color {
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
///
/// # Performance
/// Uses rayon for parallel row processing. A 256×256 tile has 65,536 pixels,
/// which are processed across multiple CPU cores for improved throughput.
/// Buffer pooling reduces allocation overhead under high load.
pub fn render_grid<F>(
    data: &[f32],
    width: usize,
    height: usize,
    min_val: f32,
    max_val: f32,
    color_fn: F,
) -> Vec<u8>
where
    F: Fn(f32) -> Color + Sync,
{
    // Use buffer pool for the pixel buffer
    crate::buffer_pool::take_pixel_buffer(width, height, |pixels| {
        render_grid_into(data, width, height, min_val, max_val, &color_fn, pixels);
    })
}

/// Render grid data into a pre-allocated buffer.
///
/// Note: `height` is only used for debug validation. The actual row count
/// is derived from the `pixels` slice length to support partial rendering.
fn render_grid_into<F>(
    data: &[f32],
    width: usize,
    height: usize,
    min_val: f32,
    max_val: f32,
    color_fn: &F,
    pixels: &mut [u8],
) where
    F: Fn(f32) -> Color + Sync,
{
    use rayon::prelude::*;

    // Validate buffer size matches expected dimensions
    debug_assert_eq!(
        pixels.len(),
        width * height * 4,
        "pixel buffer size mismatch: expected {}x{}x4={}, got {}",
        width,
        height,
        width * height * 4,
        pixels.len()
    );

    let range = max_val - min_val;
    let range = if range.abs() < 0.001 { 1.0 } else { range };

    // Process rows in parallel - each row is independent
    // row_bytes = width * 4 (RGBA)
    let row_bytes = width * 4;

    pixels
        .par_chunks_mut(row_bytes)
        .enumerate()
        .for_each(|(y, row)| {
            let data_row_start = y * width;

            for x in 0..width {
                let data_idx = data_row_start + x;
                let pixel_idx = x * 4;

                if data_idx < data.len() {
                    let value = data[data_idx];

                    // Handle NaN as transparent (for data outside geographic bounds)
                    if value.is_nan() {
                        row[pixel_idx] = 0; // R
                        row[pixel_idx + 1] = 0; // G
                        row[pixel_idx + 2] = 0; // B
                        row[pixel_idx + 3] = 0; // A (transparent)
                    } else {
                        let normalized = (value - min_val) / range;
                        let normalized = normalized.max(0.0).min(1.0);

                        let color = color_fn(normalized);

                        row[pixel_idx] = color.r;
                        row[pixel_idx + 1] = color.g;
                        row[pixel_idx + 2] = color.b;
                        row[pixel_idx + 3] = color.a;
                    }
                }
            }
        });
}
