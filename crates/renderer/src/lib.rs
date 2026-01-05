//! Image rendering for weather data visualization.
//!
//! Implements various rendering styles:
//! - Gradient/color ramp
//! - Contour lines (marching squares)
//! - Wind barbs
//! - Wind arrows
//! - Style-based color mapping
//! - Data PNG encoding for GPU shaders
//!
//! ## Performance Optimizations
//!
//! The renderer includes several optimizations for high-throughput tile serving:
//!
//! - **Pre-computed palettes**: Color palettes are computed once per style and cached.
//!   Use `StyleDefinition::compute_palette()` at load time.
//! - **Indexed PNG rendering**: `apply_style_gradient_indexed()` outputs 1 byte/pixel
//!   instead of 4, enabling 3-4x faster full pipeline performance.
//! - **Parallel processing**: Uses rayon for parallel row processing in render functions.
//! - **Buffer pooling**: Thread-local buffer pools reduce allocation pressure under load.
//!   See [`buffer_pool`] module for details.
//!
//! ## Data PNG Encoding
//!
//! The [`data_png`] module provides 16-bit PNG encoding for GPU shader consumption.
//! This is optimized for WebGL texture upload and weather data visualization,
//! similar to the approach used by Windy.com.

pub mod barbs;
pub mod buffer_pool;
pub mod contour;
pub mod data_png;
pub mod gradient;
pub mod png;
pub mod style;

// TODO: Implement rendering algorithms
