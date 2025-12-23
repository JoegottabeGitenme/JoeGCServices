//! Test data generators for creating synthetic weather-like data.
//!
//! These generators create predictable, verifiable test data patterns
//! that can be used across the test suite.

/// Creates a test grid with predictable values.
///
/// Each cell value is calculated as: `col * 1000 + row`
///
/// This makes it easy to verify that data is being read/written correctly
/// by checking that grid[row][col] == col * 1000 + row.
///
/// # Arguments
///
/// * `width` - Number of columns
/// * `height` - Number of rows
///
/// # Returns
///
/// A `Vec<f32>` in row-major order (row 0 first, then row 1, etc.)
///
/// # Example
///
/// ```
/// use test_utils::create_test_grid;
///
/// let grid = create_test_grid(10, 5);
/// assert_eq!(grid.len(), 50); // 10 * 5
/// assert_eq!(grid[0], 0.0);   // col=0, row=0 -> 0*1000 + 0
/// assert_eq!(grid[1], 1000.0); // col=1, row=0 -> 1*1000 + 0
/// assert_eq!(grid[10], 1.0);  // col=0, row=1 -> 0*1000 + 1
/// ```
pub fn create_test_grid(width: usize, height: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    for row in 0..height {
        for col in 0..width {
            data.push((col * 1000 + row) as f32);
        }
    }
    data
}

/// Creates a test grid with temperature-like values in Kelvin.
///
/// The values range from approximately 250K (-23C) to 310K (37C),
/// creating a gradient pattern similar to real weather data.
///
/// # Arguments
///
/// * `width` - Number of columns
/// * `height` - Number of rows
///
/// # Returns
///
/// A `Vec<f32>` with temperature values in Kelvin.
pub fn create_temperature_grid(width: usize, height: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    for row in 0..height {
        for col in 0..width {
            // Create a gradient from cold (top-left) to warm (bottom-right)
            let x_factor = col as f32 / width.max(1) as f32;
            let y_factor = row as f32 / height.max(1) as f32;
            // Temperature range: 250K to 310K
            let temp = 250.0 + (x_factor * 30.0) + (y_factor * 30.0);
            data.push(temp);
        }
    }
    data
}

/// Creates a test grid with wind speed values in m/s.
///
/// Values range from 0 to ~50 m/s, with a radial pattern
/// (calm in center, stronger towards edges).
///
/// # Arguments
///
/// * `width` - Number of columns
/// * `height` - Number of rows
///
/// # Returns
///
/// A `Vec<f32>` with wind speed values.
pub fn create_wind_speed_grid(width: usize, height: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    let center_x = width as f32 / 2.0;
    let center_y = height as f32 / 2.0;
    let max_dist = ((center_x * center_x) + (center_y * center_y)).sqrt();

    for row in 0..height {
        for col in 0..width {
            let dx = col as f32 - center_x;
            let dy = row as f32 - center_y;
            let dist = (dx * dx + dy * dy).sqrt();
            // Wind speed: 0 at center, up to 50 m/s at edges
            let speed = (dist / max_dist) * 50.0;
            data.push(speed);
        }
    }
    data
}

/// Creates a U-component wind grid (west-east component).
///
/// # Arguments
///
/// * `width` - Number of columns
/// * `height` - Number of rows
///
/// # Returns
///
/// A `Vec<f32>` with U wind component values in m/s.
pub fn create_u_wind_grid(width: usize, height: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    for row in 0..height {
        for _col in 0..width {
            // U component varies by latitude (row), simulating trade winds pattern
            let lat_factor = (row as f32 / height as f32 - 0.5) * 2.0; // -1 to 1
            let u = lat_factor * 20.0; // -20 to +20 m/s
            data.push(u);
        }
    }
    data
}

