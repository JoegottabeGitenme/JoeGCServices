//! EDR configuration loading and types.

use anyhow::{Context, Result};
use edr_protocol::LocationsConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Type of data in a model: forecast or observation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    /// Forecast data (e.g., HRRR, GFS, NBM) - queried by reference time + forecast hour.
    #[default]
    Forecast,
    /// Observation data (e.g., GOES, MRMS) - gridded observation data queried by observation time.
    Observation,
    /// Point observation data (e.g., METAR, TAF) - station-based observations from PostgreSQL.
    PointObservation,
}

impl DataType {
    /// Check if this is gridded observation data.
    pub fn is_observation(&self) -> bool {
        matches!(self, DataType::Observation)
    }

    /// Check if this is point observation data (METAR, TAF, etc.).
    pub fn is_point_observation(&self) -> bool {
        matches!(self, DataType::PointObservation)
    }
}

/// EDR configuration loaded from YAML files.
#[derive(Debug, Clone, Default)]
pub struct EdrConfig {
    /// Collection definitions by model.
    pub models: HashMap<String, ModelEdrConfig>,

    /// Global named locations for EDR queries.
    pub locations: LocationsConfig,
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
        let mut locations = LocationsConfig::default();

        // Read all YAML files in the directory
        for entry in
            std::fs::read_dir(path).with_context(|| format!("Failed to read directory: {}", dir))?
        {
            let entry = entry?;
            let file_path = entry.path();

            if let Some(ext) = file_path.extension() {
                if ext == "yaml" || ext == "yml" {
                    // Check if this is the locations config file
                    let file_name = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

                    if file_name == "locations" {
                        // Parse as locations config
                        let content = std::fs::read_to_string(&file_path)
                            .with_context(|| format!("Failed to read: {:?}", file_path))?;

                        locations = serde_yaml::from_str(&content).with_context(|| {
                            format!("Failed to parse locations config: {:?}", file_path)
                        })?;

                        tracing::info!(
                            "Loaded {} EDR locations from {:?}",
                            locations.locations.len(),
                            file_path
                        );
                    } else {
                        // Parse as model EDR config
                        let content = std::fs::read_to_string(&file_path)
                            .with_context(|| format!("Failed to read: {:?}", file_path))?;

                        let config: ModelEdrConfig = serde_yaml::from_str(&content)
                            .with_context(|| format!("Failed to parse: {:?}", file_path))?;

                        models.insert(config.model.clone(), config);
                    }
                }
            }
        }

        Ok(Self { models, locations })
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

    /// Data type: "forecast" (default), "observation", or "point_observation".
    /// - Forecast data (like HRRR, GFS) is queried by reference time + forecast hour.
    /// - Observation data (like GOES satellite) is queried by observation time.
    /// - Point observation data (like METAR) is queried from PostgreSQL stations.
    #[serde(default)]
    pub data_type: DataType,

    /// Collection definitions for this model.
    #[serde(default)]
    pub collections: Vec<CollectionDefinition>,

    /// Global settings for this model.
    #[serde(default)]
    pub settings: ModelSettings,

    /// Response size limits.
    #[serde(default)]
    pub limits: LimitsConfig,

    /// Source identifier for point observation data (e.g., "metar", "madis").
    /// Only used when data_type is "point_observation".
    #[serde(default)]
    pub observation_source: Option<String>,
}

