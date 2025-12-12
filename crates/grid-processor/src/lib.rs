//! Grid Processing Abstraction Layer with Zarr V3 Support
//!
//! This crate provides efficient access to chunked gridded weather data
//! using the Zarr V3 format with sharding. It enables:
//!
//! - **Partial reads**: Only fetch the chunks needed for a tile request
//! - **Efficient caching**: LRU cache for decompressed chunks
//! - **Standard format**: Zarr V3 with sharding is an industry standard
//!
//! # Architecture
//!
//! ```text
//! WMS Request
//!      │
//!      ▼
//! GridProcessor::read_region(bbox)
//!      │
//!      ├─► Calculate needed chunks (O(1) arithmetic)
//!      │
//!      ├─► Check ChunkCache for each chunk
//!      │         │
//!      │         ├─► Cache hit: return cached data
//!      │         │
//!      │         └─► Cache miss: fetch via byte-range request
//!      │
//!      └─► Assemble chunks into GridRegion
//!               │
//!               ▼
//!          Return to renderer
//! ```
//!
//! # Example
//!
//! ```ignore
//! use grid_processor::{GridProcessor, ZarrGridProcessor, BoundingBox};
//!
//! // Open a Zarr grid
//! let processor = ZarrGridProcessor::open(storage, "grids/gfs/TMP.zarr", config).await?;
//!
//! // Read a region
//! let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
//! let region = processor.read_region(&bbox).await?;
//!
//! // Use the data for rendering
//! for value in &region.data {
//!     // ...
//! }
//! ```

pub mod cache;
pub mod config;
pub mod error;
pub mod processor;
pub mod projection;
pub mod types;
pub mod writer;

// Re-export commonly used types at crate root
pub use cache::{ChunkCache, ChunkKey};
pub use config::{GridProcessorConfig, ZarrCompression};
pub use error::{GridProcessorError, Result};
pub use processor::{GridProcessor, ZarrGridProcessor};
pub use projection::{
    bilinear_interpolate, cubic_interpolate, nearest_interpolate, tile_to_bbox,
};
pub use projection::interpolation::resample_grid;
pub use types::{BoundingBox, CacheStats, GridMetadata, GridRegion, InterpolationMethod};
pub use writer::{ZarrMetadata, ZarrWriteResult, ZarrWriter};
