//! OGC API - Environmental Data Retrieval (EDR) Protocol
//!
//! This crate provides types and utilities for implementing an OGC EDR API server.
//! It follows the OGC API - Environmental Data Retrieval specification v1.1.
//!
//! # Conformance Classes
//!
//! This implementation targets the following conformance classes:
//! - Core
//! - Collections
//! - Position Query
//! - Instances
//! - CoverageJSON
//!
//! # Example
//!
//! ```rust
//! use edr_protocol::{LandingPage, Link, Collection};
//!
//! // Build a landing page response
//! let landing = LandingPage::new(
//!     "Weather EDR API",
//!     "Environmental data retrieval for weather models",
//!     "http://localhost:8083/edr",
//! );
//! ```

pub mod types;
pub mod collections;
pub mod parameters;
pub mod coverage_json;
pub mod queries;
pub mod responses;
pub mod errors;

// Re-export commonly used types
pub use types::{Link, Extent, SpatialExtent, TemporalExtent, VerticalExtent, Crs};
pub use collections::{Collection, CollectionList, DataQueries, Instance, InstanceList};
pub use parameters::{Parameter, Unit, ObservedProperty};
pub use coverage_json::{CoverageJson, Domain, DomainType, Axis, NdArray, ReferenceSystem};
pub use queries::{PositionQuery, CoordinateParseError};
pub use responses::{LandingPage, ConformanceClasses};
pub use errors::EdrError;

/// EDR API conformance class URIs
pub mod conformance {
    /// Core conformance class
    pub const CORE: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/core";
    /// Collections conformance class
    pub const COLLECTIONS: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/collections";
    /// Position query conformance class
    pub const POSITION: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/position";
    /// Instances conformance class
    pub const INSTANCES: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/instances";
    /// CoverageJSON conformance class
    pub const COVJSON: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/covjson";
    /// GeoJSON conformance class
    pub const GEOJSON: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/geojson";
}

/// Media types used in EDR responses
pub mod media_types {
    /// CoverageJSON media type
    pub const COVERAGE_JSON: &str = "application/vnd.cov+json";
    /// GeoJSON media type
    pub const GEO_JSON: &str = "application/geo+json";
    /// JSON media type
    pub const JSON: &str = "application/json";
    /// OpenAPI JSON media type
    pub const OPENAPI_JSON: &str = "application/openapi+json;version=3.0";
}
