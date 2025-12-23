//! Wind barb rendering using pre-generated SVG assets.
//!
//! This module uses SVG wind barbs from https://github.com/qulle/svg-wind-barbs
//! Licensed under BSD 2-Clause License - see assets/wind-barbs/LICENSE

use std::f64::consts::PI;

/// Embedded SVG wind barb assets (0-190 knots in 5kt increments)
const WIND_BARB_SVGS: &[(&str, &str)] = &[
    ("0", include_str!("../assets/wind-barbs/0.svg")),
    ("2", include_str!("../assets/wind-barbs/2.svg")),
    ("5", include_str!("../assets/wind-barbs/5.svg")),
    ("10", include_str!("../assets/wind-barbs/10.svg")),
    ("15", include_str!("../assets/wind-barbs/15.svg")),
    ("20", include_str!("../assets/wind-barbs/20.svg")),
    ("25", include_str!("../assets/wind-barbs/25.svg")),
    ("30", include_str!("../assets/wind-barbs/30.svg")),
    ("35", include_str!("../assets/wind-barbs/35.svg")),
    ("40", include_str!("../assets/wind-barbs/40.svg")),
    ("45", include_str!("../assets/wind-barbs/45.svg")),
    ("50", include_str!("../assets/wind-barbs/50.svg")),
    ("55", include_str!("../assets/wind-barbs/55.svg")),
    ("60", include_str!("../assets/wind-barbs/60.svg")),
    ("65", include_str!("../assets/wind-barbs/65.svg")),
    ("70", include_str!("../assets/wind-barbs/70.svg")),
    ("75", include_str!("../assets/wind-barbs/75.svg")),
    ("80", include_str!("../assets/wind-barbs/80.svg")),
    ("85", include_str!("../assets/wind-barbs/85.svg")),
    ("90", include_str!("../assets/wind-barbs/90.svg")),
    ("95", include_str!("../assets/wind-barbs/95.svg")),
    ("100", include_str!("../assets/wind-barbs/100.svg")),
    ("105", include_str!("../assets/wind-barbs/105.svg")),
    ("110", include_str!("../assets/wind-barbs/110.svg")),
    ("115", include_str!("../assets/wind-barbs/115.svg")),
    ("120", include_str!("../assets/wind-barbs/120.svg")),
    ("125", include_str!("../assets/wind-barbs/125.svg")),
    ("130", include_str!("../assets/wind-barbs/130.svg")),
    ("135", include_str!("../assets/wind-barbs/135.svg")),
    ("140", include_str!("../assets/wind-barbs/140.svg")),
    ("145", include_str!("../assets/wind-barbs/145.svg")),
    ("150", include_str!("../assets/wind-barbs/150.svg")),
    ("155", include_str!("../assets/wind-barbs/155.svg")),
    ("160", include_str!("../assets/wind-barbs/160.svg")),
    ("165", include_str!("../assets/wind-barbs/165.svg")),
    ("170", include_str!("../assets/wind-barbs/170.svg")),
    ("175", include_str!("../assets/wind-barbs/175.svg")),
    ("180", include_str!("../assets/wind-barbs/180.svg")),
    ("185", include_str!("../assets/wind-barbs/185.svg")),
    ("190", include_str!("../assets/wind-barbs/190.svg")),
];

/// Speed ranges in m/s for selecting the appropriate wind barb SVG (in knots)
/// Based on the table from svg-wind-barbs README
const SPEED_RANGES_MS: &[(f64, f64, &str)] = &[
    (0.0, 1.0, "0"),
    (1.0, 2.5, "2"),
    (2.5, 5.0, "5"),
    (5.0, 7.5, "10"),
    (7.5, 10.0, "15"),
    (10.0, 12.5, "20"),
    (12.5, 15.0, "25"),
    (15.0, 17.5, "30"),
    (17.5, 20.0, "35"),
    (20.0, 22.5, "40"),
    (22.5, 25.0, "45"),
    (25.0, 27.5, "50"),
    (27.5, 30.0, "55"),
    (30.0, 32.5, "60"),
    (32.5, 35.0, "65"),
    (35.0, 37.5, "70"),
    (37.5, 40.0, "75"),
    (40.0, 42.5, "80"),
    (42.5, 45.0, "85"),
    (45.0, 47.5, "90"),
    (47.5, 50.0, "95"),
    (50.0, 52.5, "100"),
    (52.5, 55.0, "105"),
    (55.0, 57.5, "110"),
    (57.5, 60.0, "115"),
    (60.0, 62.5, "120"),
    (62.5, 65.0, "125"),
    (65.0, 67.5, "130"),
    (67.5, 70.0, "135"),
    (70.0, 72.5, "140"),
    (72.5, 75.0, "145"),
    (75.0, 77.5, "150"),
    (77.5, 80.0, "155"),
    (80.0, 82.5, "160"),
    (82.5, 85.0, "165"),
    (85.0, 87.5, "170"),
    (87.5, 90.0, "175"),
    (90.0, 92.5, "180"),
    (92.5, 95.0, "185"),
    (95.0, 1000.0, "190"), // >= 95.0 m/s
];

