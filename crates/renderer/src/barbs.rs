//! Wind barb rendering for meteorological visualization.
//!
//! Wind barbs are standard meteorological symbols showing wind speed and direction:
//! - Staff points in direction wind is FROM (meteorological convention)
//! - Pennants (filled triangles) = 50 knots each
//! - Long barbs = 10 knots each
//! - Short barbs = 5 knots each
//! - Circle = calm (< 3 knots)

use std::f32::consts::PI;
use crate::gradient::Color;

/// Represents a single wind barb to be drawn
#[derive(Debug, Clone, Copy)]
pub struct WindBarb {
    /// X position on canvas (pixels)
    pub x: f32,
    /// Y position on canvas (pixels)
    pub y: f32,
    /// Wind speed in knots
    pub speed_knots: f32,
    /// Direction wind is FROM (radians, 0=North, clockwise)
    pub direction_rad: f32,
}

/// Configuration for wind barb rendering
#[derive(Debug, Clone)]
pub struct BarbConfig {
    /// Length of main staff in pixels
    pub staff_length: f32,
    /// Length of speed barbs in pixels
    pub barb_length: f32,
    /// Spacing between barbs along staff in pixels
    pub barb_spacing: f32,
    /// Angle of barbs from staff in degrees (typically 70°)
    pub barb_angle_deg: f32,
    /// Barb line color
    pub color: Color,
    /// Line thickness in pixels
    pub line_width: u32,
    /// Radius of calm wind circle
    pub calm_radius: f32,
}

impl Default for BarbConfig {
    fn default() -> Self {
        Self {
            staff_length: 25.0,
            barb_length: 12.0,
            barb_spacing: 4.0,
            barb_angle_deg: 70.0,
            color: Color::new(0, 0, 0, 255), // Black
            line_width: 2,
            calm_radius: 4.0,
        }
    }
}

/// Convert U/V wind components to speed (knots) and direction (radians)
///
/// # Arguments
/// - `u`: Eastward wind component (m/s)
/// - `v`: Northward wind component (m/s)
///
/// # Returns
/// (speed_knots, direction_rad) where direction is FROM (meteorological convention)
/// Direction: 0=North, π/2=East, π=South, 3π/2=West
/// Returns value in [0, 2π) range
pub fn uv_to_speed_direction(u: f32, v: f32) -> (f32, f32) {
    // Wind speed from magnitude
    let speed_ms = (u * u + v * v).sqrt();
    let speed_knots = speed_ms * 1.944; // Convert m/s to knots

    // Wind direction FROM (meteorological convention)
    // atan2(v, u) gives direction TO which wind is blowing
    // Negate to get direction FROM which wind is blowing
    let mut direction_rad = (-u).atan2(-v);
    
    // Normalize to [0, 2π)
    if direction_rad < 0.0 {
        direction_rad += 2.0 * PI;
    }

    (speed_knots, direction_rad)
}

/// Calculate barb counts for a given wind speed
///
/// Returns (pennants, long_barbs, has_short_barb)
fn calculate_barb_counts(speed_knots: f32) -> (u32, u32, bool) {
    let speed = speed_knots.round() as u32;

    let pennants = speed / 50;
    let remaining = speed % 50;
    let long_barbs = remaining / 10;
    let has_short_barb = (remaining % 10) >= 5;

    (pennants, long_barbs, has_short_barb)
}

/// Bresenham line drawing algorithm with thickness
///
/// Draws a line from (x1, y1) to (x2, y2) with optional thickness
fn draw_line(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: Color,
    thickness: u32,
) {
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    let sx = if x1 < x2 { 1 } else { -1 };
    let sy = if y1 < y2 { 1 } else { -1 };
    let mut err = (dx as i32 - dy as i32) / 2;

    let mut x = x1;
    let mut y = y1;

    loop {
        // Draw pixel with thickness
        draw_thick_pixel(pixels, width, height, x, y, color, thickness);

        if x == x2 && y == y2 {
            break;
        }

        let e2 = err;
        if e2 > -(dx as i32) {
            err -= dy as i32;
            x += sx;
        }
        if e2 < dy as i32 {
            err += dx as i32;
            y += sy;
        }
    }
}

/// Draw a thick pixel by drawing a small circle around it
fn draw_thick_pixel(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    cx: i32,
    cy: i32,
    color: Color,
    radius: u32,
) {
    let r = radius as i32;
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                let x = cx + dx;
                let y = cy + dy;

                if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                    let idx = (y as usize * width + x as usize) * 4;
                    if idx + 3 < pixels.len() {
                        pixels[idx] = color.r;
                        pixels[idx + 1] = color.g;
                        pixels[idx + 2] = color.b;
                        pixels[idx + 3] = color.a;
                    }
                }
            }
        }
    }
}

