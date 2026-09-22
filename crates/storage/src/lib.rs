//! Storage abstractions for weather-wms services.
//!
//! Provides unified interfaces for:
//! - Object storage (MinIO/S3) for grid data
//! - PostgreSQL for metadata catalog
//! - PostgreSQL + PostGIS for point observations
//! - Redis for caching

pub mod cache;
pub mod catalog;
pub mod linear_features;
pub mod object_store;
pub mod observations;
pub mod segment_conditions;
pub mod stations_bootstrap;
pub mod storm_events;
pub mod tile_memory_cache;
pub mod trail_reports;

pub use self::object_store::{
    DetailedStorageStats, ObjectStorage, ObjectStorageConfig, StorageStats,
};
pub use cache::{CacheKey, TileCache};
pub use catalog::{
    Catalog, CatalogEntry, DatasetInfo, DatasetQuery, ModelStats, ParameterAvailability,
    ParameterStats, PurgePreview,
};
pub use linear_features::{LinearFeature, LinearFeatureCatalog, LinearFeatureItem};
pub use observations::{
    Location, Observation, ObservationCatalog, ObservationInsertResult, ObservationQuery,
};
pub use segment_conditions::{SegmentCondition, SegmentConditionsCatalog};
pub use stations_bootstrap::{
    bootstrap_locations, bootstrap_populated_places, bootstrap_zip_codes,
};
pub use storm_events::{
    CountyAggregate, CountyEventsResult, StormEvent, StormEventCatalog, StormEventFeature,
};
pub use tile_memory_cache::{TileMemoryCache, TileMemoryCacheStats};
pub use trail_reports::{TrailReport, TrailReportCatalog};
