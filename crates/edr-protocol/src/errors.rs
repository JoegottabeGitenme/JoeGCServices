//! EDR API error types.

use thiserror::Error;

use crate::queries::CoordinateParseError;
use crate::responses::ExceptionResponse;

/// Errors that can occur in EDR API operations.
#[derive(Debug, Error)]
pub enum EdrError {
    /// Collection not found.
    #[error("Collection not found: {0}")]
    CollectionNotFound(String),

    /// Instance not found.
    #[error("Instance not found: {0}")]
    InstanceNotFound(String),

    /// Parameter not found.
    #[error("Parameter not found: {0}")]
    ParameterNotFound(String),

    /// Invalid query parameter.
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// Coordinate parsing error.
    #[error("Coordinate error: {0}")]
    CoordinateError(#[from] CoordinateParseError),

    /// No data available for the query.
    #[error("No data available: {0}")]
    NoDataAvailable(String),

    /// Response would be too large.
    #[error("Response too large: {0}")]
    ResponseTooLarge(String),

    /// Unsupported query type.
    #[error("Unsupported query type: {0}")]
    UnsupportedQuery(String),

    /// Unsupported output format.
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    /// Unsupported CRS.
    #[error("Unsupported CRS: {0}")]
    UnsupportedCrs(String),

    /// Internal server error.
    #[error("Internal error: {0}")]
    InternalError(String),

    /// Data access error.
    #[error("Data access error: {0}")]
    DataAccessError(String),
}

impl EdrError {
    /// Get the HTTP status code for this error.
    pub fn status_code(&self) -> u16 {
        match self {
            EdrError::CollectionNotFound(_) => 404,
            EdrError::InstanceNotFound(_) => 404,
            EdrError::ParameterNotFound(_) => 404,
            EdrError::InvalidParameter(_) => 400,
            EdrError::CoordinateError(_) => 400,
            EdrError::NoDataAvailable(_) => 404,
            EdrError::ResponseTooLarge(_) => 413,
            EdrError::UnsupportedQuery(_) => 400,
            EdrError::UnsupportedFormat(_) => 400,
            EdrError::UnsupportedCrs(_) => 400,
            EdrError::InternalError(_) => 500,
            EdrError::DataAccessError(_) => 500,
        }
    }

    /// Convert to an ExceptionResponse.
    pub fn to_exception(&self) -> ExceptionResponse {
        match self {
            EdrError::CollectionNotFound(msg) => ExceptionResponse::not_found(msg),
            EdrError::InstanceNotFound(msg) => ExceptionResponse::not_found(msg),
            EdrError::ParameterNotFound(msg) => ExceptionResponse::not_found(msg),
            EdrError::NoDataAvailable(msg) => ExceptionResponse::not_found(msg),
            EdrError::InvalidParameter(msg) => ExceptionResponse::bad_request(msg),
            EdrError::CoordinateError(e) => ExceptionResponse::bad_request(e.to_string()),
            EdrError::UnsupportedQuery(msg) => ExceptionResponse::bad_request(msg),
            EdrError::UnsupportedFormat(msg) => ExceptionResponse::bad_request(msg),
            EdrError::UnsupportedCrs(msg) => ExceptionResponse::bad_request(msg),
            EdrError::ResponseTooLarge(msg) => ExceptionResponse::payload_too_large(msg),
            EdrError::InternalError(msg) => ExceptionResponse::internal_error(msg),
            EdrError::DataAccessError(msg) => ExceptionResponse::internal_error(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_status_codes() {
        assert_eq!(EdrError::CollectionNotFound("test".to_string()).status_code(), 404);
        assert_eq!(EdrError::InstanceNotFound("test".to_string()).status_code(), 404);
        assert_eq!(EdrError::InvalidParameter("test".to_string()).status_code(), 400);
        assert_eq!(EdrError::ResponseTooLarge("test".to_string()).status_code(), 413);
        assert_eq!(EdrError::InternalError("test".to_string()).status_code(), 500);
    }

    #[test]
    fn test_error_to_exception() {
        let err = EdrError::CollectionNotFound("missing-collection".to_string());
        let exc = err.to_exception();

        assert_eq!(exc.status, Some(404));
        assert!(exc.detail.unwrap().contains("missing-collection"));
    }

    #[test]
    fn test_coordinate_error_conversion() {
        let coord_err = CoordinateParseError::OutOfRange("Longitude out of range".to_string());
        let err: EdrError = coord_err.into();

        assert_eq!(err.status_code(), 400);
        
        let exc = err.to_exception();
        assert_eq!(exc.status, Some(400));
    }

    #[test]
    fn test_error_display() {
        let err = EdrError::CollectionNotFound("test-collection".to_string());
        let display = format!("{}", err);
        assert!(display.contains("Collection not found"));
        assert!(display.contains("test-collection"));
    }

    #[test]
    fn test_response_too_large_exception() {
        let err = EdrError::ResponseTooLarge("Estimated response size: 75MB (limit: 50MB)".to_string());
        let exc = err.to_exception();

        assert_eq!(exc.status, Some(413));
        assert!(exc.type_.contains("response-too-large"));
    }
}
