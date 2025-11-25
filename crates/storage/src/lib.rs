//! Storage abstractions for weather-wms services.
//!
//! Provides unified interfaces for:
//! - Object storage (MinIO/S3) for grid data
//! - PostgreSQL for metadata catalog
//! - Redis for caching and job queues

pub mod cache;
pub mod catalog;
pub mod object_store;
pub mod queue;

pub use self::object_store::{ObjectStorage, ObjectStorageConfig};
pub use cache::{CacheKey, TileCache};
pub use catalog::{Catalog, CatalogEntry};
pub use queue::{JobQueue, JobStatus, RenderJob};
