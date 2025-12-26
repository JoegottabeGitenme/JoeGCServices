//! GRIB2 table builder and ingestion filter from model configuration files.
//!
//! Builds `Grib2Tables` by parsing model YAML configs and extracting
//! parameter codes and level mappings.
//!
//! Also builds `IngestionFilter` to determine which parameter/level
//! combinations should be ingested for each model, and provides valid_range
//! for converting sentinel values to NaN during ingestion.

use crate::error::IngestionError;
use grib2_parser::{Grib2Tables, LevelDescription};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, warn};

// ============================================================================
// Ingestion Filter Types
// ============================================================================

/// Valid data range for a parameter. Values outside this range are converted to NaN.
#[derive(Debug, Clone, Copy)]
pub struct ValidRange {
    /// Minimum valid value (inclusive).
    pub min: f32,
    /// Maximum valid value (inclusive).
    pub max: f32,
}

impl ValidRange {
    /// Create a new valid range.
    pub fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    /// Check if a value is within the valid range.
    #[inline]
    pub fn is_valid(&self, value: f32) -> bool {
        value >= self.min && value <= self.max
    }
}

/// Filter criteria for a single parameter at a specific level type.
#[derive(Debug, Clone)]
pub struct LevelFilter {
    /// Specific values allowed (e.g., [2] for "2m only", [1000, 850, 500] for pressure levels).
    /// If None, accept all values for this level type.
    pub allowed_values: Option<HashSet<u32>>,
}

/// Ingestion filter built from model configuration.
///
/// Determines which parameter/level combinations should be ingested,
/// and provides valid_range for converting sentinel values to NaN.
/// Built from the `parameters` section of model YAML config files.
#[derive(Debug, Clone, Default)]
pub struct IngestionFilter {
    /// Map: (parameter_name, level_code) → LevelFilter
    filters: HashMap<(String, u8), LevelFilter>,
    /// Map: parameter_name → ValidRange for sentinel value conversion.
    /// Values outside valid_range are converted to NaN during ingestion.
    valid_ranges: HashMap<String, ValidRange>,
}

impl IngestionFilter {
    /// Create a new empty ingestion filter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a parameter/level combination should be ingested.
    ///
    /// Returns `true` if:
    /// - The parameter name and level type are in the filter, AND
    /// - Either no specific values are required, OR the level_value is in the allowed set
    ///
    /// Returns `false` if the parameter/level combination is not defined in the config.
    pub fn should_ingest(&self, param: &str, level_type: u8, level_value: u32) -> bool {
        match self.filters.get(&(param.to_string(), level_type)) {
            Some(filter) => match &filter.allowed_values {
                Some(values) => values.contains(&level_value),
                None => true, // No specific values = accept all
            },
            None => false, // Not in filter = don't ingest
        }
    }

    /// Get the valid range for a parameter.
    ///
    /// Returns the configured valid_range for sentinel value conversion.
    /// Values outside this range should be converted to NaN during ingestion.
    ///
    /// Returns `None` if no valid_range is configured (which should be treated as an error
    /// since valid_range is now required for all parameters).
    pub fn get_valid_range(&self, param: &str) -> Option<ValidRange> {
        self.valid_ranges.get(param).copied()
    }

    /// Returns true if this filter has any parameters defined.
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// Number of parameter/level type combinations in the filter.
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Insert a filter entry for a parameter/level combination.
    fn insert(&mut self, param: String, level_code: u8, filter: LevelFilter) {
        self.filters.insert((param, level_code), filter);
    }

    /// Set the valid range for a parameter.
    fn set_valid_range(&mut self, param: String, range: ValidRange) {
        self.valid_ranges.insert(param, range);
    }
}

// ============================================================================
// Ingestion Filter Builder
// ============================================================================

