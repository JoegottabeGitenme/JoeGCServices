//! Layer configuration loader.
//!
//! Loads layer configuration from YAML files in config/layers/ directory.
//! This provides a single source of truth for which layers are exposed via WMS/WMTS,
//! including their style file mappings, units, and level definitions.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{info, warn};

/// Unit conversion types supported by the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitConversion {
    /// Kelvin to Celsius: C = K - 273.15
    KToC,
    /// Pascal to hectoPascal: hPa = Pa / 100
    PaToHPa,
    /// Meters to kilometers: km = m / 1000
    MToKm,
    /// No conversion needed
    None,
}

impl UnitConversion {
    /// Parse a conversion name from config
    pub fn from_str(s: &str) -> Self {
        match s {
            "K_to_C" => Self::KToC,
            "Pa_to_hPa" => Self::PaToHPa,
            "m_to_km" => Self::MToKm,
            _ => Self::None,
        }
    }

    /// Apply the unit conversion to a value
    pub fn apply(&self, value: f64) -> f64 {
        match self {
            Self::KToC => value - 273.15,
            Self::PaToHPa => value / 100.0,
            Self::MToKm => value / 1000.0,
            Self::None => value,
        }
    }
}

/// Unit configuration for a layer
#[derive(Debug, Clone)]
pub struct UnitConfig {
    /// Native units from the data source (e.g., "K", "Pa")
    pub native: String,
    /// Display units for users (e.g., "C", "hPa")
    pub display: String,
    /// Conversion function to apply
    pub conversion: UnitConversion,
}

impl Default for UnitConfig {
    fn default() -> Self {
        Self {
            native: String::new(),
            display: String::new(),
            conversion: UnitConversion::None,
        }
    }
}

/// Level (elevation) configuration for a layer
#[derive(Debug, Clone)]
pub struct LevelConfig {
    /// Level value string (e.g., "2 m above ground", "500 mb")
    pub value: String,
    /// Whether this is the default level
    pub default: bool,
}

/// Layer configuration loaded from YAML
#[derive(Debug, Clone)]
pub struct LayerConfig {
    /// Layer ID (e.g., "gfs_TMP")
    pub id: String,
    /// Parameter code (e.g., "TMP", "PRMSL")
    pub parameter: String,
    /// Human-readable title
    pub title: String,
    /// Description/abstract
    pub abstract_text: Option<String>,
    /// Path to style JSON file (relative to styles dir, e.g., "temperature.json")
    pub style_file: String,
    /// Unit configuration
    pub units: UnitConfig,
    /// Available levels/elevations
    pub levels: Vec<LevelConfig>,
    /// Whether this is a composite layer (e.g., wind barbs from UGRD+VGRD)
    pub composite: bool,
    /// Required parameters for composite layers
    pub requires: Vec<String>,
    /// Whether this is an accumulation parameter
    pub accumulation: bool,
}

impl LayerConfig {
    /// Get the default level for this layer
    pub fn default_level(&self) -> Option<&str> {
        self.levels
            .iter()
            .find(|l| l.default)
            .or_else(|| self.levels.first())
            .map(|l| l.value.as_str())
    }

    /// Get all level values as strings
    pub fn level_values(&self) -> Vec<String> {
        self.levels.iter().map(|l| l.value.clone()).collect()
    }
}

/// Model layer configuration - contains all layers for a weather model
#[derive(Debug, Clone)]
pub struct ModelLayerConfig {
    /// Model ID (e.g., "gfs", "hrrr")
    pub model: String,
    /// Display name (e.g., "GFS - Global Forecast System")
    pub display_name: String,
    /// Default bounding box
    pub default_bbox: Option<BoundingBoxConfig>,
    /// Layers for this model
    pub layers: Vec<LayerConfig>,
}

impl ModelLayerConfig {
    /// Find a layer by parameter code (case-insensitive)
    pub fn get_layer_by_parameter(&self, parameter: &str) -> Option<&LayerConfig> {
        self.layers
            .iter()
            .find(|l| l.parameter.eq_ignore_ascii_case(parameter))
    }

    /// Find a layer by layer ID (case-insensitive)
    pub fn get_layer_by_id(&self, id: &str) -> Option<&LayerConfig> {
        self.layers.iter().find(|l| l.id.eq_ignore_ascii_case(id))
    }
}

