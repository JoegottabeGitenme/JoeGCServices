//! GRIB2 table builder from model configuration files.
//!
//! Builds `Grib2Tables` by parsing model YAML configs and extracting
//! parameter codes and level mappings.

use grib2_parser::{Grib2Tables, LevelDescription};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, warn};

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
                let discipline = grib2.get("discipline").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                let category = grib2.get("category").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                let number = grib2.get("number").and_then(|v| v.as_u64()).unwrap_or(0) as u8;

                // Only add if not already seen (first config wins)
                params.entry((discipline, category, number)).or_insert(name.clone());
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
                        let display_text = level.get("display").and_then(|d| d.as_str())
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
}
