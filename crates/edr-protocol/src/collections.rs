//! EDR Collection types.
//!
//! Collections represent datasets available through the EDR API.
//! For weather models, collections are typically grouped by level type
//! (e.g., isobaric, surface, height above ground).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::parameters::Parameter;
use crate::types::{Extent, Link};

/// A list of collections available from the EDR API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CollectionList {
    /// Links to related resources.
    pub links: Vec<Link>,

    /// The collections.
    pub collections: Vec<Collection>,
}

impl CollectionList {
    /// Create a new collection list.
    pub fn new(collections: Vec<Collection>, base_url: &str) -> Self {
        Self {
            links: vec![Link::new(format!("{}/collections", base_url), "self")
                .with_type("application/json")],
            collections,
        }
    }
}

/// An EDR collection representing a dataset or data source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Collection {
    /// Unique identifier for the collection.
    pub id: String,

    /// Human-readable title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Detailed description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Keywords for discovery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,

    /// Links to related resources.
    pub links: Vec<Link>,

    /// Spatial, temporal, and vertical extent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extent: Option<Extent>,

    /// Available query types and their links.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_queries: Option<DataQueries>,

    /// Coordinate reference systems supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crs: Option<Vec<String>>,

    /// Output formats supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_formats: Option<Vec<String>>,

    /// Parameters available in this collection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_names: Option<HashMap<String, Parameter>>,
}

impl Collection {
    /// Create a new collection with required fields.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: None,
            description: None,
            keywords: None,
            links: Vec::new(),
            extent: None,
            data_queries: None,
            crs: None,
            output_formats: None,
            parameter_names: None,
        }
    }

    /// Set the title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the extent.
    pub fn with_extent(mut self, extent: Extent) -> Self {
        self.extent = Some(extent);
        self
    }

    /// Set the data queries.
    pub fn with_data_queries(mut self, queries: DataQueries) -> Self {
        self.data_queries = Some(queries);
        self
    }

    /// Add links.
    pub fn with_links(mut self, links: Vec<Link>) -> Self {
        self.links = links;
        self
    }

    /// Set supported CRS.
    pub fn with_crs(mut self, crs: Vec<String>) -> Self {
        self.crs = Some(crs);
        self
    }

    /// Set output formats.
    pub fn with_output_formats(mut self, formats: Vec<String>) -> Self {
        self.output_formats = Some(formats);
        self
    }

    /// Set parameters.
    pub fn with_parameters(mut self, params: HashMap<String, Parameter>) -> Self {
        self.parameter_names = Some(params);
        self
    }

    /// Build standard links for a collection.
    pub fn build_links(&mut self, base_url: &str) {
        let collection_url = format!("{}/collections/{}", base_url, self.id);
        self.links = vec![
            Link::new(&collection_url, "self").with_type("application/json"),
            Link::new(base_url, "root").with_type("application/json"),
        ];

        // Add instance link if collection supports instances
        self.links.push(
            Link::new(format!("{}/instances", collection_url), "instances")
                .with_type("application/json")
                .with_title("Model run instances"),
        );
    }
}

/// Supported data query types for a collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DataQueries {
    /// Position query (point sampling).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<QueryDescription>,

    /// Area query (polygon sampling).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<QueryDescription>,

    /// Cube query (bbox + vertical).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cube: Option<QueryDescription>,

    /// Trajectory query (line sampling).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trajectory: Option<QueryDescription>,

    /// Corridor query (buffered trajectory).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corridor: Option<QueryDescription>,

    /// Radius query (circle around point).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<QueryDescription>,

    /// Locations query (named locations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locations: Option<QueryDescription>,
}

impl DataQueries {
    /// Create a new DataQueries with position query enabled.
    pub fn with_position(base_url: &str, collection_id: &str) -> Self {
        Self {
            position: Some(QueryDescription {
                link: Link::new(
                    format!("{}/collections/{}/position", base_url, collection_id),
                    "data",
                )
                .with_type("application/json")
                .with_title("Position query"),
                variables: None,
            }),
            ..Default::default()
        }
    }

    /// Add area query support.
    pub fn with_area(mut self, base_url: &str, collection_id: &str) -> Self {
        self.area = Some(QueryDescription {
            link: Link::new(
                format!("{}/collections/{}/area", base_url, collection_id),
                "data",
            )
            .with_type("application/json")
            .with_title("Area query"),
            variables: None,
        });
        self
    }

