//! Zarr directory upload utilities.

use bytes::Bytes;
use std::path::Path;
use storage::ObjectStorage;
use tracing::debug;

use crate::error::{IngestionError, Result};

/// Upload a Zarr directory to object storage.
///
/// Recursively walks the local Zarr directory and uploads all files
/// to the specified storage prefix.
///
/// # Arguments
/// * `storage` - Object storage client (MinIO/S3)
/// * `local_path` - Local path to the Zarr directory
/// * `storage_prefix` - Target path prefix in object storage
///
/// # Returns
/// Total bytes uploaded
pub async fn upload_zarr_directory(
    storage: &ObjectStorage,
    local_path: &Path,
    storage_prefix: &str,
) -> Result<u64> {
    let mut total_size = 0u64;

    for entry in walkdir::WalkDir::new(local_path) {
        let entry = entry.map_err(|e| IngestionError::StorageUpload(e.to_string()))?;

        if entry.file_type().is_file() {
            let relative_path = entry
                .path()
                .strip_prefix(local_path)
                .map_err(|e| IngestionError::StorageUpload(e.to_string()))?;

            let storage_path = format!("{}/{}", storage_prefix, relative_path.display());

            let file_data = tokio::fs::read(entry.path()).await?;
            let file_size = file_data.len() as u64;
            total_size += file_size;

            storage
                .put(&storage_path, Bytes::from(file_data))
                .await
                .map_err(|e| IngestionError::StorageUpload(e.to_string()))?;

            debug!(path = %storage_path, size = file_size, "Uploaded Zarr file");
        }
    }

    Ok(total_size)
}
