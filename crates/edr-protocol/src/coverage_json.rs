//! CoverageJSON types for EDR query responses.
//!
//! CoverageJSON is the primary response format for EDR data queries.
//! It provides a structured way to represent coverage data with
//! domain, parameters, and ranges.
//!
//! See: <https://covjson.org/>

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::parameters::{I18nString, ObservedProperty, Parameter, Unit};

/// A CoverageJSON document containing coverage data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoverageJson {
    /// Document type (always "Coverage" for single coverage).
    #[serde(rename = "type")]
    pub type_: CoverageType,

    /// The domain defining the coverage's spatial/temporal extent.
    pub domain: Domain,

    /// Parameter definitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, CovJsonParameter>>,

    /// Data ranges for each parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranges: Option<HashMap<String, NdArray>>,
}

impl CoverageJson {
    /// Create a new CoverageJSON document for a point.
    pub fn point(x: f64, y: f64, t: Option<String>, z: Option<f64>) -> Self {
        Self {
            type_: CoverageType::Coverage,
            domain: Domain::point(x, y, t, z),
            parameters: Some(HashMap::new()),
            ranges: Some(HashMap::new()),
        }
    }

    /// Create a new CoverageJSON document for a point series (time series at a point).
    pub fn point_series(x: f64, y: f64, t_values: Vec<String>, z: Option<f64>) -> Self {
        Self {
            type_: CoverageType::Coverage,
            domain: Domain::point_series(x, y, t_values, z),
            parameters: Some(HashMap::new()),
            ranges: Some(HashMap::new()),
        }
    }

    /// Add a parameter with its value.
    pub fn with_parameter(mut self, name: &str, param: CovJsonParameter, value: f32) -> Self {
        if let Some(ref mut params) = self.parameters {
            params.insert(name.to_string(), param);
        }

        if let Some(ref mut ranges) = self.ranges {
            ranges.insert(name.to_string(), NdArray::scalar(value));
        }

        self
    }

    /// Add a parameter with a null (missing) value.
    pub fn with_parameter_null(mut self, name: &str, param: CovJsonParameter) -> Self {
        if let Some(ref mut params) = self.parameters {
            params.insert(name.to_string(), param);
        }

        if let Some(ref mut ranges) = self.ranges {
            ranges.insert(name.to_string(), NdArray::scalar_null());
        }

        self
    }

    /// Add a parameter with multiple values (e.g., for time series).
    pub fn with_parameter_array(
        mut self,
        name: &str,
        param: CovJsonParameter,
        values: Vec<f32>,
        shape: Vec<usize>,
        axis_names: Vec<String>,
    ) -> Self {
        if let Some(ref mut params) = self.parameters {
            params.insert(name.to_string(), param);
        }

        if let Some(ref mut ranges) = self.ranges {
            ranges.insert(name.to_string(), NdArray::new(values, shape, axis_names));
        }

        self
    }

    /// Add a parameter with multiple values that may include nulls.
    pub fn with_parameter_array_nullable(
        mut self,
        name: &str,
        param: CovJsonParameter,
        values: Vec<Option<f32>>,
        shape: Vec<usize>,
        axis_names: Vec<String>,
    ) -> Self {
        if let Some(ref mut params) = self.parameters {
            params.insert(name.to_string(), param);
        }

        if let Some(ref mut ranges) = self.ranges {
            ranges.insert(
                name.to_string(),
                NdArray::with_missing(values, shape, axis_names),
            );
        }

        self
    }

    /// Add a parameter for a time series (1D array along time axis).
    pub fn with_time_series(
        mut self,
        name: &str,
        param: CovJsonParameter,
        values: Vec<Option<f32>>,
    ) -> Self {
        if let Some(ref mut params) = self.parameters {
            params.insert(name.to_string(), param);
        }

        if let Some(ref mut ranges) = self.ranges {
            let shape = vec![values.len()];
            let axis_names = vec!["t".to_string()];
            ranges.insert(
                name.to_string(),
                NdArray::with_missing(values, shape, axis_names),
            );
        }

        self
    }
}

/// Coverage type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CoverageType {
    /// Single coverage.
    Coverage,
    /// Collection of coverages.
    CoverageCollection,
}

/// The domain of a coverage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Domain {
    /// Domain type (always "Domain").
    #[serde(rename = "type")]
    pub type_: String,

    /// The domain type (Point, Grid, etc.).
    #[serde(rename = "domainType")]
    pub domain_type: DomainType,

    /// Axis definitions.
    pub axes: HashMap<String, Axis>,

    /// Reference systems for axes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referencing: Option<Vec<ReferenceSystemConnection>>,
}

