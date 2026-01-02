//! Conformance endpoint handler.

use axum::{
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use edr_protocol::ConformanceClasses;

use crate::content_negotiation::check_metadata_accept;

/// GET /edr/conformance - Conformance classes
pub async fn conformance_handler(headers: HeaderMap) -> Response {
    // Check Accept header - return 406 if unsupported format requested
    if let Err(response) = check_metadata_accept(&headers) {
        return response;
    }

    let conformance = ConformanceClasses::current();

    let json = serde_json::to_string_pretty(&conformance).unwrap_or_default();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "max-age=3600")
        .body(json.into())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use edr_protocol::{conformance, ConformanceClasses};

    #[test]
    fn test_conformance_classes() {
        let conf = ConformanceClasses::current();

        // Must include core, collections, position
        assert!(conf.contains(conformance::CORE));
        assert!(conf.contains(conformance::COLLECTIONS));
        assert!(conf.contains(conformance::POSITION));
    }

    #[test]
    fn test_conformance_json() {
        let conf = ConformanceClasses::current();
        let json = serde_json::to_string(&conf).unwrap();

        // Should contain conformsTo array
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let conforms_to = parsed.get("conformsTo").unwrap().as_array().unwrap();

        assert!(!conforms_to.is_empty());
        assert!(conforms_to
            .iter()
            .any(|v| v.as_str().unwrap().contains("core")));
    }
}
