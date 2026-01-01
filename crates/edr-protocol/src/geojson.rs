//! EDR GeoJSON types for query responses.
//!
//! GeoJSON is an alternative response format for EDR data queries.
//! Per OGC EDR spec, responses must be either:
//! - `EDR GeoJSON FeatureCollection Object` for multiple features
//! - `EDR GeoJSON Feature Object` for a single feature
//!
//! See: <https://www.opengis.net/spec/ogcapi-edr-1/1.1/req/edr-geojson>

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::coverage_json::{
    Axis, AxisValue, CompositeValue, CoverageCollection, CoverageJson, DomainType,
};

/// A GeoJSON FeatureCollection for EDR responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdrFeatureCollection {
    /// Type identifier (always "FeatureCollection").
    #[serde(rename = "type")]
    pub type_: String,

    /// Array of features.
    pub features: Vec<EdrFeature>,
}

impl EdrFeatureCollection {
    /// Create a new empty FeatureCollection.
    pub fn new() -> Self {
        Self {
            type_: "FeatureCollection".to_string(),
            features: Vec::new(),
        }
    }

    /// Add a feature to the collection.
    pub fn with_feature(mut self, feature: EdrFeature) -> Self {
        self.features.push(feature);
        self
    }

    /// Add multiple features to the collection.
    pub fn with_features(mut self, features: Vec<EdrFeature>) -> Self {
        self.features.extend(features);
        self
    }
}

impl Default for EdrFeatureCollection {
    fn default() -> Self {
        Self::new()
    }
}

/// A GeoJSON Feature for EDR responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdrFeature {
    /// Type identifier (always "Feature").
    #[serde(rename = "type")]
    pub type_: String,

    /// Optional feature identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// The geometry of this feature.
    pub geometry: EdrGeometry,

    /// Properties containing parameter values and metadata.
    pub properties: EdrProperties,
}

impl EdrFeature {
    /// Create a new feature with a point geometry.
    pub fn point(lon: f64, lat: f64) -> Self {
        Self {
            type_: "Feature".to_string(),
            id: None,
            geometry: EdrGeometry::point(lon, lat),
            properties: EdrProperties::new(),
        }
    }

    /// Create a new feature with a LineString geometry.
    pub fn line_string(coordinates: Vec<[f64; 2]>) -> Self {
        Self {
            type_: "Feature".to_string(),
            id: None,
            geometry: EdrGeometry::line_string(coordinates),
            properties: EdrProperties::new(),
        }
    }

    /// Set the feature ID.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set the properties.
    pub fn with_properties(mut self, properties: EdrProperties) -> Self {
        self.properties = properties;
        self
    }
}

/// GeoJSON geometry types supported by EDR.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum EdrGeometry {
    /// A point geometry.
    Point {
        /// Coordinates as [longitude, latitude].
        coordinates: [f64; 2],
    },

    /// A line string geometry.
    LineString {
        /// Array of [longitude, latitude] coordinate pairs.
        coordinates: Vec<[f64; 2]>,
    },

    /// A polygon geometry.
    Polygon {
        /// Array of linear rings (first is exterior, rest are holes).
        /// Each ring is an array of [longitude, latitude] coordinate pairs.
        coordinates: Vec<Vec<[f64; 2]>>,
    },
}

impl EdrGeometry {
    /// Create a point geometry.
    pub fn point(lon: f64, lat: f64) -> Self {
        EdrGeometry::Point {
            coordinates: [lon, lat],
        }
    }

    /// Create a line string geometry.
    pub fn line_string(coordinates: Vec<[f64; 2]>) -> Self {
        EdrGeometry::LineString { coordinates }
    }

    /// Create a polygon geometry.
    pub fn polygon(coordinates: Vec<Vec<[f64; 2]>>) -> Self {
        EdrGeometry::Polygon { coordinates }
    }
}

