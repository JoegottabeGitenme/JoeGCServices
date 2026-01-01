//! EDR Locations types.
//!
//! Locations represent named, pre-defined points of interest that clients
//! can query using human-readable identifiers instead of raw coordinates.
//! Examples include airports (ICAO codes), weather stations (WMO IDs),
//! or cities.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A named location for EDR queries.
///
/// Locations allow clients to query data at well-known points using
/// identifiers like airport codes (KJFK) instead of coordinates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Location {
    /// Unique identifier (e.g., "KJFK", "NYC", "WMO-72403").
    pub id: String,

    /// Human-readable name.
    pub name: String,

    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Coordinates as [longitude, latitude] in CRS:84.
    pub coords: [f64; 2],

    /// Optional additional properties (type, country, etc.).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, String>,
}

impl Location {
    /// Create a new location.
    pub fn new(id: impl Into<String>, name: impl Into<String>, lon: f64, lat: f64) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            coords: [lon, lat],
            properties: HashMap::new(),
        }
    }

    /// Set the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add a property.
    pub fn with_property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.insert(key.into(), value.into());
        self
    }

    /// Get the longitude.
    pub fn lon(&self) -> f64 {
        self.coords[0]
    }

    /// Get the latitude.
    pub fn lat(&self) -> f64 {
        self.coords[1]
    }
}

/// Configuration for EDR locations loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocationsConfig {
    /// List of named locations.
    #[serde(default)]
    pub locations: Vec<Location>,
}

impl LocationsConfig {
    /// Create an empty locations config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Find a location by ID (case-insensitive).
    pub fn find(&self, id: &str) -> Option<&Location> {
        let id_upper = id.to_uppercase();
        self.locations
            .iter()
            .find(|loc| loc.id.to_uppercase() == id_upper)
    }

    /// Get all location IDs.
    pub fn ids(&self) -> Vec<&str> {
        self.locations.iter().map(|loc| loc.id.as_str()).collect()
    }

    /// Check if a location exists.
    pub fn contains(&self, id: &str) -> bool {
        self.find(id).is_some()
    }

    /// Get the number of locations.
    pub fn len(&self) -> usize {
        self.locations.len()
    }

    /// Check if there are no locations.
    pub fn is_empty(&self) -> bool {
        self.locations.is_empty()
    }
}

/// GeoJSON Feature representation of a location.
///
/// Used when returning the list of available locations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationFeature {
    /// Always "Feature".
    #[serde(rename = "type")]
    pub feature_type: String,

    /// The location ID.
    pub id: String,

    /// Point geometry.
    pub geometry: LocationGeometry,

    /// Feature properties.
    pub properties: LocationProperties,
}

impl LocationFeature {
    /// Create a new LocationFeature from a Location with a proper URI ID.
    ///
    /// Per OGC EDR spec, the Feature ID SHALL be a valid URI string that
    /// SHOULD resolve to more information about the location.
    pub fn from_location_with_uri(loc: &Location, base_url: &str, collection_id: &str) -> Self {
        let mut props = LocationProperties {
            name: loc.name.clone(),
            description: loc.description.clone(),
            datetime: None,
            extra: HashMap::new(),
        };

        // Copy location properties to feature properties
        for (k, v) in &loc.properties {
            props
                .extra
                .insert(k.clone(), serde_json::Value::String(v.clone()));
        }

        // Generate URI-style ID per OGC EDR spec requirement
        let uri_id = format!(
            "{}/collections/{}/locations/{}",
            base_url, collection_id, loc.id
        );

        Self {
            feature_type: "Feature".to_string(),
            id: uri_id,
            geometry: LocationGeometry {
                geometry_type: "Point".to_string(),
                coordinates: vec![loc.coords[0], loc.coords[1]],
            },
            properties: props,
        }
    }
}

impl From<&Location> for LocationFeature {
    fn from(loc: &Location) -> Self {
        let mut props = LocationProperties {
            name: loc.name.clone(),
            description: loc.description.clone(),
            datetime: None,
            extra: HashMap::new(),
        };

        // Copy location properties to feature properties
        for (k, v) in &loc.properties {
            props
                .extra
                .insert(k.clone(), serde_json::Value::String(v.clone()));
        }

        Self {
            feature_type: "Feature".to_string(),
            id: loc.id.clone(),
            geometry: LocationGeometry {
                geometry_type: "Point".to_string(),
                coordinates: vec![loc.coords[0], loc.coords[1]],
            },
            properties: props,
        }
    }
}

