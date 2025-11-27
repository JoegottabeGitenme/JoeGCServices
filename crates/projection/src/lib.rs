//! Coordinate reference system transformations.
//!
//! Implements map projections from scratch without external dependencies.

pub mod geographic;
pub mod geostationary;
pub mod lambert;
pub mod mercator;
pub mod polar;
pub mod transform;

pub use geostationary::Geostationary;
pub use lambert::LambertConformal;
