//! Application state and shared resources.

use anyhow::Result;
use std::env;

use storage::{Catalog, TileCache, JobQueue, ObjectStorage, ObjectStorageConfig};

/// Shared application state.
pub struct AppState {
    pub catalog: Catalog,
    pub cache: TileCache,
    pub queue: JobQueue,
    pub storage: ObjectStorage,
}

impl AppState {
    pub async fn new() -> Result<Self> {
        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://postgres:postgres@postgres:5432/weatherwms".to_string());

        let redis_url = env::var("REDIS_URL")
            .unwrap_or_else(|_| "redis://redis:6379".to_string());

        let storage_config = ObjectStorageConfig {
            endpoint: env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://minio:9000".to_string()),
            bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
            access_key_id: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
            secret_access_key: env::var("S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
            region: "us-east-1".to_string(),
            allow_http: true,
        };

        let catalog = Catalog::connect(&database_url).await?;
        let cache = TileCache::connect(&redis_url).await?;
        let queue = JobQueue::connect(&redis_url).await?;
        let storage = ObjectStorage::new(&storage_config)?;

        Ok(Self {
            catalog,
            cache,
            queue,
            storage,
        })
    }
}
