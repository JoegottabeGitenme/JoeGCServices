//! Landing page handler.

use axum::{
    extract::Extension,
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use edr_protocol::LandingPage;
use std::sync::Arc;

use crate::content_negotiation::check_metadata_accept;
use crate::state::AppState;

/// GET /edr - Landing page
pub async fn landing_handler(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    // Check Accept header - return 406 if unsupported format requested
    if let Err(response) = check_metadata_accept(&headers) {
        return response;
    }

    let landing = LandingPage::new(
        "Weather WMS EDR API",
        "OGC API - Environmental Data Retrieval for weather model data including HRRR, GFS, and more",
        &state.base_url,
    );

    let json = serde_json::to_string_pretty(&landing).unwrap_or_default();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "max-age=300")
        .body(json.into())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use edr_protocol::LandingPage;

    #[test]
    fn test_landing_page_structure() {
        let landing = LandingPage::new("Test API", "Test description", "http://localhost:8083/edr");

        // Verify required links
        assert!(landing.links.iter().any(|l| l.rel == "self"));
        assert!(landing.links.iter().any(|l| l.rel == "conformance"));
        assert!(landing.links.iter().any(|l| l.rel == "data"));
        assert!(landing.links.iter().any(|l| l.rel == "service-desc"));
    }

    #[test]
    fn test_landing_page_json() {
        let landing = LandingPage::new("Test API", "Test description", "http://localhost:8083/edr");

        let json = serde_json::to_string(&landing).unwrap();

        // Should be valid JSON with required fields
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("title").is_some());
        assert!(parsed.get("links").is_some());
    }
}