/// Configuration for wind barb rendering
#[derive(Debug, Clone)]
pub struct BarbConfig {
    /// Size of the barb icon in pixels (default: 40)
    pub size: u32,
    /// Grid spacing between barbs in pixels (default: 50)
    pub spacing: u32,
    /// Color of the barb (hex format, e.g., "#000000")
    pub color: String,
}

impl Default for BarbConfig {
    fn default() -> Self {
        Self {
            size: 108,
            spacing: 30,
            color: "#000000".to_string(),
        }
    }
}

/// Convert U and V wind components (m/s) to speed (m/s) and direction (radians FROM)
///
/// Returns (speed_ms, direction_rad) where:
/// - speed_ms: Wind speed in meters per second
/// - direction_rad: Direction FROM which wind blows in radians (0 = North, π/2 = East)
pub fn uv_to_speed_direction(u: f32, v: f32) -> (f64, f64) {
    let u = u as f64;
    let v = v as f64;

    // Calculate wind speed using Pythagorean theorem
    let speed = (u * u + v * v).sqrt();

    // Calculate direction FROM which wind blows
    // In meteorological convention:
    // - 0° = wind from North (V < 0, U = 0)
    // - 90° = wind from East (U < 0, V = 0)
    // - 180° = wind from South (V > 0, U = 0)
    // - 270° = wind from West (U > 0, V = 0)
    let mut direction = (-v).atan2(-u);

    // Normalize to [0, 2π)
    if direction < 0.0 {
        direction += 2.0 * PI;
    }

    (speed, direction)
}

/// Select appropriate wind barb SVG based on wind speed in m/s
fn select_barb_svg(speed_ms: f64) -> &'static str {
    for (min, max, knots) in SPEED_RANGES_MS {
        if speed_ms >= *min && speed_ms < *max {
            return knots;
        }
    }
    // Fallback to highest speed if beyond range
    "190"
}

/// Get SVG content for a specific wind barb
fn get_barb_svg_content(speed_ms: f64) -> Option<&'static str> {
    let knots = select_barb_svg(speed_ms);
    WIND_BARB_SVGS
        .iter()
        .find(|(k, _)| *k == knots)
        .map(|(_, svg)| *svg)
}

/// Calculate positions for wind barbs on a grid with decimation
pub fn calculate_barb_positions(width: usize, height: usize, spacing: u32) -> Vec<(usize, usize)> {
    let mut positions = Vec::new();
    let spacing = spacing as usize;

    // Start from spacing/2 to center the grid
    let offset_x = spacing / 2;
    let offset_y = spacing / 2;

    let mut y = offset_y;
    while y < height {
        let mut x = offset_x;
        while x < width {
            positions.push((x, y));
            x += spacing;
        }
        y += spacing;
    }

    positions
}