impl ModelEdrConfig {
    /// Create a DatasetQuery for the appropriate data type (forecast vs observation).
    pub fn create_query(&self, parameter: &str) -> grid_processor::DatasetQuery {
        if self.data_type.is_observation() {
            grid_processor::DatasetQuery::observation(&self.model, parameter)
        } else {
            grid_processor::DatasetQuery::forecast(&self.model, parameter)
        }
    }
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

/// Build a catalog-compatible level string from EDR config.
///
/// This converts EDR configuration (level_filter + parameter definition + optional z value)
/// into the level string format used in the catalog database.
///
/// # Arguments
/// * `level_filter` - The collection's level filter configuration
/// * `param_def` - Optional parameter definition with specific levels
/// * `z_value` - Optional z value from query parameter
///
/// # Returns
/// The level string to use for catalog lookup, or None if no level applies
pub fn build_level_string(
    level_filter: &LevelFilter,
    param_def: Option<&ParameterDefinition>,
    z_value: Option<f64>,
) -> Option<String> {
    // Use z_value if provided, otherwise use the first level from param definition
    let level_value = z_value.or_else(|| {
        param_def
            .and_then(|p| p.levels.first())
            .and_then(|l| match l {
                LevelValue::Numeric(n) => Some(*n),
                LevelValue::Named(_) => None,
            })
    });

    match level_filter.level_type.as_str() {
        "surface" => Some("surface".to_string()),
        "mean_sea_level" => Some("mean sea level".to_string()),
        "entire_atmosphere" => Some("entire atmosphere".to_string()),
        "isobaric" => {
            // Isobaric levels stored as "XXX mb"
            level_value.map(|v| format!("{} mb", v as i32))
        }
        "height_above_ground" => {
            // Height above ground stored as "X m above ground"
            level_value.map(|v| format!("{} m above ground", v as i32))
        }
        "cloud_layer" => {
            // Map cloud layer codes to names
            // GRIB2 Table 4.5: 212-214=low, 222-224=middle, 232-234=high
            // (x2=bottom, x3=top, x4=layer; some products use different codes)
            if let Some(code) = level_filter.level_code {
                match code {
                    212 | 213 | 214 => Some("low cloud layer".to_string()),
                    222 | 223 | 224 => Some("middle cloud layer".to_string()),
                    232 | 233 | 234 => Some("high cloud layer".to_string()),
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => {
            // Unknown level type, try to use named level from param
            // Map config-friendly names to database level strings
            param_def
                .and_then(|p| p.levels.first())
                .and_then(|l| match l {
                    LevelValue::Named(name) => {
                        // Map common config names to database level strings
                        let db_level = match name.as_str() {
                            "cloud_base" => "cloud base",
                            "entire_atmosphere" => "entire atmosphere",
                            "mean_sea_level" => "mean sea level",
                            // For satellite data (GOES), level names use underscores in DB
                            // (e.g., "visible_blue", "upper_vapor")
                            _ => name.as_str(),
                        };
                        Some(db_level.to_string())
                    }
                    LevelValue::Numeric(_) => None,
                })
        }
    }
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

    /// Cache policy for responses.
    #[serde(default)]
    pub cache_policy: CachePolicy,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            output_formats: default_output_formats(),
            default_crs: default_crs(),
            supported_crs: default_supported_crs(),
            cache_policy: CachePolicy::default(),
        }
    }
}

/// Cache policy settings for HTTP responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachePolicy {
    /// Cache-Control max-age for PNG responses in seconds.
    /// Default varies by typical model update frequency.
    #[serde(default = "default_png_max_age")]
    pub png_max_age: u32,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            png_max_age: default_png_max_age(),
        }
    }
}

/// Default PNG cache max-age: 5 minutes (300 seconds)
fn default_png_max_age() -> u32 {
    300
}

