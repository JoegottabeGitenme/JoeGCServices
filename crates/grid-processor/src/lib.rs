//! # Grid Processing Abstraction Layer
//!
//! This crate provides efficient access to chunked gridded weather data using
//! the Zarr V3 format. It serves as the **data abstraction layer** for OGC services
//! (WMS, WMTS, EDR, WCS) and any other consumers that need weather grid data.
//!
//! ## Key Capabilities
//!
//! - **Partial reads**: Only fetch the chunks needed for a specific geographic region
//! - **Pyramid support**: Automatically select optimal resolution for output size
//! - **Chunk caching**: LRU cache for decompressed chunks shared across requests
//! - **Unified query interface**: Find datasets by model, parameter, time, and level
//! - **Zarr V3 format**: Industry-standard format with sharding support
//!
//! ## API Levels
//!
//! This crate provides three levels of abstraction:
//!
//! ### High-Level: [`GridDataService`]
//!
//! The recommended interface for most use cases. Handles catalog queries,
//! storage access, and model-specific quirks automatically.
//!
//! ```rust,ignore
//! use grid_processor::{GridDataService, DatasetQuery, BoundingBox};
//!
//! // Create service (typically at application startup)
//! let service = GridDataService::new(catalog, storage_config, 1024).await?;
//!
//! // Query for a forecast dataset
//! let query = DatasetQuery::forecast("gfs", "TMP")
//!     .at_level("2 m above ground")
//!     .at_forecast_hour(6);
//!
//! // Read a region (for tile rendering)
//! let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
//! let region = service.read_region(&query, &bbox, Some((256, 256))).await?;
//!
//! // Query a point (for GetFeatureInfo or EDR Position)
//! let value = service.read_point(&query, -95.0, 35.0).await?;
//! ```
//!
//! ### Mid-Level: [`GridProcessorFactory`]
//!
//! For cases where you have a catalog entry and want direct control:
//!
//! ```rust,ignore
//! use grid_processor::{GridProcessorFactory, BoundingBox};
//!
//! let factory = GridProcessorFactory::new(storage, 1024);
//! let processor = factory.create_processor(&zarr_path, &metadata)?;
//! let region = processor.read_region(&bbox).await?;
//! ```
//!
//! ### Low-Level: [`GridProcessor`] Trait
//!
//! For custom implementations or direct Zarr access:
//!
//! ```rust,ignore
//! use grid_processor::{GridProcessor, ZarrGridProcessor, GridProcessorConfig};
//!
//! let processor = ZarrGridProcessor::open(store, "/path/to/array.zarr", config)?;
//! let region = processor.read_region(&bbox).await?;
//! let point = processor.read_point(-95.0, 35.0).await?;
//! ```
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Consumer Services                            │
//! │               (WMS, WMTS, EDR, WCS, Custom APIs)                │
//! └─────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     GridDataService                             │
//! │  - DatasetQuery builder for finding datasets                    │
//! │  - Catalog integration (model/param/time/level lookup)          │
//! │  - Automatic pyramid level selection                            │
//! └─────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    GridProcessorFactory                         │
//! │  - Shared ChunkCache across all requests                        │
//! │  - Shared storage connection (MinIO/S3)                         │
//! └─────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                   GridProcessor Trait                           │
//! │  - read_region(bbox) → GridRegion                               │
//! │  - read_point(lon, lat) → Option<f32>                           │
//! │  - metadata() → GridMetadata                                    │
//! └─────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    ZarrGridProcessor                            │
//! │  - Zarr V3 array access with byte-range requests                │
//! │  - Automatic chunk caching                                      │
//! └─────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                  Object Storage (MinIO/S3)                      │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Use Cases by Service
//!
//! | Service | Primary Methods | Example |
//! |---------|-----------------|---------|
//! | WMS/WMTS | `read_region()` | Tile rendering with bbox |
//! | GetFeatureInfo | `read_point()` | Point query for popup |
//! | EDR Position | `read_point()` | Single coordinate query |
//! | EDR Area | `read_region()` | Bbox query for CoverageJSON |
//! | EDR Trajectory | Multiple `read_point()` | Iterate over path |
//! | WCS GetCoverage | `read_region()` | Raw grid data export |
//!
//! ## Feature Flags
//!
//! This crate has no optional features - all functionality is always available.

pub mod cache;
pub mod config;
pub mod downsample;
pub mod error;
pub mod factory;
pub mod minio_storage;
pub mod processor;
pub mod projection;
pub mod query;
pub mod service;
#[cfg(test)]
pub mod testdata;
pub mod types;
pub mod writer;

// Re-export commonly used types at crate root
pub use cache::{ChunkCache, ChunkKey};
pub use config::{GridProcessorConfig, PyramidConfig, ZarrCompression};
pub use downsample::{generate_pyramid, DownsampleMethod, PyramidLevelData};
pub use error::{GridProcessorError, Result};
pub use factory::GridProcessorFactory;
pub use minio_storage::{create_minio_storage, MinioConfig};
pub use processor::{
    parse_multiscale_metadata, GridProcessor, MultiscaleGridProcessorFactory, ZarrGridProcessor,
};
pub use projection::interpolation::resample_grid;
pub use projection::{
    bilinear_interpolate, cubic_interpolate, nearest_interpolate,
    reproject_geostationary_to_geographic, tile_to_bbox,
};
pub use query::{DatasetQuery, PointValue, TimeSpecification};
pub use service::GridDataService;
pub use types::{
    AxisInfo, BoundingBox, CacheStats, GridMetadata, GridRegion, InterpolationMethod,
    MultiscaleMetadata, PyramidLevel,
};
pub use writer::{MultiscaleWriteResult, ZarrMetadata, ZarrWriteResult, ZarrWriter};

// Re-export storage traits for use by consumers
pub use zarrs::storage::ReadableStorageTraits;
