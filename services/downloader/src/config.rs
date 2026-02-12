//! Configuration loading for model download schedules.
//!
//! Loads model configurations from YAML files in config/models/
//!
//! Note: `#[allow(dead_code)]` is used on structs because they are deserialized
//! from YAML via serde. The compiler can't see field access through deserialization,
//! so it incorrectly reports them as unused. All fields must remain for YAML parsing.

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
    /// S3 bucket name (for aws_s3 types)
    #[serde(default)]
    pub bucket: String,
    /// Base URL for HTTP sources (e.g., `https://tgftp.nws.noaa.gov`)
    #[serde(default)]
    pub base_url: Option<String>,
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
    /// File types to expand in the pattern (e.g., ["pres", "sfc"] for AIGFS).
    /// When set, {file_type} in file_pattern is replaced with each value,
    /// generating multiple files per forecast hour.
    #[serde(default)]
    pub file_types: Option<Vec<String>>,
    /// Data format hint (e.g., "ndfd_grib2" for NDFD files with WMO headers)
    #[serde(default)]
    pub format: Option<String>,
    /// Skip Content-Length validation after download.
    /// Useful for sources like NDFD where files are updated in-place during download.
    #[serde(default)]
    pub skip_size_validation: bool,
    /// Always re-download files even if URL was previously downloaded.
    /// Useful for sources like NDFD where files have static URLs but content changes.
    /// When enabled, old download records are expired based on retention.hours.
    #[serde(default)]
    pub always_redownload: bool,
    /// Enable selective download via .idx index files.
    /// When enabled, downloads only the GRIB messages for configured parameters
    /// instead of the entire file. Falls back to full download if index unavailable.
    #[serde(default)]
    pub use_index_file: bool,
    /// Index file suffix (default: ".idx")
    #[serde(default = "default_index_suffix")]
    pub index_suffix: String,
}

fn default_index_suffix() -> String {
    ".idx".to_string()
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
    /// Maximum concurrent downloads for this model.
    /// Each model always gets at least 1 guaranteed slot.
    /// Additional slots (up to this limit) come from a shared pool.
    #[serde(default = "default_model_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_model_max_concurrent() -> usize {
    2
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
        (self.start..=self.end)
            .step_by(self.step as usize)
            .collect()
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
    pub levels: Vec<LevelConfig>,
    #[serde(default)]
    pub units: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    /// File identifier for HTTP sources (e.g., "temp" for ds.temp.bin in NDFD)
    #[serde(default)]
    pub file: Option<String>,
}

/// Level configuration for a parameter.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct LevelConfig {
    /// Level type name (e.g., "height_above_ground", "isobaric", "surface")
    #[serde(rename = "type")]
    pub level_type: Option<String>,
    /// GRIB2 level type code
    pub level_code: Option<u8>,
    /// Single level value (for height_above_ground, etc.)
    pub value: Option<u32>,
    /// Multiple level values (for isobaric levels)
    pub values: Option<Vec<u32>>,
    /// Display name (e.g., "2 m above ground")
    pub display: Option<String>,
    /// Display template for multiple values (e.g., "{value} mb")
    pub display_template: Option<String>,
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
}

impl ModelConfig {
    /// Load a model configuration from a YAML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let mut config: ModelConfig = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        // Validate and fix max_concurrent - must be at least 1 to avoid deadlock
        if config.schedule.max_concurrent == 0 {
            warn!(
                model = %config.model.id,
                "max_concurrent cannot be 0, setting to 1"
            );
            config.schedule.max_concurrent = 1;
        }

