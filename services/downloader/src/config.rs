//! Configuration loading for model download schedules.
//!
//! Loads model configurations from YAML files in config/models/

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{debug, info, warn};

/// Root configuration loaded from a model YAML file.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub model: ModelInfo,
    pub source: SourceConfig,
    pub grid: GridConfig,
    pub schedule: ScheduleConfig,
    #[serde(default)]
    pub retention: RetentionConfig,
    #[serde(default)]
    pub parameters: Vec<ParameterConfig>,
    #[serde(default)]
    pub composites: Vec<CompositeConfig>,
}

/// Basic model identification.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Data source configuration.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SourceConfig {
    #[serde(rename = "type")]
    pub source_type: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix_template: String,
    #[serde(default)]
    pub file_pattern: String,
    #[serde(default)]
    pub path_pattern: String,
    #[serde(default = "default_region")]
    pub region: String,
    #[serde(default)]
    pub compression: Option<String>,
    /// GOES-specific: product name (e.g., "ABI-L2-CMIPC")
    #[serde(default)]
    pub product: Option<String>,
    /// GOES-specific: band numbers to download
    #[serde(default)]
    pub bands: Option<Vec<u32>>,
}

fn default_region() -> String {
    "us-east-1".to_string()
}

/// Grid specification.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct GridConfig {
    pub projection: String,
    pub resolution: String,
    pub bbox: BBox,
    #[serde(default)]
    pub lon_convention: Option<String>,
    #[serde(default)]
    pub projection_params: Option<ProjectionParams>,
    #[serde(default)]
    pub dimensions: Option<Dimensions>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct BBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectionParams {
    pub lat1: Option<f64>,
    pub lon1: Option<f64>,
    pub lov: Option<f64>,
    pub latin1: Option<f64>,
    pub latin2: Option<f64>,
    pub dx: Option<f64>,
    pub dy: Option<f64>,
    pub nx: Option<u32>,
    pub ny: Option<u32>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Dimensions {
    pub x: u32,
    pub y: u32,
}

/// Schedule configuration for downloads.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleConfig {
    /// Schedule type: "forecast" (default) or "observation"
    #[serde(rename = "type", default = "default_schedule_type")]
    pub schedule_type: String,
    /// Cycles for forecast models (e.g., [0, 6, 12, 18])
    #[serde(default)]
    pub cycles: Vec<u32>,
    /// Forecast hours configuration
    #[serde(default)]
    pub forecast_hours: Option<ForecastHoursConfig>,
    /// Polling interval in seconds
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    /// Hours after cycle time that data becomes available
    #[serde(default)]
    pub delay_hours: u32,
    /// For observation data: how far back to look (minutes)
    #[serde(default)]
    pub lookback_minutes: u32,
}

fn default_schedule_type() -> String {
    "forecast".to_string()
}

fn default_poll_interval() -> u64 {
    3600
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ForecastHoursConfig {
    pub start: u32,
    pub end: u32,
    pub step: u32,
}

impl ForecastHoursConfig {
    /// Generate the list of forecast hours.
    pub fn hours(&self) -> Vec<u32> {
        (self.start..=self.end).step_by(self.step as usize).collect()
    }
}

/// Data retention settings.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RetentionConfig {
    #[serde(default = "default_retention_hours")]
    pub hours: u32,
}

fn default_retention_hours() -> u32 {
    168 // 7 days
}

/// Parameter configuration (for reference, not used in downloader).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ParameterConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub levels: Vec<serde_yaml::Value>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub units: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
}

/// Composite layer configuration.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct CompositeConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub renderer: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
}

impl ModelConfig {
    /// Load a model configuration from a YAML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        
        let config: ModelConfig = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        
        debug!(model = %config.model.id, path = %path.display(), "Loaded model config");
        Ok(config)
    }
    
    /// Get forecast hours as a Vec.
    pub fn forecast_hours(&self) -> Vec<u32> {
        self.schedule.forecast_hours
            .as_ref()
            .map(|fh| fh.hours())
            .unwrap_or_default()
    }
    
    /// Check if this is an observation-type data source (vs forecast).
    pub fn is_observation(&self) -> bool {
        self.schedule.schedule_type == "observation"
    }
}

/// Load all enabled model configurations from a directory.
pub fn load_model_configs(config_dir: &Path) -> Result<Vec<ModelConfig>> {
    let models_dir = config_dir.join("models");
    
    if !models_dir.exists() {
        warn!(path = %models_dir.display(), "Models config directory not found");
        return Ok(Vec::new());
    }
    
    let mut configs = Vec::new();
    
    for entry in std::fs::read_dir(&models_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().map_or(false, |ext| ext == "yaml" || ext == "yml") {
            match ModelConfig::load(&path) {
                Ok(config) => {
                    if config.model.enabled {
                        info!(
                            model = %config.model.id,
                            name = %config.model.name,
                            "Loaded model configuration"
                        );
                        configs.push(config);
                    } else {
                        debug!(model = %config.model.id, "Skipping disabled model");
                    }
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "Failed to load model config");
                }
            }
        }
    }
    
    info!(count = configs.len(), "Loaded model configurations");
    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_forecast_hours() {
        let fh = ForecastHoursConfig {
            start: 0,
            end: 12,
            step: 3,
        };
        assert_eq!(fh.hours(), vec![0, 3, 6, 9, 12]);
    }
    
    #[test]
    fn test_parse_gfs_config() {
        let yaml = r#"
model:
  id: gfs
  name: "GFS - Global Forecast System"
  enabled: true

source:
  type: aws_s3
  bucket: noaa-gfs-bdp-pds
  prefix_template: "gfs.{date}/{cycle:02}/atmos"
  file_pattern: "gfs.t{cycle:02}z.pgrb2.{resolution}.f{forecast:03}"
  region: us-east-1

grid:
  projection: geographic
  resolution: "0.25deg"
  bbox:
    min_lon: 0.0
    min_lat: -90.0
    max_lon: 360.0
    max_lat: 90.0

schedule:
  cycles: [0, 6, 12, 18]
  forecast_hours:
    start: 0
    end: 24
    step: 3
  poll_interval_secs: 3600
  delay_hours: 4
"#;
        
        let config: ModelConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model.id, "gfs");
        assert_eq!(config.schedule.cycles, vec![0, 6, 12, 18]);
        assert_eq!(config.forecast_hours(), vec![0, 3, 6, 9, 12, 15, 18, 21, 24]);
    }
}