/// Calculate positions for wind barbs based on global geographic grid
/// This ensures barbs align across tile boundaries
///
/// # Arguments
/// * `width` - Tile width in pixels
/// * `height` - Tile height in pixels  
/// * `bbox` - Bounding box [min_lon, min_lat, max_lon, max_lat]
/// * `spacing_degrees` - Spacing between barbs in degrees
///
/// # Returns
/// Vector of (pixel_x, pixel_y) positions within the tile
pub fn calculate_barb_positions_geographic(
    width: usize,
    height: usize,
    bbox: [f32; 4],
    spacing_degrees: f32,
) -> Vec<(usize, usize)> {
    let mut positions = Vec::new();

    let [min_lon, min_lat, max_lon, max_lat] = bbox;
    let lon_range = max_lon - min_lon;
    let lat_range = max_lat - min_lat;

    // Calculate the first barb position that aligns to the global grid
    // Round down to nearest multiple of spacing_degrees
    let first_lon = (min_lon / spacing_degrees).floor() * spacing_degrees;
    let first_lat = (min_lat / spacing_degrees).floor() * spacing_degrees;

    // Iterate through global grid positions that fall within this tile
    // Use inclusive min bounds and exclusive max bounds to prevent
    // duplicate barbs at tile boundaries (each barb belongs to one tile only)
    let mut lat = first_lat;
    while lat < max_lat + spacing_degrees {
        let mut lon = first_lon;
        while lon < max_lon + spacing_degrees {
            // Check if this position is within the tile bbox
            // Inclusive on min side, exclusive on max side to avoid duplicates
            if lon >= min_lon && lon < max_lon && lat >= min_lat && lat < max_lat {
                // Convert geographic position to pixel position
                let x = ((lon - min_lon) / lon_range * width as f32) as usize;
                let y = ((max_lat - lat) / lat_range * height as f32) as usize; // Y is inverted

                // Ensure within bounds
                if x < width && y < height {
                    positions.push((x, y));
                }
            }
            lon += spacing_degrees;
        }
        lat += spacing_degrees;
    }

    positions
}

/// Render wind barbs onto an RGBA image buffer
///
/// # Arguments
/// * `u_data` - U wind component grid (m/s)
/// * `v_data` - V wind component grid (m/s)
/// * `width` - Output image width
/// * `height` - Output image height
/// * `config` - Barb rendering configuration
///
/// # Returns
/// RGBA pixel buffer (4 bytes per pixel)
pub fn render_wind_barbs(
    u_data: &[f32],
    v_data: &[f32],
    width: usize,
    height: usize,
    config: &BarbConfig,
) -> Vec<u8> {
    // Create transparent RGBA canvas
    let mut canvas = vec![0u8; width * height * 4];

    // Calculate barb positions
    let positions = calculate_barb_positions(width, height, config.spacing);

    // Render barb at each position
    for (x, y) in positions {
        // Get index in the data grid
        let idx = y * width + x;
        if idx >= u_data.len() || idx >= v_data.len() {
            continue;
        }

        let u = u_data[idx];
        let v = v_data[idx];

        // Skip invalid data
        if u.is_nan() || v.is_nan() {
            continue;
        }

        // Convert U/V to speed and direction
        let (speed_ms, direction_rad) = uv_to_speed_direction(u, v);

        // Get appropriate SVG for this wind speed
        if let Some(svg_content) = get_barb_svg_content(speed_ms) {
            // Render the SVG barb at this position with rotation
            render_barb_at_position(
                &mut canvas,
                width,
                height,
                x,
                y,
                svg_content,
                direction_rad,
                config,
            );
        }
    }

    canvas
}

/// Render wind barbs with geographic alignment for seamless tile boundaries
///
/// # Arguments
/// * `u_data` - U wind component grid (m/s), resampled to tile dimensions
/// * `v_data` - V wind component grid (m/s), resampled to tile dimensions
/// * `width` - Output image width
/// * `height` - Output image height
/// * `bbox` - Bounding box [min_lon, min_lat, max_lon, max_lat]
/// * `config` - Barb rendering configuration
///
/// # Returns
/// RGBA pixel buffer (4 bytes per pixel)
pub fn render_wind_barbs_aligned(
    u_data: &[f32],
    v_data: &[f32],
    width: usize,
    height: usize,
    bbox: [f32; 4],
    config: &BarbConfig,
) -> Vec<u8> {
    // Create transparent RGBA canvas
    let mut canvas = vec![0u8; width * height * 4];

    // For geographic alignment, we want consistent spacing in degrees
    // that results in reasonable pixel spacing on screen.
    // Calculate degrees per pixel and multiply by desired pixel spacing
    let lon_range = bbox[2] - bbox[0];
    let degrees_per_pixel = lon_range / width as f32;

    // Use the configured spacing (in pixels) to determine degree spacing
    // This gives us consistent density based on config
    let spacing_degrees = degrees_per_pixel * config.spacing as f32;

    // Calculate barb positions using global geographic grid
    let positions = calculate_barb_positions_geographic(width, height, bbox, spacing_degrees);

    // Render barb at each position
    for (x, y) in positions {
        // Get index in the data grid
        let idx = y * width + x;
        if idx >= u_data.len() || idx >= v_data.len() {
            continue;
        }

        let u = u_data[idx];
        let v = v_data[idx];

        // Skip invalid data
        if u.is_nan() || v.is_nan() {
            continue;
        }

        // Convert U/V to speed and direction
        let (speed_ms, direction_rad) = uv_to_speed_direction(u, v);

        // Get appropriate SVG for this wind speed
        if let Some(svg_content) = get_barb_svg_content(speed_ms) {
            // Render the SVG barb at this position with rotation
            render_barb_at_position(
                &mut canvas,
                width,
                height,
                x,
                y,
                svg_content,
                direction_rad,
                config,
            );
        }
    }

    canvas
}

