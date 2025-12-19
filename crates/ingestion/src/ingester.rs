//! Main Ingester struct for weather data ingestion.

use bytes::Bytes;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tracing::warn;

use storage::{Catalog, ObjectStorage};

use crate::error::Result;
use crate::grib2;
use crate::metadata::{detect_file_type, FileType};
use crate::netcdf;

/// Options for ingestion operations.
#[derive(Debug, Clone, Default)]
pub struct IngestOptions {
    /// Override model detection from filename
    pub model: Option<String>,
    /// Override forecast hour detection from filename
    pub forecast_hour: Option<u32>,
}

/// Result of an ingestion operation.
#[derive(Debug, Clone)]
pub struct IngestionResult {
    /// Number of datasets registered in catalog
    pub datasets_registered: usize,
    /// Model identifier
    pub model: String,
    /// Reference time of the data
    pub reference_time: DateTime<Utc>,
    /// List of parameters that were ingested
    pub parameters: Vec<String>,
    /// Total bytes written to storage
    pub bytes_written: u64,
}

/// Core ingester for weather data.
///
/// Handles parsing, transformation, and storage of weather data files
/// (GRIB2, NetCDF) into Zarr format with multi-resolution pyramids.
pub struct Ingester {
    /// Object storage client (MinIO/S3)
    storage: Arc<ObjectStorage>,
    /// Catalog for dataset registration
    catalog: Catalog,
}

impl Ingester {
    /// Create a new Ingester.
    pub fn new(storage: Arc<ObjectStorage>, catalog: Catalog) -> Self {
        Self { storage, catalog }
    }

    /// Ingest a file from the filesystem.
    ///
    /// Auto-detects file type from extension and routes to appropriate handler.
    pub async fn ingest_file(
        &self,
        file_path: &str,
        options: IngestOptions,
    ) -> Result<IngestionResult> {
        // Read file
        let data = tokio::fs::read(file_path).await?;
        let data = Bytes::from(data);

        self.ingest_bytes(data, file_path, options).await
    }

    /// Ingest data from bytes.
    ///
    /// Auto-detects file type from the provided path/filename.
    pub async fn ingest_bytes(
        &self,
        data: Bytes,
        file_path: &str,
        options: IngestOptions,
    ) -> Result<IngestionResult> {
        let file_type = detect_file_type(file_path);

        match file_type {
            FileType::Grib2 => {
                grib2::ingest_grib2(&self.storage, &self.catalog, data, file_path, &options).await
            }
            FileType::Grib2Gz => {
                // Decompress and ingest
                let decompressed = grib2::decompress_gzip(&data)?;
                grib2::ingest_grib2(&self.storage, &self.catalog, decompressed, file_path, &options)
                    .await
            }
            FileType::NetCdf => {
                netcdf::ingest_netcdf(&self.storage, &self.catalog, data, file_path, &options).await
            }
            FileType::Unknown => {
                // Try to guess based on content or model
                if let Some(ref model) = options.model {
                    if model.starts_with("goes") {
                        return netcdf::ingest_netcdf(
                            &self.storage,
                            &self.catalog,
                            data,
                            file_path,
                            &options,
                        )
                        .await;
                    }
                }
                
                // Default to GRIB2
                warn!(
                    file_path = %file_path,
                    "Unknown file type, attempting GRIB2 parse"
                );
                grib2::ingest_grib2(&self.storage, &self.catalog, data, file_path, &options).await
            }
        }
    }

    /// Ingest GRIB2 data directly.
    pub async fn ingest_grib2(
        &self,
        data: Bytes,
        file_path: &str,
        options: IngestOptions,
    ) -> Result<IngestionResult> {
        grib2::ingest_grib2(&self.storage, &self.catalog, data, file_path, &options).await
    }

    /// Ingest gzip-compressed GRIB2 data.
    pub async fn ingest_grib2_gz(
        &self,
        data: Bytes,
        file_path: &str,
        options: IngestOptions,
    ) -> Result<IngestionResult> {
        let decompressed = grib2::decompress_gzip(&data)?;
        grib2::ingest_grib2(&self.storage, &self.catalog, decompressed, file_path, &options).await
    }

    /// Ingest NetCDF data directly (GOES satellite).
    pub async fn ingest_netcdf(
        &self,
        data: Bytes,
        file_path: &str,
        options: IngestOptions,
    ) -> Result<IngestionResult> {
        netcdf::ingest_netcdf(&self.storage, &self.catalog, data, file_path, &options).await
    }

    /// Get a reference to the storage client.
    pub fn storage(&self) -> &Arc<ObjectStorage> {
        &self.storage
    }

    /// Get a reference to the catalog.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
}
