//! Image rendering for weather data visualization.
//!
//! Implements various rendering styles:
//! - Gradient/color ramp
//! - Contour lines (marching squares)
//! - Wind barbs
//! - Wind arrows
//! - Style-based color mapping
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

pub mod barbs;
pub mod buffer_pool;
pub mod contour;
pub mod gradient;
pub mod png;
pub mod style;

// TODO: Implement rendering algorithms
