//! Error types for weather-wms services.

use thiserror::Error;

/// Result type alias using WmsError.
pub type WmsResult<T> = Result<T, WmsError>;

/// Primary error type for WMS operations.
#[derive(Debug, Error)]
pub enum WmsError {
    // === WMS Protocol Errors ===
    #[error("Missing required parameter: {0}")]
    MissingParameter(String),

    #[error("Invalid parameter value for '{param}': {message}")]
    InvalidParameter { param: String, message: String },

    #[error("Layer not found: {0}")]
    LayerNotFound(String),

    #[error("Style not found: {0}")]
    StyleNotFound(String),

    #[error("Invalid CRS: {0}")]
    InvalidCrs(String),

    #[error("Invalid BBOX: {0}")]
    InvalidBbox(String),

    #[error("Invalid time specification: {0}")]
    InvalidTime(String),

    #[error("Requested format not supported: {0}")]
    UnsupportedFormat(String),

    // === Data Errors ===
    #[error("Data not available for time: {0}")]
    DataNotAvailable(String),

    #[error("Failed to read data: {0}")]
    DataReadError(String),

    #[error("Invalid GRIB2 data: {0}")]
    Grib2Error(String),

    #[error("Invalid NetCDF data: {0}")]
    NetCdfError(String),

    // === Storage Errors ===
    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Cache error: {0}")]
    CacheError(String),

    // === Rendering Errors ===
    #[error("Rendering failed: {0}")]
    RenderError(String),

    #[error("Projection error: {0}")]
    ProjectionError(String),

    // === Infrastructure Errors ===
    #[error("Internal server error: {0}")]
    InternalError(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Request timeout")]
    Timeout,
}

impl WmsError {
    /// Get the OGC WMS exception code for this error.
    pub fn wms_exception_code(&self) -> &'static str {
        match self {
            WmsError::MissingParameter(_) => "MissingParameterValue",
            WmsError::InvalidParameter { .. } => "InvalidParameterValue",
            WmsError::LayerNotFound(_) => "LayerNotDefined",
            WmsError::StyleNotFound(_) => "StyleNotDefined",
            WmsError::InvalidCrs(_) => "InvalidCRS",
            WmsError::InvalidBbox(_) => "InvalidBBox",
            WmsError::InvalidTime(_) => "InvalidDimensionValue",
            WmsError::UnsupportedFormat(_) => "InvalidFormat",
            WmsError::DataNotAvailable(_) => "MissingDimensionValue",
            _ => "NoApplicableCode",
        }
    }

    /// Get the HTTP status code for this error.
    pub fn http_status_code(&self) -> u16 {
        match self {
            WmsError::MissingParameter(_)
            | WmsError::InvalidParameter { .. }
            | WmsError::InvalidCrs(_)
            | WmsError::InvalidBbox(_)
            | WmsError::InvalidTime(_)
            | WmsError::UnsupportedFormat(_) => 400,
            
            WmsError::LayerNotFound(_)
            | WmsError::StyleNotFound(_)
            | WmsError::DataNotAvailable(_) => 404,
            
            WmsError::ServiceUnavailable(_) => 503,
            WmsError::Timeout => 504,
            
            _ => 500,
        }
    }
}

// Conversion from common error types
impl From<std::io::Error> for WmsError {
    fn from(err: std::io::Error) -> Self {
        WmsError::InternalError(err.to_string())
    }
}

impl From<serde_json::Error> for WmsError {
    fn from(err: serde_json::Error) -> Self {
        WmsError::InternalError(format!("JSON error: {}", err))
    }
}
