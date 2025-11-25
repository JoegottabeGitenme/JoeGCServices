//! Common types and utilities shared across all weather-wms services.

pub mod bbox;
pub mod crs;
pub mod error;
pub mod layer;
pub mod time;
pub mod grid;
pub mod style;
pub mod tile;

pub use bbox::BoundingBox;
pub use crs::{Crs, CrsCode};
pub use error::{WmsError, WmsResult};
pub use layer::{Layer, LayerId, LayerMetadata};
pub use time::{TimeRange, ValidTime};
pub use grid::{GridSpec, GridPoint};
pub use style::{StyleConfig, StyleDefinition, GradientConfig, Color};
pub use tile::{TileMatrix, TileMatrixSet, TileCoord};
