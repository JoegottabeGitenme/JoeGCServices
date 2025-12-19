//! Error types for the ingestion crate.

use thiserror::Error;

/// Errors that can occur during ingestion.
#[derive(Error, Debug)]
pub enum IngestionError {
    #[error("Failed to read file: {0}")]
    FileRead(#[from] std::io::Error),

    #[error("Failed to parse GRIB2 data: {0}")]
    Grib2Parse(String),

    #[error("Failed to parse NetCDF data: {0}")]
    NetcdfParse(String),

    #[error("Failed to write Zarr data: {0}")]
    ZarrWrite(String),

    #[error("Failed to upload to storage: {0}")]
    StorageUpload(String),

    #[error("Failed to register in catalog: {0}")]
    CatalogRegister(String),

    #[error("Unknown file type: {0}")]
    UnknownFileType(String),

    #[error("Missing required metadata: {0}")]
    MissingMetadata(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Decompression failed: {0}")]
    Decompression(String),

    #[error("Projection error: {0}")]
    Projection(String),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// Result type for ingestion operations.
pub type Result<T> = std::result::Result<T, IngestionError>;
