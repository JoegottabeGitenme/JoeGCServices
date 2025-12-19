//! Weather data ingestion library.
//!
//! Provides core logic for ingesting weather data (GRIB2, NetCDF) into
//! Zarr format with multi-resolution pyramids.
//!
//! # Architecture
//!
//! This crate is designed to be used by both the `ingester` service and
//! the `wms-api` service (for backwards compatibility). It handles:
//!
//! - GRIB2 parsing and parameter extraction (GFS, HRRR, MRMS)
//! - NetCDF parsing and reprojection (GOES satellite)
//! - Zarr pyramid generation
//! - Upload to object storage (MinIO/S3)
//! - Catalog registration (PostgreSQL)
//!
//! # Example
//!
//! ```ignore
//! use ingestion::{Ingester, IngestOptions};
//! use storage::{Catalog, ObjectStorage, ObjectStorageConfig};
//! use std::sync::Arc;
//!
//! // Create storage and catalog connections
//! let storage = Arc::new(ObjectStorage::new(&config)?);
//! let catalog = Catalog::connect(&database_url).await?;
//!
//! // Create ingester
//! let ingester = Ingester::new(storage, catalog);
//!
//! // Ingest a file
//! let result = ingester.ingest_file(
//!     "/path/to/gfs.grib2",
//!     IngestOptions::default(),
//! ).await?;
//!
//! println!("Registered {} datasets", result.datasets_registered);
//! ```

pub mod config;
pub mod error;
mod grib2;
pub mod metadata;
mod netcdf;
mod upload;
mod ingester;

// Re-exports
pub use error::{IngestionError, Result};
pub use ingester::{Ingester, IngestOptions, IngestionResult};
pub use metadata::{
    detect_file_type, extract_forecast_hour, extract_model_from_filename, extract_mrms_param,
    get_bbox_from_grid, get_model_bbox, goes_band_to_parameter, parse_goes_filename, FileType,
    GoesFileInfo,
};
pub use config::{
    should_ingest_parameter, standard_pressure_levels, target_grib2_parameters, ParameterSpec,
};