/// Build an ingestion filter from a model's YAML configuration.
///
/// This reads the `parameters` section from the model's config file and builds
/// a filter that determines which parameter/level combinations to ingest.
///
/// # Errors
///
/// Returns an error (and logs a CRITICAL message) if:
/// - The model config file does not exist
/// - The YAML is invalid or malformed
/// - No parameters are defined in the config
///
/// Callers should treat errors as fatal startup errors.
///
/// # Example
///
/// ```ignore
/// let filter = build_filter_for_model("gfs")?;
/// if filter.should_ingest("TMP", 103, 2) {
///     // Ingest 2m temperature
/// }
/// ```
pub fn build_filter_for_model(model: &str) -> Result<Arc<IngestionFilter>, IngestionError> {
    let config_path = get_models_dir().join(format!("{}.yaml", model));

    if !config_path.exists() {
        error!(
            model = %model,
            path = ?config_path,
            "CRITICAL: Model configuration file not found. Ingestion cannot proceed. \
             Please ensure the model config exists at the expected path."
        );
        return Err(IngestionError::InvalidConfig(format!(
            "Model config not found: {}. Expected at {:?}",
            model, config_path
        )));
    }

    let mut filter = IngestionFilter::new();
    load_filter_from_config(&config_path, &mut filter).map_err(|e| {
        error!(
            model = %model,
            path = ?config_path,
            error = %e,
            "CRITICAL: Failed to load model configuration. Ingestion cannot proceed."
        );
        e
    })?;

    if filter.is_empty() {
        error!(
            model = %model,
            path = ?config_path,
            "CRITICAL: Model config has no parameters defined for ingestion. \
             Add parameters to the 'parameters' section in the config file."
        );
        return Err(IngestionError::InvalidConfig(format!(
            "Model {} has no parameters defined for ingestion in {:?}",
            model, config_path
        )));
    }

    debug!(
        model = %model,
        parameter_level_combos = filter.len(),
        "Built ingestion filter for model"
    );

    Ok(Arc::new(filter))
}

/// Load ingestion filter criteria from a model config file.
fn load_filter_from_config(
    path: &Path,
    filter: &mut IngestionFilter,
) -> Result<(), IngestionError> {
    let contents = fs::read_to_string(path)
        .map_err(|e| IngestionError::InvalidConfig(format!("Cannot read {:?}: {}", path, e)))?;

    let yaml: serde_yaml::Value = serde_yaml::from_str(&contents)
        .map_err(|e| IngestionError::InvalidConfig(format!("Invalid YAML in {:?}: {}", path, e)))?;

    let parameters = yaml
        .get("parameters")
        .and_then(|p| p.as_sequence())
        .ok_or_else(|| {
            IngestionError::InvalidConfig(format!("Missing 'parameters' section in {:?}", path))
        })?;

    // Track parameters missing valid_range for error reporting
    let mut missing_valid_range: Vec<String> = Vec::new();

    for param in parameters {
        let name = match param.get("name").and_then(|n| n.as_str()) {
            Some(n) => n.to_string(),
            None => continue, // Skip entries without a name
        };

        // Parse valid_range: [min, max] - required for sentinel value conversion
        if let Some(valid_range) = param.get("valid_range").and_then(|v| v.as_sequence()) {
            if valid_range.len() >= 2 {
                let min = valid_range[0].as_f64().map(|v| v as f32);
                let max = valid_range[1].as_f64().map(|v| v as f32);
                if let (Some(min), Some(max)) = (min, max) {
                    filter.set_valid_range(name.clone(), ValidRange::new(min, max));
                }
            }
        } else {
            // Track missing valid_range (only add once per param name)
            if !missing_valid_range.contains(&name) {
                missing_valid_range.push(name.clone());
            }
        }

        let levels = match param.get("levels").and_then(|l| l.as_sequence()) {
            Some(l) => l,
            None => continue, // Skip parameters without levels defined
        };

        for level in levels {
            let level_code = match level.get("level_code").and_then(|v| v.as_u64()) {
                Some(code) => code as u8,
                None => continue, // Skip levels without level_code
            };

            // Check for single value or array of values
            let allowed_values = if let Some(value) = level.get("value").and_then(|v| v.as_u64()) {
                // Single value: value: 2
                Some(HashSet::from([value as u32]))
            } else if let Some(values) = level.get("values").and_then(|v| v.as_sequence()) {
                // Array of values: values: [1000, 850, 500]
                let set: HashSet<u32> = values
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .collect();
                if set.is_empty() {
                    None
                } else {
                    Some(set)
                }
            } else {
                // No value specified = accept all values for this level type
                None
            };

            filter.insert(name.clone(), level_code, LevelFilter { allowed_values });
        }
    }

    // Fail if any parameters are missing valid_range
    if !missing_valid_range.is_empty() {
        return Err(IngestionError::InvalidConfig(format!(
            "Parameters missing required 'valid_range' in {:?}: {}. \
             Add valid_range: [min, max] to each parameter for sentinel value handling.",
            path,
            missing_valid_range.join(", ")
        )));
    }

    Ok(())
}

// ============================================================================
// GRIB2 Tables Builder
// ============================================================================

