//! Core EDR types used across the API.

use serde::{Deserialize, Serialize};

/// A hyperlink to a related resource.
///
/// Links are used throughout the EDR API to enable navigation and discovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Link {
    /// The URI of the linked resource.
    pub href: String,

    /// The relationship type (e.g., "self", "data", "conformance").
    pub rel: String,

    /// The media type of the linked resource.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,

    /// A human-readable title for the link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Whether the link is a URI template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub templated: Option<bool>,

    /// The language of the linked resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hreflang: Option<String>,
}

impl Link {
    /// Create a new link with required fields.
    pub fn new(href: impl Into<String>, rel: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            rel: rel.into(),
            type_: None,
            title: None,
            templated: None,
            hreflang: None,
        }
    }

    /// Set the media type.
    pub fn with_type(mut self, type_: impl Into<String>) -> Self {
        self.type_ = Some(type_.into());
        self
    }

    /// Set the title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Mark as a URI template.
    pub fn templated(mut self) -> Self {
        self.templated = Some(true);
        self
    }
}

/// The spatial and temporal extent of a collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Extent {
    /// The spatial extent of the collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spatial: Option<SpatialExtent>,

    /// The temporal extent of the collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal: Option<TemporalExtent>,

    /// The vertical extent of the collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical: Option<VerticalExtent>,
}

impl Extent {
    /// Create an empty extent.
    pub fn empty() -> Self {
        Self {
            spatial: None,
            temporal: None,
            vertical: None,
        }
    }

    /// Create an extent with spatial bounds.
    pub fn with_spatial(bbox: [f64; 4], crs: Option<String>) -> Self {
        Self {
            spatial: Some(SpatialExtent {
                bbox: vec![bbox.to_vec()],
                crs: crs.unwrap_or_else(|| "CRS:84".to_string()),
            }),
            temporal: None,
            vertical: None,
        }
    }

    /// Add temporal extent to this extent (builder pattern).
    pub fn with_temporal(mut self, temporal: TemporalExtent) -> Self {
        self.temporal = Some(temporal);
        self
    }

    /// Add vertical extent to this extent (builder pattern).
    pub fn with_vertical(mut self, vertical: VerticalExtent) -> Self {
        self.vertical = Some(vertical);
        self
    }

    /// Set the spatial extent (builder pattern).
    pub fn set_spatial(mut self, bbox: [f64; 4], crs: Option<String>) -> Self {
        self.spatial = Some(SpatialExtent {
            bbox: vec![bbox.to_vec()],
            crs: crs.unwrap_or_else(|| "CRS:84".to_string()),
        });
        self
    }
}

/// Spatial extent with bounding box.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpatialExtent {
    /// Bounding boxes as [west, south, east, north] arrays.
    /// May have multiple boxes for collections spanning the antimeridian.
    pub bbox: Vec<Vec<f64>>,

    /// Coordinate reference system (default: CRS:84).
    #[serde(default = "default_crs")]
    pub crs: String,
}

fn default_crs() -> String {
    "CRS:84".to_string()
}

/// Temporal extent with time intervals.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemporalExtent {
    /// Time intervals as [start, end] pairs (ISO 8601).
    /// null values indicate open-ended intervals.
    pub interval: Vec<Vec<Option<String>>>,

    /// Available time values (ISO 8601 timestamps).
    /// Lists all discrete times available in the collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,

    /// Temporal reference system (default: Gregorian).
    #[serde(default = "default_trs")]
    pub trs: String,
}

fn default_trs() -> String {
    "TIMECRS[\"DateTime\",TDATUM[\"Gregorian Calendar\"],CS[TemporalDateTime,1],AXIS[\"Time (T)\",future]]".to_string()
}

impl TemporalExtent {
    /// Create a temporal extent from start and end times.
    pub fn new(start: Option<String>, end: Option<String>) -> Self {
        Self {
            interval: vec![vec![start, end]],
            values: None,
            trs: default_trs(),
        }
    }

    /// Add available time values.
    pub fn with_values(mut self, values: Vec<String>) -> Self {
        self.values = Some(values);
        self
    }
}

/// Vertical extent with level values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerticalExtent {
    /// Vertical level values.
    pub interval: Vec<Vec<Option<f64>>>,

    /// Vertical reference system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vrs: Option<String>,
}

impl VerticalExtent {
    /// Create a vertical extent from min and max values.
    pub fn new(min: f64, max: f64) -> Self {
        Self {
            interval: vec![vec![Some(min), Some(max)]],
            vrs: None,
        }
    }

    /// Create a vertical extent with specific level values.
    pub fn with_levels(levels: Vec<f64>, vrs: Option<String>) -> Self {
        Self {
            interval: levels.into_iter().map(|l| vec![Some(l), Some(l)]).collect(),
            vrs,
        }
    }
}

