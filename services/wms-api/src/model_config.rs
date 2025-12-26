//! Model dimension configuration.
//!
//! Loads dimension configuration from model YAML files to determine
//! which WMS/WMTS dimensions each model supports.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{debug, warn};

/// Dimension type for a model - determines which WMS dimensions are exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DimensionType {
    /// Forecast models use RUN (model init time) + FORECAST (hours ahead) dimensions
    #[default]
    Forecast,
    /// Observation data uses TIME dimension (ISO8601 timestamps)
    Observation,
}

impl DimensionType {
    /// Check if this is an observation-type model
    pub fn is_observation(&self) -> bool {
        matches!(self, DimensionType::Observation)
    }

    /// Check if this is a forecast-type model
    pub fn is_forecast(&self) -> bool {
        matches!(self, DimensionType::Forecast)
    }
}

/// Dimension configuration for a model.
#[derive(Debug, Clone)]
pub struct ModelDimensionConfig {
    /// The type of time dimensions (forecast vs observation)
    pub dimension_type: DimensionType,
    /// Whether RUN dimension is enabled (for forecast models)
    pub has_run: bool,
    /// Whether FORECAST dimension is enabled (for forecast models)
    pub has_forecast: bool,
    /// Whether TIME dimension is enabled (for observation models)
    pub has_time: bool,
    /// Whether ELEVATION dimension is enabled (vertical levels)
    pub has_elevation: bool,
    /// Whether this model requires reading the full grid (no partial bbox reads).
    ///
    /// Set to true for models with non-geographic projections (e.g., Lambert Conformal)
    /// where the relationship between grid indices and geographic coordinates is non-linear.
    /// For these models, partial bbox reads would produce incorrect results.
    pub requires_full_grid: bool,
}

impl Default for ModelDimensionConfig {
    fn default() -> Self {
        // Default to forecast model with all dimensions
        Self {
            dimension_type: DimensionType::Forecast,
            has_run: true,
            has_forecast: true,
            has_time: false,
            has_elevation: true,
            requires_full_grid: false, // Most models support partial reads
        }
    }
}

/// YAML structure for parsing dimension config from model files.
#[derive(Debug, Deserialize)]
struct YamlDimensionsConfig {
    #[serde(rename = "type", default)]
    dimension_type: Option<String>,
    #[serde(default)]
    run: Option<bool>,
    #[serde(default)]
    forecast: Option<bool>,
    #[serde(default)]
    time: Option<bool>,
    #[serde(default)]
    elevation: Option<bool>,
}

/// Partial YAML structure for extracting just the dimensions section.
#[derive(Debug, Deserialize)]
struct YamlModelFile {
    model: YamlModelMetadata,
    #[serde(default)]
    dimensions: Option<YamlDimensionsConfig>,
    #[serde(default)]
    schedule: Option<YamlScheduleConfig>,
    #[serde(default)]
    grid: Option<YamlGridConfig>,
}