/// Draw the main staff line and return the tip position
fn draw_staff(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x: f32,
    y: f32,
    direction: f32,
    config: &BarbConfig,
) -> (f32, f32) {
    // Calculate tip position (staff points FROM direction)
    let tip_x = x + direction.sin() * config.staff_length;
    let tip_y = y - direction.cos() * config.staff_length;

    // Draw line from base to tip
    draw_line(
        pixels,
        width,
        height,
        x as i32,
        y as i32,
        tip_x as i32,
        tip_y as i32,
        config.color,
        config.line_width,
    );

    (tip_x, tip_y)
}

/// Draw a pennant (50 knots - filled triangle)
fn draw_pennant(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    staff_x: f32,
    staff_y: f32,
    direction: f32,
    config: &BarbConfig,
) -> (f32, f32) {
    // Pennant is a filled triangle at the tip of the staff
    // Two sides at barb angle, one side is staff

    // Perpendicular direction (90° to the right of staff)
    let perp_dir = direction + PI / 2.0;

    // Triangle vertices:
    // p1: tip of staff
    let p1_x = staff_x;
    let p1_y = staff_y;

    // p2: end of first barb (perpendicular)
    let p2_x = staff_x + perp_dir.cos() * config.barb_length;
    let p2_y = staff_y + perp_dir.sin() * config.barb_length;

    // p3: point back along staff for pennant width
    let pennant_depth = config.barb_spacing * 1.5; // Wider than regular barb
    let p3_x = staff_x - direction.sin() * pennant_depth;
    let p3_y = staff_y + direction.cos() * pennant_depth;

    // Draw filled triangle
    fill_triangle(pixels, width, height, p1_x, p1_y, p2_x, p2_y, p3_x, p3_y, config.color);

    // Draw outline
    draw_line(
        pixels,
        width,
        height,
        p1_x as i32,
        p1_y as i32,
        p2_x as i32,
        p2_y as i32,
        config.color,
        config.line_width,
    );
    draw_line(
        pixels,
        width,
        height,
        p2_x as i32,
        p2_y as i32,
        p3_x as i32,
        p3_y as i32,
        config.color,
        config.line_width,
    );
    draw_line(
        pixels,
        width,
        height,
        p3_x as i32,
        p3_y as i32,
        p1_x as i32,
        p1_y as i32,
        config.color,
        config.line_width,
    );

    // Return position after this pennant (back along staff)
    (p3_x, p3_y)
}

/// Draw a long barb (10 knots)
fn draw_barb(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    staff_x: f32,
    staff_y: f32,
    direction: f32,
    is_long: bool,
    config: &BarbConfig,
) -> (f32, f32) {
    // Perpendicular direction (90° to the right of staff)
    let perp_dir = direction + PI / 2.0;

    // Length depends on whether it's a long barb (10kt) or short (5kt)
    let barb_len = if is_long { config.barb_length } else { config.barb_length / 2.0 };

    // Barb end point
    let barb_x = staff_x + perp_dir.cos() * barb_len;
    let barb_y = staff_y + perp_dir.sin() * barb_len;

    // Draw from barb end to staff
    draw_line(
        pixels,
        width,
        height,
        barb_x as i32,
        barb_y as i32,
        staff_x as i32,
        staff_y as i32,
        config.color,
        config.line_width,
    );

    // Move back along staff for next barb
    let next_x = staff_x - direction.sin() * config.barb_spacing;
    let next_y = staff_y + direction.cos() * config.barb_spacing;

    (next_x, next_y)
}

/// Draw a calm wind circle (< 3 knots)
fn draw_circle(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    cx: f32,
    cy: f32,
    radius: f32,
    color: Color,
) {
    let r = radius as i32;
    let cx = cx as i32;
    let cy = cy as i32;

    for dy in -r..=r {
        for dx in -r..=r {
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            if (dist - radius).abs() < 1.5 {
                let x = cx + dx;
                let y = cy + dy;

                if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                    let idx = (y as usize * width + x as usize) * 4;
                    if idx + 3 < pixels.len() {
                        pixels[idx] = color.r;
                        pixels[idx + 1] = color.g;
                        pixels[idx + 2] = color.b;
                        pixels[idx + 3] = color.a;
                    }
                }
            }
        }
    }
}