/// GeoJSON Point geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationGeometry {
    #[serde(rename = "type")]
    pub geometry_type: String,

    /// [longitude, latitude]
    pub coordinates: Vec<f64>,
}

/// Properties for a location feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationProperties {
    /// Human-readable name.
    pub name: String,

    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Temporal extent (if queried with datetime).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,

    /// Additional properties.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// GeoJSON FeatureCollection of locations.
///
/// Returned by GET /collections/{collectionId}/locations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationFeatureCollection {
    /// Always "FeatureCollection".
    #[serde(rename = "type")]
    pub collection_type: String,

    /// The location features.
    pub features: Vec<LocationFeature>,

    /// Number of features.
    #[serde(rename = "numberReturned", skip_serializing_if = "Option::is_none")]
    pub number_returned: Option<usize>,
}

impl LocationFeatureCollection {
    /// Create a new feature collection from locations.
    pub fn from_locations(locations: &[Location]) -> Self {
        let features: Vec<LocationFeature> = locations.iter().map(LocationFeature::from).collect();
        let count = features.len();

        Self {
            collection_type: "FeatureCollection".to_string(),
            features,
            number_returned: Some(count),
        }
    }

    /// Create from a LocationsConfig with proper URI-style IDs.
    ///
    /// Per OGC EDR spec, the Feature ID SHALL be a valid URI string.
    pub fn from_config_with_uris(
        config: &LocationsConfig,
        base_url: &str,
        collection_id: &str,
    ) -> Self {
        let features: Vec<LocationFeature> = config
            .locations
            .iter()
            .map(|loc| LocationFeature::from_location_with_uri(loc, base_url, collection_id))
            .collect();
        let count = features.len();

        Self {
            collection_type: "FeatureCollection".to_string(),
            features,
            number_returned: Some(count),
        }
    }

    /// Create from a LocationsConfig (legacy - uses simple IDs).
    pub fn from_config(config: &LocationsConfig) -> Self {
        Self::from_locations(&config.locations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_location_creation() {
        let loc = Location::new("KJFK", "JFK Airport", -73.7781, 40.6413)
            .with_description("New York, NY")
            .with_property("type", "airport");

        assert_eq!(loc.id, "KJFK");
        assert_eq!(loc.lon(), -73.7781);
        assert_eq!(loc.lat(), 40.6413);
        assert_eq!(loc.properties.get("type"), Some(&"airport".to_string()));
    }

    #[test]
    fn test_locations_config_find() {
        let config = LocationsConfig {
            locations: vec![
                Location::new("KJFK", "JFK", -73.7781, 40.6413),
                Location::new("KLAX", "LAX", -118.4085, 33.9416),
            ],
        };

        assert!(config.find("KJFK").is_some());
        assert!(config.find("kjfk").is_some()); // Case insensitive
        assert!(config.find("UNKNOWN").is_none());
    }

    #[test]
    fn test_location_to_geojson() {
        let loc = Location::new("KJFK", "JFK Airport", -73.7781, 40.6413);
        let feature = LocationFeature::from(&loc);

        assert_eq!(feature.feature_type, "Feature");
        assert_eq!(feature.id, "KJFK");
        assert_eq!(feature.geometry.geometry_type, "Point");
        assert_eq!(feature.geometry.coordinates, vec![-73.7781, 40.6413]);
    }

    #[test]
    fn test_feature_collection() {
        let locations = vec![
            Location::new("KJFK", "JFK", -73.7781, 40.6413),
            Location::new("KLAX", "LAX", -118.4085, 33.9416),
        ];

        let fc = LocationFeatureCollection::from_locations(&locations);

        assert_eq!(fc.collection_type, "FeatureCollection");
        assert_eq!(fc.features.len(), 2);
        assert_eq!(fc.number_returned, Some(2));
    }
}
