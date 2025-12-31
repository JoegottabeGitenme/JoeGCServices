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

    /// Create a new CoverageJSON document for a vertical profile (multiple z levels at a point).
    pub fn vertical_profile(x: f64, y: f64, t: Option<String>, z_values: Vec<f64>) -> Self {
        Self {
            type_: CoverageType::Coverage,
            domain: Domain::vertical_profile(x, y, t, z_values),
            parameters: Some(HashMap::new()),
            ranges: Some(HashMap::new()),
        }
    }

    /// Add a parameter with values for a vertical profile (1D array along z axis).
    pub fn with_vertical_profile_data(
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
            let axis_names = vec!["z".to_string()];
            ranges.insert(
                name.to_string(),
                NdArray::with_missing(values, shape, axis_names),
            );
        }

        self
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

/// A CoverageJSON Collection containing multiple coverages.
/// Used for MULTIPOINT queries where each point returns a separate coverage,
/// or for corridor queries where multiple trajectories are returned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoverageCollection {
    /// Document type (always "CoverageCollection").
    #[serde(rename = "type")]
    pub type_: CoverageType,

    /// The domain type shared by all coverages in the collection.
    /// For corridor queries, this is typically "Trajectory".
    #[serde(rename = "domainType", skip_serializing_if = "Option::is_none")]
    pub domain_type: Option<DomainType>,

    /// Collection of individual coverages.
    pub coverages: Vec<CoverageJson>,

    /// Shared parameter definitions (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, CovJsonParameter>>,

    /// Reference systems used by the coverages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referencing: Option<Vec<ReferenceSystemConnection>>,
}

impl CoverageCollection {
    /// Create a new empty CoverageCollection.
    pub fn new() -> Self {
        Self {
            type_: CoverageType::CoverageCollection,
            domain_type: None,
            coverages: Vec::new(),
            parameters: None,
            referencing: None,
        }
    }

    /// Set the domain type for this collection.
    pub fn with_domain_type(mut self, domain_type: DomainType) -> Self {
        self.domain_type = Some(domain_type);
        self
    }

    /// Add a coverage to the collection.
    pub fn with_coverage(mut self, coverage: CoverageJson) -> Self {
        self.coverages.push(coverage);
        self
    }

    /// Set the shared parameters.
    pub fn with_parameters(mut self, params: HashMap<String, CovJsonParameter>) -> Self {
        self.parameters = Some(params);
        self
    }

    /// Set the referencing systems.
    pub fn with_referencing(mut self, refs: Vec<ReferenceSystemConnection>) -> Self {
        self.referencing = Some(refs);
        self
    }
}

impl Default for CoverageCollection {
    fn default() -> Self {
        Self::new()
    }
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
        axes.insert("x".to_string(), Axis::Values { values: vec![AxisValue::Float(x)] });
        axes.insert("y".to_string(), Axis::Values { values: vec![AxisValue::Float(y)] });

        if let Some(t) = t {
            axes.insert("t".to_string(), Axis::Values { values: vec![AxisValue::String(t)] });
        }

        if let Some(z) = z {
            axes.insert("z".to_string(), Axis::Values { values: vec![AxisValue::Float(z)] });
        }

        let referencing = vec![ReferenceSystemConnection {
            coordinates: vec!["x".to_string(), "y".to_string()],
            system: ReferenceSystem::geographic("http://www.opengis.net/def/crs/EPSG/0/4326"),
        }];

