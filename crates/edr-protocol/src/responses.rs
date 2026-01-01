//! EDR API response types.
//!
//! This module contains types for the landing page, conformance,
//! and other metadata responses.

use serde::{Deserialize, Serialize};

use crate::conformance;
use crate::types::Link;

/// Landing page response for the EDR API root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LandingPage {
    /// Title of the API.
    pub title: String,

    /// Description of the API.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Links to related resources.
    pub links: Vec<Link>,
}

impl LandingPage {
    /// Create a new landing page with standard links.
    pub fn new(title: impl Into<String>, description: impl Into<String>, base_url: &str) -> Self {
        let links = vec![
            Link::new(base_url, "self")
                .with_type("application/json")
                .with_title("This document"),
            Link::new(format!("{}/conformance", base_url), "conformance")
                .with_type("application/json")
                .with_title("Conformance classes"),
            Link::new(format!("{}/collections", base_url), "data")
                .with_type("application/json")
                .with_title("Collections"),
            Link::new(format!("{}/api", base_url), "service-desc")
                .with_type("application/vnd.oai.openapi+json;version=3.0")
                .with_title("OpenAPI definition"),
            Link::new(format!("{}/api.html", base_url), "service-doc")
                .with_type("text/html")
                .with_title("API documentation"),
        ];

        Self {
            title: title.into(),
            description: Some(description.into()),
            links,
        }
    }
}

/// Conformance declaration response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConformanceClasses {
    /// List of conformance class URIs.
    #[serde(rename = "conformsTo")]
    pub conforms_to: Vec<String>,
}

impl ConformanceClasses {
    /// Create conformance classes for the current implementation.
    ///
    /// Includes: Core, Collections, Position, Area, Radius, Trajectory, Corridor, Cube, Instances, CoverageJSON, GeoJSON
    pub fn current() -> Self {
        Self {
            conforms_to: vec![
                conformance::CORE.to_string(),
                conformance::COLLECTIONS.to_string(),
                conformance::POSITION.to_string(),
                conformance::AREA.to_string(),
                conformance::RADIUS.to_string(),
                conformance::TRAJECTORY.to_string(),
                conformance::CORRIDOR.to_string(),
                conformance::CUBE.to_string(),
                conformance::INSTANCES.to_string(),
                conformance::COVJSON.to_string(),
                // Note: conf/geojson requires the 'locations' query endpoint which returns
                // GeoJSON FeatureCollections for named locations. We don't implement this yet.
                // We DO support GeoJSON as an output format for data queries via f=geojson.
                // conformance::GEOJSON.to_string(),
            ],
        }
    }

    /// Create minimal conformance (Core only).
    pub fn minimal() -> Self {
        Self {
            conforms_to: vec![conformance::CORE.to_string()],
        }
    }

    /// Add a conformance class.
    pub fn with_class(mut self, class: &str) -> Self {
        if !self.conforms_to.contains(&class.to_string()) {
            self.conforms_to.push(class.to_string());
        }
        self
    }

    /// Check if a conformance class is declared.
    pub fn contains(&self, class: &str) -> bool {
        self.conforms_to.contains(&class.to_string())
    }
}

/// Exception response for errors.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExceptionResponse {
    /// Exception type identifier.
    #[serde(rename = "type")]
    pub type_: String,

    /// Human-readable title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// HTTP status code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,

    /// Detailed error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// URI of the request that caused the error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

impl ExceptionResponse {
    /// Create a new exception response.
    pub fn new(type_: impl Into<String>, status: u16, detail: impl Into<String>) -> Self {
        Self {
            type_: type_.into(),
            title: None,
            status: Some(status),
            detail: Some(detail.into()),
            instance: None,
        }
    }

