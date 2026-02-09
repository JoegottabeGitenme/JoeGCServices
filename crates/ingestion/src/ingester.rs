//! Main Ingester struct for weather data ingestion.

use bytes::Bytes;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tracing::warn;

use storage::{Catalog, ObjectStorage};

use crate::error::Result;
use crate::geotiff;
use crate::grib2;
use crate::metadata::{detect_file_type, FileType};
use crate::netcdf;
use grib2_parser::strip_wmo_headers;

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
                grib2::ingest_grib2(
                    &self.storage,
                    &self.catalog,
                    decompressed,
                    file_path,
                    &options,
                )
                .await
            }
            FileType::NdfdGrib2 => {
                // Strip WMO bulletin headers and ingest as GRIB2
                let stripped = strip_wmo_headers(&data);
                grib2::ingest_grib2(
                    &self.storage,
                    &self.catalog,
                    Bytes::from(stripped),
                    file_path,
                    &options,
                )
                .await
            }
            FileType::NetCdf => {
                netcdf::ingest_netcdf(&self.storage, &self.catalog, data, file_path, &options).await
            }
            FileType::GeoTiff | FileType::GeoTiffGz => {
                // GeoTIFF files (VIIRS light pollution) - handles both compressed and uncompressed
                geotiff::ingest_geotiff(&self.storage, &self.catalog, data, file_path, &options)
                    .await
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
        grib2::ingest_grib2(
            &self.storage,
            &self.catalog,
            decompressed,
            file_path,
            &options,
        )
        .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone};

    // Tests for IngestOptions
    #[test]
    fn test_ingest_options_default() {
        let opts = IngestOptions::default();
        assert!(opts.model.is_none());
        assert!(opts.forecast_hour.is_none());
    }

    #[test]
    fn test_ingest_options_with_model() {
        let opts = IngestOptions {
            model: Some("gfs".to_string()),
            forecast_hour: None,
        };
        assert_eq!(opts.model, Some("gfs".to_string()));
        assert!(opts.forecast_hour.is_none());
    }

    #[test]
    fn test_ingest_options_with_forecast_hour() {
        let opts = IngestOptions {
            model: None,
            forecast_hour: Some(12),
        };
        assert!(opts.model.is_none());
        assert_eq!(opts.forecast_hour, Some(12));
    }

    #[test]
    fn test_ingest_options_fully_specified() {
        let opts = IngestOptions {
            model: Some("hrrr".to_string()),
            forecast_hour: Some(6),
        };
        assert_eq!(opts.model, Some("hrrr".to_string()));
        assert_eq!(opts.forecast_hour, Some(6));
    }

    #[test]
    fn test_ingest_options_clone() {
        let opts = IngestOptions {
            model: Some("nam".to_string()),
            forecast_hour: Some(18),
        };
        let cloned = opts.clone();
        assert_eq!(cloned.model, opts.model);
        assert_eq!(cloned.forecast_hour, opts.forecast_hour);
    }

    #[test]
    fn test_ingest_options_debug() {
        let opts = IngestOptions {
            model: Some("test".to_string()),
            forecast_hour: Some(3),
        };
        let debug_str = format!("{:?}", opts);
        assert!(debug_str.contains("IngestOptions"));
        assert!(debug_str.contains("test"));
        assert!(debug_str.contains("3"));
    }

    // Tests for IngestionResult
    #[test]
    fn test_ingestion_result_creation() {
        let result = IngestionResult {
            datasets_registered: 5,
            model: "gfs".to_string(),
            reference_time: Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap(),
            parameters: vec!["TMP".to_string(), "UGRD".to_string()],
            bytes_written: 1024 * 1024,
        };

        assert_eq!(result.datasets_registered, 5);
        assert_eq!(result.model, "gfs");
        assert_eq!(result.reference_time.year(), 2024);
        assert_eq!(result.parameters.len(), 2);
        assert_eq!(result.bytes_written, 1024 * 1024);
    }

    #[test]
    fn test_ingestion_result_empty_parameters() {
        let result = IngestionResult {
            datasets_registered: 0,
            model: "empty".to_string(),
            reference_time: Utc::now(),
            parameters: vec![],
            bytes_written: 0,
        };

        assert_eq!(result.datasets_registered, 0);
        assert!(result.parameters.is_empty());
        assert_eq!(result.bytes_written, 0);
    }

    #[test]
    fn test_ingestion_result_clone() {
        let result = IngestionResult {
            datasets_registered: 3,
            model: "hrrr".to_string(),
            reference_time: Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap(),
            parameters: vec!["APCP".to_string()],
            bytes_written: 2048,
        };

        let cloned = result.clone();
        assert_eq!(cloned.datasets_registered, result.datasets_registered);
        assert_eq!(cloned.model, result.model);
        assert_eq!(cloned.reference_time, result.reference_time);
        assert_eq!(cloned.parameters, result.parameters);
        assert_eq!(cloned.bytes_written, result.bytes_written);
    }

    #[test]
    fn test_ingestion_result_debug() {
        let result = IngestionResult {
            datasets_registered: 10,
            model: "goes18".to_string(),
            reference_time: Utc::now(),
            parameters: vec!["CMI_C02".to_string()],
            bytes_written: 500_000,
        };

        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("IngestionResult"));
        assert!(debug_str.contains("goes18"));
        assert!(debug_str.contains("CMI_C02"));
        assert!(debug_str.contains("500000"));
    }

    #[test]
    fn test_ingestion_result_many_parameters() {
        let params: Vec<String> = (1..=16).map(|i| format!("CMI_C{:02}", i)).collect();

        let result = IngestionResult {
            datasets_registered: 16,
            model: "goes19".to_string(),
            reference_time: Utc::now(),
            parameters: params.clone(),
            bytes_written: 16 * 1024 * 1024,
        };

        assert_eq!(result.parameters.len(), 16);
        assert!(result.parameters.contains(&"CMI_C01".to_string()));
        assert!(result.parameters.contains(&"CMI_C16".to_string()));
    }
}
