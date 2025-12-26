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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_messages() {
        let err = IngestionError::Grib2Parse("invalid magic bytes".to_string());
        assert_eq!(
            err.to_string(),
            "Failed to parse GRIB2 data: invalid magic bytes"
        );

        let err = IngestionError::NetcdfParse("missing variable".to_string());
        assert_eq!(
            err.to_string(),
            "Failed to parse NetCDF data: missing variable"
        );

        let err = IngestionError::ZarrWrite("chunk write failed".to_string());
        assert_eq!(
            err.to_string(),
            "Failed to write Zarr data: chunk write failed"
        );

        let err = IngestionError::StorageUpload("connection refused".to_string());
        assert_eq!(
            err.to_string(),
            "Failed to upload to storage: connection refused"
        );

        let err = IngestionError::CatalogRegister("duplicate entry".to_string());
        assert_eq!(
            err.to_string(),
            "Failed to register in catalog: duplicate entry"
        );

        let err = IngestionError::UnknownFileType(".xyz".to_string());
        assert_eq!(err.to_string(), "Unknown file type: .xyz");

        let err = IngestionError::MissingMetadata("reference_time".to_string());
        assert_eq!(err.to_string(), "Missing required metadata: reference_time");

        let err = IngestionError::InvalidConfig("invalid bucket name".to_string());
        assert_eq!(
            err.to_string(),
            "Invalid configuration: invalid bucket name"
        );

        let err = IngestionError::Decompression("corrupted gzip".to_string());
        assert_eq!(err.to_string(), "Decompression failed: corrupted gzip");

        let err = IngestionError::Projection("unsupported projection".to_string());
        assert_eq!(err.to_string(), "Projection error: unsupported projection");
    }

    #[test]
    fn test_error_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: IngestionError = io_err.into();

        assert!(matches!(err, IngestionError::FileRead(_)));
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn test_error_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("something went wrong");
        let err: IngestionError = anyhow_err.into();

        assert!(matches!(err, IngestionError::Other(_)));
        assert!(err.to_string().contains("something went wrong"));
    }

    #[test]
    fn test_error_debug_impl() {
        let err = IngestionError::Grib2Parse("test".to_string());
        let debug_str = format!("{:?}", err);

        assert!(debug_str.contains("Grib2Parse"));
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn test_result_type() {
        fn returns_ok() -> Result<i32> {
            Ok(42)
        }

        fn returns_err() -> Result<i32> {
            Err(IngestionError::InvalidConfig("test".to_string()))
        }

        assert_eq!(returns_ok().unwrap(), 42);
        assert!(returns_err().is_err());
    }
}
