//! GRIB2 parameter and level lookup tables.
//!
//! This module provides configurable lookup tables for translating
//! GRIB2 numeric codes into human-readable parameter names and level descriptions.
//!
//! Tables are built from model configuration YAML files, allowing the mapping
//! to be configured without code changes.

use std::collections::HashMap;

/// Lookup key for parameter: (discipline, category, number)
pub type ParamKey = (u8, u8, u8);

/// Level description - either static text or a template with {value} placeholder
#[derive(Debug, Clone)]
pub enum LevelDescription {
    /// Static description (e.g., "surface", "mean sea level")
    Static(String),
    /// Template with {value} placeholder (e.g., "{value} mb", "{value} m above ground")
    Template(String),
}

impl LevelDescription {
    /// Format the level description, substituting placeholders if it's a template.
    ///
    /// Supported placeholders:
    /// - `{value}` - Raw level value (e.g., 100000 for 1000 mb in Pa)
    /// - `{value_mb}` - Value converted from Pa to mb (divided by 100)
    pub fn format(&self, value: u32) -> String {
        match self {
            LevelDescription::Static(s) => s.clone(),
            LevelDescription::Template(t) => {
                let result = t.replace("{value}", &value.to_string());
                // Handle Pa to mb conversion for isobaric levels
                result.replace("{value_mb}", &(value / 100).to_string())
            }
        }
    }
}

/// GRIB2 parameter and level lookup tables.
///
/// Built from model configuration files and passed to the GRIB2 reader
/// to translate numeric codes into readable names.
#[derive(Debug, Clone, Default)]
pub struct Grib2Tables {
    /// (discipline, category, number) -> parameter short name (e.g., "TMP", "UGRD")
    parameters: HashMap<ParamKey, String>,
    /// level_type -> description pattern
    levels: HashMap<u8, LevelDescription>,
}

impl Grib2Tables {
    /// Create empty tables
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a parameter mapping
    ///
    /// # Arguments
    /// * `discipline` - GRIB2 discipline code
    /// * `category` - Parameter category within discipline
    /// * `number` - Parameter number within category
    /// * `name` - Short parameter name (e.g., "TMP", "UGRD")
    pub fn add_parameter(&mut self, discipline: u8, category: u8, number: u8, name: String) {
        self.parameters.insert((discipline, category, number), name);
    }

    /// Add a level description mapping
    ///
    /// # Arguments
    /// * `level_type` - GRIB2 level type code
    /// * `description` - Static or template description
    pub fn add_level(&mut self, level_type: u8, description: LevelDescription) {
        self.levels.insert(level_type, description);
    }

    /// Look up parameter short name by GRIB2 codes.
    ///
    /// Returns "P{discipline}_{category}_{number}" if not found.
    pub fn get_parameter_name(&self, discipline: u8, category: u8, number: u8) -> String {
        self.parameters
            .get(&(discipline, category, number))
            .cloned()
            .unwrap_or_else(|| format!("P{}_{}_{}", discipline, category, number))
    }

    /// Look up level description by type code and value.
    ///
    /// Returns "Level type {type} value {value}" if not found.
    pub fn get_level_description(&self, level_type: u8, level_value: u32) -> String {
        match self.levels.get(&level_type) {
            Some(desc) => desc.format(level_value),
            None => format!("Level type {} value {}", level_type, level_value),
        }
    }

    /// Get the number of parameters in the table
    pub fn parameter_count(&self) -> usize {
        self.parameters.len()
    }

    /// Get the number of level types in the table
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Check if the tables are empty
    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty() && self.levels.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tables() -> Grib2Tables {
        let mut tables = Grib2Tables::new();

        // Add some parameters
        tables.add_parameter(0, 0, 0, "TMP".to_string());
        tables.add_parameter(0, 2, 2, "UGRD".to_string());
        tables.add_parameter(0, 2, 3, "VGRD".to_string());
        tables.add_parameter(0, 3, 1, "PRMSL".to_string());
        tables.add_parameter(209, 0, 16, "REFL".to_string());

        // Add some levels
        tables.add_level(1, LevelDescription::Static("surface".to_string()));
        tables.add_level(100, LevelDescription::Template("{value} mb".to_string()));
        tables.add_level(101, LevelDescription::Static("mean sea level".to_string()));
        tables.add_level(
            103,
            LevelDescription::Template("{value} m above ground".to_string()),
        );
        tables.add_level(
            200,
            LevelDescription::Static("entire atmosphere".to_string()),
        );

        tables
    }

    #[test]
    fn test_parameter_lookup() {
        let tables = create_test_tables();

        assert_eq!(tables.get_parameter_name(0, 0, 0), "TMP");
        assert_eq!(tables.get_parameter_name(0, 2, 2), "UGRD");
        assert_eq!(tables.get_parameter_name(0, 2, 3), "VGRD");
        assert_eq!(tables.get_parameter_name(0, 3, 1), "PRMSL");
        assert_eq!(tables.get_parameter_name(209, 0, 16), "REFL");
    }

    #[test]
    fn test_parameter_not_found() {
        let tables = create_test_tables();

        // Unknown parameter returns formatted code
        assert_eq!(tables.get_parameter_name(99, 99, 99), "P99_99_99");
        assert_eq!(tables.get_parameter_name(0, 0, 99), "P0_0_99");
    }

    #[test]
    fn test_level_static_description() {
        let tables = create_test_tables();

        assert_eq!(tables.get_level_description(1, 0), "surface");
        assert_eq!(tables.get_level_description(101, 0), "mean sea level");
        assert_eq!(tables.get_level_description(200, 0), "entire atmosphere");
    }

    #[test]
    fn test_level_template_description() {
        let tables = create_test_tables();

        assert_eq!(tables.get_level_description(100, 500), "500 mb");
        assert_eq!(tables.get_level_description(100, 850), "850 mb");
        assert_eq!(tables.get_level_description(103, 2), "2 m above ground");
        assert_eq!(tables.get_level_description(103, 10), "10 m above ground");
    }

    #[test]
    fn test_level_not_found() {
        let tables = create_test_tables();

        assert_eq!(
            tables.get_level_description(99, 123),
            "Level type 99 value 123"
        );
    }

    #[test]
    fn test_counts() {
        let tables = create_test_tables();

        assert_eq!(tables.parameter_count(), 5);
        assert_eq!(tables.level_count(), 5);
        assert!(!tables.is_empty());
    }

    #[test]
    fn test_empty_tables() {
        let tables = Grib2Tables::new();

        assert_eq!(tables.parameter_count(), 0);
        assert_eq!(tables.level_count(), 0);
        assert!(tables.is_empty());

        // Should still return formatted fallbacks
        assert_eq!(tables.get_parameter_name(0, 0, 0), "P0_0_0");
        assert_eq!(tables.get_level_description(1, 0), "Level type 1 value 0");
    }
}