/// Get the models config directory path.
///
/// Checks CONFIG_DIR environment variable first, falls back to "config/models".
fn get_models_dir() -> PathBuf {
    if let Ok(config_dir) = env::var("CONFIG_DIR") {
        PathBuf::from(config_dir).join("models")
    } else {
        PathBuf::from("config/models")
    }
}

/// Build Grib2Tables from all model configuration files in config/models/.
///
/// This reads all .yaml files in the models directory and extracts:
/// - Parameter mappings from `grib2: {discipline, category, number}` to parameter names
/// - Level mappings from `level_code` to display text (with {value} templates)
///
/// The config directory can be overridden via the CONFIG_DIR environment variable.
///
/// Returns an Arc-wrapped tables instance suitable for sharing across readers.
pub fn build_tables_from_configs() -> Arc<Grib2Tables> {
    let mut tables = Grib2Tables::new();
    let models_dir = get_models_dir();

    if !models_dir.exists() {
        warn!(path = ?models_dir, "Models config directory not found, using empty tables");
        return Arc::new(tables);
    }

    // Track seen parameters to avoid duplicates
    let mut seen_params: HashMap<(u8, u8, u8), String> = HashMap::new();
    let mut seen_levels: HashMap<u8, LevelDescription> = HashMap::new();

    // Read all model YAML files
    if let Ok(entries) = fs::read_dir(models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                if let Err(e) = load_model_config(&path, &mut seen_params, &mut seen_levels) {
                    warn!(path = ?path, error = %e, "Failed to load model config");
                }
            }
        }
    }

    // Add all collected mappings to tables
    for ((discipline, category, number), name) in seen_params {
        tables.add_parameter(discipline, category, number, name);
    }
    for (level_type, description) in seen_levels {
        tables.add_level(level_type, description);
    }

    debug!(
        parameters = tables.parameter_count(),
        levels = tables.level_count(),
        "Built GRIB2 tables from model configs"
    );

    Arc::new(tables)
}