impl Domain {
    /// Create a point domain.
    pub fn point(x: f64, y: f64, t: Option<String>, z: Option<f64>) -> Self {
        let mut axes = HashMap::new();
        axes.insert("x".to_string(), Axis::Values(vec![AxisValue::Float(x)]));
        axes.insert("y".to_string(), Axis::Values(vec![AxisValue::Float(y)]));

        if let Some(t) = t {
            axes.insert("t".to_string(), Axis::Values(vec![AxisValue::String(t)]));
        }

        if let Some(z) = z {
            axes.insert("z".to_string(), Axis::Values(vec![AxisValue::Float(z)]));
        }

        let referencing = vec![ReferenceSystemConnection {
            coordinates: vec!["x".to_string(), "y".to_string()],
            system: ReferenceSystem::Geographic {
                id: "http://www.opengis.net/def/crs/EPSG/0/4326".to_string(),
            },
        }];

        Self {
            type_: "Domain".to_string(),
            domain_type: DomainType::Point,
            axes,
            referencing: Some(referencing),
        }
    }

    /// Create a point series domain (time series at a single point).
    pub fn point_series(x: f64, y: f64, t_values: Vec<String>, z: Option<f64>) -> Self {
        let mut axes = HashMap::new();
        axes.insert("x".to_string(), Axis::Values(vec![AxisValue::Float(x)]));
        axes.insert("y".to_string(), Axis::Values(vec![AxisValue::Float(y)]));

        // Time axis with multiple values
        axes.insert(
            "t".to_string(),
            Axis::Values(t_values.into_iter().map(AxisValue::String).collect()),
        );

        if let Some(z) = z {
            axes.insert("z".to_string(), Axis::Values(vec![AxisValue::Float(z)]));
        }

        let mut referencing = vec![ReferenceSystemConnection {
            coordinates: vec!["x".to_string(), "y".to_string()],
            system: ReferenceSystem::Geographic {
                id: "http://www.opengis.net/def/crs/EPSG/0/4326".to_string(),
            },
        }];

        // Add temporal reference system
        referencing.push(ReferenceSystemConnection {
            coordinates: vec!["t".to_string()],
            system: ReferenceSystem::Temporal {
                calendar: "Gregorian".to_string(),
            },
        });

        Self {
            type_: "Domain".to_string(),
            domain_type: DomainType::PointSeries,
            axes,
            referencing: Some(referencing),
        }
    }

    /// Create a grid domain.
    pub fn grid(
        x_values: Vec<f64>,
        y_values: Vec<f64>,
        t_values: Option<Vec<String>>,
        z_values: Option<Vec<f64>>,
    ) -> Self {
        let mut axes = HashMap::new();
        axes.insert(
            "x".to_string(),
            Axis::Values(x_values.into_iter().map(AxisValue::Float).collect()),
        );
        axes.insert(
            "y".to_string(),
            Axis::Values(y_values.into_iter().map(AxisValue::Float).collect()),
        );

        if let Some(t) = t_values {
            axes.insert(
                "t".to_string(),
                Axis::Values(t.into_iter().map(AxisValue::String).collect()),
            );
        }

        if let Some(z) = z_values {
            axes.insert(
                "z".to_string(),
                Axis::Values(z.into_iter().map(AxisValue::Float).collect()),
            );
        }

        let referencing = vec![ReferenceSystemConnection {
            coordinates: vec!["x".to_string(), "y".to_string()],
            system: ReferenceSystem::Geographic {
                id: "http://www.opengis.net/def/crs/EPSG/0/4326".to_string(),
            },
        }];

        Self {
            type_: "Domain".to_string(),
            domain_type: DomainType::Grid,
            axes,
            referencing: Some(referencing),
        }
    }
}

/// Domain types supported by CoverageJSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DomainType {
    /// Point (0D).
    Point,
    /// Point series (time series at a point).
    PointSeries,
    /// Vertical profile at a point.
    VerticalProfile,
    /// Grid (2D or higher).
    Grid,
    /// Trajectory (1D path through space).
    Trajectory,
    /// Multi-point set.
    MultiPoint,
}

/// An axis in the domain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Axis {
    /// Explicit list of values.
    Values(Vec<AxisValue>),
    /// Regular axis defined by start, stop, and number of points.
    Regular { start: f64, stop: f64, num: usize },
}