fn default_output_formats() -> Vec<String> {
    // Supported output formats for EDR data queries
    // Note: image/png is supported for area queries only (returns 16-bit encoded data for GPU shaders)
    vec![
        "application/vnd.cov+json".to_string(),
        "application/geo+json".to_string(),
        "image/png".to_string(),
    ]
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

    /// Maximum area for PNG area queries in square degrees.
    /// PNG queries typically request larger areas (e.g., full CONUS for GPU rendering).
    /// Falls back to max_area_sq_degrees if not set.
    #[serde(default)]
    pub max_area_sq_degrees_png: Option<f64>,

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
            max_area_sq_degrees_png: None, // Falls back to max_area_sq_degrees
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
    fn test_cache_policy_defaults() {
        let policy = CachePolicy::default();
        assert_eq!(policy.png_max_age, 300); // 5 minutes
    }

    #[test]
    fn test_cache_policy_yaml_parsing() {
        let yaml = r#"
model: test
collections: []
settings:
  cache_policy:
    png_max_age: 3600
"#;
        let config: ModelEdrConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.settings.cache_policy.png_max_age, 3600);
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

    #[test]
    fn test_goes_config_yaml_parsing() {
        // Test parsing GOES-style observation config with top_of_atmosphere level
        let yaml = r#"
model: goes18
collections:
  - id: goes18-infrared
    title: "GOES-18 Infrared"
    description: "Infrared window brightness temperatures"
    level_filter:
      level_type: top_of_atmosphere
      level_code: 8
    parameters:
      - name: CMI_C13
        levels: [clean_ir]
      - name: CMI_C14
        levels: [ir]
    run_mode: instances
  - id: goes18-infrared-latest
    title: "GOES-18 Infrared (Latest)"
    description: "Most recent infrared imagery"
    level_filter:
      level_type: top_of_atmosphere
      level_code: 8
    parameters:
      - name: CMI_C13
        levels: [clean_ir]
    run_mode: latest
settings:
  output_formats:
    - application/vnd.cov+json
    - application/geo+json
limits:
  max_time_steps: 36
  max_vertical_levels: 1
  max_area_sq_degrees: 50
"#;

        let config: ModelEdrConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model, "goes18");
        assert_eq!(config.collections.len(), 2);

        // Check instances collection
        let instances_coll = &config.collections[0];
        assert_eq!(instances_coll.id, "goes18-infrared");
        assert_eq!(instances_coll.level_filter.level_type, "top_of_atmosphere");
        assert_eq!(instances_coll.level_filter.level_code, Some(8));
        assert_eq!(instances_coll.parameters.len(), 2);
        assert!(matches!(instances_coll.run_mode, RunMode::Instances));

        // Check named levels parse correctly
        let param = &instances_coll.parameters[0];
        assert_eq!(param.name, "CMI_C13");
        assert_eq!(param.levels.len(), 1);
        assert!(matches!(&param.levels[0], LevelValue::Named(s) if s == "clean_ir"));

        // Check latest collection
        let latest_coll = &config.collections[1];
        assert_eq!(latest_coll.id, "goes18-infrared-latest");
        assert!(matches!(latest_coll.run_mode, RunMode::Latest));

        // Check limits
        assert_eq!(config.limits.max_time_steps, 36);
        assert_eq!(config.limits.max_vertical_levels, 1);
        assert_eq!(config.limits.max_area_sq_degrees, Some(50.0));
    }

    #[test]
    fn test_png_area_limit_parsing() {
        let yaml = r#"
model: hrrr
collections:
  - id: hrrr-surface
    title: "Surface"
    level_filter:
      level_type: surface
      level_code: 1
    parameters:
      - name: TMP
        levels: [surface]
limits:
  max_area_sq_degrees: 100
  max_area_sq_degrees_png: 2500
"#;

        let config: ModelEdrConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.limits.max_area_sq_degrees, Some(100.0));
        assert_eq!(config.limits.max_area_sq_degrees_png, Some(2500.0));
    }

    #[test]
    fn test_png_area_limit_defaults_to_none() {
        let yaml = r#"
model: test
collections:
  - id: test
    title: "Test"
    level_filter:
      level_type: surface
      level_code: 1
    parameters:
      - name: TMP
        levels: [surface]
limits:
  max_area_sq_degrees: 100
"#;

        let config: ModelEdrConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.limits.max_area_sq_degrees, Some(100.0));
        assert_eq!(config.limits.max_area_sq_degrees_png, None);
    }
}
