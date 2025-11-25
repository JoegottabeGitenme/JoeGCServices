//! Common types and utilities shared across all weather-wms services.

pub mod bbox;
pub mod crs;
pub mod error;
pub mod grid;
pub mod layer;
pub mod style;
pub mod tile;
pub mod time;

pub use bbox::BoundingBox;
pub use crs::{Crs, CrsCode};
pub use error::{WmsError, WmsResult};
pub use grid::{GridPoint, GridSpec};
pub use layer::{Layer, LayerId, LayerMetadata};
pub use style::{Color, GradientConfig, StyleConfig, StyleDefinition};
pub use tile::{TileCoord, TileMatrix, TileMatrixSet};
pub use time::{TimeRange, ValidTime};