        debug!(model = %config.model.id, path = %path.display(), "Loaded model config");
        Ok(config)
    }

    /// Get forecast hours as a Vec.
    pub fn forecast_hours(&self) -> Vec<u32> {
        self.schedule
            .forecast_hours
            .as_ref()
            .map(|fh| fh.hours())
            .unwrap_or_default()
    }

    /// Check if this is an observation-type data source (vs forecast).
    pub fn is_observation(&self) -> bool {
        self.schedule.schedule_type == "observation"
    }

    /// Get the lookback period in minutes for observation data.
    /// Uses retention.hours converted to minutes for observation models.
    /// Falls back to schedule.lookback_minutes if retention is not set or zero.
    pub fn lookback_minutes(&self) -> u32 {
        if self.is_observation() {
            // Use retention hours as the lookback period
            let retention_minutes = self.retention.hours * 60;
            if retention_minutes > 0 {
                retention_minutes
            } else {
                // Fallback to schedule.lookback_minutes if retention not configured
                self.schedule.lookback_minutes
            }
        } else {
            // For forecast models, use the configured lookback_minutes (usually 0)
            self.schedule.lookback_minutes
        }
    }

    /// Build parameter filters for selective download from .idx files.
    ///
    /// Extracts parameter names and level strings from the configuration
    /// and returns filters suitable for matching against .idx file entries.
    pub fn build_param_filters(&self) -> Vec<(String, String)> {
        use crate::grib_index::level_to_idx_string;

        let mut filters = Vec::new();

        for param in &self.parameters {
            let param_name = &param.name;

            for level in &param.levels {
                // Try to get the level code
                let (level_code, level_type_unknown) = if let Some(code) = level.level_code {
                    (code, false)
                } else {
                    // Map level type name to code if not specified
                    match level.level_type.as_deref() {
                        Some("surface") => (1, false),
                        Some("isobaric") => (100, false),
                        Some("mean_sea_level") => (101, false),
                        Some("height_above_ground") => (103, false),
                        Some("entire_atmosphere") => (200, false),
                        Some("cloud_layer") | Some("low_cloud_layer") => (214, false),
                        Some("middle_cloud_layer") => (224, false),
                        Some("high_cloud_layer") => (234, false),
                        Some(unknown_type) => {
                            warn!(
                                model = %self.model.id,
                                param = %param_name,
                                level_type = %unknown_type,
                                "Unknown level type, parameter may not be included in selective download"
                            );
                            (0, true)
                        }
                        None => (0, true),
                    }
                };

                // If we have multiple values (e.g., isobaric levels)
                if let Some(ref values) = level.values {
                    for &value in values {
                        // For isobaric levels, values in config are in mb but .idx uses Pa
                        // Convert to what level_to_idx_string expects (Pa)
                        let value_for_idx = if level_code == 100 {
                            // Config stores mb values, multiply by 100 for Pa
                            value * 100
                        } else {
                            value
                        };

                        if let Some(level_str) =
                            level_to_idx_string(level_code, Some(value_for_idx))
                        {
                            filters.push((param_name.clone(), level_str));
                        } else if level_type_unknown {
                            // Already warned above
                        } else {
                            warn!(
                                model = %self.model.id,
                                param = %param_name,
                                level_code = level_code,
                                value = value,
                                "Could not convert level to .idx string"
                            );
                        }
                    }
                } else if let Some(value) = level.value {
                    // Single value
                    if let Some(level_str) = level_to_idx_string(level_code, Some(value)) {
                        filters.push((param_name.clone(), level_str));
                    } else if !level_type_unknown {
                        warn!(
                            model = %self.model.id,
                            param = %param_name,
                            level_code = level_code,
                            value = value,
                            "Could not convert level to .idx string"
                        );
                    }
                } else {
                    // No value (surface, entire_atmosphere, etc.)
                    if let Some(level_str) = level_to_idx_string(level_code, None) {
                        filters.push((param_name.clone(), level_str));
                    } else if let Some(ref display) = level.display {
                        // Use display string directly if we can't derive it
                        filters.push((param_name.clone(), display.clone()));
                    } else if !level_type_unknown {
                        warn!(
                            model = %self.model.id,
                            param = %param_name,
                            level_code = level_code,
                            "Could not convert level to .idx string and no display fallback"
                        );
                    }
                }
            }
        }

        // Deduplicate filters
        filters.sort();
        filters.dedup();

        debug!(
            model = %self.model.id,
            filter_count = filters.len(),
            "Built parameter filters for selective download"
        );

        filters
    }
}

/// Development environment limits for local testing.
/// These environment variables cap the forecast hours, retention, and cycles
/// to reduce data volume during local development.
#[derive(Debug, Default)]
struct DevLimits {
    /// Maximum forecast hours (caps forecast_hours.end)
    max_forecast_hours: Option<u32>,
    /// Maximum retention hours (caps retention.hours)
    max_retention_hours: Option<u32>,
    /// Maximum number of cycles (truncates cycles list)
    max_cycles: Option<usize>,
}