/// YAML structure for parsing grid config from model files.
#[derive(Debug, Deserialize)]
struct YamlGridConfig {
    /// Projection type (e.g., "geographic", "lambert_conformal")
    #[serde(default)]
    projection: Option<String>,
    /// Explicit flag to require full grid reads (overrides projection inference)
    #[serde(default)]
    requires_full_grid: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct YamlModelMetadata {
    id: String,
}

#[derive(Debug, Deserialize)]
struct YamlScheduleConfig {
    #[serde(rename = "type")]
    schedule_type: Option<String>,
}

/// Registry of model dimension configurations.
#[derive(Debug, Clone, Default)]
pub struct ModelDimensionRegistry {
    configs: HashMap<String, ModelDimensionConfig>,
}

impl ModelDimensionRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
        }
    }

    /// Load dimension configurations from all model YAML files in a directory.
    pub fn load_from_directory<P: AsRef<Path>>(config_dir: P) -> Self {
        let mut registry = Self::new();
        let models_dir = config_dir.as_ref().join("models");

        if !models_dir.exists() {
            warn!(path = ?models_dir, "Models config directory not found");
            return registry;
        }

        let entries = match fs::read_dir(&models_dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!(error = %e, path = ?models_dir, "Failed to read models directory");
                return registry;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                if let Some(config) = Self::load_model_config(&path) {
                    debug!(
                        model = %config.0,
                        dimension_type = ?config.1.dimension_type,
                        "Loaded dimension config"
                    );
                    registry.configs.insert(config.0, config.1);
                }
            }
        }

        debug!(
            count = registry.configs.len(),
            models = ?registry.configs.keys().collect::<Vec<_>>(),
            "Loaded model dimension configs"
        );

        registry
    }

    /// Load dimension config from a single YAML file.
    fn load_model_config<P: AsRef<Path>>(path: P) -> Option<(String, ModelDimensionConfig)> {
        let contents = fs::read_to_string(path.as_ref()).ok()?;
        let yaml: YamlModelFile = serde_yaml::from_str(&contents).ok()?;

        let model_id = yaml.model.id.clone();

        // Determine if full grid reads are required
        // Can be set explicitly via grid.requires_full_grid, or inferred from projection
        let requires_full_grid = if let Some(ref grid) = yaml.grid {
            // Explicit setting takes precedence
            grid.requires_full_grid.unwrap_or_else(|| {
                // Infer from projection type - non-geographic projections require full grid
                match grid.projection.as_deref() {
                    Some("lambert_conformal")
                    | Some("polar_stereographic")
                    | Some("mercator")
                    | Some("geostationary") => true,
                    Some("geographic") | Some("lat_lon") | Some("equidistant_cylindrical") => false,
                    _ => false, // Default to allowing partial reads
                }
            })
        } else {
            false
        };

        // Determine dimension type from explicit config or infer from schedule
        let config = if let Some(dims) = yaml.dimensions {
            // Explicit dimensions config
            let dimension_type = match dims.dimension_type.as_deref() {
                Some("observation") => DimensionType::Observation,
                Some("forecast") | None => DimensionType::Forecast,
                Some(other) => {
                    warn!(model = %model_id, invalid_type = %other, "Unknown dimension type, defaulting to forecast");
                    DimensionType::Forecast
                }
            };

            ModelDimensionConfig {
                dimension_type,
                has_run: dims.run.unwrap_or(dimension_type.is_forecast()),
                has_forecast: dims.forecast.unwrap_or(dimension_type.is_forecast()),
                has_time: dims.time.unwrap_or(dimension_type.is_observation()),
                has_elevation: dims.elevation.unwrap_or(true),
                requires_full_grid,
            }
        } else if let Some(schedule) = yaml.schedule {
            // Infer from schedule.type if no explicit dimensions config
            let dimension_type = match schedule.schedule_type.as_deref() {
                Some("observation") => DimensionType::Observation,
                _ => DimensionType::Forecast,
            };

            ModelDimensionConfig {
                dimension_type,
                has_run: dimension_type.is_forecast(),
                has_forecast: dimension_type.is_forecast(),
                has_time: dimension_type.is_observation(),
                has_elevation: true,
                requires_full_grid,
            }
        } else {
            // Default to forecast
            let mut config = ModelDimensionConfig::default();
            config.requires_full_grid = requires_full_grid;
            config
        };

        Some((model_id, config))
    }

    /// Get dimension config for a model.
    /// Returns default (forecast) config if model not found.
    pub fn get(&self, model: &str) -> ModelDimensionConfig {
        self.configs.get(model).cloned().unwrap_or_default()
    }

    /// Get dimension type for a model.
    pub fn get_dimension_type(&self, model: &str) -> DimensionType {
        self.get(model).dimension_type
    }

    /// Check if a model is observation type.
    pub fn is_observation(&self, model: &str) -> bool {
        self.get_dimension_type(model).is_observation()
    }

    /// Check if a model is forecast type.
    pub fn is_forecast(&self, model: &str) -> bool {
        self.get_dimension_type(model).is_forecast()
    }

    /// Check if a model requires full grid reads (no partial bbox optimization).
    ///
    /// Returns true for models with non-geographic projections where partial
    /// bbox reads would produce incorrect results.
    pub fn requires_full_grid(&self, model: &str) -> bool {
        self.get(model).requires_full_grid
    }

    /// Get all registered model IDs.
    pub fn models(&self) -> Vec<&str> {
        self.configs.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ModelDimensionConfig::default();
        assert!(config.dimension_type.is_forecast());
        assert!(config.has_run);
        assert!(config.has_forecast);
        assert!(!config.has_time);
        assert!(config.has_elevation);
    }

    #[test]
    fn test_registry_unknown_model() {
        let registry = ModelDimensionRegistry::new();
        // Unknown models should return default (forecast) config
        let config = registry.get("unknown_model");
        assert!(config.dimension_type.is_forecast());
    }
}
