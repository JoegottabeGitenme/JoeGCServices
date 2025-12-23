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

pub mod cache;
pub mod config;
pub mod downsample;
pub mod error;
pub mod minio_storage;
pub mod processor;
pub mod projection;
pub mod types;
pub mod writer;

// Re-export commonly used types at crate root
pub use cache::{ChunkCache, ChunkKey};
pub use config::{GridProcessorConfig, PyramidConfig, ZarrCompression};
pub use downsample::{DownsampleMethod, generate_pyramid, PyramidLevelData};
pub use error::{GridProcessorError, Result};
pub use minio_storage::{create_minio_storage, MinioConfig};
pub use processor::{GridProcessor, MultiscaleGridProcessorFactory, parse_multiscale_metadata, ZarrGridProcessor};
pub use projection::{
    bilinear_interpolate, cubic_interpolate, nearest_interpolate, tile_to_bbox,
    reproject_geostationary_to_geographic,
};
pub use projection::interpolation::resample_grid;
pub use types::{
    AxisInfo, BoundingBox, CacheStats, GridMetadata, GridRegion, InterpolationMethod,
    MultiscaleMetadata, PyramidLevel,
};
pub use writer::{MultiscaleWriteResult, ZarrMetadata, ZarrWriteResult, ZarrWriter};

// Re-export storage traits for use by consumers
pub use zarrs::storage::ReadableStorageTraits;
