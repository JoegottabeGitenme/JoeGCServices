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
//! # Parameter Filtering
//!
//! Which parameters are ingested is controlled by each model's YAML config file
//! in `config/models/`. See [`tables::IngestionFilter`] for details.

pub mod error;
mod grib2;
mod ingester;
pub mod metadata;
mod netcdf;
pub mod tables;
mod upload;

// Re-exports
pub use error::{IngestionError, Result};
pub use ingester::{IngestOptions, Ingester, IngestionResult};
pub use metadata::{
    detect_file_type, extract_forecast_hour, extract_model_from_filename, extract_mrms_param,
    get_bbox_from_grid, get_model_bbox, goes_band_to_parameter, parse_goes_filename, FileType,
    GoesFileInfo,
};
pub use tables::{
    build_filter_for_model, build_tables_for_model, build_tables_from_configs, IngestionFilter,
    LevelFilter, ValidRange,
};