    /// Add cube query support.
    pub fn with_cube(mut self, base_url: &str, collection_id: &str) -> Self {
        self.cube = Some(QueryDescription {
            link: Link::new(
                format!("{}/collections/{}/cube", base_url, collection_id),
                "data",
            )
            .with_type("application/json")
            .with_title("Cube query"),
            variables: None,
        });
        self
    }

    /// Add radius query support.
    pub fn with_radius(mut self, base_url: &str, collection_id: &str) -> Self {
        self.radius = Some(QueryDescription {
            link: Link::new(
                format!("{}/collections/{}/radius", base_url, collection_id),
                "data",
            )
            .with_type("application/vnd.cov+json")
            .with_title("Radius query"),
            variables: None,
        });
        self
    }

    /// Add trajectory query support.
    pub fn with_trajectory(mut self, base_url: &str, collection_id: &str) -> Self {
        self.trajectory = Some(QueryDescription {
            link: Link::new(
                format!("{}/collections/{}/trajectory", base_url, collection_id),
                "data",
            )
            .with_type("application/vnd.cov+json")
            .with_title("Trajectory query"),
            variables: None,
        });
        self
    }

    /// Add corridor query support.
    ///
    /// Per OGC EDR spec, corridor queries must advertise supported width_units and height_units.
    pub fn with_corridor(mut self, base_url: &str, collection_id: &str) -> Self {
        self.corridor = Some(QueryDescription {
            link: Link::new(
                format!("{}/collections/{}/corridor", base_url, collection_id),
                "data",
            )
            .with_type("application/vnd.cov+json")
            .with_title("Corridor query"),
            variables: Some(QueryVariables {
                // Supported width units (distance units)
                width_units: Some(vec![
                    "km".to_string(),
                    "m".to_string(),
                    "mi".to_string(),
                    "nm".to_string(),
                ]),
                // Supported height units (distance + pressure units)
                height_units: Some(vec![
                    "m".to_string(),
                    "km".to_string(),
                    "hPa".to_string(),
                    "mb".to_string(),
                ]),
            }),
        });
        self
    }
}

/// Description of a query endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryDescription {
    /// Link to the query endpoint.
    pub link: Link,

    /// Variables/settings specific to this query type.
    /// For corridor queries, this includes width_units and height_units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<QueryVariables>,
}

/// Variables/settings specific to a query type.
///
/// Per OGC EDR spec, corridor queries must advertise supported width_units and height_units.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct QueryVariables {
    /// Supported width units for corridor queries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_units: Option<Vec<String>>,

    /// Supported height units for corridor queries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_units: Option<Vec<String>>,
}

/// A list of instances (model runs) for a collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstanceList {
    /// Links to related resources.
    pub links: Vec<Link>,

    /// The instances.
    pub instances: Vec<Instance>,
}

impl InstanceList {
    /// Create a new instance list.
    pub fn new(instances: Vec<Instance>, base_url: &str, collection_id: &str) -> Self {
        Self {
            links: vec![
                Link::new(
                    format!("{}/collections/{}/instances", base_url, collection_id),
                    "self",
                )
                .with_type("application/json"),
                Link::new(
                    format!("{}/collections/{}", base_url, collection_id),
                    "collection",
                )
                .with_type("application/json"),
            ],
            instances,
        }
    }
}

/// An instance represents a specific version of a collection,
/// typically a model run in the context of weather data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Instance {
    /// Unique identifier for the instance (usually ISO8601 datetime).
    pub id: String,

    /// Human-readable title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Detailed description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Links to related resources.
    pub links: Vec<Link>,

    /// Temporal extent specific to this instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extent: Option<Extent>,

    /// Data queries available for this instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_queries: Option<DataQueries>,
}