/// Properties for an EDR GeoJSON feature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct EdrProperties {
    /// Datetime of the observation/forecast.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,

    /// Vertical level (z coordinate).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z: Option<f64>,

    /// Parameter values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, ParameterValue>>,
}

impl EdrProperties {
    /// Create new empty properties.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the datetime.
    pub fn with_datetime(mut self, datetime: impl Into<String>) -> Self {
        self.datetime = Some(datetime.into());
        self
    }

    /// Set the vertical level.
    pub fn with_z(mut self, z: f64) -> Self {
        self.z = Some(z);
        self
    }

    /// Add a parameter value.
    pub fn with_parameter(mut self, name: impl Into<String>, value: ParameterValue) -> Self {
        if self.parameters.is_none() {
            self.parameters = Some(HashMap::new());
        }
        if let Some(ref mut params) = self.parameters {
            params.insert(name.into(), value);
        }
        self
    }
}

/// A parameter value in GeoJSON properties.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParameterValue {
    /// The numeric value (null if missing).
    pub value: Option<f32>,

    /// Unit of measurement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl ParameterValue {
    /// Create a new parameter value.
    pub fn new(value: Option<f32>) -> Self {
        Self { value, unit: None }
    }

    /// Create a parameter value with unit.
    pub fn with_unit(value: Option<f32>, unit: impl Into<String>) -> Self {
        Self {
            value,
            unit: Some(unit.into()),
        }
    }
}

// =============================================================================
// Conversion from CoverageJSON to GeoJSON
// =============================================================================

impl From<CoverageJson> for EdrFeatureCollection {
    fn from(coverage: CoverageJson) -> Self {
        coverage_to_feature_collection(&coverage)
    }
}

impl From<&CoverageJson> for EdrFeatureCollection {
    fn from(coverage: &CoverageJson) -> Self {
        coverage_to_feature_collection(coverage)
    }
}

impl From<CoverageCollection> for EdrFeatureCollection {
    fn from(collection: CoverageCollection) -> Self {
        let mut fc = EdrFeatureCollection::new();
        for coverage in &collection.coverages {
            let sub_fc = coverage_to_feature_collection(coverage);
            fc.features.extend(sub_fc.features);
        }
        fc
    }
}

impl From<&CoverageCollection> for EdrFeatureCollection {
    fn from(collection: &CoverageCollection) -> Self {
        let mut fc = EdrFeatureCollection::new();
        for coverage in &collection.coverages {
            let sub_fc = coverage_to_feature_collection(coverage);
            fc.features.extend(sub_fc.features);
        }
        fc
    }
}

/// Convert a CoverageJSON document to a GeoJSON FeatureCollection.
fn coverage_to_feature_collection(coverage: &CoverageJson) -> EdrFeatureCollection {
    match coverage.domain.domain_type {
        DomainType::Point => convert_point_coverage(coverage),
        DomainType::PointSeries => convert_point_series_coverage(coverage),
        DomainType::VerticalProfile => convert_vertical_profile_coverage(coverage),
        DomainType::Grid => convert_grid_coverage(coverage),
        DomainType::Trajectory => convert_trajectory_coverage(coverage),
        DomainType::MultiPoint => convert_multipoint_coverage(coverage),
    }
}

/// Convert a Point domain coverage to GeoJSON.
/// Returns a FeatureCollection with a single Feature.
fn convert_point_coverage(coverage: &CoverageJson) -> EdrFeatureCollection {
    let (x, y) = get_single_xy(&coverage.domain.axes);
    let t = get_single_time(&coverage.domain.axes);
    let z = get_single_z(&coverage.domain.axes);

    let mut properties = EdrProperties::new();
    if let Some(datetime) = t {
        properties = properties.with_datetime(datetime);
    }
    if let Some(z_val) = z {
        properties = properties.with_z(z_val);
    }

    // Add parameter values
    if let (Some(params), Some(ranges)) = (&coverage.parameters, &coverage.ranges) {
        for (name, param) in params {
            if let Some(range) = ranges.get(name) {
                let value = range.values.first().copied().flatten();
                let unit_str = param
                    .unit
                    .as_ref()
                    .and_then(|u| u.symbol.as_ref())
                    .map(|s| s.value().to_string())
                    .unwrap_or_default();
                properties =
                    properties.with_parameter(name, ParameterValue::with_unit(value, unit_str));
            }
        }
    }

    let feature = EdrFeature::point(x, y).with_properties(properties);
    EdrFeatureCollection::new().with_feature(feature)
}