impl Axis {
    /// Get the number of values in this axis.
    pub fn len(&self) -> usize {
        match self {
            Axis::Values(v) => v.len(),
            Axis::Regular { num, .. } => *num,
        }
    }

    /// Check if axis is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A value on an axis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AxisValue {
    /// Floating-point value (coordinates, levels).
    Float(f64),
    /// String value (timestamps).
    String(String),
}

/// Connection between axes and their reference system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReferenceSystemConnection {
    /// Axes that use this reference system.
    pub coordinates: Vec<String>,

    /// The reference system.
    pub system: ReferenceSystem,
}

/// Reference system definitions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ReferenceSystem {
    /// Geographic coordinate reference system.
    #[serde(rename = "GeographicCRS")]
    Geographic {
        /// CRS identifier URI.
        id: String,
    },

    /// Temporal reference system.
    #[serde(rename = "TemporalRS")]
    Temporal {
        /// Calendar system (e.g., "Gregorian").
        calendar: String,
    },

    /// Vertical reference system.
    #[serde(rename = "VerticalCRS")]
    Vertical {
        /// CRS identifier URI.
        id: String,
    },

    /// Identifier-based reference system.
    #[serde(rename = "IdentifierRS")]
    Identifier {
        /// Target concept URI.
        #[serde(rename = "targetConcept")]
        target_concept: String,
    },
}

/// A parameter in CoverageJSON format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CovJsonParameter {
    /// Type (always "Parameter").
    #[serde(rename = "type")]
    pub type_: String,

    /// Description of the parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<I18nString>,

    /// The observed property.
    #[serde(rename = "observedProperty")]
    pub observed_property: ObservedProperty,

    /// Unit of measurement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<Unit>,
}

impl CovJsonParameter {
    /// Create a new CoverageJSON parameter.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            type_: "Parameter".to_string(),
            description: None,
            observed_property: ObservedProperty::new(label),
            unit: None,
        }
    }

    /// Set the unit.
    pub fn with_unit(mut self, unit: Unit) -> Self {
        self.unit = Some(unit);
        self
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(I18nString::english(&desc.into()));
        self
    }

    /// Convert from an EDR Parameter.
    pub fn from_parameter(param: &Parameter) -> Self {
        Self {
            type_: "Parameter".to_string(),
            description: param.description.clone(),
            observed_property: param.observed_property.clone(),
            unit: param.unit.clone(),
        }
    }
}

/// N-dimensional array containing data values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NdArray {
    /// Type (always "NdArray").
    #[serde(rename = "type")]
    pub type_: String,

    /// Data type of values.
    #[serde(rename = "dataType")]
    pub data_type: String,

    /// Names of axes in order.
    #[serde(rename = "axisNames", skip_serializing_if = "Option::is_none")]
    pub axis_names: Option<Vec<String>>,

    /// Shape of the array.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<Vec<usize>>,

    /// The data values (may contain null for missing data).
    pub values: Vec<Option<f32>>,
}

impl NdArray {
    /// Create a scalar (single value) array.
    pub fn scalar(value: f32) -> Self {
        Self {
            type_: "NdArray".to_string(),
            data_type: "float".to_string(),
            axis_names: None,
            shape: None,
            values: vec![Some(value)],
        }
    }

    /// Create a scalar null (missing value) array.
    pub fn scalar_null() -> Self {
        Self {
            type_: "NdArray".to_string(),
            data_type: "float".to_string(),
            axis_names: None,
            shape: None,
            values: vec![None],
        }
    }

    /// Create an N-dimensional array.
    pub fn new(values: Vec<f32>, shape: Vec<usize>, axis_names: Vec<String>) -> Self {
        Self {
            type_: "NdArray".to_string(),
            data_type: "float".to_string(),
            axis_names: Some(axis_names),
            shape: Some(shape),
            values: values.into_iter().map(Some).collect(),
        }
    }

