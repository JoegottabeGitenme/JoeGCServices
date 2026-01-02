//! Parameter metadata types for EDR collections.
//!
//! Parameters describe the data variables available within a collection,
//! including their units, observed properties, and descriptive metadata.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A parameter (observed property) available in a collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Parameter {
    /// The type of parameter (always "Parameter").
    #[serde(rename = "type")]
    pub type_: String,

    /// Unique identifier for the parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Human-readable label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Multi-language description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<I18nString>,

    /// The observed property.
    #[serde(rename = "observedProperty")]
    pub observed_property: ObservedProperty,

    /// Unit of measurement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<Unit>,

    /// Available vertical levels for this parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extent: Option<ParameterExtent>,
}

impl Parameter {
    /// Create a new parameter.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        let id = id.into();
        let label = label.into();
        Self {
            type_: "Parameter".to_string(),
            id: Some(id.clone()),
            label: Some(label.clone()),
            description: None,
            observed_property: ObservedProperty {
                id: None,
                label: Some(I18nString::english(&label)),
                description: None,
                categories: None,
            },
            unit: None,
            extent: None,
        }
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(I18nString::english(&desc.into()));
        self
    }

    /// Set the unit.
    pub fn with_unit(mut self, unit: Unit) -> Self {
        self.unit = Some(unit);
        self
    }

    /// Set the unit from symbol.
    pub fn with_unit_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.unit = Some(Unit::from_symbol(symbol));
        self
    }

    /// Set available levels.
    pub fn with_levels(mut self, levels: Vec<f64>) -> Self {
        self.extent = Some(ParameterExtent {
            vertical: Some(VerticalParameterExtent {
                values: levels,
                vrs: None,
            }),
        });
        self
    }
}

/// Internationalized string supporting multiple languages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum I18nString {
    /// Simple string (assumes English).
    Simple(String),
    /// Map of language codes to strings.
    Localized(HashMap<String, String>),
}

impl I18nString {
    /// Create an English-only i18n string.
    pub fn english(s: &str) -> Self {
        let mut map = HashMap::new();
        map.insert("en".to_string(), s.to_string());
        I18nString::Localized(map)
    }

    /// Get the English text, or any available text.
    pub fn text(&self) -> &str {
        match self {
            I18nString::Simple(s) => s,
            I18nString::Localized(map) => map
                .get("en")
                .map(|s| s.as_str())
                .unwrap_or_else(|| map.values().next().map(|s| s.as_str()).unwrap_or("")),
        }
    }
}

/// The observed property being measured.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObservedProperty {
    /// URI identifier for the property.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Human-readable label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<I18nString>,

    /// Description of the property.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<I18nString>,

    /// Categories for categorical data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<Category>>,
}

impl ObservedProperty {
    /// Create a new observed property with a label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: None,
            label: Some(I18nString::english(&label.into())),
            description: None,
            categories: None,
        }
    }

    /// Set the ID.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(I18nString::english(&desc.into()));
        self
    }
}

/// A category for categorical observed properties.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Category {
    /// Category identifier.
    pub id: String,

    /// Human-readable label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<I18nString>,

    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<I18nString>,
}

/// Unit of measurement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Unit {
    /// Human-readable label for the unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<I18nString>,

    /// Symbol or abbreviation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<UnitSymbol>,
}

impl Unit {
    /// Create a unit from a symbol string.
    pub fn from_symbol(symbol: impl Into<String>) -> Self {
        Self {
            label: None,
            symbol: Some(UnitSymbol::Simple(symbol.into())),
        }
    }

    /// Create a unit with label and symbol.
    pub fn new(label: impl Into<String>, symbol: impl Into<String>) -> Self {
        Self {
            label: Some(I18nString::english(&label.into())),
            symbol: Some(UnitSymbol::Simple(symbol.into())),
        }
    }

    /// Common units
    pub fn kelvin() -> Self {
        Self::new("Kelvin", "K")
    }

    pub fn celsius() -> Self {
        Self::new("Celsius", "°C")
    }

    pub fn meters_per_second() -> Self {
        Self::new("Meters per second", "m/s")
    }

    pub fn pascals() -> Self {
        Self::new("Pascals", "Pa")
    }

    pub fn hectopascals() -> Self {
        Self::new("Hectopascals", "hPa")
    }

    pub fn percent() -> Self {
        Self::new("Percent", "%")
    }

    pub fn dbz() -> Self {
        Self::new("Decibels relative to Z", "dBZ")
    }

    pub fn joules_per_kg() -> Self {
        Self::new("Joules per kilogram", "J/kg")
    }

    pub fn kg_per_m2() -> Self {
        Self::new("Kilograms per square meter", "kg/m²")
    }

    pub fn meters() -> Self {
        Self::new("Meters", "m")
    }
}

