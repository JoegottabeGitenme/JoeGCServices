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

pub mod collections;
pub mod coverage_json;
pub mod errors;
pub mod parameters;
pub mod queries;
pub mod responses;
pub mod types;

// Re-export commonly used types
pub use collections::{Collection, CollectionList, DataQueries, Instance, InstanceList};
pub use coverage_json::{Axis, CoverageCollection, CoverageJson, Domain, DomainType, NdArray, ReferenceSystem};
pub use errors::EdrError;
pub use parameters::{ObservedProperty, Parameter, Unit};
pub use queries::{AreaQuery, BboxQuery, CoordinateParseError, CorridorQuery, DateTimeQuery, DistanceUnit, LineStringType, ParsedCoords, ParsedPolygons, ParsedTrajectory, PositionQuery, RadiusQuery, TrajectoryQuery, TrajectoryWaypoint, VerticalUnit};
pub use responses::{ConformanceClasses, LandingPage};
pub use types::{Crs, Extent, Link, SpatialExtent, TemporalExtent, VerticalExtent};

/// EDR API conformance class URIs
pub mod conformance {
    /// Core conformance class
    pub const CORE: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/core";
    /// Collections conformance class
    pub const COLLECTIONS: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/collections";
    /// Position query conformance class
    pub const POSITION: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/position";
    /// Area query conformance class
    pub const AREA: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/area";
    /// Radius query conformance class
    pub const RADIUS: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/radius";
    /// Cube query conformance class
    pub const CUBE: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/cube";
    /// Trajectory query conformance class
    pub const TRAJECTORY: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/trajectory";
    /// Corridor query conformance class
    pub const CORRIDOR: &str = "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/corridor";
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