/// Convert a PointSeries (time series) coverage to GeoJSON.
/// Each time step becomes a separate Feature.
fn convert_point_series_coverage(coverage: &CoverageJson) -> EdrFeatureCollection {
    let (x, y) = get_single_xy(&coverage.domain.axes);
    let times = get_time_values(&coverage.domain.axes);
    let z = get_single_z(&coverage.domain.axes);

    let mut fc = EdrFeatureCollection::new();

    for (i, t) in times.iter().enumerate() {
        let mut properties = EdrProperties::new().with_datetime(t);
        if let Some(z_val) = z {
            properties = properties.with_z(z_val);
        }

        // Get parameter value at this time index
        if let (Some(params), Some(ranges)) = (&coverage.parameters, &coverage.ranges) {
            for (name, param) in params {
                if let Some(range) = ranges.get(name) {
                    let value = range.values.get(i).copied().flatten();
                    let unit_str = param
                        .unit
                        .as_ref()
                        .and_then(|u| u.symbol.as_ref())
                        .map(|s| s.value().to_string())
                        .unwrap_or_default();
                    properties =
                        properties.with_parameter(name, ParameterValue::with_unit(value, unit_str));
                }
            }
        }

        let feature = EdrFeature::point(x, y)
            .with_id(format!("t{}", i))
            .with_properties(properties);
        fc.features.push(feature);
    }

    fc
}

/// Convert a VerticalProfile coverage to GeoJSON.
/// Each z-level becomes a separate Feature.
fn convert_vertical_profile_coverage(coverage: &CoverageJson) -> EdrFeatureCollection {
    let (x, y) = get_single_xy(&coverage.domain.axes);
    let t = get_single_time(&coverage.domain.axes);
    let z_values = get_z_values(&coverage.domain.axes);

    let mut fc = EdrFeatureCollection::new();

    for (i, z) in z_values.iter().enumerate() {
        let mut properties = EdrProperties::new().with_z(*z);
        if let Some(ref datetime) = t {
            properties = properties.with_datetime(datetime);
        }

        // Get parameter value at this z index
        if let (Some(params), Some(ranges)) = (&coverage.parameters, &coverage.ranges) {
            for (name, param) in params {
                if let Some(range) = ranges.get(name) {
                    let value = range.values.get(i).copied().flatten();
                    let unit_str = param
                        .unit
                        .as_ref()
                        .and_then(|u| u.symbol.as_ref())
                        .map(|s| s.value().to_string())
                        .unwrap_or_default();
                    properties =
                        properties.with_parameter(name, ParameterValue::with_unit(value, unit_str));
                }
            }
        }

        let feature = EdrFeature::point(x, y)
            .with_id(format!("z{}", i))
            .with_properties(properties);
        fc.features.push(feature);
    }

    fc
}

