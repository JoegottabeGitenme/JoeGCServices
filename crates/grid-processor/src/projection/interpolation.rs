//! Interpolation methods for grid resampling.

/// Nearest neighbor interpolation.
///
/// Returns the value of the nearest grid point.
pub fn nearest_interpolate(
    data: &[f32],
    width: usize,
    height: usize,
    x: f64,
    y: f64,
) -> f32 {
    let col = x.round() as usize;
    let row = y.round() as usize;

    if col >= width || row >= height {
        return f32::NAN;
    }

    data[row * width + col]
}

/// Bilinear interpolation.
///
/// Smoothly interpolates between the four nearest grid points.
pub fn bilinear_interpolate(
    data: &[f32],
    width: usize,
    height: usize,
    x: f64,
    y: f64,
) -> f32 {
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);

    if x0 >= width || y0 >= height {
        return f32::NAN;
    }

    let xf = (x - x0 as f64) as f32;
    let yf = (y - y0 as f64) as f32;

    let v00 = data[y0 * width + x0];
    let v10 = data[y0 * width + x1];
    let v01 = data[y1 * width + x0];
    let v11 = data[y1 * width + x1];

    // Handle NaN values - if any corner is NaN, return NaN
    if v00.is_nan() || v10.is_nan() || v01.is_nan() || v11.is_nan() {
        return f32::NAN;
    }

    // Bilinear interpolation formula
    let top = v00 * (1.0 - xf) + v10 * xf;
    let bottom = v01 * (1.0 - xf) + v11 * xf;
    top * (1.0 - yf) + bottom * yf
}

/// Bicubic interpolation.
///
/// Uses 16 surrounding points for smoother interpolation.
pub fn cubic_interpolate(
    data: &[f32],
    width: usize,
    height: usize,
    x: f64,
    y: f64,
) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;

    let xf = (x - xi as f64) as f32;
    let yf = (y - yi as f64) as f32;

    // Sample 4x4 grid of points
    let mut values = [[0.0f32; 4]; 4];

    for j in 0..4 {
        for i in 0..4 {
            let px = (xi + i - 1).clamp(0, width as i32 - 1) as usize;
            let py = (yi + j - 1).clamp(0, height as i32 - 1) as usize;
            values[j as usize][i as usize] = data[py * width + px];

            // If any value is NaN, fall back to bilinear
            if values[j as usize][i as usize].is_nan() {
                return bilinear_interpolate(data, width, height, x, y);
            }
        }
    }

    // Cubic interpolation along x for each row
    let mut row_values = [0.0f32; 4];
    for j in 0..4 {
        row_values[j] = cubic_1d(
            values[j][0],
            values[j][1],
            values[j][2],
            values[j][3],
            xf,
        );
    }

    // Cubic interpolation along y
    cubic_1d(row_values[0], row_values[1], row_values[2], row_values[3], yf)
}

/// 1D cubic interpolation using Catmull-Rom spline.
fn cubic_1d(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;

    // Catmull-Rom coefficients
    let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let b = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c = -0.5 * p0 + 0.5 * p2;
    let d = p1;

    a * t3 + b * t2 + c * t + d
}

/// Resample a grid to a new size.
///
/// # Arguments
/// * `data` - Source grid data
/// * `src_width` - Source width
/// * `src_height` - Source height
/// * `dst_width` - Destination width
/// * `dst_height` - Destination height
/// * `method` - Interpolation method
///
/// # Returns
/// Resampled grid data
pub fn resample_grid(
    data: &[f32],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
    method: crate::types::InterpolationMethod,
) -> Vec<f32> {
    let mut output = vec![f32::NAN; dst_width * dst_height];

    let scale_x = (src_width - 1) as f64 / (dst_width - 1).max(1) as f64;
    let scale_y = (src_height - 1) as f64 / (dst_height - 1).max(1) as f64;

    for dy in 0..dst_height {
        for dx in 0..dst_width {
            let sx = dx as f64 * scale_x;
            let sy = dy as f64 * scale_y;

            let value = match method {
                crate::types::InterpolationMethod::Nearest => {
                    nearest_interpolate(data, src_width, src_height, sx, sy)
                }
                crate::types::InterpolationMethod::Bilinear => {
                    bilinear_interpolate(data, src_width, src_height, sx, sy)
                }
                crate::types::InterpolationMethod::Cubic => {
                    cubic_interpolate(data, src_width, src_height, sx, sy)
                }
            };

            output[dy * dst_width + dx] = value;
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nearest_interpolate() {
        let data: Vec<f32> = vec![
            1.0, 2.0, 3.0,
            4.0, 5.0, 6.0,
            7.0, 8.0, 9.0,
        ];

        assert_eq!(nearest_interpolate(&data, 3, 3, 0.0, 0.0), 1.0);
        assert_eq!(nearest_interpolate(&data, 3, 3, 1.0, 1.0), 5.0);
        assert_eq!(nearest_interpolate(&data, 3, 3, 0.4, 0.4), 1.0);
        assert_eq!(nearest_interpolate(&data, 3, 3, 0.6, 0.6), 5.0);
    }

    #[test]
    fn test_bilinear_interpolate() {
        let data: Vec<f32> = vec![
            1.0, 2.0,
            3.0, 4.0,
        ];

        // Corners
        assert_eq!(bilinear_interpolate(&data, 2, 2, 0.0, 0.0), 1.0);
        assert_eq!(bilinear_interpolate(&data, 2, 2, 1.0, 0.0), 2.0);
        assert_eq!(bilinear_interpolate(&data, 2, 2, 0.0, 1.0), 3.0);
        assert_eq!(bilinear_interpolate(&data, 2, 2, 1.0, 1.0), 4.0);

        // Center
        let center = bilinear_interpolate(&data, 2, 2, 0.5, 0.5);
        assert!((center - 2.5).abs() < 0.001);
    }

    #[test]
    fn test_bilinear_with_nan() {
        let data: Vec<f32> = vec![
            1.0, f32::NAN,
            3.0, 4.0,
        ];

        // Should return NaN when any corner is NaN
        let result = bilinear_interpolate(&data, 2, 2, 0.5, 0.5);
        assert!(result.is_nan());
    }

    #[test]
    fn test_resample_grid() {
        let data: Vec<f32> = vec![
            1.0, 2.0, 3.0,
            4.0, 5.0, 6.0,
            7.0, 8.0, 9.0,
        ];

        // Upsample to 5x5
        let result = resample_grid(
            &data,
            3,
            3,
            5,
            5,
            crate::types::InterpolationMethod::Bilinear,
        );

        assert_eq!(result.len(), 25);
        // Corners should be preserved
        assert!((result[0] - 1.0).abs() < 0.001);
        assert!((result[4] - 3.0).abs() < 0.001);
        assert!((result[20] - 7.0).abs() < 0.001);
        assert!((result[24] - 9.0).abs() < 0.001);
    }
}