/// Render a single wind barb SVG at a specific position with rotation
fn render_barb_at_position(
    canvas: &mut [u8],
    canvas_width: usize,
    canvas_height: usize,
    x: usize,
    y: usize,
    svg_content: &str,
    direction_rad: f64,
    config: &BarbConfig,
) {
    // Parse and render the SVG
    let opt = usvg::Options::default();
    let tree = match usvg::Tree::from_str(svg_content, &opt) {
        Ok(t) => t,
        Err(_) => return, // Skip on parse error
    };

    // Calculate size for rendering
    let size = config.size;

    // Create a pixmap for the SVG
    let mut pixmap = match tiny_skia::Pixmap::new(size, size) {
        Some(p) => p,
        None => return,
    };

    // Get SVG original size (wind barb SVGs are 250x250)
    let svg_size = tree.size();
    let svg_width = svg_size.width();
    let svg_height = svg_size.height();

    // Calculate scale to fit SVG into our pixmap
    let scale = (size as f32 / svg_width).min(size as f32 / svg_height);

    // Center of output pixmap
    let center = size as f32 / 2.0;
    // Center of SVG (in SVG coordinates)
    let svg_center = svg_width / 2.0;

    // Convert direction from radians to degrees
    // SVG barbs point upward (North) by default
    // direction_rad is in math convention (0=East, π/2=North, π=West, 3π/2=South)
    // Adjust by -90 degrees since SVG points up but our 0 is East
    let angle_deg = ((direction_rad - PI / 2.0) * 180.0 / PI) as f32;

    // Build transform to:
    // 1. Move SVG center to origin
    // 2. Rotate around origin
    // 3. Move back
    // 4. Scale to fit pixmap
    // 5. Center in pixmap
    //
    // With post_* operations applied left-to-right, we build:
    // point -> translate(-svg_center) -> rotate -> translate(svg_center) -> scale -> translate(offset)

    let scaled_offset = center - (svg_center * scale);

    let transform = tiny_skia::Transform::identity()
        .post_translate(-svg_center, -svg_center) // Move SVG center to origin
        .post_rotate(angle_deg) // Rotate around origin
        .post_translate(svg_center, svg_center) // Move back
        .post_scale(scale, scale) // Scale down
        .post_translate(scaled_offset, scaled_offset); // Center in pixmap

    // Render the SVG tree onto the pixmap
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Composite the pixmap onto the main canvas
    composite_barb_onto_canvas(
        canvas,
        canvas_width,
        canvas_height,
        &pixmap,
        x,
        y,
        size as usize,
    );
}

