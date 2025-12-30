//! Application state for the EDR API.

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;

use grid_processor::{GridDataService, MinioConfig};
use storage::Catalog;

use crate::config::EdrConfig;

/// Shared application state.
pub struct AppState {
    /// Database catalog for metadata queries.
    pub catalog: Arc<Catalog>,

    /// High-level grid data service for data access.
    pub grid_data_service: GridDataService,

    /// EDR configuration (hot-reloadable).
    pub edr_config: Arc<RwLock<EdrConfig>>,

    /// Base URL for building links.
    pub base_url: String,
}

impl AppState {
    /// Create a new AppState from environment configuration.
    pub async fn new() -> Result<Self> {
        // Get database URL
        let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://weatherwms:weatherwms@localhost:5432/weatherwms".to_string()
        });

        // Get S3/MinIO configuration
        let s3_endpoint =
            std::env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".to_string());
        let s3_bucket = std::env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string());
        let s3_access_key =
            std::env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string());
        let s3_secret_key =
            std::env::var("S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".to_string());

        // Get base URL for links
        let base_url = std::env::var("EDR_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:8083/edr".to_string());

        // Create catalog
        let catalog = Arc::new(Catalog::connect(&database_url).await?);

        // Create MinIO config
        let minio_config = MinioConfig {
            endpoint: s3_endpoint,
            bucket: s3_bucket,
            access_key_id: s3_access_key,
            secret_access_key: s3_secret_key,
            region: "us-east-1".to_string(),
            allow_http: true,
        };

        // Get chunk cache size from environment
        let chunk_cache_size_mb: usize = std::env::var("EDR_CHUNK_CACHE_MB")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(256);

        // Create high-level grid data service
        let grid_data_service =
            GridDataService::new(Arc::clone(&catalog), minio_config, chunk_cache_size_mb);

        // Load EDR config
        let edr_config = EdrConfig::load_from_dir("config/edr")?;

        Ok(Self {
            catalog,
            grid_data_service,
            edr_config: Arc::new(RwLock::new(edr_config)),
            base_url,
        })
    }

    /// Reload EDR configuration from disk.
    pub async fn reload_config(&self) -> Result<()> {
        let new_config = EdrConfig::load_from_dir("config/edr")?;
        let mut config = self.edr_config.write().await;
        *config = new_config;
        Ok(())
    }
}