    /// Set the title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the instance URI.
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    /// Create a 404 Not Found exception.
    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(
            "http://www.opengis.net/def/exceptions/ogcapi-edr-1/1.0/not-found",
            404,
            detail,
        )
        .with_title("Not Found")
    }

    /// Create a 400 Bad Request exception.
    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(
            "http://www.opengis.net/def/exceptions/ogcapi-edr-1/1.0/invalid-parameter-value",
            400,
            detail,
        )
        .with_title("Bad Request")
    }

    /// Create a 413 Payload Too Large exception.
    pub fn payload_too_large(detail: impl Into<String>) -> Self {
        Self::new(
            "http://www.opengis.net/def/exceptions/ogcapi-edr-1/1.0/response-too-large",
            413,
            detail,
        )
        .with_title("Payload Too Large")
    }

    /// Create a 500 Internal Server Error exception.
    pub fn internal_error(detail: impl Into<String>) -> Self {
        Self::new(
            "http://www.opengis.net/def/exceptions/ogcapi-edr-1/1.0/server-error",
            500,
            detail,
        )
        .with_title("Internal Server Error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_landing_page() {
        let landing = LandingPage::new(
            "Weather EDR API",
            "Environmental data retrieval for weather models",
            "http://localhost:8083/edr",
        );

        assert_eq!(landing.title, "Weather EDR API");
        assert!(landing.description.is_some());

        // Should have at least self, conformance, data, and service-desc links
        assert!(landing.links.len() >= 4);
        assert!(landing.links.iter().any(|l| l.rel == "self"));
        assert!(landing.links.iter().any(|l| l.rel == "conformance"));
        assert!(landing.links.iter().any(|l| l.rel == "data"));
        assert!(landing.links.iter().any(|l| l.rel == "service-desc"));
    }

    #[test]
    fn test_landing_page_serialization() {
        let landing = LandingPage::new("Test API", "Test description", "http://localhost:8083/edr");

        let json = serde_json::to_string_pretty(&landing).unwrap();
        // Pretty-printed JSON has spaces after colons
        assert!(
            json.contains("\"title\": \"Test API\"") || json.contains("\"title\":\"Test API\"")
        );
        assert!(json.contains("\"description\""));
        assert!(json.contains("\"links\""));
        assert!(json.contains("\"rel\": \"self\"") || json.contains("\"rel\":\"self\""));
    }

    #[test]
    fn test_conformance_current() {
        let conf = ConformanceClasses::current();

        assert!(conf.contains(conformance::CORE));
        assert!(conf.contains(conformance::COLLECTIONS));
        assert!(conf.contains(conformance::POSITION));
        assert!(conf.contains(conformance::AREA));
        assert!(conf.contains(conformance::RADIUS));
        assert!(conf.contains(conformance::TRAJECTORY));
        assert!(conf.contains(conformance::CORRIDOR));
        assert!(conf.contains(conformance::CUBE));
        assert!(conf.contains(conformance::INSTANCES));
        assert!(conf.contains(conformance::COVJSON));
    }

    #[test]
    fn test_conformance_minimal() {
        let conf = ConformanceClasses::minimal();

        assert!(conf.contains(conformance::CORE));
        assert!(!conf.contains(conformance::COLLECTIONS));
    }

    #[test]
    fn test_conformance_with_class() {
        let conf = ConformanceClasses::minimal().with_class(conformance::GEOJSON);

        assert!(conf.contains(conformance::CORE));
        assert!(conf.contains(conformance::GEOJSON));
    }

    #[test]
    fn test_conformance_serialization() {
        let conf = ConformanceClasses::current();
        let json = serde_json::to_string(&conf).unwrap();

        assert!(json.contains("\"conformsTo\""));
        assert!(json.contains("conf/core"));
        assert!(json.contains("conf/position"));
    }

    #[test]
    fn test_exception_not_found() {
        let exc = ExceptionResponse::not_found("Collection not found: test-collection");

        assert_eq!(exc.status, Some(404));
        assert_eq!(exc.title, Some("Not Found".to_string()));
        assert!(exc.detail.unwrap().contains("test-collection"));
    }

    #[test]
    fn test_exception_bad_request() {
        let exc = ExceptionResponse::bad_request("Invalid coordinate format");

        assert_eq!(exc.status, Some(400));
        assert_eq!(exc.title, Some("Bad Request".to_string()));
    }

    #[test]
    fn test_exception_payload_too_large() {
        let exc = ExceptionResponse::payload_too_large("Response would exceed 50MB limit");

        assert_eq!(exc.status, Some(413));
        assert!(exc.type_.contains("response-too-large"));
    }

    #[test]
    fn test_exception_internal_error() {
        let exc = ExceptionResponse::internal_error("Database connection failed");

        assert_eq!(exc.status, Some(500));
    }

    #[test]
    fn test_exception_serialization() {
        let exc = ExceptionResponse::not_found("Collection not found")
            .with_instance("/edr/collections/missing");

        let json = serde_json::to_string_pretty(&exc).unwrap();
        assert!(json.contains("\"type\""));
        // Pretty-printed JSON has spaces after colons
        assert!(json.contains("\"status\": 404") || json.contains("\"status\":404"));
        assert!(
            json.contains("\"title\": \"Not Found\"") || json.contains("\"title\":\"Not Found\"")
        );
        assert!(json.contains("\"detail\""));
        assert!(json.contains("\"instance\""));
    }
}