impl Instance {
    /// Create a new instance.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: None,
            description: None,
            links: Vec::new(),
            extent: None,
            data_queries: None,
        }
    }

    /// Set the title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the extent.
    pub fn with_extent(mut self, extent: Extent) -> Self {
        self.extent = Some(extent);
        self
    }

    /// Set data queries.
    pub fn with_data_queries(mut self, queries: DataQueries) -> Self {
        self.data_queries = Some(queries);
        self
    }

    /// Build standard links for an instance.
    pub fn build_links(&mut self, base_url: &str, collection_id: &str) {
        let instance_url = format!(
            "{}/collections/{}/instances/{}",
            base_url, collection_id, self.id
        );
        self.links = vec![
            Link::new(&instance_url, "self").with_type("application/json"),
            Link::new(
                format!("{}/collections/{}", base_url, collection_id),
                "collection",
            )
            .with_type("application/json"),
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collection_builder() {
        let collection = Collection::new("hrrr-isobaric")
            .with_title("HRRR Isobaric Levels")
            .with_description("Upper-air parameters on pressure levels");

        assert_eq!(collection.id, "hrrr-isobaric");
        assert_eq!(collection.title, Some("HRRR Isobaric Levels".to_string()));
        assert_eq!(
            collection.description,
            Some("Upper-air parameters on pressure levels".to_string())
        );
    }

    #[test]
    fn test_collection_build_links() {
        let mut collection = Collection::new("hrrr-surface");
        collection.build_links("http://localhost:8083/edr");

        assert!(collection.links.len() >= 2);
        assert!(collection.links.iter().any(|l| l.rel == "self"));
        assert!(collection.links.iter().any(|l| l.rel == "instances"));
    }

    #[test]
    fn test_collection_serialization() {
        let collection = Collection::new("test-collection")
            .with_title("Test Collection")
            .with_crs(vec!["CRS:84".to_string(), "EPSG:4326".to_string()])
            .with_output_formats(vec!["application/vnd.cov+json".to_string()]);

        let json = serde_json::to_string_pretty(&collection).unwrap();
        assert!(
            json.contains("\"id\": \"test-collection\"")
                || json.contains("\"id\":\"test-collection\"")
        );
        assert!(
            json.contains("\"title\": \"Test Collection\"")
                || json.contains("\"title\":\"Test Collection\"")
        );
        assert!(json.contains("\"crs\""));
        assert!(json.contains("CRS:84"));
    }

    #[test]
    fn test_collection_list() {
        let collections = vec![
            Collection::new("col1").with_title("Collection 1"),
            Collection::new("col2").with_title("Collection 2"),
        ];

        let list = CollectionList::new(collections, "http://localhost:8083/edr");
        assert_eq!(list.collections.len(), 2);
        assert!(list.links.iter().any(|l| l.rel == "self"));
    }

    #[test]
    fn test_data_queries() {
        let queries = DataQueries::with_position("http://localhost:8083/edr", "hrrr-surface")
            .with_area("http://localhost:8083/edr", "hrrr-surface")
            .with_cube("http://localhost:8083/edr", "hrrr-surface");

        assert!(queries.position.is_some());
        assert!(queries.area.is_some());
        assert!(queries.cube.is_some());
        assert!(queries.trajectory.is_none());

        let json = serde_json::to_string_pretty(&queries).unwrap();
        assert!(json.contains("\"position\""));
        assert!(json.contains("\"area\""));
        assert!(json.contains("\"cube\""));
        assert!(!json.contains("\"trajectory\""));
    }

    #[test]
    fn test_instance() {
        let mut instance =
            Instance::new("2024-12-29T12:00:00Z").with_title("HRRR Run 2024-12-29 12Z");

        instance.build_links("http://localhost:8083/edr", "hrrr-isobaric");

        assert_eq!(instance.id, "2024-12-29T12:00:00Z");
        assert!(instance.links.iter().any(|l| l.rel == "self"));
        assert!(instance.links.iter().any(|l| l.rel == "collection"));
    }

    #[test]
    fn test_instance_list() {
        let instances = vec![
            Instance::new("2024-12-29T12:00:00Z"),
            Instance::new("2024-12-29T06:00:00Z"),
        ];

        let list = InstanceList::new(instances, "http://localhost:8083/edr", "hrrr-isobaric");
        assert_eq!(list.instances.len(), 2);
        assert!(list.links.iter().any(|l| l.rel == "self"));
        assert!(list.links.iter().any(|l| l.rel == "collection"));
    }

    #[test]
    fn test_collection_with_parameters() {
        use crate::parameters::Parameter;
        use std::collections::HashMap;

        let mut params = HashMap::new();
        params.insert("TMP".to_string(), Parameter::new("TMP", "Temperature"));

        let collection = Collection::new("test").with_parameters(params);

        assert!(collection.parameter_names.is_some());
        let params = collection.parameter_names.unwrap();
        assert!(params.contains_key("TMP"));
    }
}