/// Composite a rendered barb pixmap onto the main canvas
fn composite_barb_onto_canvas(
    canvas: &mut [u8],
    canvas_width: usize,
    canvas_height: usize,
    pixmap: &tiny_skia::Pixmap,
    center_x: usize,
    center_y: usize,
    size: usize,
) {
    let half_size = size / 2;

    // Calculate bounds
    let start_x = center_x.saturating_sub(half_size);
    let start_y = center_y.saturating_sub(half_size);

    // Use actual pixmap dimensions for correct indexing
    let pixmap_width = pixmap.width() as usize;
    let pixmap_height = pixmap.height() as usize;

    // Composite each pixel
    for py in 0..pixmap_height.min(size) {
        for px in 0..pixmap_width.min(size) {
            let canvas_x = start_x + px;
            let canvas_y = start_y + py;

            // Check bounds
            if canvas_x >= canvas_width || canvas_y >= canvas_height {
                continue;
            }

            // Get pixel from source pixmap (RGBA premultiplied)
            // Use pixmap_width for correct row stride
            let src_idx = (py * pixmap_width + px) * 4;
            let src_data = pixmap.data();
            if src_idx + 3 >= src_data.len() {
                continue;
            }

            let src_r = src_data[src_idx];
            let src_g = src_data[src_idx + 1];
            let src_b = src_data[src_idx + 2];
            let src_a = src_data[src_idx + 3];

            // Skip fully transparent pixels
            if src_a == 0 {
                continue;
            }

            // Get destination pixel
            let dst_idx = (canvas_y * canvas_width + canvas_x) * 4;

            // Alpha blending (source-over compositing)
            let dst_r = canvas[dst_idx];
            let dst_g = canvas[dst_idx + 1];
            let dst_b = canvas[dst_idx + 2];
            let dst_a = canvas[dst_idx + 3];

            let src_a_f = src_a as f32 / 255.0;
            let dst_a_f = dst_a as f32 / 255.0;

            // Premultiply alpha for proper blending
            let out_a = src_a_f + dst_a_f * (1.0 - src_a_f);

            if out_a > 0.0 {
                let out_r = ((src_r as f32 * src_a_f + dst_r as f32 * dst_a_f * (1.0 - src_a_f))
                    / out_a) as u8;
                let out_g = ((src_g as f32 * src_a_f + dst_g as f32 * dst_a_f * (1.0 - src_a_f))
                    / out_a) as u8;
                let out_b = ((src_b as f32 * src_a_f + dst_b as f32 * dst_a_f * (1.0 - src_a_f))
                    / out_a) as u8;

                canvas[dst_idx] = out_r;
                canvas[dst_idx + 1] = out_g;
                canvas[dst_idx + 2] = out_b;
                canvas[dst_idx + 3] = (out_a * 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uv_to_speed_direction_north_wind() {
        // North wind: U=0, V=-10 (wind FROM north)
        // atan2(10, 0) = π/2 (90 degrees in math convention, pointing up)
        let (speed, dir) = uv_to_speed_direction(0.0, -10.0);
        assert!((speed - 10.0).abs() < 0.01, "Speed should be 10 m/s");
        assert!(
            (dir - PI / 2.0).abs() < 0.1,
            "Direction should be ~π/2 (North in math convention)"
        );
    }

    #[test]
    fn test_uv_to_speed_direction_east_wind() {
        // East wind: U=-10, V=0 (wind FROM east)
        // atan2(0, 10) = 0 (0 degrees in math convention, pointing right)
        let (speed, dir) = uv_to_speed_direction(-10.0, 0.0);
        assert!((speed - 10.0).abs() < 0.01, "Speed should be 10 m/s");
        assert!(
            (dir - 0.0).abs() < 0.1,
            "Direction should be ~0 (East in math convention)"
        );
    }

    #[test]
    fn test_select_barb_svg() {
        assert_eq!(select_barb_svg(0.0), "0");
        assert_eq!(select_barb_svg(1.5), "2");
        assert_eq!(select_barb_svg(3.0), "5");
        assert_eq!(select_barb_svg(6.0), "10");
        assert_eq!(select_barb_svg(26.0), "50");
        assert_eq!(select_barb_svg(100.0), "190");
    }

    #[test]
    fn test_calculate_barb_positions() {
        let positions = calculate_barb_positions(200, 200, 50);
        assert!(!positions.is_empty(), "Should generate some positions");

        // Check that positions are reasonably spaced
        if positions.len() >= 2 {
            let spacing = positions[1].0 - positions[0].0;
            assert_eq!(spacing, 50, "Horizontal spacing should be 50 pixels");
        }
    }

    #[test]
    fn test_render_wind_barbs_dimension_check() {
        let u_data = vec![5.0; 100 * 100];
        let v_data = vec![5.0; 100 * 100];
        let config = BarbConfig::default();

        let canvas = render_wind_barbs(&u_data, &v_data, 100, 100, &config);
        assert_eq!(canvas.len(), 100 * 100 * 4, "Canvas should be correct size");
    }
}
