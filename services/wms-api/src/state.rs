//! Application state and shared resources.

use anyhow::Result;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;

use storage::{Catalog, GribCache, JobQueue, ObjectStorage, ObjectStorageConfig, TileCache};
use crate::metrics::MetricsCollector;

/// Shared application state.
pub struct AppState {
    pub catalog: Catalog,
    pub cache: Mutex<TileCache>,
    pub queue: JobQueue,
    pub storage: Arc<ObjectStorage>,
    pub grib_cache: GribCache,
    pub metrics: Arc<MetricsCollector>,
}

impl AppState {
    pub async fn new() -> Result<Self> {
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@postgres:5432/weatherwms".to_string()
        });

        let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());
        
        // Parse connection pool sizes from environment
        let db_pool_size = env::var("DATABASE_POOL_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(20); // Increased from 10 to 20 default

        // Parse GRIB cache size from environment
        let grib_cache_size = env::var("GRIB_CACHE_SIZE")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(500); // Default: 500 entries (~2.5GB RAM)

        let storage_config = ObjectStorageConfig {
            endpoint: env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://minio:9000".to_string()),
            bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
            access_key_id: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
            secret_access_key: env::var("S3_SECRET_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            region: "us-east-1".to_string(),
            allow_http: true,
        };

        let catalog = Catalog::connect_with_pool_size(&database_url, db_pool_size).await?;
        let cache = TileCache::connect(&redis_url).await?;
        let queue = JobQueue::connect(&redis_url).await?;
        let storage = Arc::new(ObjectStorage::new(&storage_config)?);
        let metrics = Arc::new(MetricsCollector::new());
        
        // Create GRIB cache with shared storage reference
        let grib_cache = GribCache::new(grib_cache_size, storage.clone());

        Ok(Self {
            catalog,
            cache: Mutex::new(cache),
            queue,
            storage,
            grib_cache,
            metrics,
        })
    }
}
