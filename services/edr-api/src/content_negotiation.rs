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
    "application/geo+json",
    "application/json", // We can return CoverageJSON as JSON
];

/// Supported media types for metadata queries (collections, landing, etc.)
pub const METADATA_MEDIA_TYPES: &[&str] = &["application/json"];

/// Output format for EDR data query responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// CoverageJSON format (default)
    #[default]
    CoverageJson,
    /// GeoJSON format
    GeoJson,
}

impl OutputFormat {
    /// Get the Content-Type header value for this format.
    pub fn content_type(&self) -> &'static str {
        match self {
            OutputFormat::CoverageJson => "application/vnd.cov+json",
            OutputFormat::GeoJson => "application/geo+json",
        }
    }

    /// Parse format from the `f` query parameter value.
    /// Supports various aliases for convenience.
    pub fn from_query_param(f: &str) -> Option<Self> {
        match f.to_lowercase().as_str() {
            "covjson" | "coveragejson" | "application/vnd.cov+json" => {
                Some(OutputFormat::CoverageJson)
            }
            "geojson" | "geo+json" | "application/geo+json" => Some(OutputFormat::GeoJson),
            "json" | "application/json" => {
                // Default to CoverageJSON for generic JSON request
                Some(OutputFormat::CoverageJson)
            }
            _ => None,
        }
    }

    /// Parse format from Accept header media type.
    pub fn from_media_type(media_type: &str) -> Option<Self> {
        match media_type {
            "application/vnd.cov+json" | "application/prs.coverage+json" => {
                Some(OutputFormat::CoverageJson)
            }
            "application/geo+json" => Some(OutputFormat::GeoJson),
            "application/json" => {
                // Default to CoverageJSON for generic JSON
                Some(OutputFormat::CoverageJson)
            }
            _ => None,
        }
    }
}