/// Convert a Grid domain coverage to GeoJSON.
/// Each grid point becomes a separate Feature.
fn convert_grid_coverage(coverage: &CoverageJson) -> EdrFeatureCollection {
    let x_values = get_axis_float_values(&coverage.domain.axes, "x");
    let y_values = get_axis_float_values(&coverage.domain.axes, "y");
    let t = get_single_time(&coverage.domain.axes);
    let z = get_single_z(&coverage.domain.axes);

    let mut fc = EdrFeatureCollection::new();
    let mut idx = 0;

    // Grid data is typically stored in row-major order (y, x)
    for (yi, y) in y_values.iter().enumerate() {
        for (xi, x) in x_values.iter().enumerate() {
            let mut properties = EdrProperties::new();
            if let Some(ref datetime) = t {
                properties = properties.with_datetime(datetime);
            }
            if let Some(z_val) = z {
                properties = properties.with_z(z_val);
            }

            // Get parameter value at this grid index
            if let (Some(params), Some(ranges)) = (&coverage.parameters, &coverage.ranges) {
                for (name, param) in params {
                    if let Some(range) = ranges.get(name) {
                        let value = range.values.get(idx).copied().flatten();
                        let unit_str = param
                            .unit
                            .as_ref()
                            .and_then(|u| u.symbol.as_ref())
                            .map(|s| s.value().to_string())
                            .unwrap_or_default();
                        properties = properties
                            .with_parameter(name, ParameterValue::with_unit(value, unit_str));
                    }
                }
            }

            let feature = EdrFeature::point(*x, *y)
                .with_id(format!("y{}x{}", yi, xi))
                .with_properties(properties);
            fc.features.push(feature);
            idx += 1;
        }
    }

    fc
}

/// Convert a Trajectory domain coverage to GeoJSON.
/// Each waypoint becomes a separate Feature with Point geometry.
fn convert_trajectory_coverage(coverage: &CoverageJson) -> EdrFeatureCollection {
    let mut fc = EdrFeatureCollection::new();

    // Trajectory uses composite axis
    if let Some(Axis::Composite(composite)) = coverage.domain.axes.get("composite") {
        // Find indices for coordinates in the tuple
        let t_idx = composite.coordinates.iter().position(|c| c == "t");
        let x_idx = composite.coordinates.iter().position(|c| c == "x");
        let y_idx = composite.coordinates.iter().position(|c| c == "y");
        let z_idx = composite.coordinates.iter().position(|c| c == "z");

        for (i, point) in composite.values.iter().enumerate() {
            let mut properties = EdrProperties::new();

            // Extract coordinates from composite tuple
            let x = x_idx.and_then(|idx| point.get(idx)).and_then(|v| match v {
                CompositeValue::Float(f) => Some(*f),
                _ => None,
            });
            let y = y_idx.and_then(|idx| point.get(idx)).and_then(|v| match v {
                CompositeValue::Float(f) => Some(*f),
                _ => None,
            });

            if let Some(t_i) = t_idx {
                if let Some(CompositeValue::String(t)) = point.get(t_i) {
                    properties = properties.with_datetime(t);
                }
            }
            if let Some(z_i) = z_idx {
                if let Some(CompositeValue::Float(z)) = point.get(z_i) {
                    properties = properties.with_z(*z);
                }
            }

            // Get parameter value at this waypoint
            if let (Some(params), Some(ranges)) = (&coverage.parameters, &coverage.ranges) {
                for (name, param) in params {
                    if let Some(range) = ranges.get(name) {
                        let value = range.values.get(i).copied().flatten();
                        let unit_str = param
                            .unit
                            .as_ref()
                            .and_then(|u| u.symbol.as_ref())
                            .map(|s| s.value().to_string())
                            .unwrap_or_default();
                        properties = properties
                            .with_parameter(name, ParameterValue::with_unit(value, unit_str));
                    }
                }
            }

            if let (Some(lon), Some(lat)) = (x, y) {
                let feature = EdrFeature::point(lon, lat)
                    .with_id(format!("wp{}", i))
                    .with_properties(properties);
                fc.features.push(feature);
            }
        }
    }

    fc
}

/// Convert a MultiPoint domain coverage to GeoJSON.
fn convert_multipoint_coverage(coverage: &CoverageJson) -> EdrFeatureCollection {
    // MultiPoint is similar to Grid but with explicit point coordinates
    convert_grid_coverage(coverage)
}

// =============================================================================
// Helper functions for extracting values from CoverageJSON axes
// =============================================================================

