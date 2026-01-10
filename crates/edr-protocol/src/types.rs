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

    /// Query-specific variables (for EDR data query links).
    /// Contains output_formats, and for corridor queries: width_units, height_units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<LinkVariables>,
}

/// Variables associated with a query link.
/// Used to advertise supported output formats and query-specific options.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LinkVariables {
    /// Supported output formats for this query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_formats: Option<Vec<String>>,

    /// Supported width units for corridor queries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_units: Option<Vec<String>>,

    /// Supported height units for corridor queries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_units: Option<Vec<String>>,
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
            variables: None,
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

    /// Set query variables (output_formats, width_units, height_units).
    pub fn with_variables(mut self, variables: LinkVariables) -> Self {
        self.variables = Some(variables);
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
///
/// Per OGC EDR spec (Table C.8):
/// - `interval`: Array of [min, max] pairs describing the overall vertical range
/// - `values`: Array of discrete height/level values supported by the collection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerticalExtent {
    /// Vertical level interval as [min, max] pairs.
    /// Each inner array should contain two values: minimum and maximum vertical level.
    pub interval: Vec<Vec<Option<f64>>>,

    /// Available vertical level values.
    /// Lists all discrete levels available in the collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<f64>>,

    /// Vertical reference system (e.g., "hPa", "m").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vrs: Option<String>,
}

impl VerticalExtent {
    /// Create a vertical extent from min and max values (continuous range).
    /// Use this when discrete level values are not known.
    pub fn new(min: f64, max: f64) -> Self {
        Self {
            interval: vec![vec![Some(min), Some(max)]],
            values: None,
            vrs: None,
        }
    }

    /// Create a vertical extent with specific level values.
    /// Sets interval to [min, max] of the levels and values to the full list.
    pub fn with_levels(mut levels: Vec<f64>, vrs: Option<String>) -> Self {
        if levels.is_empty() {
            return Self {
                interval: vec![],
                values: None,
                vrs,
            };
        }

        // Sort levels to find min/max
        levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let min = levels.first().copied();
        let max = levels.last().copied();

        Self {
            interval: vec![vec![min, max]],
            values: Some(levels),
            vrs,
        }
    }

    /// Add available level values (builder pattern).
    pub fn with_values(mut self, values: Vec<f64>) -> Self {
        self.values = Some(values);
        self
    }

    /// Set the vertical reference system (builder pattern).
    pub fn with_vrs(mut self, vrs: impl Into<String>) -> Self {
        self.vrs = Some(vrs.into());
        self
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
        let link = Link::new("http://example.com", "self").with_type("application/json");

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
        // new() doesn't set values (for continuous ranges)
        assert!(extent.values.is_none());
    }

    #[test]
    fn test_vertical_extent_with_levels() {
        let extent = VerticalExtent::with_levels(
            vec![1000.0, 850.0, 700.0, 500.0, 300.0, 250.0],
            Some("hPa".to_string()),
        );
        // interval should be a single [min, max] pair
        assert_eq!(extent.interval.len(), 1);
        assert_eq!(extent.interval[0][0], Some(250.0)); // min (sorted)
        assert_eq!(extent.interval[0][1], Some(1000.0)); // max (sorted)
                                                         // values should contain all levels, sorted
        assert!(extent.values.is_some());
        let values = extent.values.unwrap();
        assert_eq!(values.len(), 6);
        assert_eq!(values[0], 250.0); // sorted ascending
        assert_eq!(values[5], 1000.0);
        assert_eq!(extent.vrs, Some("hPa".to_string()));
    }

    #[test]
    fn test_vertical_extent_with_levels_empty() {
        let extent = VerticalExtent::with_levels(vec![], Some("hPa".to_string()));
        assert!(extent.interval.is_empty());
        assert!(extent.values.is_none());
        assert_eq!(extent.vrs, Some("hPa".to_string()));
    }

    #[test]
    fn test_vertical_extent_builder() {
        let extent = VerticalExtent::new(0.0, 10000.0)
            .with_values(vec![0.0, 1000.0, 5000.0, 10000.0])
            .with_vrs("m");
        assert_eq!(extent.vrs, Some("m".to_string()));
        assert!(extent.values.is_some());
        assert_eq!(extent.values.unwrap().len(), 4);
    }

    #[test]
    fn test_vertical_extent_serialization() {
        let extent =
            VerticalExtent::with_levels(vec![1000.0, 500.0, 250.0], Some("hPa".to_string()));
        let json = serde_json::to_string(&extent).unwrap();
        // Should have interval with [min, max]
        assert!(json.contains("\"interval\""));
        // Should have values array
        assert!(json.contains("\"values\""));
        // Should have vrs
        assert!(json.contains("\"vrs\""));
        assert!(json.contains("\"hPa\""));
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
