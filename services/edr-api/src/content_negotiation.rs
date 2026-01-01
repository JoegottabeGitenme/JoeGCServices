//! Content negotiation utilities for Accept header handling.
//!
//! Per OGC EDR spec and RFC 7231, the server should respect Accept headers
//! and return 406 Not Acceptable if the requested format is not supported.

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use edr_protocol::responses::ExceptionResponse;

/// Supported media types for data queries (position, area, etc.)
pub const DATA_QUERY_MEDIA_TYPES: &[&str] = &[
    "application/vnd.cov+json",
    "application/prs.coverage+json",
    "application/json", // We can return CoverageJSON as JSON
];

/// Supported media types for metadata queries (collections, landing, etc.)
pub const METADATA_MEDIA_TYPES: &[&str] = &["application/json"];

/// Check if the Accept header is compatible with the supported media types.
/// Returns Ok(()) if compatible, or an error Response if not.
pub fn check_accept_header(headers: &HeaderMap, supported_types: &[&str]) -> Result<(), Response> {
    // Get Accept header, default to */* if not present
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("*/*");

    // Parse Accept header - it can have multiple types separated by commas
    // Each type can have quality values like "application/json;q=0.9"
    let accepted_types: Vec<&str> = accept
        .split(',')
        .map(|s| s.split(';').next().unwrap_or("").trim())
        .filter(|s| !s.is_empty())
        .collect();

    // Check if any accepted type matches our supported types
    for accepted in &accepted_types {
        // Handle wildcards
        if *accepted == "*/*" {
            return Ok(());
        }

        // Handle type/* wildcards (e.g., "application/*")
        if accepted.ends_with("/*") {
            let prefix = &accepted[..accepted.len() - 1]; // "application/"
            for supported in supported_types {
                if supported.starts_with(prefix) {
                    return Ok(());
                }
            }
            continue;
        }

        // Exact match only (no partial matching)
        for supported in supported_types {
            if *accepted == *supported {
                return Ok(());
            }
        }
    }

    // No acceptable format found - return 406
    Err(not_acceptable_response(&accepted_types, supported_types))
}

/// Create a 406 Not Acceptable response
fn not_acceptable_response(requested: &[&str], supported: &[&str]) -> Response {
    let exc = ExceptionResponse::new(
        "http://www.opengis.net/def/exceptions/ogcapi-edr-1/1.0/invalid-parameter-value",
        406,
        format!(
            "Content negotiation failed. Requested format(s) '{}' not supported. Supported formats: {}",
            requested.join(", "),
            supported.iter().filter(|s| **s != "*/*").cloned().collect::<Vec<_>>().join(", ")
        ),
    )
    .with_title("Not Acceptable");

    let json = serde_json::to_string(&exc).unwrap_or_default();

    Response::builder()
        .status(StatusCode::NOT_ACCEPTABLE)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json.into())
        .unwrap()
}

/// Helper to check Accept header for data queries
pub fn check_data_query_accept(headers: &HeaderMap) -> Result<(), Response> {
    check_accept_header(headers, DATA_QUERY_MEDIA_TYPES)
}

/// Helper to check Accept header for metadata queries
pub fn check_metadata_accept(headers: &HeaderMap) -> Result<(), Response> {
    check_accept_header(headers, METADATA_MEDIA_TYPES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn make_headers(accept: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_str(accept).unwrap());
        headers
    }

    #[test]
    fn test_accept_covjson() {
        let headers = make_headers("application/vnd.cov+json");
        assert!(check_data_query_accept(&headers).is_ok());
    }

    #[test]
    fn test_accept_prs_coverage_json() {
        let headers = make_headers("application/prs.coverage+json");
        assert!(check_data_query_accept(&headers).is_ok());
    }

    #[test]
    fn test_accept_json() {
        let headers = make_headers("application/json");
        assert!(check_data_query_accept(&headers).is_ok());
        assert!(check_metadata_accept(&headers).is_ok());
    }

    #[test]
    fn test_accept_wildcard() {
        let headers = make_headers("*/*");
        assert!(check_data_query_accept(&headers).is_ok());
        assert!(check_metadata_accept(&headers).is_ok());
    }

    #[test]
    fn test_accept_application_wildcard() {
        let headers = make_headers("application/*");
        assert!(check_data_query_accept(&headers).is_ok());
        assert!(check_metadata_accept(&headers).is_ok());
    }

    #[test]
    fn test_accept_no_header() {
        let headers = HeaderMap::new();
        // No Accept header defaults to */*
        assert!(check_data_query_accept(&headers).is_ok());
        assert!(check_metadata_accept(&headers).is_ok());
    }

    #[test]
    fn test_accept_multiple_types() {
        let headers = make_headers("text/html, application/json, */*;q=0.1");
        assert!(check_data_query_accept(&headers).is_ok());
    }

    #[test]
    fn test_accept_with_quality() {
        let headers = make_headers("application/vnd.cov+json;q=0.9");
        assert!(check_data_query_accept(&headers).is_ok());
    }

    #[test]
    fn test_reject_unsupported_type() {
        let headers = make_headers("text/html");
        assert!(check_data_query_accept(&headers).is_err());
        assert!(check_metadata_accept(&headers).is_err());
    }

    #[test]
    fn test_reject_geojson() {
        // GeoJSON is not supported for EDR data queries
        let headers = make_headers("application/geo+json");
        assert!(check_data_query_accept(&headers).is_err());
    }

    #[test]
    fn test_reject_xml() {
        let headers = make_headers("application/xml");
        assert!(check_data_query_accept(&headers).is_err());
        assert!(check_metadata_accept(&headers).is_err());
    }

    #[test]
    fn test_reject_text_wildcard() {
        // text/* should not match application/* types
        let headers = make_headers("text/*");
        assert!(check_data_query_accept(&headers).is_err());
        assert!(check_metadata_accept(&headers).is_err());
    }

    #[test]
    fn test_browser_default_accept() {
        // Typical browser Accept header includes */* so should pass
        let headers =
            make_headers("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8");
        assert!(check_data_query_accept(&headers).is_ok());
    }

    #[test]
    fn test_strict_no_wildcard() {
        // Only unsupported types without wildcard should fail
        let headers = make_headers("text/html, application/xml");
        assert!(check_data_query_accept(&headers).is_err());
    }
}
