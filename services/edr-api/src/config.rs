//! EDR configuration loading and types.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// EDR configuration loaded from YAML files.
#[derive(Debug, Clone, Default)]
pub struct EdrConfig {
    /// Collection definitions by model.
    pub models: HashMap<String, ModelEdrConfig>,
}

impl EdrConfig {
    /// Load configuration from a directory of YAML files.
    pub fn load_from_dir(dir: &str) -> Result<Self> {
        let path = Path::new(dir);

        // If directory doesn't exist, return default config
        if !path.exists() {
            tracing::warn!(
                "EDR config directory {} does not exist, using defaults",
                dir
            );
            return Ok(Self::default());
        }

        let mut models = HashMap::new();

        // Read all YAML files in the directory
        for entry in
            std::fs::read_dir(path).with_context(|| format!("Failed to read directory: {}", dir))?
        {
            let entry = entry?;
            let file_path = entry.path();

            if let Some(ext) = file_path.extension() {
                if ext == "yaml" || ext == "yml" {
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("Failed to read: {:?}", file_path))?;

                    let config: ModelEdrConfig = serde_yaml::from_str(&content)
                        .with_context(|| format!("Failed to parse: {:?}", file_path))?;

                    models.insert(config.model.clone(), config);
                }
            }
        }

        Ok(Self { models })
    }

    /// Get all collection definitions across all models.
    pub fn all_collections(&self) -> Vec<&CollectionDefinition> {
        self.models
            .values()
            .flat_map(|m| m.collections.iter())
            .collect()
    }

    /// Find a collection by ID.
    pub fn find_collection(&self, id: &str) -> Option<(&ModelEdrConfig, &CollectionDefinition)> {
        for model_config in self.models.values() {
            if let Some(collection) = model_config.collections.iter().find(|c| c.id == id) {
                return Some((model_config, collection));
            }
        }
        None
    }
}

/// Configuration for a single model's EDR exposure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEdrConfig {
    /// Model identifier (e.g., "hrrr", "gfs").
    pub model: String,

    /// Collection definitions for this model.
    #[serde(default)]
    pub collections: Vec<CollectionDefinition>,

    /// Global settings for this model.
    #[serde(default)]
    pub settings: ModelSettings,

    /// Response size limits.
    #[serde(default)]
    pub limits: LimitsConfig,
}

/// Definition of an EDR collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionDefinition {
    /// Unique collection identifier.
    pub id: String,

    /// Human-readable title.
    #[serde(default)]
    pub title: String,

    /// Description of the collection.
    #[serde(default)]
    pub description: String,

    /// Level type filter for this collection.
    #[serde(default)]
    pub level_filter: LevelFilter,

    /// Parameters exposed in this collection.
    #[serde(default)]
    pub parameters: Vec<ParameterDefinition>,

    /// Run mode (instances or latest).
    #[serde(default)]
    pub run_mode: RunMode,
}

/// Filter for selecting levels by type.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LevelFilter {
    /// Level type name (e.g., "isobaric", "surface").
    #[serde(default)]
    pub level_type: String,

    /// Level code(s) from GRIB2.
    #[serde(default)]
    pub level_code: Option<i32>,

    /// Multiple level codes.
    #[serde(default)]
    pub level_codes: Option<Vec<i32>>,
}

impl LevelFilter {
    /// Check if a level code matches this filter.
    pub fn matches(&self, code: i32) -> bool {
        if let Some(single) = self.level_code {
            return code == single;
        }
        if let Some(ref codes) = self.level_codes {
            return codes.contains(&code);
        }
        true // No filter = match all
    }
}

/// Parameter definition within a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDefinition {
    /// Parameter name (e.g., "TMP", "UGRD").
    pub name: String,

    /// Specific levels to expose (if not all from filter).
    #[serde(default)]
    pub levels: Vec<LevelValue>,
}

/// A level value (can be numeric or string).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LevelValue {
    Numeric(f64),
    Named(String),
}

/// Run mode for a collection.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    /// Create instance endpoints for each model run.
    #[default]
    Instances,
    /// Only expose the latest run (no instances).
    Latest,
}

/// Model-wide settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
    /// Output formats supported.
    #[serde(default = "default_output_formats")]
    pub output_formats: Vec<String>,

    /// Default CRS.
    #[serde(default = "default_crs")]
    pub default_crs: String,

    /// Supported CRS list.
    #[serde(default = "default_supported_crs")]
    pub supported_crs: Vec<String>,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            output_formats: default_output_formats(),
            default_crs: default_crs(),
            supported_crs: default_supported_crs(),
        }
    }
}