        Self {
            type_: "Domain".to_string(),
            domain_type: DomainType::Point,
            axes,
            referencing: Some(referencing),
        }
    }

    /// Create a point domain with multiple z values (vertical profile at a point).
    pub fn vertical_profile(x: f64, y: f64, t: Option<String>, z_values: Vec<f64>) -> Self {
        let mut axes = HashMap::new();
        axes.insert("x".to_string(), Axis::Values { values: vec![AxisValue::Float(x)] });
        axes.insert("y".to_string(), Axis::Values { values: vec![AxisValue::Float(y)] });

        if let Some(t) = t {
            axes.insert("t".to_string(), Axis::Values { values: vec![AxisValue::String(t)] });
        }

        // Multiple z values for vertical profile
        axes.insert(
            "z".to_string(),
            Axis::Values { values: z_values.into_iter().map(AxisValue::Float).collect() },
        );

        let mut referencing = vec![ReferenceSystemConnection {
            coordinates: vec!["x".to_string(), "y".to_string()],
            system: ReferenceSystem::geographic("http://www.opengis.net/def/crs/EPSG/0/4326"),
        }];

        // Add vertical reference system
        referencing.push(ReferenceSystemConnection {
            coordinates: vec!["z".to_string()],
            system: ReferenceSystem::vertical("http://www.opengis.net/def/crs/OGC/0/Unknown"),
        });

        Self {
            type_: "Domain".to_string(),
            domain_type: DomainType::VerticalProfile,
            axes,
            referencing: Some(referencing),
        }
    }

    /// Create a point series domain (time series at a single point).
    pub fn point_series(x: f64, y: f64, t_values: Vec<String>, z: Option<f64>) -> Self {
        let mut axes = HashMap::new();
        axes.insert("x".to_string(), Axis::Values { values: vec![AxisValue::Float(x)] });
        axes.insert("y".to_string(), Axis::Values { values: vec![AxisValue::Float(y)] });

        // Time axis with multiple values
        axes.insert(
            "t".to_string(),
            Axis::Values { values: t_values.into_iter().map(AxisValue::String).collect() },
        );

        if let Some(z) = z {
            axes.insert("z".to_string(), Axis::Values { values: vec![AxisValue::Float(z)] });
        }

        let mut referencing = vec![ReferenceSystemConnection {
            coordinates: vec!["x".to_string(), "y".to_string()],
            system: ReferenceSystem::geographic("http://www.opengis.net/def/crs/EPSG/0/4326"),
        }];

        // Add temporal reference system
        referencing.push(ReferenceSystemConnection {
            coordinates: vec!["t".to_string()],
            system: ReferenceSystem::temporal_gregorian(),
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
            Axis::Values { values: x_values.into_iter().map(AxisValue::Float).collect() },
        );
        axes.insert(
            "y".to_string(),
            Axis::Values { values: y_values.into_iter().map(AxisValue::Float).collect() },
        );

        if let Some(t) = t_values {
            axes.insert(
                "t".to_string(),
                Axis::Values { values: t.into_iter().map(AxisValue::String).collect() },
            );
        }

        if let Some(z) = z_values {
            axes.insert(
                "z".to_string(),
                Axis::Values { values: z.into_iter().map(AxisValue::Float).collect() },
            );
        }

        let referencing = vec![ReferenceSystemConnection {
            coordinates: vec!["x".to_string(), "y".to_string()],
            system: ReferenceSystem::geographic("http://www.opengis.net/def/crs/EPSG/0/4326"),
        }];

        Self {
            type_: "Domain".to_string(),
            domain_type: DomainType::Grid,
            axes,
            referencing: Some(referencing),
        }
    }

    /// Create a trajectory domain.
    ///
    /// A trajectory is a path through space (and optionally time and/or vertical levels).
    /// The x (longitude) and y (latitude) coordinates define the path, while t and z are optional.
    /// For CoverageJSON Trajectory domain, uses a composite axis with tuple values.
    ///
    /// Per CoverageJSON spec, trajectory domains use a single "composite" axis
    /// containing tuples of [t, x, y] or [t, x, y, z] values (lon/lat order).
    pub fn trajectory(
        x_values: Vec<f64>,
        y_values: Vec<f64>,
        t_values: Option<Vec<String>>,
        z_values: Option<Vec<f64>>,
    ) -> Self {
        let mut axes = HashMap::new();
        let num_points = x_values.len();

        // Determine which coordinates we have
        let has_t = t_values.is_some();
        let has_z = z_values.is_some();

        // Build coordinate names for the composite axis
        // Order: t (if present), x (lon), y (lat), z (if present)
        let mut coord_names = Vec::new();
        if has_t {
            coord_names.push("t".to_string());
        }
        coord_names.push("x".to_string()); // longitude
        coord_names.push("y".to_string()); // latitude
        if has_z {
            coord_names.push("z".to_string());
        }

        // Build tuple values for each point
        let mut tuple_values: Vec<Vec<CompositeValue>> = Vec::with_capacity(num_points);

        // Get time values - either one per point or one for all
        let t_vec = t_values.unwrap_or_default();
        let single_time = t_vec.len() == 1;

        // Get z values
        let z_vec = z_values.unwrap_or_default();
        let single_z = z_vec.len() == 1;

        for i in 0..num_points {
            let mut point: Vec<CompositeValue> = Vec::new();

            // Add time if present
            if has_t {
                let t_idx = if single_time { 0 } else { i.min(t_vec.len() - 1) };
                point.push(CompositeValue::String(t_vec[t_idx].clone()));
            }

            // Add values in lat/lon order (y first, then x) for correct map display
            // The labels say "x", "y" but viewers expect lat, lon order in the values
            point.push(CompositeValue::Float(y_values[i])); // latitude
            point.push(CompositeValue::Float(x_values[i])); // longitude

            // Add z if present
            if has_z {
                let z_idx = if single_z { 0 } else { i.min(z_vec.len() - 1) };
                point.push(CompositeValue::Float(z_vec[z_idx]));
            }

            tuple_values.push(point);
        }

        // Create the composite axis
        let composite = CompositeAxis::new(coord_names.clone(), tuple_values);
        axes.insert("composite".to_string(), Axis::Composite(composite));

        // Build referencing - reference the coordinates within the composite
        let mut referencing = vec![ReferenceSystemConnection {
            coordinates: vec!["x".to_string(), "y".to_string()],
            system: ReferenceSystem::geographic("http://www.opengis.net/def/crs/EPSG/0/4326"),
        }];

        if has_t {
            referencing.push(ReferenceSystemConnection {
                coordinates: vec!["t".to_string()],
                system: ReferenceSystem::temporal_gregorian(),
            });
        }

        if has_z {
            referencing.push(ReferenceSystemConnection {
                coordinates: vec!["z".to_string()],
                system: ReferenceSystem::vertical("http://www.opengis.net/def/crs/OGC/0/Unknown"),
            });
        }

        Self {
            type_: "Domain".to_string(),
            domain_type: DomainType::Trajectory,
            axes,
            referencing: Some(referencing),
        }
    }

    /// Create a cube grid domain with regular axes (start/stop/num format).
    ///
    /// This is used for cube queries where we want to express the grid as a bounding box
    /// with a specific number of grid points.
    pub fn cube_grid(
        x_start: f64,
        x_stop: f64,
        x_num: usize,
        y_start: f64,
        y_stop: f64,
        y_num: usize,
        t_value: Option<String>,
        z_value: f64,
    ) -> Self {
        let mut axes = HashMap::new();

        // Use Regular axis format for x and y
        axes.insert(
            "x".to_string(),
            Axis::Regular {
                start: x_start,
                stop: x_stop,
                num: x_num,
            },
        );
        axes.insert(
            "y".to_string(),
            Axis::Regular {
                start: y_start,
                stop: y_stop,
                num: y_num,
            },
        );

        // Z is a single value for each coverage in the collection
        axes.insert(
            "z".to_string(),
            Axis::Values { values: vec![AxisValue::Float(z_value)] },
        );

        // Time if provided
        if let Some(t) = t_value {
            axes.insert(
                "t".to_string(),
                Axis::Values { values: vec![AxisValue::String(t)] },
            );
        }

        Self {
            type_: "Domain".to_string(),
            domain_type: DomainType::Grid,
            axes,
            referencing: None, // Referencing is at collection level for cube
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
    /// Explicit list of values with "values" key.
    Values {
        values: Vec<AxisValue>,
    },
    /// Regular axis defined by start, stop, and number of points.
    Regular { start: f64, stop: f64, num: usize },
    /// Composite axis for trajectory domains (tuple of coordinates).
    Composite(CompositeAxis),
}

impl Axis {
    /// Get the number of values in this axis.
    pub fn len(&self) -> usize {
        match self {
            Axis::Values { values } => values.len(),
            Axis::Regular { num, .. } => *num,
            Axis::Composite(c) => c.values.len(),
        }
    }

    /// Check if axis is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A composite axis for trajectory domains.
/// Each value is a tuple of coordinates (e.g., [t, x, y] or [t, x, y, z]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompositeAxis {
    /// Data type - always "tuple" for composite axes.
    #[serde(rename = "dataType")]
    pub data_type: String,

    /// The coordinate names in order (e.g., ["t", "x", "y", "z"]).
    pub coordinates: Vec<String>,

    /// The tuple values - each inner vec is one point with values in coordinate order.
    pub values: Vec<Vec<CompositeValue>>,
}

impl CompositeAxis {
    /// Create a new composite axis.
    pub fn new(coordinates: Vec<String>, values: Vec<Vec<CompositeValue>>) -> Self {
        Self {
            data_type: "tuple".to_string(),
            coordinates,
            values,
        }
    }
}

/// A value in a composite axis tuple - can be string (time) or float (coordinates).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum CompositeValue {
    /// String value (timestamps).
    String(String),
    /// Floating-point value (coordinates, levels).
    Float(f64),
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
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Optional WKT representation.
        #[serde(skip_serializing_if = "Option::is_none")]
        wkt: Option<String>,
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
        /// CRS identifier URI (optional if cs is provided).
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Coordinate system definition (optional).
        #[serde(skip_serializing_if = "Option::is_none")]
        cs: Option<VerticalCoordinateSystem>,
    },

    /// Identifier-based reference system.
    #[serde(rename = "IdentifierRS")]
    Identifier {
        /// Target concept URI.
        #[serde(rename = "targetConcept")]
        target_concept: String,
    },
}

