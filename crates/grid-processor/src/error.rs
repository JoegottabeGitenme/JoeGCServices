//! Error types for grid processing.

use thiserror::Error;

/// Errors that can occur during grid processing.
#[derive(Error, Debug)]
pub enum GridProcessorError {
    /// Failed to open the grid data source.
    #[error("failed to open grid: {0}")]
    OpenFailed(String),

    /// Failed to read data from the grid.
    #[error("failed to read grid data: {0}")]
    ReadFailed(String),

    /// The requested region is outside the grid bounds.
    #[error("requested region {requested:?} is outside grid bounds {grid:?}")]
    OutOfBounds { requested: String, grid: String },

    /// Invalid metadata in the grid file.
    #[error("invalid grid metadata: {0}")]
    InvalidMetadata(String),

    /// Zarr format error.
    #[error("Zarr format error: {0}")]
    ZarrError(String),

    /// Storage/IO error.
    #[error("storage error: {0}")]
    StorageError(String),

    /// Decompression error.
    #[error("decompression error: {0}")]
    DecompressionError(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    ConfigError(String),

    /// Cache error.
    #[error("cache error: {0}")]
    CacheError(String),

    /// Projection error.
    #[error("projection error: {0}")]
    ProjectionError(String),

    /// Interpolation error.
    #[error("interpolation error: {0}")]
    InterpolationError(String),

    /// Storage connection error.
    #[error("storage connection error: {0}")]
    Storage(String),

    /// Metadata parsing error.
    #[error("metadata error: {0}")]
    Metadata(String),

    /// Catalog query error.
    #[error("catalog error: {0}")]
    Catalog(String),

    /// Dataset not found.
    #[error("dataset not found: {0}")]
    NotFound(String),

    /// Data unavailable — catalog entry exists but storage data is missing or expired.
    #[error("data unavailable: {0}")]
    DataUnavailable(String),
}

impl GridProcessorError {
    /// Create an OpenFailed error.
    pub fn open_failed(msg: impl Into<String>) -> Self {
        Self::OpenFailed(msg.into())
    }

    /// Create a ReadFailed error.
    pub fn read_failed(msg: impl Into<String>) -> Self {
        Self::ReadFailed(msg.into())
    }

    /// Create an OutOfBounds error.
    pub fn out_of_bounds(requested: impl Into<String>, grid: impl Into<String>) -> Self {
        Self::OutOfBounds {
            requested: requested.into(),
            grid: grid.into(),
        }
    }

    /// Create an InvalidMetadata error.
    pub fn invalid_metadata(msg: impl Into<String>) -> Self {
        Self::InvalidMetadata(msg.into())
    }

    /// Create a ZarrError.
    pub fn zarr_error(msg: impl Into<String>) -> Self {
        Self::ZarrError(msg.into())
    }

    /// Create a StorageError.
    pub fn storage_error(msg: impl Into<String>) -> Self {
        Self::StorageError(msg.into())
    }

    /// Create a DataUnavailable error.
    pub fn data_unavailable(msg: impl Into<String>) -> Self {
        Self::DataUnavailable(msg.into())
    }

    /// Check if this error indicates data is unavailable (missing from storage).
    pub fn is_data_unavailable(&self) -> bool {
        matches!(self, Self::DataUnavailable(_))
    }
}

impl From<std::io::Error> for GridProcessorError {
    fn from(err: std::io::Error) -> Self {
        Self::StorageError(err.to_string())
    }
}

impl From<serde_json::Error> for GridProcessorError {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidMetadata(err.to_string())
    }
}

/// Result type for grid processor operations.
pub type Result<T> = std::result::Result<T, GridProcessorError>;

#[cfg(test)]
mod tests {
    use super::*;

    // Test error display messages
    #[test]
    fn test_error_display_open_failed() {
        let err = GridProcessorError::OpenFailed("connection timeout".to_string());
        assert_eq!(err.to_string(), "failed to open grid: connection timeout");
    }

    #[test]
    fn test_error_display_read_failed() {
        let err = GridProcessorError::ReadFailed("chunk missing".to_string());
        assert_eq!(err.to_string(), "failed to read grid data: chunk missing");
    }

    #[test]
    fn test_error_display_out_of_bounds() {
        let err = GridProcessorError::OutOfBounds {
            requested: "[-180, -90, 180, 90]".to_string(),
            grid: "[-130, 20, -60, 55]".to_string(),
        };
        assert!(err.to_string().contains("outside grid bounds"));
        assert!(err.to_string().contains("[-180, -90, 180, 90]"));
    }

    #[test]
    fn test_error_display_invalid_metadata() {
        let err = GridProcessorError::InvalidMetadata("missing shape attribute".to_string());
        assert_eq!(
            err.to_string(),
            "invalid grid metadata: missing shape attribute"
        );
    }

    #[test]
    fn test_error_display_zarr_error() {
        let err = GridProcessorError::ZarrError("invalid chunk dimensions".to_string());
        assert_eq!(
            err.to_string(),
            "Zarr format error: invalid chunk dimensions"
        );
    }

    #[test]
    fn test_error_display_storage_error() {
        let err = GridProcessorError::StorageError("bucket not found".to_string());
        assert_eq!(err.to_string(), "storage error: bucket not found");
    }

    #[test]
    fn test_error_display_decompression() {
        let err = GridProcessorError::DecompressionError("corrupted data".to_string());
        assert_eq!(err.to_string(), "decompression error: corrupted data");
    }