/// Unit symbol representation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum UnitSymbol {
    /// Simple string symbol.
    Simple(String),
    /// Structured symbol with type and value.
    Structured {
        /// Symbol value.
        value: String,
        /// Symbol type (e.g., "<http://www.opengis.net/def/uom/UCUM/>").
        #[serde(rename = "type")]
        type_: Option<String>,
    },
}

impl UnitSymbol {
    /// Get the symbol string.
    pub fn value(&self) -> &str {
        match self {
            UnitSymbol::Simple(s) => s,
            UnitSymbol::Structured { value, .. } => value,
        }
    }
}

/// Parameter-specific extent (e.g., available vertical levels).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParameterExtent {
    /// Vertical levels available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical: Option<VerticalParameterExtent>,
}

/// Vertical extent for a parameter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerticalParameterExtent {
    /// Available level values.
    pub values: Vec<f64>,

    /// Vertical reference system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vrs: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_creation() {
        let param = Parameter::new("TMP", "Temperature");
        assert_eq!(param.type_, "Parameter");
        assert_eq!(param.id, Some("TMP".to_string()));
        assert_eq!(param.label, Some("Temperature".to_string()));
    }

    #[test]
    fn test_parameter_with_unit() {
        let param = Parameter::new("TMP", "Temperature")
            .with_unit(Unit::kelvin())
            .with_description("Air temperature");

        assert!(param.unit.is_some());
        assert!(param.description.is_some());

        let unit = param.unit.unwrap();
        assert_eq!(unit.symbol.unwrap().value(), "K");
    }

    #[test]
    fn test_parameter_with_levels() {
        let param = Parameter::new("TMP", "Temperature")
            .with_levels(vec![1000.0, 850.0, 700.0, 500.0, 300.0, 250.0]);

        assert!(param.extent.is_some());
        let extent = param.extent.unwrap();
        let vertical = extent.vertical.unwrap();
        assert_eq!(vertical.values.len(), 6);
        assert_eq!(vertical.values[0], 1000.0);
    }

    #[test]
    fn test_parameter_serialization() {
        let param = Parameter::new("TMP", "Temperature").with_unit(Unit::kelvin());

        let json = serde_json::to_string_pretty(&param).unwrap();
        // Pretty-printed JSON has spaces after colons
        assert!(
            json.contains("\"type\": \"Parameter\"") || json.contains("\"type\":\"Parameter\"")
        );
        assert!(json.contains("\"id\": \"TMP\"") || json.contains("\"id\":\"TMP\""));
        assert!(json.contains("\"observedProperty\""));
    }

    #[test]
    fn test_i18n_string_english() {
        let s = I18nString::english("Temperature");
        assert_eq!(s.text(), "Temperature");

        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"en\":\"Temperature\""));
    }

    #[test]
    fn test_i18n_string_simple() {
        let s = I18nString::Simple("Temperature".to_string());
        assert_eq!(s.text(), "Temperature");
    }

    #[test]
    fn test_observed_property() {
        let prop = ObservedProperty::new("Temperature")
            .with_id("http://vocab.nerc.ac.uk/collection/P07/current/CFSN0023/")
            .with_description("Air temperature measured at a given location");

        assert!(prop.id.is_some());
        assert!(prop.label.is_some());
        assert!(prop.description.is_some());
    }

    #[test]
    fn test_unit_presets() {
        let kelvin = Unit::kelvin();
        assert_eq!(kelvin.symbol.as_ref().unwrap().value(), "K");

        let mps = Unit::meters_per_second();
        assert_eq!(mps.symbol.as_ref().unwrap().value(), "m/s");

        let dbz = Unit::dbz();
        assert_eq!(dbz.symbol.as_ref().unwrap().value(), "dBZ");
    }

    #[test]
    fn test_unit_serialization() {
        let unit = Unit::kelvin();
        let json = serde_json::to_string(&unit).unwrap();
        assert!(json.contains("\"symbol\":\"K\""));
    }

    #[test]
    fn test_unit_deserialization() {
        // Simple symbol format
        let json = r#"{"label":{"en":"Kelvin"},"symbol":"K"}"#;
        let unit: Unit = serde_json::from_str(json).unwrap();
        assert_eq!(unit.symbol.as_ref().unwrap().value(), "K");
    }

    #[test]
    fn test_full_parameter_roundtrip() {
        let param = Parameter::new("REFC", "Reflectivity")
            .with_description("Composite reflectivity at 1000m AGL")
            .with_unit(Unit::dbz())
            .with_levels(vec![1000.0]);

        let json = serde_json::to_string_pretty(&param).unwrap();
        let parsed: Parameter = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, param.id);
        assert_eq!(parsed.label, param.label);
    }
}