impl ReferenceSystem {
    /// Create a simple geographic CRS with just an ID.
    pub fn geographic(id: &str) -> Self {
        ReferenceSystem::Geographic {
            id: Some(id.to_string()),
            wkt: None,
        }
    }

    /// Create a geographic CRS with ID and WKT.
    pub fn geographic_with_wkt(id: &str, wkt: &str) -> Self {
        ReferenceSystem::Geographic {
            id: Some(id.to_string()),
            wkt: Some(wkt.to_string()),
        }
    }

    /// Create a temporal CRS with Gregorian calendar.
    pub fn temporal_gregorian() -> Self {
        ReferenceSystem::Temporal {
            calendar: "Gregorian".to_string(),
        }
    }

    /// Create a simple vertical CRS with just an ID.
    pub fn vertical(id: &str) -> Self {
        ReferenceSystem::Vertical {
            id: Some(id.to_string()),
            cs: None,
        }
    }

    /// Create a vertical CRS with coordinate system details.
    pub fn vertical_with_cs(cs: VerticalCoordinateSystem) -> Self {
        ReferenceSystem::Vertical { id: None, cs: Some(cs) }
    }
}

/// Vertical coordinate system definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerticalCoordinateSystem {
    /// Coordinate system axes.
    #[serde(rename = "csAxes")]
    pub cs_axes: Vec<VerticalCSAxis>,
}