impl DevLimits {
    /// Load development limits from environment variables.
    fn from_env() -> Self {
        let max_forecast_hours = std::env::var("DEV_MAX_FORECAST_HOURS")
            .ok()
            .and_then(|v| v.parse().ok());

        let max_retention_hours = std::env::var("DEV_MAX_RETENTION_HOURS")
            .ok()
            .and_then(|v| v.parse().ok());

        let max_cycles = std::env::var("DEV_MAX_CYCLES")
            .ok()
            .and_then(|v| v.parse().ok());

        Self {
            max_forecast_hours,
            max_retention_hours,
            max_cycles,
        }
    }

    /// Check if any limits are configured.
    fn is_active(&self) -> bool {
        self.max_forecast_hours.is_some()
            || self.max_retention_hours.is_some()
            || self.max_cycles.is_some()
    }

    /// Apply limits to a model configuration.
    fn apply(&self, config: &mut ModelConfig) {
        // Cap forecast hours
        if let Some(max_hours) = self.max_forecast_hours {
            if let Some(ref mut fh) = config.schedule.forecast_hours {
                if fh.end > max_hours {
                    debug!(
                        model = %config.model.id,
                        original = fh.end,
                        capped = max_hours,
                        "Capping forecast_hours.end due to DEV_MAX_FORECAST_HOURS"
                    );
                    fh.end = max_hours;
                }
            }
        }

        // Cap retention hours.
        // Skip for high-latency sources (nasa_gesdisc) where retention must exceed
        // the data delay (e.g., NLDAS has 96h delay + 720h retention). Capping
        // retention below delay_hours makes the discovery window empty.
        if let Some(max_hours) = self.max_retention_hours {
            let skip_retention_cap = config.source.source_type == "nasa_gesdisc";
            if skip_retention_cap {
                debug!(
                    model = %config.model.id,
                    retention = config.retention.hours,
                    delay_hours = config.schedule.delay_hours,
                    "Skipping DEV_MAX_RETENTION_HOURS for high-latency source (nasa_gesdisc)"
                );
            } else if config.retention.hours > max_hours {
                debug!(
                    model = %config.model.id,
                    original = config.retention.hours,
                    capped = max_hours,
                    "Capping retention.hours due to DEV_MAX_RETENTION_HOURS"
                );
                config.retention.hours = max_hours;
            }
        }

        // Truncate cycles list
        if let Some(max_cycles) = self.max_cycles {
            if config.schedule.cycles.len() > max_cycles {
                debug!(
                    model = %config.model.id,
                    original = config.schedule.cycles.len(),
                    capped = max_cycles,
                    "Truncating cycles list due to DEV_MAX_CYCLES"
                );
                config.schedule.cycles.truncate(max_cycles);
            }
        }
    }
}

