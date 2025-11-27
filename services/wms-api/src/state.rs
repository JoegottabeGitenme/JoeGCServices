//! Application state and shared resources.

use anyhow::Result;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;

use storage::{Catalog, JobQueue, ObjectStorage, ObjectStorageConfig, TileCache};
use crate::metrics::MetricsCollector;

/// Shared application state.
pub struct AppState {
    pub catalog: Catalog,
    pub cache: Mutex<TileCache>,
    pub queue: JobQueue,
    pub storage: ObjectStorage,
    pub metrics: Arc<MetricsCollector>,
}

impl AppState {
    pub async fn new() -> Result<Self> {
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@postgres:5432/weatherwms".to_string()
        });

        let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());

        let storage_config = ObjectStorageConfig {
            endpoint: env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://minio:9000".to_string()),
            bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
            access_key_id: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
            secret_access_key: env::var("S3_SECRET_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            region: "us-east-1".to_string(),
            allow_http: true,
        };

        let catalog = Catalog::connect(&database_url).await?;
        let cache = TileCache::connect(&redis_url).await?;
        let queue = JobQueue::connect(&redis_url).await?;
        let storage = ObjectStorage::new(&storage_config)?;
        let metrics = Arc::new(MetricsCollector::new());

        Ok(Self {
            catalog,
            cache: Mutex::new(cache),
            queue,
            storage,
            metrics,
        })
    }
}
