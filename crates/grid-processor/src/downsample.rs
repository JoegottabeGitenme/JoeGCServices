//! Downsampling functions for generating pyramid levels.
//!
//! This module provides functions to reduce grid resolution by a factor of 2,
//! supporting different methods appropriate for various weather data types.

use serde::{Deserialize, Serialize};

/// Method used to downsample grid data.
///
/// The choice of method affects data quality and should be matched to the
/// parameter type:
/// - **Mean**: Best for continuous data (temperature, humidity, pressure)
/// - **Max**: Best for peak/threshold data (reflectivity, precipitation rate)
/// - **Nearest**: Fast, preserves exact values, good for categorical data
///
/// NOTE: These defaults may need parameter-specific tuning in the future.
/// For example, precipitation accumulation might benefit from Sum rather than Mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DownsampleMethod {
    /// Average of 2x2 block - good for continuous data (temperature, humidity)
    #[default]
    Mean,
    /// Maximum of 2x2 block - preserves peaks (reflectivity, precipitation)
    Max,
    /// Top-left value of 2x2 block - fast, preserves exact values
    Nearest,
}

impl DownsampleMethod {
    /// Get the appropriate downsample method for a parameter.
    ///
    /// This provides sensible defaults based on parameter characteristics.
    /// TODO: Make this configurable via ingestion config in the future.
    pub fn for_parameter(parameter: &str) -> Self {
        let param_upper = parameter.to_uppercase();
        
        // Reflectivity and precipitation - use max to preserve storm signatures
        if param_upper.contains("REFL") 
            || param_upper.contains("PRECIP_RATE")
            || param_upper.contains("DBZ")
        {
            return DownsampleMethod::Max;
        }
        
        // Most weather data benefits from averaging
        // Temperature, humidity, pressure, wind components, etc.
        DownsampleMethod::Mean
    }
}

/// Downsample a 2D grid by a factor of 2.
///
/// Takes a grid of size (width, height) and produces a grid of size
/// (width/2, height/2), rounded down for odd dimensions.
///
/// # Arguments
/// * `data` - Input grid data in row-major order
/// * `width` - Width of input grid
/// * `height` - Height of input grid
/// * `method` - Downsampling method to use
///
/// # Returns
/// Tuple of (downsampled_data, new_width, new_height)
pub fn downsample_2x(
    data: &[f32],
    width: usize,
    height: usize,
    method: DownsampleMethod,
) -> (Vec<f32>, usize, usize) {
    let new_width = width / 2;
    let new_height = height / 2;
    
    if new_width == 0 || new_height == 0 {
        return (vec![], 0, 0);
    }
    
    let mut output = vec![f32::NAN; new_width * new_height];
    
    for out_y in 0..new_height {
        for out_x in 0..new_width {
            let in_x = out_x * 2;
            let in_y = out_y * 2;
            
            // Get 2x2 block values
            let v00 = data.get(in_y * width + in_x).copied().unwrap_or(f32::NAN);
            let v10 = data.get(in_y * width + in_x + 1).copied().unwrap_or(f32::NAN);
            let v01 = data.get((in_y + 1) * width + in_x).copied().unwrap_or(f32::NAN);
            let v11 = data.get((in_y + 1) * width + in_x + 1).copied().unwrap_or(f32::NAN);
            
            let result = match method {
                DownsampleMethod::Mean => mean_of_block(v00, v10, v01, v11),
                DownsampleMethod::Max => max_of_block(v00, v10, v01, v11),
                DownsampleMethod::Nearest => v00, // Top-left value
            };
            
            output[out_y * new_width + out_x] = result;
        }
    }
    
    (output, new_width, new_height)
}

/// Calculate mean of a 2x2 block, handling NaN values.
///
/// If all values are NaN, returns NaN.
/// Otherwise, returns the mean of valid (non-NaN) values.
#[inline]
fn mean_of_block(v00: f32, v10: f32, v01: f32, v11: f32) -> f32 {
    let values = [v00, v10, v01, v11];
    let mut sum = 0.0f32;
    let mut count = 0;
    
    for &v in &values {
        if !v.is_nan() {
            sum += v;
            count += 1;
        }
    }
    
    if count == 0 {
        f32::NAN
    } else {
        sum / count as f32
    }
}

/// Calculate maximum of a 2x2 block, handling NaN values.
///
/// If all values are NaN, returns NaN.
/// Otherwise, returns the maximum of valid (non-NaN) values.
#[inline]
fn max_of_block(v00: f32, v10: f32, v01: f32, v11: f32) -> f32 {
    let values = [v00, v10, v01, v11];
    let mut max = f32::NEG_INFINITY;
    let mut has_valid = false;
    
    for &v in &values {
        if !v.is_nan() {
            has_valid = true;
            if v > max {
                max = v;
            }
        }
    }
    
    if has_valid {
        max
    } else {
        f32::NAN
    }
}

/// Result of pyramid generation for a single level.
#[derive(Debug, Clone)]
pub struct PyramidLevelData {
    /// The downsampled data
    pub data: Vec<f32>,
    /// Width at this level
    pub width: usize,
    /// Height at this level
    pub height: usize,
    /// Level index (0 = native, 1 = 2x downsampled, etc.)
    pub level: u32,
    /// Scale factor relative to native (1, 2, 4, 8, ...)
    pub scale: u32,
}