    #[test]
    fn test_error_display_config() {
        let err = GridProcessorError::ConfigError("invalid chunk size".to_string());
        assert_eq!(err.to_string(), "configuration error: invalid chunk size");
    }

    #[test]
    fn test_error_display_cache() {
        let err = GridProcessorError::CacheError("cache full".to_string());
        assert_eq!(err.to_string(), "cache error: cache full");
    }

    #[test]
    fn test_error_display_projection() {
        let err = GridProcessorError::ProjectionError("unsupported CRS".to_string());
        assert_eq!(err.to_string(), "projection error: unsupported CRS");
    }

    #[test]
    fn test_error_display_interpolation() {
        let err = GridProcessorError::InterpolationError("no valid neighbors".to_string());
        assert_eq!(err.to_string(), "interpolation error: no valid neighbors");
    }

    #[test]
    fn test_error_display_storage() {
        let err = GridProcessorError::Storage("MinIO unreachable".to_string());
        assert_eq!(
            err.to_string(),
            "storage connection error: MinIO unreachable"
        );
    }

    #[test]
    fn test_error_display_metadata() {
        let err = GridProcessorError::Metadata("invalid JSON".to_string());
        assert_eq!(err.to_string(), "metadata error: invalid JSON");
    }

    #[test]
    fn test_error_display_catalog() {
        let err = GridProcessorError::Catalog("query failed".to_string());
        assert_eq!(err.to_string(), "catalog error: query failed");
    }

    #[test]
    fn test_error_display_not_found() {
        let err = GridProcessorError::NotFound("gfs/TMP/2024-01-01".to_string());
        assert_eq!(err.to_string(), "dataset not found: gfs/TMP/2024-01-01");
    }

    #[test]
    fn test_error_display_data_unavailable() {
        let err = GridProcessorError::DataUnavailable("data expired".to_string());
        assert_eq!(err.to_string(), "data unavailable: data expired");
    }

    // Test helper constructors
    #[test]
    fn test_open_failed_helper() {
        let err = GridProcessorError::open_failed("test error");
        assert!(matches!(err, GridProcessorError::OpenFailed(_)));
        assert!(err.to_string().contains("test error"));
    }

    #[test]
    fn test_read_failed_helper() {
        let err = GridProcessorError::read_failed("chunk not found");
        assert!(matches!(err, GridProcessorError::ReadFailed(_)));
        assert!(err.to_string().contains("chunk not found"));
    }

    #[test]
    fn test_out_of_bounds_helper() {
        let err = GridProcessorError::out_of_bounds("request", "grid");
        assert!(matches!(err, GridProcessorError::OutOfBounds { .. }));
        assert!(err.to_string().contains("request"));
        assert!(err.to_string().contains("grid"));
    }

    #[test]
    fn test_invalid_metadata_helper() {
        let err = GridProcessorError::invalid_metadata("bad shape");
        assert!(matches!(err, GridProcessorError::InvalidMetadata(_)));
        assert!(err.to_string().contains("bad shape"));
    }

    #[test]
    fn test_zarr_error_helper() {
        let err = GridProcessorError::zarr_error("invalid codec");
        assert!(matches!(err, GridProcessorError::ZarrError(_)));
        assert!(err.to_string().contains("invalid codec"));
    }

    #[test]
    fn test_storage_error_helper() {
        let err = GridProcessorError::storage_error("io failure");
        assert!(matches!(err, GridProcessorError::StorageError(_)));
        assert!(err.to_string().contains("io failure"));
    }

    #[test]
    fn test_data_unavailable_helper() {
        let err = GridProcessorError::data_unavailable("no data at path");
        assert!(matches!(err, GridProcessorError::DataUnavailable(_)));
        assert!(err.to_string().contains("no data at path"));
    }

    #[test]
    fn test_is_data_unavailable() {
        let unavailable = GridProcessorError::data_unavailable("expired");
        assert!(unavailable.is_data_unavailable());

        let not_found = GridProcessorError::NotFound("missing".to_string());
        assert!(!not_found.is_data_unavailable());

        let open_failed = GridProcessorError::open_failed("io error");
        assert!(!open_failed.is_data_unavailable());
    }

    // Test From implementations
    #[test]
    fn test_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: GridProcessorError = io_err.into();
        assert!(matches!(err, GridProcessorError::StorageError(_)));
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn test_from_serde_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let err: GridProcessorError = json_err.into();
        assert!(matches!(err, GridProcessorError::InvalidMetadata(_)));
    }

    // Test Debug impl
    #[test]
    fn test_error_debug() {
        let err = GridProcessorError::NotFound("test".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("NotFound"));
        assert!(debug_str.contains("test"));
    }

    // Test Result type alias
    #[test]
    fn test_result_type_ok() {
        fn returns_ok() -> Result<i32> {
            Ok(42)
        }
        assert_eq!(returns_ok().unwrap(), 42);
    }

    #[test]
    fn test_result_type_err() {
        fn returns_err() -> Result<i32> {
            Err(GridProcessorError::NotFound("missing".to_string()))
        }
        assert!(returns_err().is_err());
    }

    // Test helper constructors accept various string types
    #[test]
    fn test_helpers_accept_string_types() {
        // String
        let _ = GridProcessorError::open_failed(String::from("owned"));
        // &str
        let _ = GridProcessorError::open_failed("borrowed");
        // &String
        let s = String::from("ref");
        let _ = GridProcessorError::open_failed(&s);
    }
}