    /// Create an array with missing data support.
    pub fn with_missing(
        values: Vec<Option<f32>>,
        shape: Vec<usize>,
        axis_names: Vec<String>,
    ) -> Self {
        Self {
            type_: "NdArray".to_string(),
            data_type: "float".to_string(),
            axis_names: Some(axis_names),
            shape: Some(shape),
            values,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_coverage() {
        let cov = CoverageJson::point(
            -97.5,
            35.2,
            Some("2024-12-29T12:00:00Z".to_string()),
            Some(2.0),
        );

        assert_eq!(cov.type_, CoverageType::Coverage);
        assert_eq!(cov.domain.domain_type, DomainType::Point);
        assert!(cov.domain.axes.contains_key("x"));
        assert!(cov.domain.axes.contains_key("y"));
        assert!(cov.domain.axes.contains_key("t"));
        assert!(cov.domain.axes.contains_key("z"));
    }

    #[test]
    fn test_coverage_with_parameter() {
        let param = CovJsonParameter::new("Temperature").with_unit(Unit::kelvin());

        let cov = CoverageJson::point(-97.5, 35.2, None, None).with_parameter("TMP", param, 288.5);

        let params = cov.parameters.unwrap();
        assert!(params.contains_key("TMP"));

        let ranges = cov.ranges.unwrap();
        assert!(ranges.contains_key("TMP"));
        assert_eq!(ranges["TMP"].values[0], Some(288.5));
    }

    #[test]
    fn test_coverage_serialization() {
        let param = CovJsonParameter::new("Temperature").with_unit(Unit::kelvin());

        let cov = CoverageJson::point(-97.5, 35.2, Some("2024-12-29T12:00:00Z".to_string()), None)
            .with_parameter("TMP", param, 288.5);

        let json = serde_json::to_string_pretty(&cov).unwrap();

        // Pretty-printed JSON has spaces after colons
        assert!(json.contains("\"type\": \"Coverage\"") || json.contains("\"type\":\"Coverage\""));
        assert!(
            json.contains("\"domainType\": \"Point\"") || json.contains("\"domainType\":\"Point\"")
        );
        assert!(json.contains("\"TMP\""));
        assert!(json.contains("288.5"));
    }

    #[test]
    fn test_domain_point() {
        let domain = Domain::point(
            -97.5,
            35.2,
            Some("2024-12-29T12:00:00Z".to_string()),
            Some(850.0),
        );

        assert_eq!(domain.domain_type, DomainType::Point);
        assert_eq!(domain.axes.len(), 4);

        let x = &domain.axes["x"];
        if let Axis::Values(values) = x {
            if let AxisValue::Float(v) = &values[0] {
                assert_eq!(*v, -97.5);
            } else {
                panic!("Expected float value");
            }
        } else {
            panic!("Expected Values axis");
        }
    }

    #[test]
    fn test_domain_grid() {
        let domain = Domain::grid(
            vec![-97.5, -97.4, -97.3],
            vec![35.1, 35.2, 35.3],
            None,
            Some(vec![850.0, 700.0, 500.0]),
        );

        assert_eq!(domain.domain_type, DomainType::Grid);
        assert_eq!(domain.axes["x"].len(), 3);
        assert_eq!(domain.axes["y"].len(), 3);
        assert_eq!(domain.axes["z"].len(), 3);
    }

    #[test]
    fn test_ndarray_scalar() {
        let arr = NdArray::scalar(288.5);
        assert_eq!(arr.values, vec![Some(288.5)]);
        assert!(arr.shape.is_none());
        assert!(arr.axis_names.is_none());
    }

    #[test]
    fn test_ndarray_multidim() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let arr = NdArray::new(
            values.clone(),
            vec![2, 3],
            vec!["y".to_string(), "x".to_string()],
        );

        assert_eq!(arr.shape, Some(vec![2, 3]));
        assert_eq!(arr.axis_names, Some(vec!["y".to_string(), "x".to_string()]));
        assert_eq!(arr.values.len(), 6);
    }

    #[test]
    fn test_ndarray_with_missing() {
        let values = vec![Some(1.0), None, Some(3.0), Some(4.0)];
        let arr = NdArray::with_missing(
            values.clone(),
            vec![2, 2],
            vec!["y".to_string(), "x".to_string()],
        );

        assert_eq!(arr.values[0], Some(1.0));
        assert_eq!(arr.values[1], None);
        assert_eq!(arr.values[2], Some(3.0));
    }

    #[test]
    fn test_reference_systems() {
        let geo = ReferenceSystem::Geographic {
            id: "http://www.opengis.net/def/crs/EPSG/0/4326".to_string(),
        };
        let json = serde_json::to_string(&geo).unwrap();
        assert!(json.contains("GeographicCRS"));

        let temporal = ReferenceSystem::Temporal {
            calendar: "Gregorian".to_string(),
        };
        let json = serde_json::to_string(&temporal).unwrap();
        assert!(json.contains("TemporalRS"));
        assert!(json.contains("Gregorian"));
    }

    #[test]
    fn test_covjson_parameter_from_edr_parameter() {
        let param = Parameter::new("TMP", "Temperature").with_unit(Unit::kelvin());

        let cov_param = CovJsonParameter::from_parameter(&param);
        assert_eq!(cov_param.type_, "Parameter");
        assert!(cov_param.unit.is_some());
    }

    #[test]
    fn test_full_coverage_roundtrip() {
        let param = CovJsonParameter::new("Temperature")
            .with_unit(Unit::kelvin())
            .with_description("Air temperature at 2m");

        let cov = CoverageJson::point(
            -97.5,
            35.2,
            Some("2024-12-29T12:00:00Z".to_string()),
            Some(2.0),
        )
        .with_parameter("TMP", param, 288.5);

        let json = serde_json::to_string(&cov).unwrap();
        let parsed: CoverageJson = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.type_, CoverageType::Coverage);
        assert_eq!(parsed.domain.domain_type, DomainType::Point);
    }