/// Negotiate the output format based on the `f` query parameter and Accept header.
///
/// Priority:
/// 1. If `f` query parameter is provided (and non-empty), use it (explicit format request)
/// 2. Otherwise, check Accept header for preferred format
/// 3. Default to CoverageJSON if no preference is specified
///
/// Returns an error Response if the requested format is not supported.
pub fn negotiate_format(
    headers: &HeaderMap,
    f_param: Option<&str>,
) -> Result<OutputFormat, Response> {
    // First, check the `f` query parameter (highest priority)
    // Treat empty string as no parameter (OGC test suite sends f= with empty value)
    if let Some(f) = f_param {
        if f.is_empty() {
            // Empty f= parameter is treated as "use default"
            // Fall through to Accept header negotiation
        } else if let Some(format) = OutputFormat::from_query_param(f) {
            return Ok(format);
        } else {
            // Invalid format parameter
            return Err(invalid_format_response(f));
        }
    }

    // Next, check Accept header
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("*/*");

    // Parse Accept header with quality values
    let mut accepted_types: Vec<(&str, f32)> = accept
        .split(',')
        .filter_map(|s| {
            let parts: Vec<&str> = s.split(';').collect();
            let media_type = parts.first()?.trim();
            if media_type.is_empty() {
                return None;
            }

            // Parse quality value (default 1.0)
            let quality = parts
                .iter()
                .find_map(|p| {
                    let p = p.trim();
                    if p.starts_with("q=") {
                        p[2..].parse::<f32>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(1.0);

            Some((media_type, quality))
        })
        .collect();

    // Sort by quality (highest first)
    accepted_types.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Find the first matching format
    for (media_type, _) in &accepted_types {
        // Handle wildcards - default to CoverageJSON
        if *media_type == "*/*" || *media_type == "application/*" {
            return Ok(OutputFormat::CoverageJson);
        }

        if let Some(format) = OutputFormat::from_media_type(media_type) {
            return Ok(format);
        }
    }

    // Check if any type is acceptable (no Accept header or only wildcards)
    if accepted_types.is_empty() {
        return Ok(OutputFormat::CoverageJson);
    }

    // No acceptable format found - return 406
    let requested: Vec<&str> = accepted_types.iter().map(|(t, _)| *t).collect();
    Err(not_acceptable_response(&requested, DATA_QUERY_MEDIA_TYPES))
}

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

/// Create a 400 Bad Request response for invalid format parameter
fn invalid_format_response(format: &str) -> Response {
    let exc = ExceptionResponse::new(
        "http://www.opengis.net/def/exceptions/ogcapi-edr-1/1.0/invalid-parameter-value",
        400,
        format!(
            "Invalid output format '{}'. Supported formats: CoverageJSON, GeoJSON",
            format
        ),
    )
    .with_title("Bad Request");

    let json = serde_json::to_string(&exc).unwrap_or_default();

    Response::builder()
        .status(StatusCode::BAD_REQUEST)
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
    fn test_accept_geojson() {
        let headers = make_headers("application/geo+json");
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

    // Tests for OutputFormat
    #[test]
    fn test_output_format_from_query_param() {
        assert_eq!(
            OutputFormat::from_query_param("CoverageJSON"),
            Some(OutputFormat::CoverageJson)
        );
        assert_eq!(
            OutputFormat::from_query_param("covjson"),
            Some(OutputFormat::CoverageJson)
        );
        assert_eq!(
            OutputFormat::from_query_param("GeoJSON"),
            Some(OutputFormat::GeoJson)
        );
        assert_eq!(
            OutputFormat::from_query_param("geojson"),
            Some(OutputFormat::GeoJson)
        );
        assert_eq!(
            OutputFormat::from_query_param("json"),
            Some(OutputFormat::CoverageJson)
        );
        assert_eq!(OutputFormat::from_query_param("xml"), None);
    }

    #[test]
    fn test_output_format_content_type() {
        assert_eq!(
            OutputFormat::CoverageJson.content_type(),
            "application/vnd.cov+json"
        );
        assert_eq!(OutputFormat::GeoJson.content_type(), "application/geo+json");
    }

    // Tests for negotiate_format
    #[test]
    fn test_negotiate_format_with_f_param() {
        let headers = HeaderMap::new();

        // f parameter takes priority
        assert_eq!(
            negotiate_format(&headers, Some("geojson")).unwrap(),
            OutputFormat::GeoJson
        );
        assert_eq!(
            negotiate_format(&headers, Some("CoverageJSON")).unwrap(),
            OutputFormat::CoverageJson
        );
    }

    #[test]
    fn test_negotiate_format_invalid_f_param() {
        let headers = HeaderMap::new();
        assert!(negotiate_format(&headers, Some("xml")).is_err());
    }

    #[test]
    fn test_negotiate_format_empty_f_param() {
        // Empty f= parameter should be treated as "use default"
        // OGC test suite sends f= with empty value
        let headers = HeaderMap::new();
        assert_eq!(
            negotiate_format(&headers, Some("")).unwrap(),
            OutputFormat::CoverageJson
        );

        // With Accept header, empty f= should use Accept header preference
        let headers = make_headers("application/geo+json");
        assert_eq!(
            negotiate_format(&headers, Some("")).unwrap(),
            OutputFormat::GeoJson
        );
    }

    #[test]
    fn test_negotiate_format_from_accept_header() {
        // GeoJSON preferred
        let headers = make_headers("application/geo+json");
        assert_eq!(
            negotiate_format(&headers, None).unwrap(),
            OutputFormat::GeoJson
        );

        // CoverageJSON preferred
        let headers = make_headers("application/vnd.cov+json");
        assert_eq!(
            negotiate_format(&headers, None).unwrap(),
            OutputFormat::CoverageJson
        );
    }

    #[test]
    fn test_negotiate_format_with_quality() {
        // GeoJSON has higher quality than CoverageJSON
        let headers = make_headers("application/vnd.cov+json;q=0.5, application/geo+json;q=0.9");
        assert_eq!(
            negotiate_format(&headers, None).unwrap(),
            OutputFormat::GeoJson
        );

        // CoverageJSON has higher quality
        let headers = make_headers("application/geo+json;q=0.5, application/vnd.cov+json;q=0.9");
        assert_eq!(
            negotiate_format(&headers, None).unwrap(),
            OutputFormat::CoverageJson
        );
    }

    #[test]
    fn test_negotiate_format_default() {
        // No Accept header - default to CoverageJSON
        let headers = HeaderMap::new();
        assert_eq!(
            negotiate_format(&headers, None).unwrap(),
            OutputFormat::CoverageJson
        );

        // Wildcard - default to CoverageJSON
        let headers = make_headers("*/*");
        assert_eq!(
            negotiate_format(&headers, None).unwrap(),
            OutputFormat::CoverageJson
        );
    }

    #[test]
    fn test_negotiate_format_not_acceptable() {
        let headers = make_headers("text/html");
        assert!(negotiate_format(&headers, None).is_err());
    }
}