/// Load parameter and level mappings from a single model config file.
fn load_model_config(
    path: &Path,
    params: &mut HashMap<(u8, u8, u8), String>,
    levels: &mut HashMap<u8, LevelDescription>,
) -> anyhow::Result<()> {
    let contents = fs::read_to_string(path)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&contents)?;

    // Extract parameters array
    if let Some(parameters) = yaml.get("parameters").and_then(|p| p.as_sequence()) {
        for param in parameters {
            // Get parameter name
            let name = match param.get("name").and_then(|n| n.as_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            // Get grib2 codes
            if let Some(grib2) = param.get("grib2") {
                let discipline = grib2
                    .get("discipline")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u8;
                let category = grib2.get("category").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                let number = grib2.get("number").and_then(|v| v.as_u64()).unwrap_or(0) as u8;

                // Only add if not already seen (first config wins)
                params
                    .entry((discipline, category, number))
                    .or_insert(name.clone());
            }

            // Extract level definitions
            if let Some(param_levels) = param.get("levels").and_then(|l| l.as_sequence()) {
                for level in param_levels {
                    if let Some(level_code) = level.get("level_code").and_then(|v| v.as_u64()) {
                        let level_code = level_code as u8;

                        // Skip if already defined
                        if levels.contains_key(&level_code) {
                            continue;
                        }

                        // Get display text - check both "display" and "display_template" fields
                        let display_text = level
                            .get("display")
                            .and_then(|d| d.as_str())
                            .or_else(|| level.get("display_template").and_then(|d| d.as_str()));

                        if let Some(display) = display_text {
                            let description = if display.contains("{value}") {
                                LevelDescription::Template(display.to_string())
                            } else {
                                LevelDescription::Static(display.to_string())
                            };
                            levels.insert(level_code, description);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Build tables for a specific model only.
///
/// This is useful when you know which model you're processing and want
/// to avoid loading configs for other models.
///
/// The config directory can be overridden via the CONFIG_DIR environment variable.
pub fn build_tables_for_model(model: &str) -> Arc<Grib2Tables> {
    let mut tables = Grib2Tables::new();
    let config_path = get_models_dir().join(format!("{}.yaml", model));

    if !config_path.exists() {
        debug!(model = %model, path = ?config_path, "Model config not found, using empty tables");
        return Arc::new(tables);
    }

    let mut params = HashMap::new();
    let mut levels = HashMap::new();

    if let Err(e) = load_model_config(&config_path, &mut params, &mut levels) {
        warn!(model = %model, error = %e, "Failed to load model config");
        return Arc::new(tables);
    }

    for ((discipline, category, number), name) in params {
        tables.add_parameter(discipline, category, number, name);
    }
    for (level_type, description) in levels {
        tables.add_level(level_type, description);
    }

    debug!(
        model = %model,
        parameters = tables.parameter_count(),
        levels = tables.level_count(),
        "Built GRIB2 tables for model"
    );

    Arc::new(tables)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn create_test_config(dir: &Path, name: &str, content: &str) {
        let path = dir.join(format!("{}.yaml", name));
        let mut file = fs::File::create(path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_load_model_config_basic() {
        let dir = tempdir().unwrap();
        let config = r#"
model:
  id: test
  name: "Test Model"

parameters:
  - name: TMP
    grib2:
      discipline: 0
      category: 0
      number: 0
    levels:
      - type: surface
        level_code: 1
        display: "surface"
      - type: height_above_ground
        level_code: 103
        display: "{value} m above ground"
  - name: UGRD
    grib2:
      discipline: 0
      category: 2
      number: 2
    levels:
      - type: height_above_ground
        level_code: 103
        display: "{value} m above ground"
"#;
        create_test_config(dir.path(), "test", config);

        let mut params = HashMap::new();
        let mut levels = HashMap::new();
        load_model_config(&dir.path().join("test.yaml"), &mut params, &mut levels).unwrap();

        // Check parameters
        assert_eq!(params.get(&(0, 0, 0)), Some(&"TMP".to_string()));
        assert_eq!(params.get(&(0, 2, 2)), Some(&"UGRD".to_string()));

        // Check levels
        assert!(levels.contains_key(&1));
        assert!(levels.contains_key(&103));
    }

    #[test]
    fn test_level_description_formats() {
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: TEST
    grib2:
      discipline: 0
      category: 0
      number: 0
    levels:
      - level_code: 1
        display: "surface"
      - level_code: 100
        display: "{value} mb"
      - level_code: 200
        display: "entire atmosphere"
"#;
        create_test_config(dir.path(), "test", config);

        let mut params = HashMap::new();
        let mut levels = HashMap::new();
        load_model_config(&dir.path().join("test.yaml"), &mut params, &mut levels).unwrap();

        // Static level
        match levels.get(&1) {
            Some(LevelDescription::Static(s)) => assert_eq!(s, "surface"),
            _ => panic!("Expected static description for surface"),
        }

        // Template level
        match levels.get(&100) {
            Some(LevelDescription::Template(t)) => assert_eq!(t, "{value} mb"),
            _ => panic!("Expected template description for isobaric"),
        }

        // Static level
        match levels.get(&200) {
            Some(LevelDescription::Static(s)) => assert_eq!(s, "entire atmosphere"),
            _ => panic!("Expected static description for entire atmosphere"),
        }
    }

    #[test]
    fn test_missing_grib2_section() {
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: TMP
    levels:
      - level_code: 1
        display: "surface"
"#;
        create_test_config(dir.path(), "test", config);

        let mut params = HashMap::new();
        let mut levels = HashMap::new();
        load_model_config(&dir.path().join("test.yaml"), &mut params, &mut levels).unwrap();

        // Parameter should not be added without grib2 section
        assert!(params.is_empty());
        // But level should still be extracted
        assert!(levels.contains_key(&1));
    }

    #[test]
    fn test_first_config_wins() {
        // When the same parameter code appears in multiple configs,
        // the first one processed should win. Since we can't control
        // file order in read_dir, we test this with a single config
        // containing duplicate entries.
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: FIRST
    grib2:
      discipline: 0
      category: 0
      number: 0
  - name: SECOND
    grib2:
      discipline: 0
      category: 0
      number: 0
"#;
        create_test_config(dir.path(), "test", config);

        let mut params = HashMap::new();
        let mut levels = HashMap::new();
        load_model_config(&dir.path().join("test.yaml"), &mut params, &mut levels).unwrap();

        // First entry should win
        assert_eq!(params.get(&(0, 0, 0)), Some(&"FIRST".to_string()));
    }

    // ========================================================================
    // Ingestion Filter Tests
    // ========================================================================

    #[test]
    fn test_ingestion_filter_should_ingest_single_value() {
        let mut filter = IngestionFilter::new();
        filter.insert(
            "TMP".to_string(),
            103,
            LevelFilter {
                allowed_values: Some(HashSet::from([2])),
            },
        );

        // Should accept 2m temperature
        assert!(filter.should_ingest("TMP", 103, 2));

        // Should reject 10m temperature (not in allowed values)
        assert!(!filter.should_ingest("TMP", 103, 10));

        // Should reject different level type
        assert!(!filter.should_ingest("TMP", 100, 2));
    }

    #[test]
    fn test_ingestion_filter_should_ingest_multiple_values() {
        let mut filter = IngestionFilter::new();
        filter.insert(
            "TMP".to_string(),
            100,
            LevelFilter {
                allowed_values: Some(HashSet::from([1000, 850, 500, 300])),
            },
        );

        // Should accept defined pressure levels
        assert!(filter.should_ingest("TMP", 100, 1000));
        assert!(filter.should_ingest("TMP", 100, 850));
        assert!(filter.should_ingest("TMP", 100, 500));
        assert!(filter.should_ingest("TMP", 100, 300));

        // Should reject undefined pressure levels
        assert!(!filter.should_ingest("TMP", 100, 925));
        assert!(!filter.should_ingest("TMP", 100, 700));
    }

    #[test]
    fn test_ingestion_filter_should_ingest_any_value() {
        let mut filter = IngestionFilter::new();
        filter.insert(
            "TCDC".to_string(),
            200,
            LevelFilter {
                allowed_values: None, // Accept all values
            },
        );

        // Should accept any value for this level type
        assert!(filter.should_ingest("TCDC", 200, 0));
        assert!(filter.should_ingest("TCDC", 200, 100));
        assert!(filter.should_ingest("TCDC", 200, 99999));

        // Should still reject different level type
        assert!(!filter.should_ingest("TCDC", 103, 0));
    }

    #[test]
    fn test_ingestion_filter_rejects_unknown_param() {
        let mut filter = IngestionFilter::new();
        filter.insert(
            "TMP".to_string(),
            103,
            LevelFilter {
                allowed_values: Some(HashSet::from([2])),
            },
        );

        // Should reject unknown parameter
        assert!(!filter.should_ingest("UNKNOWN", 103, 2));
        assert!(!filter.should_ingest("UGRD", 103, 10));
    }

    #[test]
    fn test_ingestion_filter_is_empty() {
        let filter = IngestionFilter::new();
        assert!(filter.is_empty());
        assert_eq!(filter.len(), 0);

        let mut filter2 = IngestionFilter::new();
        filter2.insert(
            "TMP".to_string(),
            103,
            LevelFilter {
                allowed_values: None,
            },
        );
        assert!(!filter2.is_empty());
        assert_eq!(filter2.len(), 1);
    }

    #[test]
    fn test_load_filter_from_config_single_value() {
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: TMP
    valid_range: [150, 350]
    levels:
      - level_code: 103
        value: 2
        display: "2 m above ground"
"#;
        create_test_config(dir.path(), "test", config);

        let mut filter = IngestionFilter::new();
        load_filter_from_config(&dir.path().join("test.yaml"), &mut filter).unwrap();

        assert!(filter.should_ingest("TMP", 103, 2));
        assert!(!filter.should_ingest("TMP", 103, 10));

        // Also check valid_range was parsed
        let range = filter.get_valid_range("TMP").unwrap();
        assert_eq!(range.min, 150.0);
        assert_eq!(range.max, 350.0);
    }

    #[test]
    fn test_load_filter_from_config_multiple_values() {
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: TMP
    valid_range: [150, 350]
    levels:
      - level_code: 100
        values: [1000, 850, 500, 300]
        display_template: "{value} mb"
"#;
        create_test_config(dir.path(), "test", config);

        let mut filter = IngestionFilter::new();
        load_filter_from_config(&dir.path().join("test.yaml"), &mut filter).unwrap();

        assert!(filter.should_ingest("TMP", 100, 1000));
        assert!(filter.should_ingest("TMP", 100, 850));
        assert!(filter.should_ingest("TMP", 100, 500));
        assert!(filter.should_ingest("TMP", 100, 300));
        assert!(!filter.should_ingest("TMP", 100, 925)); // Not in list
    }

    #[test]
    fn test_load_filter_from_config_no_value_accepts_all() {
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: TCDC
    valid_range: [0, 100]
    levels:
      - level_code: 200
        display: "entire atmosphere"
"#;
        create_test_config(dir.path(), "test", config);

        let mut filter = IngestionFilter::new();
        load_filter_from_config(&dir.path().join("test.yaml"), &mut filter).unwrap();

        // Should accept any value when no value/values specified
        assert!(filter.should_ingest("TCDC", 200, 0));
        assert!(filter.should_ingest("TCDC", 200, 12345));
    }

    #[test]
    fn test_load_filter_from_config_multiple_params() {
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: TMP
    valid_range: [150, 350]
    levels:
      - level_code: 103
        value: 2
  - name: UGRD
    valid_range: [-200, 200]
    levels:
      - level_code: 103
        value: 10
  - name: VGRD
    valid_range: [-200, 200]
    levels:
      - level_code: 103
        value: 10
"#;
        create_test_config(dir.path(), "test", config);

        let mut filter = IngestionFilter::new();
        load_filter_from_config(&dir.path().join("test.yaml"), &mut filter).unwrap();

        assert!(filter.should_ingest("TMP", 103, 2));
        assert!(filter.should_ingest("UGRD", 103, 10));
        assert!(filter.should_ingest("VGRD", 103, 10));
        assert!(!filter.should_ingest("TMP", 103, 10)); // Wrong value
        assert!(!filter.should_ingest("UGRD", 103, 2)); // Wrong value
    }

    #[test]
    fn test_load_filter_missing_parameters_section() {
        let dir = tempdir().unwrap();
        let config = r#"
model:
  id: test
  name: "Test Model"
"#;
        create_test_config(dir.path(), "test", config);

        let mut filter = IngestionFilter::new();
        let result = load_filter_from_config(&dir.path().join("test.yaml"), &mut filter);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, IngestionError::InvalidConfig(_)));
    }

    #[test]
    fn test_load_filter_invalid_yaml() {
        let dir = tempdir().unwrap();
        let config = "this is not valid yaml: [";
        create_test_config(dir.path(), "test", config);

        let mut filter = IngestionFilter::new();
        let result = load_filter_from_config(&dir.path().join("test.yaml"), &mut filter);

        assert!(result.is_err());
    }

    #[test]
    fn test_load_filter_param_without_levels_skipped() {
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: TMP
    valid_range: [150, 350]
    description: "Temperature without levels"
  - name: UGRD
    valid_range: [-200, 200]
    levels:
      - level_code: 103
        value: 10
"#;
        create_test_config(dir.path(), "test", config);

        let mut filter = IngestionFilter::new();
        load_filter_from_config(&dir.path().join("test.yaml"), &mut filter).unwrap();

        // TMP should not be in filter (no levels)
        assert!(!filter.should_ingest("TMP", 103, 2));
        // UGRD should be in filter
        assert!(filter.should_ingest("UGRD", 103, 10));
    }

    #[test]
    fn test_load_filter_level_without_level_code_skipped() {
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: TMP
    valid_range: [150, 350]
    levels:
      - type: surface
        display: "surface"
      - level_code: 103
        value: 2
"#;
        create_test_config(dir.path(), "test", config);

        let mut filter = IngestionFilter::new();
        load_filter_from_config(&dir.path().join("test.yaml"), &mut filter).unwrap();

        // Only level_code: 103 should be in filter
        assert!(filter.should_ingest("TMP", 103, 2));
        assert_eq!(filter.len(), 1);
    }

    #[test]
    fn test_load_filter_missing_valid_range_fails() {
        let dir = tempdir().unwrap();
        let config = r#"
parameters:
  - name: TMP
    levels:
      - level_code: 103
        value: 2
"#;
        create_test_config(dir.path(), "test", config);

        let mut filter = IngestionFilter::new();
        let result = load_filter_from_config(&dir.path().join("test.yaml"), &mut filter);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, IngestionError::InvalidConfig(_)));
        // Verify the error message mentions TMP
        let err_msg = format!("{}", err);
        assert!(err_msg.contains("TMP"));
        assert!(err_msg.contains("valid_range"));
    }

    #[test]
    fn test_valid_range_is_valid() {
        let range = ValidRange::new(0.0, 100.0);
        assert!(range.is_valid(0.0));
        assert!(range.is_valid(50.0));
        assert!(range.is_valid(100.0));
        assert!(!range.is_valid(-1.0));
        assert!(!range.is_valid(101.0));
        assert!(!range.is_valid(-999.0)); // Sentinel value
    }

    #[test]
    fn test_valid_range_negative_range() {
        // CIN is typically negative
        let range = ValidRange::new(-1000.0, 0.0);
        assert!(range.is_valid(-500.0));
        assert!(range.is_valid(-1000.0));
        assert!(range.is_valid(0.0));
        assert!(!range.is_valid(-1001.0));
        assert!(!range.is_valid(1.0));
    }
}
