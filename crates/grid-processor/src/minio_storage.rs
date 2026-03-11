//! MinIO/S3 storage backend for Zarr access.
//!
//! This module provides helper functions for creating MinIO-compatible
//! storage backends that work with the zarrs crate.

use std::sync::Arc;
use std::time::Duration;

// Use the direct object_store crate (version must match what zarrs_object_store uses)
use object_store::aws::AmazonS3Builder;
use object_store::ClientOptions;
use zarrs_object_store::AsyncObjectStore;
use zarrs_storage::storage_adapter::async_to_sync::{
    AsyncToSyncBlockOn, AsyncToSyncStorageAdapter,
};

use crate::error::{GridProcessorError, Result};

/// Blocking executor that works from within a tokio runtime.
///
/// Uses `tokio::task::block_in_place` to move the current task to a blocking
/// thread, then uses the runtime handle to drive the future. This avoids the
/// "cannot start a runtime from within a runtime" error.
#[derive(Clone, Copy)]
pub struct TokioBlockOn;

impl AsyncToSyncBlockOn for TokioBlockOn {
    fn block_on<F: core::future::Future>(&self, future: F) -> F::Output {
        // block_in_place moves the current task off the async worker thread
        // so we can safely call block_on without nesting runtimes
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
    }
}

/// Configuration for connecting to MinIO/S3.
#[derive(Debug, Clone)]
pub struct MinioConfig {
    /// S3/MinIO endpoint URL (e.g., "http://minio:9000")
    pub endpoint: String,
    /// Bucket name
    pub bucket: String,
    /// Access key ID
    pub access_key_id: String,
    /// Secret access key
    pub secret_access_key: String,
    /// AWS region (use "us-east-1" for MinIO)
    pub region: String,
    /// Allow HTTP (required for local MinIO)
    pub allow_http: bool,
}

impl Default for MinioConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://minio:9000".to_string(),
            bucket: "weather-data".to_string(),
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            region: "us-east-1".to_string(),
            allow_http: true,
        }
    }
}

impl MinioConfig {
    /// Create config from environment variables.
    pub fn from_env() -> Self {
        Self {
            endpoint: std::env::var("S3_ENDPOINT")
                .unwrap_or_else(|_| "http://minio:9000".to_string()),
            bucket: std::env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
            access_key_id: std::env::var("S3_ACCESS_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            secret_access_key: std::env::var("S3_SECRET_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
            allow_http: std::env::var("S3_ALLOW_HTTP")
                .map(|v| v.to_lowercase() == "true" || v == "1")
                .unwrap_or(true),
        }
    }
}

/// Storage type alias for MinIO-backed Zarr access (async).
pub type AsyncMinioStorage = AsyncObjectStore<object_store::aws::AmazonS3>;

/// Storage type alias for MinIO-backed Zarr access (sync adapter).
/// This type implements ReadableStorageTraits and can be used with ZarrGridProcessor.
pub type MinioStorage = AsyncToSyncStorageAdapter<AsyncMinioStorage, TokioBlockOn>;

/// Create a MinIO-compatible storage backend for Zarr access.
///
/// This function creates an object_store client configured for MinIO,
/// wraps it in AsyncObjectStore, and then wraps that in an async-to-sync
/// adapter for use with the synchronous zarrs API.
///
/// # Arguments
/// * `config` - MinIO connection configuration
///
/// # Returns
/// An Arc-wrapped storage adapter that implements ReadableStorageTraits
pub fn create_minio_storage(config: &MinioConfig) -> Result<Arc<MinioStorage>> {
    // Configure connection pool and timeouts to prevent connection exhaustion
    // when many concurrent tile renders share this client.
    let client_options = ClientOptions::new()
        .with_pool_max_idle_per_host(32)
        .with_connect_timeout(Duration::from_secs(5))
        .with_timeout(Duration::from_secs(30))
        .with_pool_idle_timeout(Duration::from_secs(60));

    let s3 = AmazonS3Builder::new()
        .with_endpoint(&config.endpoint)
        .with_bucket_name(&config.bucket)
        .with_access_key_id(&config.access_key_id)
        .with_secret_access_key(&config.secret_access_key)
        .with_region(&config.region)
        .with_client_options(client_options)
        .with_allow_http(config.allow_http)
        .build()
        .map_err(|e| {
            GridProcessorError::open_failed(format!("Failed to create S3 client: {}", e))
        })?;

    let async_store = Arc::new(AsyncObjectStore::new(s3));

    // Use TokioBlockOn which uses block_in_place + Handle::current().block_on()
    // This is safe to call from within tokio async contexts
    let block_on = TokioBlockOn;

    // Wrap in async-to-sync adapter
    let sync_store = AsyncToSyncStorageAdapter::new(async_store, block_on);

    Ok(Arc::new(sync_store))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = MinioConfig::default();
        assert_eq!(config.endpoint, "http://minio:9000");
        assert_eq!(config.bucket, "weather-data");
        assert!(config.allow_http);
    }

    #[test]
    fn test_create_minio_storage_succeeds_with_defaults() {
        // Verifies that create_minio_storage builds a valid S3 client
        // with ClientOptions (pool size, timeouts) without panicking.
        let config = MinioConfig::default();
        let result = create_minio_storage(&config);
        assert!(
            result.is_ok(),
            "create_minio_storage should succeed with default config"
        );
    }

    #[test]
    fn test_create_minio_storage_returns_arc() {
        // Verify the returned storage is Arc-wrapped and can be cloned cheaply
        let config = MinioConfig::default();
        let store = create_minio_storage(&config).unwrap();
        let store_clone = store.clone();
        // Both point to the same underlying storage (Arc strong count = 2)
        assert_eq!(Arc::strong_count(&store), 2);
        drop(store_clone);
        assert_eq!(Arc::strong_count(&store), 1);
    }

    #[test]
    fn test_create_minio_storage_custom_endpoint() {
        // Ensure custom endpoints (e.g., k8s service names) work
        let config = MinioConfig {
            endpoint: "http://joegcservices-minio:9000".to_string(),
            bucket: "weather-data".to_string(),
            access_key_id: "testkey".to_string(),
            secret_access_key: "testsecret".to_string(),
            region: "us-east-2".to_string(),
            allow_http: true,
        };
        let result = create_minio_storage(&config);
        assert!(result.is_ok(), "Should handle custom endpoint");
    }
}
