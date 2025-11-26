//! Image rendering for weather data visualization.
//!
//! Implements various rendering styles:
//! - Gradient/color ramp
//! - Contour lines (marching squares)
//! - Wind barbs
//! - Wind arrows
//! - Style-based color mapping

pub mod barbs;
pub mod contour;
pub mod gradient;
pub mod png;
pub mod style;

// TODO: Implement rendering algorithms