/// Get single x, y coordinates from axes.
fn get_single_xy(axes: &HashMap<String, Axis>) -> (f64, f64) {
    let x = get_first_axis_float(axes, "x").unwrap_or(0.0);
    let y = get_first_axis_float(axes, "y").unwrap_or(0.0);
    (x, y)
}

/// Get single time value from axes.
fn get_single_time(axes: &HashMap<String, Axis>) -> Option<String> {
    get_first_axis_string(axes, "t")
}

/// Get single z value from axes.
fn get_single_z(axes: &HashMap<String, Axis>) -> Option<f64> {
    get_first_axis_float(axes, "z")
}

/// Get all time values from axes.
fn get_time_values(axes: &HashMap<String, Axis>) -> Vec<String> {
    get_axis_string_values(axes, "t")
}

/// Get all z values from axes.
fn get_z_values(axes: &HashMap<String, Axis>) -> Vec<f64> {
    get_axis_float_values(axes, "x")
        .is_empty()
        .then(|| Vec::new())
        .unwrap_or_else(|| get_axis_float_values(axes, "z"))
}

/// Get the first float value from an axis.
fn get_first_axis_float(axes: &HashMap<String, Axis>, name: &str) -> Option<f64> {
    match axes.get(name)? {
        Axis::Values { values } => values.first().and_then(|v| match v {
            AxisValue::Float(f) => Some(*f),
            _ => None,
        }),
        Axis::Regular { start, .. } => Some(*start),
        Axis::Composite(_) => None,
    }
}

/// Get the first string value from an axis.
fn get_first_axis_string(axes: &HashMap<String, Axis>, name: &str) -> Option<String> {
    match axes.get(name)? {
        Axis::Values { values } => values.first().and_then(|v| match v {
            AxisValue::String(s) => Some(s.clone()),
            _ => None,
        }),
        _ => None,
    }
}

/// Get all float values from an axis.
fn get_axis_float_values(axes: &HashMap<String, Axis>, name: &str) -> Vec<f64> {
    match axes.get(name) {
        Some(Axis::Values { values }) => values
            .iter()
            .filter_map(|v| match v {
                AxisValue::Float(f) => Some(*f),
                _ => None,
            })
            .collect(),
        Some(Axis::Regular { start, stop, num }) => {
            if *num <= 1 {
                return vec![*start];
            }
            let step = (stop - start) / (*num - 1) as f64;
            (0..*num).map(|i| start + step * i as f64).collect()
        }
        _ => Vec::new(),
    }
}

