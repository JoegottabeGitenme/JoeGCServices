//! Object storage interface for grid data (MinIO/S3 compatible).

use bytes::Bytes;
use object_store::{aws::AmazonS3Builder, path::Path, ObjectStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, instrument};

use wms_common::{WmsError, WmsResult};

/// Configuration for object storage connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectStorageConfig {
    /// S3/MinIO endpoint URL
    pub endpoint: String,
    /// Bucket name
    pub bucket: String,
    /// Access key ID
    pub access_key_id: String,
    /// Secret access key
    pub secret_access_key: String,
    /// AWS region (use "us-east-1" for MinIO)
    pub region: String,
    /// Allow HTTP (for local MinIO)
    pub allow_http: bool,
}

impl Default for ObjectStorageConfig {
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

/// Object storage client for weather data.
pub struct ObjectStorage {
    store: Arc<dyn ObjectStore>,
    bucket: String,
}

impl ObjectStorage {
    /// Create a new object storage client from config.
    pub fn new(config: &ObjectStorageConfig) -> WmsResult<Self> {
        let mut builder = AmazonS3Builder::new()
            .with_endpoint(&config.endpoint)
            .with_bucket_name(&config.bucket)
            .with_access_key_id(&config.access_key_id)
            .with_secret_access_key(&config.secret_access_key)
            .with_region(&config.region);

        if config.allow_http {
            builder = builder.with_allow_http(true);
        }

        let store = builder
            .build()
            .map_err(|e| WmsError::StorageError(format!("Failed to create S3 client: {}", e)))?;

        Ok(Self {
            store: Arc::new(store),
            bucket: config.bucket.clone(),
        })
    }

    /// Write bytes to a path in the bucket.
    #[instrument(skip(self, data), fields(bucket = %self.bucket, path = %path))]
    pub async fn put(&self, path: &str, data: Bytes) -> WmsResult<()> {
        let location = Path::from(path);
        debug!(size = data.len(), "Writing object");

        self.store
            .put(&location, data.into())
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to write {}: {}", path, e)))?;

        Ok(())
    }

    /// Read bytes from a path.
    #[instrument(skip(self), fields(bucket = %self.bucket, path = %path))]
    pub async fn get(&self, path: &str) -> WmsResult<Bytes> {
        let location = Path::from(path);

        let result = self
            .store
            .get(&location)
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to read {}: {}", path, e)))?;

        let bytes = result
            .bytes()
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to read bytes: {}", e)))?;

        debug!(size = bytes.len(), "Read object");
        Ok(bytes)
    }

    /// Read a byte range from a path.
    #[instrument(skip(self), fields(bucket = %self.bucket, path = %path))]
    pub async fn get_range(&self, path: &str, start: usize, end: usize) -> WmsResult<Bytes> {
        let location = Path::from(path);

        let result = self
            .store
            .get_range(&location, start..end)
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to read range {}: {}", path, e)))?;

        Ok(result)
    }

    /// Check if an object exists.
    pub async fn exists(&self, path: &str) -> WmsResult<bool> {
        let location = Path::from(path);

        match self.store.head(&location).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(WmsError::StorageError(format!(
                "Failed to check {}: {}",
                path, e
            ))),
        }
    }

    /// List objects with a given prefix.
    pub async fn list(&self, prefix: &str) -> WmsResult<Vec<String>> {
        use futures::TryStreamExt;

        let prefix_path = Path::from(prefix);
        let mut paths = Vec::new();

        let mut stream = self.store.list(Some(&prefix_path));
        while let Some(meta) = stream
            .try_next()
            .await
            .map_err(|e| WmsError::StorageError(format!("List failed: {}", e)))?
        {
            paths.push(meta.location.to_string());
        }

        Ok(paths)
    }

    /// Delete an object.
    #[instrument(skip(self), fields(bucket = %self.bucket, path = %path))]
    pub async fn delete(&self, path: &str) -> WmsResult<()> {
        let location = Path::from(path);

        self.store
            .delete(&location)
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to delete {}: {}", path, e)))?;

        Ok(())
    }

    /// Get storage statistics (total size and object count).
    pub async fn stats(&self) -> WmsResult<StorageStats> {
        use futures::TryStreamExt;

        let mut total_size: u64 = 0;
        let mut object_count: u64 = 0;

        let mut stream = self.store.list(None);
        while let Some(meta) = stream
            .try_next()
            .await
            .map_err(|e| WmsError::StorageError(format!("List failed: {}", e)))?
        {
            total_size += meta.size as u64;
            object_count += 1;
        }

        Ok(StorageStats {
            total_size,
            object_count,
            bucket: self.bucket.clone(),
        })
    }
}

/// Storage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    /// Total size in bytes
    pub total_size: u64,
    /// Number of objects
    pub object_count: u64,
    /// Bucket name
    pub bucket: String,
}

/// Path builder for consistent storage layout.
pub struct StoragePath;

impl StoragePath {
    /// Build path for raw ingested files.
    /// Format: raw/{model}/{date}/{cycle}/{filename}
    pub fn raw_file(model: &str, date: &str, cycle: &str, filename: &str) -> String {
        format!("raw/{}/{}/{}/{}", model, date, cycle, filename)
    }

    /// Build path for processed grid chunks.
    /// Format: grids/{model}/{parameter}/{date}/{cycle}/{fhr}/{chunk_id}.bin
    pub fn grid_chunk(
        model: &str,
        parameter: &str,
        date: &str,
        cycle: &str,
        forecast_hour: u32,
        chunk_id: u32,
    ) -> String {
        format!(
            "grids/{}/{}/{}/{}/{:03}/{}.bin",
            model, parameter, date, cycle, forecast_hour, chunk_id
        )
    }

    /// Build path for layer metadata.
    /// Format: meta/{model}/{parameter}/metadata.json
    pub fn layer_metadata(model: &str, parameter: &str) -> String {
        format!("meta/{}/{}/metadata.json", model, parameter)
    }

    /// Build path for pre-rendered tiles.
    /// Format: tiles/{layer}/{style}/{z}/{x}/{y}.png
    pub fn tile(layer: &str, style: &str, z: u32, x: u32, y: u32) -> String {
        format!("tiles/{}/{}/{}/{}/{}.png", layer, style, z, x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_paths() {
        assert_eq!(
            StoragePath::raw_file("gfs", "20240115", "12", "gfs.t12z.pgrb2.0p25.f006"),
            "raw/gfs/20240115/12/gfs.t12z.pgrb2.0p25.f006"
        );

        assert_eq!(
            StoragePath::grid_chunk("gfs", "temperature_2m", "20240115", "12", 6, 42),
            "grids/gfs/temperature_2m/20240115/12/006/42.bin"
        );
    }
}
