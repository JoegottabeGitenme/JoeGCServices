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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_netcdf_error_display() {
        let err = NetCdfError::MissingData("CMI variable".to_string());
        assert!(err.to_string().contains("CMI variable"));

        let err = NetCdfError::InvalidFormat("bad header".to_string());
        assert!(err.to_string().contains("bad header"));

        let err = NetCdfError::CommandError("ncdump failed".to_string());
        assert!(err.to_string().contains("ncdump failed"));
    }

    #[test]
    fn test_netcdf_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: NetCdfError = io_err.into();
        assert!(err.to_string().contains("file not found"));
    }
}