/// Creates a V-component wind grid (south-north component).
///
/// # Arguments
///
/// * `width` - Number of columns
/// * `height` - Number of rows
///
/// # Returns
///
/// A `Vec<f32>` with V wind component values in m/s.
pub fn create_v_wind_grid(width: usize, height: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    for _row in 0..height {
        for col in 0..width {
            // V component varies by longitude (col)
            let lon_factor = (col as f32 / width as f32 - 0.5) * 2.0; // -1 to 1
            let v = lon_factor * 15.0; // -15 to +15 m/s
            data.push(v);
        }
    }
    data
}

/// Creates a grid with random-ish but deterministic precipitation values.
///
/// Uses a simple hash-based approach for reproducibility.
///
/// # Arguments
///
/// * `width` - Number of columns
/// * `height` - Number of rows
/// * `seed` - Seed value for deterministic generation
///
/// # Returns
///
/// A `Vec<f32>` with precipitation values in mm.
pub fn create_precipitation_grid(width: usize, height: usize, seed: u32) -> Vec<f32> {
    let mut data = Vec::with_capacity(width * height);
    for row in 0..height {
        for col in 0..width {
            // Simple deterministic pseudo-random
            let hash = simple_hash(col as u32, row as u32, seed);
            // Most values are 0 (no precip), some are up to 50mm
            let precip = if hash % 4 == 0 {
                (hash % 5000) as f32 / 100.0 // 0 to 50mm
            } else {
                0.0
            };
            data.push(precip);
        }
    }
    data
}

/// Simple deterministic hash for reproducible test data.
fn simple_hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = seed;
    h = h.wrapping_mul(31).wrapping_add(x);
    h = h.wrapping_mul(31).wrapping_add(y);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^= h >> 16;
    h
}

/// Creates a grid filled with a constant value.
///
/// Useful for testing edge cases and simple scenarios.
///
/// # Arguments
///
/// * `width` - Number of columns
/// * `height` - Number of rows
/// * `value` - The constant value to fill
///
/// # Returns
///
/// A `Vec<f32>` filled with the constant value.
pub fn create_constant_grid(width: usize, height: usize, value: f32) -> Vec<f32> {
    vec![value; width * height]
}

/// Creates a grid with NaN values at specified positions.
///
/// Useful for testing missing data handling.
///
/// # Arguments
///
/// * `width` - Number of columns
/// * `height` - Number of rows
/// * `nan_positions` - List of (col, row) positions that should be NaN
///
/// # Returns
///
/// A `Vec<f32>` with NaN at specified positions, zeros elsewhere.
pub fn create_grid_with_nans(
    width: usize,
    height: usize,
    nan_positions: &[(usize, usize)],
) -> Vec<f32> {
    let mut data = vec![0.0f32; width * height];
    for &(col, row) in nan_positions {
        if col < width && row < height {
            data[row * width + col] = f32::NAN;
        }
    }
    data
}

/// Creates RGBA pixel data for a simple test pattern.
///
/// Creates a gradient pattern useful for testing PNG encoding.
///
/// # Arguments
///
/// * `width` - Image width in pixels
/// * `height` - Image height in pixels
///
/// # Returns
///
/// A `Vec<u8>` with RGBA pixel data (4 bytes per pixel).
pub fn create_test_rgba_pixels(width: usize, height: usize) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let r = ((x as f32 / width as f32) * 255.0) as u8;
            let g = ((y as f32 / height as f32) * 255.0) as u8;
            let b = 128u8;
            let a = 255u8;
            pixels.extend_from_slice(&[r, g, b, a]);
        }
    }
    pixels
}