/// Fill a triangle using scan line algorithm
fn fill_triangle(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    color: Color,
) {
    let min_y = y1.min(y2).min(y3).ceil() as i32;
    let max_y = y1.max(y2).max(y3).floor() as i32;

    for y in min_y..=max_y {
        let y_f = y as f32;

        // Find intersections with triangle edges
        let mut x_intersections = Vec::new();

        // Edge 1-2
        if (y1 - y2).abs() > 0.01 {
            let t = (y_f - y1) / (y2 - y1);
            if t >= 0.0 && t <= 1.0 {
                let x = x1 + t * (x2 - x1);
                x_intersections.push(x);
            }
        }

        // Edge 2-3
        if (y2 - y3).abs() > 0.01 {
            let t = (y_f - y2) / (y3 - y2);
            if t >= 0.0 && t <= 1.0 {
                let x = x2 + t * (x3 - x2);
                x_intersections.push(x);
            }
        }

        // Edge 3-1
        if (y3 - y1).abs() > 0.01 {
            let t = (y_f - y3) / (y1 - y3);
            if t >= 0.0 && t <= 1.0 {
                let x = x3 + t * (x1 - x3);
                x_intersections.push(x);
            }
        }

        // Sort intersections and fill
        if x_intersections.len() >= 2 {
            x_intersections.sort_by(|a, b| a.partial_cmp(b).unwrap());

            let min_x = x_intersections[0].ceil() as i32;
            let max_x = x_intersections[x_intersections.len() - 1].floor() as i32;

            for x in min_x..=max_x {
                if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                    let idx = (y as usize * width + x as usize) * 4;
                    if idx + 3 < pixels.len() {
                        pixels[idx] = color.r;
                        pixels[idx + 1] = color.g;
                        pixels[idx + 2] = color.b;
                        pixels[idx + 3] = color.a;
                    }
                }
            }
        }
    }
}

/// Draw a single wind barb onto a pixel buffer
pub fn draw_barb_on_canvas(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    barb: &WindBarb,
    config: &BarbConfig,
) {
    // Handle calm wind specially
    if barb.speed_knots < 3.0 {
        draw_circle(pixels, width, height, barb.x, barb.y, config.calm_radius, config.color);
        return;
    }

    // Draw the main staff
    let (tip_x, tip_y) = draw_staff(pixels, width, height, barb.x, barb.y, barb.direction_rad, config);

    // Calculate barb counts
    let (pennants, long_barbs, has_short_barb) = calculate_barb_counts(barb.speed_knots);

    let mut current_x = tip_x;
    let mut current_y = tip_y;

    // Draw pennants (50 knots each)
    for _ in 0..pennants {
        let (next_x, next_y) = draw_pennant(pixels, width, height, current_x, current_y, barb.direction_rad, config);
        current_x = next_x;
        current_y = next_y;
    }

    // Draw long barbs (10 knots each)
    for _ in 0..long_barbs {
        let (next_x, next_y) = draw_barb(pixels, width, height, current_x, current_y, barb.direction_rad, true, config);
        current_x = next_x;
        current_y = next_y;
    }

    // Draw short barb if needed (5 knots)
    if has_short_barb {
        let _ = draw_barb(pixels, width, height, current_x, current_y, barb.direction_rad, false, config);
    }
}

/// Calculate positions for wind barbs by decimating the grid
fn calculate_barb_positions(
    u_data: &[f32],
    v_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    output_width: usize,
    output_height: usize,
    spacing: usize,
) -> Vec<WindBarb> {
    let mut barbs = Vec::new();

    // Calculate decimation factors
    let x_step = (output_width as f32 / spacing as f32).max(1.0) as usize;
    let y_step = (output_height as f32 / spacing as f32).max(1.0) as usize;

    // Sample at decimated positions
    for out_y in (0..output_height).step_by(y_step) {
        for out_x in (0..output_width).step_by(x_step) {
            // Map output position to grid indices
            let grid_x = (out_x as f32 / output_width as f32 * grid_width as f32) as usize;
            let grid_y = (out_y as f32 / output_height as f32 * grid_height as f32) as usize;

            let grid_x = grid_x.min(grid_width - 1);
            let grid_y = grid_y.min(grid_height - 1);

            let idx = grid_y * grid_width + grid_x;

            if idx < u_data.len() && idx < v_data.len() {
                let u = u_data[idx];
                let v = v_data[idx];

                // Skip if either is NaN or zero
                if u.is_finite() && v.is_finite() {
                    let (speed_knots, direction_rad) = uv_to_speed_direction(u, v);

                    barbs.push(WindBarb {
                        x: out_x as f32,
                        y: out_y as f32,
                        speed_knots,
                        direction_rad,
                    });
                }
            }
        }
    }

    barbs
}

