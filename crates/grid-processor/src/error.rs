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
    OutOfBounds {
        requested: String,
        grid: String,
    },

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