/// Get all string values from an axis.
fn get_axis_string_values(axes: &HashMap<String, Axis>, name: &str) -> Vec<String> {
    match axes.get(name) {
        Some(Axis::Values { values }) => values
            .iter()
            .filter_map(|v| match v {
                AxisValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage_json::CovJsonParameter;
    use crate::parameters::Unit;

    #[test]
    fn test_feature_collection_new() {
        let fc = EdrFeatureCollection::new();
        assert_eq!(fc.type_, "FeatureCollection");
        assert!(fc.features.is_empty());
    }

    #[test]
    fn test_feature_point() {
        let feature = EdrFeature::point(-97.5, 35.2);
        assert_eq!(feature.type_, "Feature");
        match feature.geometry {
            EdrGeometry::Point { coordinates } => {
                assert_eq!(coordinates[0], -97.5);
                assert_eq!(coordinates[1], 35.2);
            }
            _ => panic!("Expected Point geometry"),
        }
    }

    #[test]
    fn test_properties_builder() {
        let props = EdrProperties::new()
            .with_datetime("2024-12-29T12:00:00Z")
            .with_z(850.0)
            .with_parameter("TMP", ParameterValue::with_unit(Some(288.5), "K"));

        assert_eq!(props.datetime, Some("2024-12-29T12:00:00Z".to_string()));
        assert_eq!(props.z, Some(850.0));

        let params = props.parameters.unwrap();
        let tmp = params.get("TMP").unwrap();
        assert_eq!(tmp.value, Some(288.5));
        assert_eq!(tmp.unit, Some("K".to_string()));
    }

    #[test]
    fn test_convert_point_coverage() {
        let param = CovJsonParameter::new("Temperature").with_unit(Unit::kelvin());
        let coverage = CoverageJson::point(
            -97.5,
            35.2,
            Some("2024-12-29T12:00:00Z".to_string()),
            Some(2.0),
        )
        .with_parameter("TMP", param, 288.5);

        let fc = EdrFeatureCollection::from(&coverage);

        assert_eq!(fc.features.len(), 1);
        let feature = &fc.features[0];

        match &feature.geometry {
            EdrGeometry::Point { coordinates } => {
                assert_eq!(coordinates[0], -97.5);
                assert_eq!(coordinates[1], 35.2);
            }
            _ => panic!("Expected Point geometry"),
        }

        assert_eq!(
            feature.properties.datetime,
            Some("2024-12-29T12:00:00Z".to_string())
        );
        assert_eq!(feature.properties.z, Some(2.0));

        let params = feature.properties.parameters.as_ref().unwrap();
        let tmp = params.get("TMP").unwrap();
        assert_eq!(tmp.value, Some(288.5));
    }

    #[test]
    fn test_convert_point_series_coverage() {
        let times = vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
            "2024-12-29T14:00:00Z".to_string(),
        ];

        let param = CovJsonParameter::new("Temperature").with_unit(Unit::kelvin());
        let values = vec![Some(288.5), Some(289.0), Some(289.5)];

        let coverage = CoverageJson::point_series(-97.5, 35.2, times, Some(2.0))
            .with_time_series("TMP", param, values);

        let fc = EdrFeatureCollection::from(&coverage);

        assert_eq!(fc.features.len(), 3);

        // Check first feature
        assert_eq!(
            fc.features[0].properties.datetime,
            Some("2024-12-29T12:00:00Z".to_string())
        );
        let params = fc.features[0].properties.parameters.as_ref().unwrap();
        assert_eq!(params.get("TMP").unwrap().value, Some(288.5));

        // Check third feature
        assert_eq!(
            fc.features[2].properties.datetime,
            Some("2024-12-29T14:00:00Z".to_string())
        );
        let params = fc.features[2].properties.parameters.as_ref().unwrap();
        assert_eq!(params.get("TMP").unwrap().value, Some(289.5));
    }

    #[test]
    fn test_geojson_serialization() {
        let feature = EdrFeature::point(-97.5, 35.2)
            .with_id("test-1")
            .with_properties(
                EdrProperties::new()
                    .with_datetime("2024-12-29T12:00:00Z")
                    .with_parameter("TMP", ParameterValue::with_unit(Some(288.5), "K")),
            );

        let fc = EdrFeatureCollection::new().with_feature(feature);

        let json = serde_json::to_string_pretty(&fc).unwrap();

        assert!(
            json.contains("\"type\": \"FeatureCollection\"")
                || json.contains("\"type\":\"FeatureCollection\"")
        );
        assert!(json.contains("\"type\": \"Feature\"") || json.contains("\"type\":\"Feature\""));
        assert!(json.contains("\"type\": \"Point\"") || json.contains("\"type\":\"Point\""));
        assert!(json.contains("-97.5"));
        assert!(json.contains("35.2"));
        assert!(json.contains("288.5"));
    }

    #[test]
    fn test_coverage_collection_conversion() {
        let cov1 = CoverageJson::point(-97.5, 35.2, Some("2024-12-29T12:00:00Z".to_string()), None);
        let cov2 = CoverageJson::point(-98.0, 36.0, Some("2024-12-29T12:00:00Z".to_string()), None);

        let collection = CoverageCollection::new()
            .with_coverage(cov1)
            .with_coverage(cov2);

        let fc = EdrFeatureCollection::from(&collection);

        assert_eq!(fc.features.len(), 2);
    }
}
