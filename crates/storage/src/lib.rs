//! Storage abstractions for weather-wms services.
//!
//! Provides unified interfaces for:
//! - Object storage (MinIO/S3) for grid data
//! - PostgreSQL for metadata catalog
//! - Redis for caching

pub mod cache;
pub mod catalog;
pub mod object_store;
pub mod tile_memory_cache;

pub use self::object_store::{ObjectStorage, ObjectStorageConfig, StorageStats, DetailedStorageStats};
pub use cache::{CacheKey, TileCache};
pub use catalog::{Catalog, CatalogEntry, DatasetInfo, DatasetQuery, ModelStats, ParameterAvailability, ParameterStats, PurgePreview};
pub use tile_memory_cache::{TileMemoryCache, TileMemoryCacheStats};
