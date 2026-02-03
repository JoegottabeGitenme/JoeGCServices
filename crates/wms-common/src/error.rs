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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // WMS Exception Code Tests
    // =========================================================================

    #[test]
    fn test_exception_code_missing_parameter() {
        let err = WmsError::MissingParameter("LAYERS".to_string());
        assert_eq!(err.wms_exception_code(), "MissingParameterValue");
    }

    #[test]
    fn test_exception_code_invalid_parameter() {
        let err = WmsError::InvalidParameter {
            param: "WIDTH".to_string(),
            message: "must be positive".to_string(),
        };
        assert_eq!(err.wms_exception_code(), "InvalidParameterValue");
    }

    #[test]
    fn test_exception_code_layer_not_found() {
        let err = WmsError::LayerNotFound("unknown:layer".to_string());
        assert_eq!(err.wms_exception_code(), "LayerNotDefined");
    }

    #[test]
    fn test_exception_code_style_not_found() {
        let err = WmsError::StyleNotFound("nonexistent".to_string());
        assert_eq!(err.wms_exception_code(), "StyleNotDefined");
    }

    #[test]
    fn test_exception_code_invalid_crs() {
        let err = WmsError::InvalidCrs("EPSG:99999".to_string());
        assert_eq!(err.wms_exception_code(), "InvalidCRS");
    }

    #[test]
    fn test_exception_code_invalid_bbox() {
        let err = WmsError::InvalidBbox("malformed".to_string());
        assert_eq!(err.wms_exception_code(), "InvalidBBox");
    }

    #[test]
    fn test_exception_code_invalid_time() {
        let err = WmsError::InvalidTime("not-a-date".to_string());
        assert_eq!(err.wms_exception_code(), "InvalidDimensionValue");
    }

    #[test]
    fn test_exception_code_unsupported_format() {
        let err = WmsError::UnsupportedFormat("image/bmp".to_string());
        assert_eq!(err.wms_exception_code(), "InvalidFormat");
    }

    #[test]
    fn test_exception_code_data_not_available() {
        let err = WmsError::DataNotAvailable("2024-01-01T00:00:00Z".to_string());
        assert_eq!(err.wms_exception_code(), "MissingDimensionValue");
    }

    #[test]
    fn test_exception_code_fallback() {
        // All other errors should return NoApplicableCode
        let errors = vec![
            WmsError::DataReadError("read failed".to_string()),
            WmsError::Grib2Error("invalid grib".to_string()),
            WmsError::NetCdfError("invalid netcdf".to_string()),
            WmsError::StorageError("storage failed".to_string()),
            WmsError::DatabaseError("db failed".to_string()),
            WmsError::CacheError("cache failed".to_string()),
            WmsError::RenderError("render failed".to_string()),
            WmsError::ProjectionError("proj failed".to_string()),
            WmsError::InternalError("internal".to_string()),
            WmsError::ServiceUnavailable("unavailable".to_string()),
            WmsError::Timeout,
        ];

        for err in errors {
            assert_eq!(
                err.wms_exception_code(),
                "NoApplicableCode",
                "Expected NoApplicableCode for {:?}",
                err
            );
        }
    }

    // =========================================================================
    // HTTP Status Code Tests
    // =========================================================================

    #[test]
    fn test_http_status_400_bad_request() {
        let errors_400 = vec![
            WmsError::MissingParameter("param".to_string()),
            WmsError::InvalidParameter {
                param: "p".to_string(),
                message: "m".to_string(),
            },
            WmsError::InvalidCrs("crs".to_string()),
            WmsError::InvalidBbox("bbox".to_string()),
            WmsError::InvalidTime("time".to_string()),
            WmsError::UnsupportedFormat("format".to_string()),
        ];

        for err in errors_400 {
            assert_eq!(err.http_status_code(), 400, "Expected 400 for {:?}", err);
        }
    }

    #[test]
    fn test_http_status_404_not_found() {
        let errors_404 = vec![
            WmsError::LayerNotFound("layer".to_string()),
            WmsError::StyleNotFound("style".to_string()),
            WmsError::DataNotAvailable("time".to_string()),
        ];

        for err in errors_404 {
            assert_eq!(err.http_status_code(), 404, "Expected 404 for {:?}", err);
        }
    }

    #[test]
    fn test_http_status_500_internal() {
        let errors_500 = vec![
            WmsError::DataReadError("read".to_string()),
            WmsError::Grib2Error("grib".to_string()),
            WmsError::NetCdfError("netcdf".to_string()),
            WmsError::StorageError("storage".to_string()),
            WmsError::DatabaseError("db".to_string()),
            WmsError::CacheError("cache".to_string()),
            WmsError::RenderError("render".to_string()),
            WmsError::ProjectionError("proj".to_string()),
            WmsError::InternalError("internal".to_string()),
        ];

        for err in errors_500 {
            assert_eq!(err.http_status_code(), 500, "Expected 500 for {:?}", err);
        }
    }

    #[test]
    fn test_http_status_503_unavailable() {
        let err = WmsError::ServiceUnavailable("maintenance".to_string());
        assert_eq!(err.http_status_code(), 503);
    }

    #[test]
    fn test_http_status_504_timeout() {
        let err = WmsError::Timeout;
        assert_eq!(err.http_status_code(), 504);
    }

    // =========================================================================
    // Error Conversion Tests
    // =========================================================================

    #[test]
    fn test_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let wms_err: WmsError = io_err.into();

        match wms_err {
            WmsError::InternalError(msg) => {
                assert!(msg.contains("file not found"));
            }
            _ => panic!("Expected InternalError"),
        }
    }

    #[test]
    fn test_from_serde_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let wms_err: WmsError = json_err.into();

        match wms_err {
            WmsError::InternalError(msg) => {
                assert!(msg.contains("JSON error"));
            }
            _ => panic!("Expected InternalError"),
        }
    }

    // =========================================================================
    // Error Display Tests
    // =========================================================================

    #[test]
    fn test_error_display_missing_parameter() {
        let err = WmsError::MissingParameter("LAYERS".to_string());
        assert_eq!(format!("{}", err), "Missing required parameter: LAYERS");
    }

    #[test]
    fn test_error_display_invalid_parameter() {
        let err = WmsError::InvalidParameter {
            param: "WIDTH".to_string(),
            message: "must be positive".to_string(),
        };
        assert_eq!(
            format!("{}", err),
            "Invalid parameter value for 'WIDTH': must be positive"
        );
    }

    #[test]
    fn test_error_display_timeout() {
        let err = WmsError::Timeout;
        assert_eq!(format!("{}", err), "Request timeout");
    }
}