/// Bounding box configuration
#[derive(Debug, Clone)]
pub struct BoundingBoxConfig {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl Default for BoundingBoxConfig {
    fn default() -> Self {
        Self {
            west: -180.0,
            south: -90.0,
            east: 180.0,
            north: 90.0,
        }
    }
}

// ============================================================================
// YAML Parsing Structures
// ============================================================================

#[derive(Debug, Deserialize)]
struct YamlLayerFile {
    model: String,
    display_name: String,
    #[serde(default)]
    default_bbox: Option<YamlBoundingBox>,
    layers: Vec<YamlLayer>,
}

#[derive(Debug, Deserialize)]
struct YamlBoundingBox {
    west: f64,
    south: f64,
    east: f64,
    north: f64,
}

#[derive(Debug, Deserialize)]
struct YamlLayer {
    id: String,
    parameter: String,
    title: String,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    style_file: String,
    #[serde(default)]
    units: Option<YamlUnits>,
    #[serde(default)]
    levels: Vec<YamlLevel>,
    #[serde(default)]
    composite: bool,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    accumulation: bool,
}

#[derive(Debug, Deserialize, Default)]
struct YamlUnits {
    native: Option<String>,
    display: Option<String>,
    conversion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct YamlLevel {
    value: String,
    #[serde(default)]
    default: bool,
}

// ============================================================================
// Registry
// ============================================================================

/// Registry of layer configurations for all models.
#[derive(Debug, Clone, Default)]
pub struct LayerConfigRegistry {
    /// Configs keyed by model ID
    configs: HashMap<String, ModelLayerConfig>,
    /// Style config directory path
    style_dir: String,
}

impl LayerConfigRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            style_dir: "./config/styles".to_string(),
        }
    }

    /// Load layer configurations from a directory (e.g., config/layers/)
    pub fn load_from_directory<P: AsRef<Path>>(config_dir: P) -> Self {
        let mut registry = Self::new();
        registry.reload_from_directory(config_dir);
        registry
    }

    /// Reload layer configurations from a directory (hot reload support)
    /// Returns the number of models loaded and total layers
    pub fn reload_from_directory<P: AsRef<Path>>(&mut self, config_dir: P) -> (usize, usize) {
        let layers_dir = config_dir.as_ref().join("layers");

        // Set style directory relative to config dir
        self.style_dir = config_dir
            .as_ref()
            .join("styles")
            .to_string_lossy()
            .to_string();

        // Clear existing configs before reload
        self.configs.clear();

        if !layers_dir.exists() {
            warn!(path = ?layers_dir, "Layers config directory not found");
            return (0, 0);
        }

        let entries = match fs::read_dir(&layers_dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!(error = %e, path = ?layers_dir, "Failed to read layers directory");
                return (0, 0);
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                if let Some(config) = Self::load_layer_file(&path) {
                    info!(
                        model = %config.model,
                        layers = config.layers.len(),
                        "Loaded layer config"
                    );
                    self.configs.insert(config.model.clone(), config);
                }
            }
        }

        let models = self.configs.len();
        let layers = self.total_layers();

        info!(
            models = models,
            total_layers = layers,
            "Layer config registry loaded"
        );

        (models, layers)
    }

    /// Load a single layer config file
    fn load_layer_file<P: AsRef<Path>>(path: P) -> Option<ModelLayerConfig> {
        let contents = match fs::read_to_string(path.as_ref()) {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, path = ?path.as_ref(), "Failed to read layer file");
                return None;
            }
        };

        let yaml: YamlLayerFile = match serde_yaml::from_str(&contents) {
            Ok(y) => y,
            Err(e) => {
                warn!(error = %e, path = ?path.as_ref(), "Failed to parse layer file");
                return None;
            }
        };

        let layers = yaml
            .layers
            .into_iter()
            .map(|l| LayerConfig {
                id: l.id,
                parameter: l.parameter,
                title: l.title,
                abstract_text: l.abstract_text,
                style_file: l.style_file,
                units: l
                    .units
                    .map(|u| UnitConfig {
                        native: u.native.unwrap_or_default(),
                        display: u.display.unwrap_or_default(),
                        conversion: u
                            .conversion
                            .map(|c| UnitConversion::from_str(&c))
                            .unwrap_or(UnitConversion::None),
                    })
                    .unwrap_or_default(),
                levels: l
                    .levels
                    .into_iter()
                    .map(|lv| LevelConfig {
                        value: lv.value,
                        default: lv.default,
                    })
                    .collect(),
                composite: l.composite,
                requires: l.requires,
                accumulation: l.accumulation,
            })
            .collect();

        Some(ModelLayerConfig {
            model: yaml.model,
            display_name: yaml.display_name,
            default_bbox: yaml.default_bbox.map(|b| BoundingBoxConfig {
                west: b.west,
                south: b.south,
                east: b.east,
                north: b.north,
            }),
            layers,
        })
    }

    /// Get total number of layers across all models
    pub fn total_layers(&self) -> usize {
        self.configs.values().map(|c| c.layers.len()).sum()
    }

    /// Get layer config for a specific model
    pub fn get_model(&self, model: &str) -> Option<&ModelLayerConfig> {
        self.configs.get(model)
    }

    /// Get all model IDs
    pub fn models(&self) -> Vec<&str> {
        self.configs.keys().map(|s| s.as_str()).collect()
    }

    /// Get layer config by layer ID (e.g., "gfs_TMP")
    pub fn get_layer(&self, layer_id: &str) -> Option<&LayerConfig> {
        // Parse layer_id format: "model_parameter"
        let parts: Vec<&str> = layer_id.splitn(2, '_').collect();
        if parts.len() < 2 {
            return None;
        }

        let model = parts[0];
        self.configs
            .get(model)
            .and_then(|m| m.get_layer_by_id(layer_id))
    }

    /// Get layer config by model and parameter
    pub fn get_layer_by_param(&self, model: &str, parameter: &str) -> Option<&LayerConfig> {
        self.configs
            .get(model)
            .and_then(|m| m.get_layer_by_parameter(parameter))
    }

    /// Get the full path to a style file for a layer
    pub fn get_style_path(&self, layer: &LayerConfig) -> String {
        format!("{}/{}", self.style_dir, layer.style_file)
    }

    /// Get style file path for a model/parameter combination.
    /// Returns None if no layer config is found.
    pub fn try_get_style_file(&self, model: &str, parameter: &str) -> Option<String> {
        self.get_layer_by_param(model, parameter)
            .map(|layer| format!("{}/{}", self.style_dir, layer.style_file))
    }

    /// Get style file path for a model/parameter combination.
    ///
    /// # Panics
    /// Panics if no layer config is found for the model/parameter combination.
    /// Use `try_get_style_file` if you need to handle missing configs gracefully.
    pub fn get_style_file_for_parameter(&self, model: &str, parameter: &str) -> String {
        self.try_get_style_file(model, parameter)
            .unwrap_or_else(|| {
                panic!(
                    "No layer config found for model='{}', parameter='{}'. \
                 Add this layer to config/layers/{}.yaml or remove it from the catalog.",
                    model, parameter, model
                )
            })
    }

    /// Check if a model has layer configs loaded
    pub fn has_model(&self, model: &str) -> bool {
        self.configs.contains_key(model)
    }

    /// Check if a specific model/parameter combination has a layer config
    pub fn has_layer(&self, model: &str, parameter: &str) -> bool {
        self.get_layer_by_param(model, parameter).is_some()
    }

    /// Try to get display name for a model. Returns None if not found.
    pub fn try_get_model_display_name(&self, model: &str) -> Option<String> {
        self.configs.get(model).map(|c| c.display_name.clone())
    }

    /// Get display name for a model.
    ///
    /// # Panics
    /// Panics if no model config is found.
    pub fn get_model_display_name(&self, model: &str) -> String {
        self.try_get_model_display_name(model).unwrap_or_else(|| {
            panic!(
                "No layer config found for model='{}'. \
                 Add config/layers/{}.yaml with model configuration.",
                model, model
            )
        })
    }

    /// Try to get display name for a parameter. Returns None if not found.
    pub fn try_get_parameter_display_name(&self, model: &str, parameter: &str) -> Option<String> {
        self.get_layer_by_param(model, parameter)
            .map(|l| l.title.clone())
    }

    /// Get display name for a parameter.
    ///
    /// # Panics
    /// Panics if no layer config is found for the model/parameter combination.
    pub fn get_parameter_display_name(&self, model: &str, parameter: &str) -> String {
        self.try_get_parameter_display_name(model, parameter)
            .unwrap_or_else(|| {
                panic!(
                    "No layer config found for model='{}', parameter='{}'. \
                 Add this layer to config/layers/{}.yaml.",
                    model, parameter, model
                )
            })
    }

    /// Validate that all parameters in the catalog have layer configs.
    /// Returns a list of (model, parameter) tuples that are missing configs.
    pub fn find_missing_configs(
        &self,
        models: &[String],
        model_params: &std::collections::HashMap<String, Vec<String>>,
    ) -> Vec<(String, String)> {
        let mut missing = Vec::new();

        for model in models {
            if let Some(params) = model_params.get(model) {
                for param in params {
                    // Skip WIND_BARBS as it's a synthetic layer
                    if param == "WIND_BARBS" {
                        continue;
                    }
                    if !self.has_layer(model, param) {
                        missing.push((model.clone(), param.clone()));
                    }
                }
            }
        }

        missing
    }

    /// Validate that all parameters in the catalog have layer configs.
    /// Returns Ok(()) if all configs are present, or Err with a descriptive message.
    pub fn validate_catalog_coverage(
        &self,
        models: &[String],
        model_params: &std::collections::HashMap<String, Vec<String>>,
    ) -> Result<(), String> {
        let missing = self.find_missing_configs(models, model_params);

        if missing.is_empty() {
            return Ok(());
        }

        // Group by model for better error messages
        let mut by_model: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (model, param) in missing {
            by_model.entry(model).or_default().push(param);
        }

        let mut error_parts = Vec::new();
        for (model, params) in by_model {
            error_parts.push(format!(
                "  - {}: {} (add to config/layers/{}.yaml)",
                model,
                params.join(", "),
                model
            ));
        }

        Err(format!(
            "Missing layer configurations for the following parameters:\n{}\n\n\
             Each parameter in the catalog must have a layer config that specifies:\n\
             - style_file: Which JSON style file to use\n\
             - title: Human-readable name\n\
             - units: Native and display units with optional conversion",
            error_parts.join("\n")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unit_conversion() {
        // Use epsilon comparison for floating point
        let eps = 1e-9;
        assert!((UnitConversion::KToC.apply(273.15) - 0.0).abs() < eps);
        assert!((UnitConversion::KToC.apply(300.0) - 26.85).abs() < eps);
        assert!((UnitConversion::PaToHPa.apply(101325.0) - 1013.25).abs() < eps);
        assert!((UnitConversion::MToKm.apply(1000.0) - 1.0).abs() < eps);
        assert!((UnitConversion::None.apply(42.0) - 42.0).abs() < eps);
    }

    #[test]
    fn test_unit_conversion_from_str() {
        assert_eq!(UnitConversion::from_str("K_to_C"), UnitConversion::KToC);
        assert_eq!(
            UnitConversion::from_str("Pa_to_hPa"),
            UnitConversion::PaToHPa
        );
        assert_eq!(UnitConversion::from_str("m_to_km"), UnitConversion::MToKm);
        assert_eq!(UnitConversion::from_str("unknown"), UnitConversion::None);
    }

    #[test]
    fn test_layer_default_level() {
        let layer = LayerConfig {
            id: "test".to_string(),
            parameter: "TMP".to_string(),
            title: "Temperature".to_string(),
            abstract_text: None,
            style_file: "temperature.json".to_string(),
            units: UnitConfig::default(),
            levels: vec![
                LevelConfig {
                    value: "1000 mb".to_string(),
                    default: false,
                },
                LevelConfig {
                    value: "2 m above ground".to_string(),
                    default: true,
                },
                LevelConfig {
                    value: "500 mb".to_string(),
                    default: false,
                },
            ],
            composite: false,
            requires: vec![],
            accumulation: false,
        };

        assert_eq!(layer.default_level(), Some("2 m above ground"));
    }

    #[test]
    fn test_empty_registry() {
        let registry = LayerConfigRegistry::new();
        assert_eq!(registry.total_layers(), 0);
        assert!(registry.get_model("gfs").is_none());
    }
}
