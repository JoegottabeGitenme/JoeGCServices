//! Coordinate reference system transformations.
//!
//! Implements map projections from scratch without external dependencies.

pub mod geographic;
pub mod geostationary;
pub mod lambert;
pub mod lut;
pub mod mercator;
pub mod polar;
pub mod transform;

pub use geostationary::Geostationary;
pub use lambert::LambertConformal;
pub use lut::{
    compute_all_luts, compute_tile_lut, resample_with_lut, ProjectionLutCache, TileGridLut,
    TileLutKey, PIXELS_PER_TILE, TILE_SIZE,
};