/// Load all enabled model configurations from a directory.
pub fn load_model_configs(config_dir: &Path) -> Result<Vec<ModelConfig>> {
    let models_dir = config_dir.join("models");

    if !models_dir.exists() {
        warn!(path = %models_dir.display(), "Models config directory not found");
        return Ok(Vec::new());
    }

    // Load development limits from environment
    let dev_limits = DevLimits::from_env();
    if dev_limits.is_active() {
        info!(
            max_forecast_hours = ?dev_limits.max_forecast_hours,
            max_retention_hours = ?dev_limits.max_retention_hours,
            max_cycles = ?dev_limits.max_cycles,
            "Development limits active - capping config values"
        );
    }

    let mut configs = Vec::new();

    for entry in std::fs::read_dir(&models_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path
            .extension()
            .is_some_and(|ext| ext == "yaml" || ext == "yml")
        {
            match ModelConfig::load(&path) {
                Ok(mut config) => {
                    if config.model.enabled {
                        // Apply dev limits if configured
                        if dev_limits.is_active() {
                            dev_limits.apply(&mut config);
                        }

                        info!(
                            model = %config.model.id,
                            name = %config.model.name,
                            cycles = ?config.schedule.cycles.len(),
                            forecast_hours = ?config.forecast_hours().len(),
                            retention_hours = config.retention.hours,
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
        assert_eq!(
            config.forecast_hours(),
            vec![0, 3, 6, 9, 12, 15, 18, 21, 24]
        );
    }

    #[test]
    fn test_observation_lookback_uses_retention() {
        let yaml = r#"
model:
  id: mrms
  name: "MRMS"
  enabled: true

source:
  type: aws_s3
  bucket: noaa-mrms-pds

grid:
  projection: latlon
  resolution: "0.01deg"
  bbox:
    min_lon: -130.0
    min_lat: 20.0
    max_lon: -60.0
    max_lat: 55.0

schedule:
  type: observation
  poll_interval_secs: 120
  lookback_minutes: 30

retention:
  hours: 2
"#;

        let config: ModelConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model.id, "mrms");
        assert!(config.is_observation());
        // For observation models, lookback_minutes should use retention.hours * 60
        assert_eq!(config.lookback_minutes(), 120); // 2 hours * 60 = 120 minutes
    }

    #[test]
    fn test_forecast_lookback_uses_schedule() {
        let yaml = r#"
model:
  id: gfs
  name: "GFS"
  enabled: true

source:
  type: aws_s3
  bucket: noaa-gfs-bdp-pds

grid:
  projection: geographic
  resolution: "0.25deg"
  bbox:
    min_lon: 0.0
    min_lat: -90.0
    max_lon: 360.0
    max_lat: 90.0

schedule:
  type: forecast
  cycles: [0, 6, 12, 18]
  poll_interval_secs: 3600
  lookback_minutes: 60
"#;

        let config: ModelConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.model.id, "gfs");
        assert!(!config.is_observation());
        // For forecast models, lookback_minutes should use schedule.lookback_minutes
        assert_eq!(config.lookback_minutes(), 60);
    }

    #[test]
    fn test_observation_fallback_to_schedule_lookback() {
        let yaml = r#"
model:
  id: test
  name: "Test"
  enabled: true

source:
  type: aws_s3
  bucket: test

grid:
  projection: latlon
  resolution: "1deg"
  bbox:
    min_lon: 0.0
    min_lat: 0.0
    max_lon: 1.0
    max_lat: 1.0

schedule:
  type: observation
  poll_interval_secs: 120
  lookback_minutes: 45

retention:
  hours: 0
"#;

        let config: ModelConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.is_observation());
        // When retention.hours is 0, should fallback to schedule.lookback_minutes
        assert_eq!(config.lookback_minutes(), 45);
    }

    #[test]
    fn test_dev_limits_skip_retention_for_nasa_gesdisc() {
        let yaml = r#"
model:
  id: nldas-noah
  name: "NLDAS-2 Noah"
  enabled: true

source:
  type: nasa_gesdisc
  base_url: "https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/NLDAS_NOAH0125_H.2.0"

grid:
  projection: geographic
  resolution: "0.125deg"
  bbox:
    min_lon: -125.0
    min_lat: 25.0
    max_lon: -67.0
    max_lat: 53.0

schedule:
  type: observation
  poll_interval_secs: 3600
  delay_hours: 96

retention:
  hours: 720
"#;

        let mut config: ModelConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.retention.hours, 720);

        // Apply dev limits with a low retention cap
        let dev_limits = DevLimits {
            max_forecast_hours: None,
            max_retention_hours: Some(2),
            max_cycles: None,
        };
        dev_limits.apply(&mut config);

        // nasa_gesdisc sources should NOT have retention capped
        assert_eq!(
            config.retention.hours, 720,
            "nasa_gesdisc models should skip DEV_MAX_RETENTION_HOURS"
        );
    }

    #[test]
    fn test_dev_limits_still_cap_regular_models() {
        let yaml = r#"
model:
  id: mrms
  name: "MRMS"
  enabled: true

source:
  type: aws_s3
  bucket: noaa-mrms-pds

grid:
  projection: latlon
  resolution: "0.01deg"
  bbox:
    min_lon: -130.0
    min_lat: 20.0
    max_lon: -60.0
    max_lat: 55.0

schedule:
  type: observation
  poll_interval_secs: 120

retention:
  hours: 48
"#;

        let mut config: ModelConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.retention.hours, 48);

        let dev_limits = DevLimits {
            max_forecast_hours: None,
            max_retention_hours: Some(2),
            max_cycles: None,
        };
        dev_limits.apply(&mut config);

        // Regular models should still get capped
        assert_eq!(
            config.retention.hours, 2,
            "Regular models should be capped by DEV_MAX_RETENTION_HOURS"
        );
    }
}