fn default_output_formats() -> Vec<String> {
    // Only advertise formats that are actually implemented
    // TODO: Add "application/geo+json" when GeoJSON output is implemented
    vec!["application/vnd.cov+json".to_string()]
}

fn default_crs() -> String {
    "CRS:84".to_string()
}

fn default_supported_crs() -> Vec<String> {
    // Only advertise CRS that are actually implemented
    // Currently we only support WGS84/CRS:84 (coordinates are always lon/lat)
    // TODO: Add "EPSG:4326" when axis order handling is implemented (lat/lon vs lon/lat)
    vec!["CRS:84".to_string()]
}

/// Response size limit configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsConfig {
    /// Maximum parameters per request.
    #[serde(default = "default_max_params")]
    pub max_parameters_per_request: usize,

    /// Maximum time steps per request.
    #[serde(default = "default_max_time_steps")]
    pub max_time_steps: usize,

    /// Maximum vertical levels per request.
    #[serde(default = "default_max_levels")]
    pub max_vertical_levels: usize,

    /// Maximum response size in MB.
    #[serde(default = "default_max_response_mb")]
    pub max_response_size_mb: usize,

    /// Maximum area for area/cube queries in square degrees.
    #[serde(default = "default_max_area")]
    pub max_area_sq_degrees: Option<f64>,

    /// Maximum radius for radius queries in km.
    #[serde(default = "default_max_radius")]
    pub max_radius_km: Option<f64>,

    /// Maximum points in a trajectory.
    #[serde(default = "default_max_trajectory_points")]
    pub max_trajectory_points: Option<usize>,

    /// Maximum corridor length in km.
    #[serde(default = "default_max_corridor_length")]
    pub max_corridor_length_km: Option<f64>,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_parameters_per_request: default_max_params(),
            max_time_steps: default_max_time_steps(),
            max_vertical_levels: default_max_levels(),
            max_response_size_mb: default_max_response_mb(),
            max_area_sq_degrees: default_max_area(),
            max_radius_km: default_max_radius(),
            max_trajectory_points: default_max_trajectory_points(),
            max_corridor_length_km: default_max_corridor_length(),
        }
    }
}

fn default_max_params() -> usize {
    10
}
fn default_max_time_steps() -> usize {
    48
}
fn default_max_levels() -> usize {
    20
}
fn default_max_response_mb() -> usize {
    50
}
fn default_max_area() -> Option<f64> {
    Some(100.0) // ~1000km x 1000km at equator
}
fn default_max_radius() -> Option<f64> {
    Some(500.0) // 500 km
}
fn default_max_trajectory_points() -> Option<usize> {
    Some(100)
}
fn default_max_corridor_length() -> Option<f64> {
    Some(2000.0) // 2000 km
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_filter_single_code() {
        let filter = LevelFilter {
            level_type: "isobaric".to_string(),
            level_code: Some(100),
            level_codes: None,
        };

        assert!(filter.matches(100));
        assert!(!filter.matches(1));
    }

    #[test]
    fn test_level_filter_multiple_codes() {
        let filter = LevelFilter {
            level_type: "cloud_layer".to_string(),
            level_code: None,
            level_codes: Some(vec![212, 222, 232]),
        };

        assert!(filter.matches(212));
        assert!(filter.matches(222));
        assert!(!filter.matches(100));
    }

    #[test]
    fn test_level_filter_no_filter() {
        let filter = LevelFilter::default();
        assert!(filter.matches(100));
        assert!(filter.matches(1));
    }

    #[test]
    fn test_default_settings() {
        let settings = ModelSettings::default();
        assert!(!settings.output_formats.is_empty());
        assert_eq!(settings.default_crs, "CRS:84");
    }

    #[test]
    fn test_config_yaml_parsing() {
        let yaml = r#"
model: hrrr
collections:
  - id: hrrr-isobaric
    title: "HRRR Isobaric"
    description: "Upper-air parameters"
    level_filter:
      level_type: isobaric
      level_code: 100
    parameters:
      - name: TMP
        levels: [850, 700, 500]
    run_mode: instances
settings:
  output_formats:
    - application/vnd.cov+json
limits:
  max_parameters_per_request: 5
"#;

        let config: ModelEdrConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model, "hrrr");
        assert_eq!(config.collections.len(), 1);
        assert_eq!(config.collections[0].id, "hrrr-isobaric");
        assert_eq!(config.collections[0].parameters.len(), 1);
        assert_eq!(config.limits.max_parameters_per_request, 5);
    }
}
