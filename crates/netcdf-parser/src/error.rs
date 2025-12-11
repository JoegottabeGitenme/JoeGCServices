//! Error types for NetCDF parsing operations.

use thiserror::Error;

/// Result type for NetCDF parser operations.
pub type NetCdfResult<T> = Result<T, NetCdfError>;

/// Error types for NetCDF parsing.
#[derive(Error, Debug)]
pub enum NetCdfError {
    /// File I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Missing required variable or attribute
    #[error("Missing required data: {0}")]
    MissingData(String),

    /// Invalid data format
    #[error("Invalid data format: {0}")]
    InvalidFormat(String),

    /// Command execution error (for ncdump fallback)
    #[error("Command execution failed: {0}")]
    CommandError(String),
}