/// Render wind barbs from U/V component grids
///
/// # Arguments
/// - `u_data`: U-component grid (eastward wind, m/s)
/// - `v_data`: V-component grid (northward wind, m/s)
/// - `grid_width`: Source grid width
/// - `grid_height`: Source grid height
/// - `output_width`: Output image width
/// - `output_height`: Output image height
/// - `barb_spacing`: Approximate pixel spacing between barbs
/// - `config`: Barb rendering configuration
///
/// # Returns
/// RGBA pixel data (4 bytes per pixel)
pub fn render_wind_barbs(
    u_data: &[f32],
    v_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    output_width: usize,
    output_height: usize,
    barb_spacing: usize,
    config: &BarbConfig,
) -> Vec<u8> {
    // Create transparent output buffer
    let mut pixels = vec![0u8; output_width * output_height * 4];

    // Calculate barb positions
    let barbs = calculate_barb_positions(u_data, v_data, grid_width, grid_height, output_width, output_height, barb_spacing);

    // Draw each barb
    for barb in barbs {
        draw_barb_on_canvas(&mut pixels, output_width, output_height, &barb, config);
    }

    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uv_to_speed_direction_east_wind() {
        // Pure eastward wind (u=10, v=0) -> from West (270° or 3π/2)
        let (speed, dir) = uv_to_speed_direction(10.0, 0.0);
        assert!((speed - 19.44).abs() < 0.1); // ~10 m/s = 19.44 knots
        // atan2(-u, -v) = atan2(-10, 0) = -π/2, normalized to 3π/2
        let expected = 3.0 * PI / 2.0;
        assert!((dir - expected).abs() < 0.1, "Expected {}, got {}", expected, dir);
    }

    #[test]
    fn test_uv_to_speed_direction_north_wind() {
        // Pure northward wind (u=0, v=10) -> from South (180° or π)
        let (speed, dir) = uv_to_speed_direction(0.0, 10.0);
        assert!((speed - 19.44).abs() < 0.1);
        // atan2(-u, -v) = atan2(0, -10) = π, normalized to π
        assert!((dir - PI).abs() < 0.1, "Expected {}, got {}", PI, dir);
    }

    #[test]
    fn test_calculate_barb_counts_25knots() {
        // 25 knots = 0 pennants + 2 long (20) + 1 short (5)
        let (pent, long, short) = calculate_barb_counts(25.0);
        assert_eq!(pent, 0);
        assert_eq!(long, 2);
        assert!(short);
    }

    #[test]
    fn test_calculate_barb_counts_65knots() {
        // 65 knots = 1 pennant (50) + 1 long (10) + 1 short (5)
        let (pent, long, short) = calculate_barb_counts(65.0);
        assert_eq!(pent, 1);
        assert_eq!(long, 1);
        assert!(short);
    }

    #[test]
    fn test_calculate_barb_counts_100knots() {
        // 100 knots = 2 pennants
        let (pent, long, short) = calculate_barb_counts(100.0);
        assert_eq!(pent, 2);
        assert_eq!(long, 0);
        assert!(!short);
    }

    #[test]
    fn test_calm_wind() {
        let (speed, _) = uv_to_speed_direction(0.5, 0.5);
        assert!(speed < 3.0); // Should be calm
    }

    #[test]
    fn test_draw_barb_creates_pixels() {
        let mut pixels = vec![0u8; 256 * 256 * 4];
        let barb = WindBarb {
            x: 128.0,
            y: 128.0,
            speed_knots: 25.0,
            direction_rad: 0.0, // From North
        };
        draw_barb_on_canvas(&mut pixels, 256, 256, &barb, &BarbConfig::default());

        // Verify pixels were modified (not all black)
        let non_zero = pixels.iter().any(|&p| p != 0);
        assert!(non_zero, "Barb rendering should modify pixels");
    }

    #[test]
    fn test_calm_circle_creates_pixels() {
        let mut pixels = vec![0u8; 256 * 256 * 4];
        let barb = WindBarb {
            x: 128.0,
            y: 128.0,
            speed_knots: 1.0, // Calm
            direction_rad: 0.0,
        };
        draw_barb_on_canvas(&mut pixels, 256, 256, &barb, &BarbConfig::default());

        // Verify pixels were modified
        let non_zero = pixels.iter().any(|&p| p != 0);
        assert!(non_zero, "Calm circle should modify pixels");
    }

    #[test]
    fn test_render_wind_barbs_dimension_check() {
        let u_data = vec![5.0; 100 * 50]; // 100x50 grid
        let v_data = vec![3.0; 100 * 50];

        let pixels = render_wind_barbs(&u_data, &v_data, 100, 50, 512, 256, 50, &BarbConfig::default());

        // Should be 512x256 RGBA
        assert_eq!(pixels.len(), 512 * 256 * 4);
    }
}