/// Creates RGBA pixel data with a limited color palette.
///
/// Creates pixels using only colors from a weather-like palette,
/// suitable for testing indexed PNG encoding.
///
/// # Arguments
///
/// * `width` - Image width in pixels
/// * `height` - Image height in pixels
///
/// # Returns
///
/// A `Vec<u8>` with RGBA pixel data using ~20 unique colors.
pub fn create_weather_palette_pixels(width: usize, height: usize) -> Vec<u8> {
    // Temperature-like color palette
    let palette: [(u8, u8, u8); 20] = [
        (102, 0, 214), // -40C purple
        (0, 51, 255),  // -30C blue
        (0, 128, 255), // -20C light blue
        (0, 191, 255), // -10C cyan
        (0, 255, 255), // 0C cyan
        (0, 255, 191), // 5C teal
        (0, 255, 128), // 10C green
        (0, 255, 0),   // 15C bright green
        (128, 255, 0), // 20C yellow-green
        (191, 255, 0), // 22C
        (255, 255, 0), // 25C yellow
        (255, 220, 0), // 27C
        (255, 191, 0), // 30C orange
        (255, 128, 0), // 33C
        (255, 64, 0),  // 36C red-orange
        (255, 0, 0),   // 40C red
        (214, 0, 0),   // 42C dark red
        (178, 0, 0),   // 45C
        (139, 0, 0),   // 48C maroon
        (100, 0, 0),   // 50C dark maroon
    ];

    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            // Map position to palette index
            let idx =
                ((x as f32 / width as f32 * 0.3 + y as f32 / height as f32 * 0.7) * 19.0) as usize;
            let idx = idx.min(19);
            let (r, g, b) = palette[idx];
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_test_grid() {
        let grid = create_test_grid(10, 5);
        assert_eq!(grid.len(), 50);
        assert_eq!(grid[0], 0.0); // col=0, row=0
        assert_eq!(grid[1], 1000.0); // col=1, row=0
        assert_eq!(grid[10], 1.0); // col=0, row=1
        assert_eq!(grid[11], 1001.0); // col=1, row=1
    }

    #[test]
    fn test_create_temperature_grid() {
        let grid = create_temperature_grid(100, 100);
        assert_eq!(grid.len(), 10000);
        // Check temperature range
        let min = grid.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = grid.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(min >= 250.0);
        assert!(max <= 310.0);
    }

    #[test]
    fn test_create_wind_speed_grid() {
        let grid = create_wind_speed_grid(100, 100);
        assert_eq!(grid.len(), 10000);
        // Center should be calm
        let center_idx = 50 * 100 + 50;
        assert!(grid[center_idx] < 5.0);
        // Corners should be windy
        assert!(grid[0] > 30.0); // top-left corner
    }

    #[test]
    fn test_create_constant_grid() {
        let grid = create_constant_grid(10, 10, 42.0);
        assert_eq!(grid.len(), 100);
        assert!(grid.iter().all(|&v| v == 42.0));
    }

    #[test]
    fn test_create_grid_with_nans() {
        let grid = create_grid_with_nans(10, 10, &[(5, 5), (0, 0)]);
        assert!(grid[0].is_nan()); // (0, 0)
        assert!(grid[55].is_nan()); // (5, 5) = row 5 * 10 + col 5
        assert!(!grid[1].is_nan()); // (1, 0) should be 0.0
    }

    #[test]
    fn test_create_test_rgba_pixels() {
        let pixels = create_test_rgba_pixels(16, 16);
        assert_eq!(pixels.len(), 16 * 16 * 4);
        // First pixel should be (0, 0, 128, 255) - dark blue
        assert_eq!(pixels[0], 0); // R
        assert_eq!(pixels[1], 0); // G
        assert_eq!(pixels[2], 128); // B
        assert_eq!(pixels[3], 255); // A
    }

    #[test]
    fn test_create_weather_palette_pixels() {
        let pixels = create_weather_palette_pixels(256, 256);
        assert_eq!(pixels.len(), 256 * 256 * 4);
        // All pixels should be fully opaque
        for chunk in pixels.chunks_exact(4) {
            assert_eq!(chunk[3], 255);
        }
    }

    #[test]
    fn test_precipitation_deterministic() {
        let grid1 = create_precipitation_grid(100, 100, 42);
        let grid2 = create_precipitation_grid(100, 100, 42);
        assert_eq!(grid1, grid2, "Same seed should produce same data");

        let grid3 = create_precipitation_grid(100, 100, 43);
        assert_ne!(grid1, grid3, "Different seed should produce different data");
    }
}
