//! Storage abstractions for weather-wms services.
//!
//! Provides unified interfaces for:
//! - Object storage (MinIO/S3) for grid data
//! - PostgreSQL for metadata catalog
//! - Redis for caching and job queues

pub mod cache;
pub mod catalog;
pub mod grib_cache;
pub mod grid_cache;
pub mod object_store;
pub mod queue;
pub mod tile_memory_cache;

pub use self::object_store::{ObjectStorage, ObjectStorageConfig, StorageStats, DetailedStorageStats};
pub use cache::{CacheKey, TileCache};
pub use catalog::{Catalog, CatalogEntry, DatasetQuery, PurgePreview};
pub use grib_cache::{GribCache, CacheStats as GribCacheStats};
pub use grid_cache::{GridDataCache, CachedGridData, GoesProjectionParams, GridCacheStats};
pub use queue::{JobQueue, JobStatus, RenderJob};
pub use tile_memory_cache::{TileMemoryCache, TileMemoryCacheStats};