/// Coordinate Reference System identifier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Crs {
    /// The CRS identifier URI.
    #[serde(rename = "crs")]
    pub id: String,

    /// Optional WKT representation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wkt: Option<String>,
}

impl Crs {
    /// CRS:84 (WGS84 lon/lat)
    pub fn crs84() -> Self {
        Self {
            id: "CRS:84".to_string(),
            wkt: None,
        }
    }

    /// EPSG:4326 (WGS84 lat/lon)
    pub fn epsg4326() -> Self {
        Self {
            id: "http://www.opengis.net/def/crs/EPSG/0/4326".to_string(),
            wkt: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_creation() {
        let link = Link::new("http://example.com", "self");
        assert_eq!(link.href, "http://example.com");
        assert_eq!(link.rel, "self");
        assert!(link.type_.is_none());
    }

    #[test]
    fn test_link_builder() {
        let link = Link::new("http://example.com/data", "data")
            .with_type("application/json")
            .with_title("Data endpoint")
            .templated();

        assert_eq!(link.href, "http://example.com/data");
        assert_eq!(link.rel, "data");
        assert_eq!(link.type_, Some("application/json".to_string()));
        assert_eq!(link.title, Some("Data endpoint".to_string()));
        assert_eq!(link.templated, Some(true));
    }

    #[test]
    fn test_link_serialization() {
        let link = Link::new("http://example.com", "self")
            .with_type("application/json");

        let json = serde_json::to_string(&link).unwrap();
        assert!(json.contains("\"href\":\"http://example.com\""));
        assert!(json.contains("\"rel\":\"self\""));
        assert!(json.contains("\"type\":\"application/json\""));
        // Ensure None fields are skipped
        assert!(!json.contains("\"title\""));
        assert!(!json.contains("\"templated\""));
    }

    #[test]
    fn test_link_deserialization() {
        let json = r#"{"href":"http://example.com","rel":"self","type":"application/json"}"#;
        let link: Link = serde_json::from_str(json).unwrap();
        assert_eq!(link.href, "http://example.com");
        assert_eq!(link.rel, "self");
        assert_eq!(link.type_, Some("application/json".to_string()));
    }

    #[test]
    fn test_extent_empty() {
        let extent = Extent::empty();
        assert!(extent.spatial.is_none());
        assert!(extent.temporal.is_none());
        assert!(extent.vertical.is_none());
    }

    #[test]
    fn test_extent_with_spatial() {
        let extent = Extent::with_spatial([-180.0, -90.0, 180.0, 90.0], None);
        let spatial = extent.spatial.unwrap();
        assert_eq!(spatial.bbox, vec![vec![-180.0, -90.0, 180.0, 90.0]]);
        assert_eq!(spatial.crs, "CRS:84");
    }

    #[test]
    fn test_temporal_extent() {
        let extent = TemporalExtent::new(
            Some("2024-01-01T00:00:00Z".to_string()),
            Some("2024-12-31T23:59:59Z".to_string()),
        );
        assert_eq!(extent.interval.len(), 1);
        assert_eq!(
            extent.interval[0][0],
            Some("2024-01-01T00:00:00Z".to_string())
        );
    }

    #[test]
    fn test_vertical_extent() {
        let extent = VerticalExtent::new(1000.0, 250.0);
        assert_eq!(extent.interval.len(), 1);
        assert_eq!(extent.interval[0][0], Some(1000.0));
        assert_eq!(extent.interval[0][1], Some(250.0));
    }

    #[test]
    fn test_vertical_extent_with_levels() {
        let extent = VerticalExtent::with_levels(
            vec![1000.0, 850.0, 700.0, 500.0, 300.0, 250.0],
            Some("hPa".to_string()),
        );
        assert_eq!(extent.interval.len(), 6);
        assert_eq!(extent.vrs, Some("hPa".to_string()));
    }

    #[test]
    fn test_crs_presets() {
        let crs84 = Crs::crs84();
        assert_eq!(crs84.id, "CRS:84");

        let epsg4326 = Crs::epsg4326();
        assert!(epsg4326.id.contains("EPSG"));
        assert!(epsg4326.id.contains("4326"));
    }

    #[test]
    fn test_extent_serialization() {
        let extent = Extent {
            spatial: Some(SpatialExtent {
                bbox: vec![vec![-125.0, 24.0, -66.0, 50.0]],
                crs: "CRS:84".to_string(),
            }),
            temporal: Some(TemporalExtent::new(
                Some("2024-01-01T00:00:00Z".to_string()),
                None,
            )),
            vertical: Some(VerticalExtent::new(1000.0, 250.0)),
        };

        let json = serde_json::to_string_pretty(&extent).unwrap();
        assert!(json.contains("\"bbox\""));
        assert!(json.contains("\"interval\""));
        assert!(json.contains("-125"));
    }
}