/// Generate all pyramid levels for a grid.
///
/// Repeatedly downsamples by factor of 2 until the smaller dimension
/// is less than `min_dimension`.
///
/// # Arguments
/// * `data` - Input grid data at native resolution
/// * `width` - Width of input grid
/// * `height` - Height of input grid
/// * `min_dimension` - Stop when min(width, height) < this value
/// * `method` - Downsampling method to use
///
/// # Returns
/// Vector of pyramid levels, starting with level 0 (native resolution).
pub fn generate_pyramid(
    data: &[f32],
    width: usize,
    height: usize,
    min_dimension: usize,
    method: DownsampleMethod,
) -> Vec<PyramidLevelData> {
    let mut levels = Vec::new();
    
    // Level 0 is the native resolution (we don't copy the data, caller handles it)
    levels.push(PyramidLevelData {
        data: Vec::new(), // Empty - caller already has native data
        width,
        height,
        level: 0,
        scale: 1,
    });
    
    // Generate downsampled levels
    let mut current_data = data.to_vec();
    let mut current_width = width;
    let mut current_height = height;
    let mut current_level = 0u32;
    let mut current_scale = 1u32;
    
    loop {
        // Calculate what the next level dimensions would be
        let next_width = current_width / 2;
        let next_height = current_height / 2;
        
        // Stop if the next level would be below min_dimension or zero
        if next_width == 0 || next_height == 0 {
            break;
        }
        let next_min_dim = next_width.min(next_height);
        if next_min_dim < min_dimension {
            break;
        }
        
        // Downsample
        let (new_data, new_width, new_height) = 
            downsample_2x(&current_data, current_width, current_height, method);
        
        current_level += 1;
        current_scale *= 2;
        
        levels.push(PyramidLevelData {
            data: new_data.clone(),
            width: new_width,
            height: new_height,
            level: current_level,
            scale: current_scale,
        });
        
        current_data = new_data;
        current_width = new_width;
        current_height = new_height;
    }
    
    levels
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_downsample_2x_mean() {
        // 4x4 grid with values 1-16
        let data: Vec<f32> = (1..=16).map(|x| x as f32).collect();
        let (result, w, h) = downsample_2x(&data, 4, 4, DownsampleMethod::Mean);
        
        assert_eq!(w, 2);
        assert_eq!(h, 2);
        assert_eq!(result.len(), 4);
        
        // Top-left 2x2 block: 1,2,5,6 -> mean = 3.5
        assert!((result[0] - 3.5).abs() < 0.001);
        // Top-right 2x2 block: 3,4,7,8 -> mean = 5.5
        assert!((result[1] - 5.5).abs() < 0.001);
    }
    
    #[test]
    fn test_downsample_2x_max() {
        let data: Vec<f32> = (1..=16).map(|x| x as f32).collect();
        let (result, w, h) = downsample_2x(&data, 4, 4, DownsampleMethod::Max);
        
        assert_eq!(w, 2);
        assert_eq!(h, 2);
        
        // Top-left 2x2 block: 1,2,5,6 -> max = 6
        assert!((result[0] - 6.0).abs() < 0.001);
        // Top-right 2x2 block: 3,4,7,8 -> max = 8
        assert!((result[1] - 8.0).abs() < 0.001);
    }
    
    #[test]
    fn test_downsample_2x_nearest() {
        let data: Vec<f32> = (1..=16).map(|x| x as f32).collect();
        let (result, _, _) = downsample_2x(&data, 4, 4, DownsampleMethod::Nearest);
        
        // Top-left of each 2x2 block
        assert!((result[0] - 1.0).abs() < 0.001);
        assert!((result[1] - 3.0).abs() < 0.001);
    }
    
    #[test]
    fn test_downsample_handles_nan() {
        let data = vec![1.0, f32::NAN, 3.0, 4.0];
        let (result, w, h) = downsample_2x(&data, 2, 2, DownsampleMethod::Mean);
        
        assert_eq!(w, 1);
        assert_eq!(h, 1);
        // Mean of 1, 3, 4 (ignoring NaN) = 8/3 ≈ 2.667
        assert!((result[0] - 2.667).abs() < 0.01);
    }
    
    #[test]
    fn test_generate_pyramid() {
        // 16x16 grid
        let data: Vec<f32> = (0..256).map(|x| x as f32).collect();
        let levels = generate_pyramid(&data, 16, 16, 4, DownsampleMethod::Mean);
        
        // Should have: 16x16, 8x8, 4x4 = 3 levels
        // Level 4 (2x2) would be below min_dimension=4, so not included
        assert_eq!(levels.len(), 3);
        
        assert_eq!(levels[0].width, 16);
        assert_eq!(levels[0].height, 16);
        assert_eq!(levels[0].level, 0);
        assert_eq!(levels[0].scale, 1);
        
        assert_eq!(levels[1].width, 8);
        assert_eq!(levels[1].height, 8);
        assert_eq!(levels[1].level, 1);
        assert_eq!(levels[1].scale, 2);
        
        assert_eq!(levels[2].width, 4);
        assert_eq!(levels[2].height, 4);
        assert_eq!(levels[2].level, 2);
        assert_eq!(levels[2].scale, 4);
    }
    
    #[test]
    fn test_downsample_method_for_parameter() {
        assert_eq!(DownsampleMethod::for_parameter("TMP"), DownsampleMethod::Mean);
        assert_eq!(DownsampleMethod::for_parameter("RH"), DownsampleMethod::Mean);
        assert_eq!(DownsampleMethod::for_parameter("REFL"), DownsampleMethod::Max);
        assert_eq!(DownsampleMethod::for_parameter("PRECIP_RATE"), DownsampleMethod::Max);
    }
}