    #[test]
    fn test_point_series_coverage() {
        let times = vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
            "2024-12-29T14:00:00Z".to_string(),
        ];

        let cov = CoverageJson::point_series(-97.5, 35.2, times.clone(), Some(2.0));

        assert_eq!(cov.type_, CoverageType::Coverage);
        assert_eq!(cov.domain.domain_type, DomainType::PointSeries);
        assert_eq!(cov.domain.axes["t"].len(), 3);
    }

    #[test]
    fn test_point_series_with_time_series_data() {
        let times = vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
            "2024-12-29T14:00:00Z".to_string(),
        ];

        let param = CovJsonParameter::new("Temperature").with_unit(Unit::kelvin());
        let values = vec![Some(288.5), Some(289.0), Some(289.5)];

        let cov = CoverageJson::point_series(-97.5, 35.2, times, Some(2.0))
            .with_time_series("TMP", param, values);

        let ranges = cov.ranges.unwrap();
        assert!(ranges.contains_key("TMP"));
        assert_eq!(ranges["TMP"].values.len(), 3);
        assert_eq!(ranges["TMP"].shape, Some(vec![3]));
        assert_eq!(ranges["TMP"].axis_names, Some(vec!["t".to_string()]));
    }

    #[test]
    fn test_point_series_with_nulls() {
        let times = vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
            "2024-12-29T14:00:00Z".to_string(),
        ];

        let param = CovJsonParameter::new("Temperature").with_unit(Unit::kelvin());
        let values = vec![Some(288.5), None, Some(289.5)]; // Middle value is missing

        let cov = CoverageJson::point_series(-97.5, 35.2, times, None)
            .with_time_series("TMP", param, values);

        let ranges = cov.ranges.unwrap();
        assert_eq!(ranges["TMP"].values[0], Some(288.5));
        assert_eq!(ranges["TMP"].values[1], None);
        assert_eq!(ranges["TMP"].values[2], Some(289.5));
    }

    #[test]
    fn test_point_series_serialization() {
        let times = vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
        ];

        let param = CovJsonParameter::new("Temperature").with_unit(Unit::kelvin());
        let values = vec![Some(288.5), Some(289.0)];

        let cov = CoverageJson::point_series(-97.5, 35.2, times, None)
            .with_time_series("TMP", param, values);

        let json = serde_json::to_string_pretty(&cov).unwrap();
        assert!(
            json.contains("\"domainType\": \"PointSeries\"")
                || json.contains("\"domainType\":\"PointSeries\"")
        );

        // Verify it roundtrips
        let parsed: CoverageJson = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.domain.domain_type, DomainType::PointSeries);
    }

    #[test]
    fn test_domain_point_series() {
        let domain = Domain::point_series(
            -97.5,
            35.2,
            vec![
                "2024-12-29T12:00:00Z".to_string(),
                "2024-12-29T13:00:00Z".to_string(),
            ],
            Some(850.0),
        );

        assert_eq!(domain.domain_type, DomainType::PointSeries);
        assert_eq!(domain.axes.len(), 4); // x, y, t, z
        assert_eq!(domain.axes["t"].len(), 2);

        // Check referencing includes temporal RS
        let refs = domain.referencing.unwrap();
        assert!(refs
            .iter()
            .any(|r| r.coordinates.contains(&"t".to_string())));
    }
}