impl VerticalCoordinateSystem {
    /// Create a vertical coordinate system for isobaric (pressure) levels.
    pub fn isobaric() -> Self {
        Self {
            cs_axes: vec![VerticalCSAxis {
                name: I18nString::english("Isobaric level"),
                direction: "down".to_string(),
                unit: VerticalUnit {
                    symbol: "hPa".to_string(),
                },
            }],
        }
    }

    /// Create a vertical coordinate system for height above ground.
    pub fn height_above_ground() -> Self {
        Self {
            cs_axes: vec![VerticalCSAxis {
                name: I18nString::english("Height above ground"),
                direction: "up".to_string(),
                unit: VerticalUnit {
                    symbol: "m".to_string(),
                },
            }],
        }
    }
}

/// An axis in a vertical coordinate system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerticalCSAxis {
    /// Name of the axis.
    pub name: I18nString,
    /// Direction of positive values (up, down).
    pub direction: String,
    /// Unit of measurement.
    pub unit: VerticalUnit,
}

/// Unit for vertical axis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerticalUnit {
    /// Unit symbol (e.g., "hPa", "m").
    pub symbol: String,
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
        if let Axis::Values { values } = x {
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
        let geo = ReferenceSystem::geographic("http://www.opengis.net/def/crs/EPSG/0/4326");
        let json = serde_json::to_string(&geo).unwrap();
        assert!(json.contains("GeographicCRS"));

        let temporal = ReferenceSystem::temporal_gregorian();
        let json = serde_json::to_string(&temporal).unwrap();
        assert!(json.contains("TemporalRS"));
        assert!(json.contains("Gregorian"));

        // Test vertical with CS
        let vertical = ReferenceSystem::vertical_with_cs(VerticalCoordinateSystem::isobaric());
        let json = serde_json::to_string(&vertical).unwrap();
        assert!(json.contains("VerticalCRS"));
        assert!(json.contains("hPa"));
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
