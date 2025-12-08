//! Configuration loader for weather-wms ingester
//!
//! Loads and validates YAML configuration files for:
//! - Global ingestion settings (ingestion.yaml)
//! - Model configurations (models/*.yaml)
//! - GRIB2 parameter tables (parameters/*.yaml)
//!
//! Supports environment variable substitution using ${VAR} syntax.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ============================================================================
// Ingestion Configuration (ingestion.yaml)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionConfig {
    pub database: DatabaseConfig,
    pub cache: CacheConfig,
    pub storage: StorageConfig,
    pub download: DownloadConfig,
    pub ingestion: IngestionBehaviorConfig,
    pub logging: LoggingConfig,
    pub metrics: MetricsConfig,
    pub health: HealthConfig,
    pub models: ModelsLoadConfig,
    pub parameters: ParametersLoadConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    pub max_connections: u32,
    pub connection_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
    pub database: u32,
    pub ttl_secs: u64,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub base_path: String,
    pub temp_path: String,
    pub cache_path: String,
    pub models: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadConfig {
    pub parallel_downloads: u32,
    pub max_retries: u32,
    pub retry_delay_secs: u64,
    pub connect_timeout_secs: u64,
    pub read_timeout_secs: u64,
    pub user_agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionBehaviorConfig {
    pub lookback_hours: u64,
    pub cleanup_enabled: bool,
    pub cleanup_interval_hours: u64,
    pub batch_size: usize,
    pub deduplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub port: u16,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    pub enabled: bool,
    pub port: u16,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsLoadConfig {
    pub config_dir: String,
    pub enabled: String,
    pub disabled: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParametersLoadConfig {
    pub tables_dir: String,
    pub tables: Vec<String>,
}

// ============================================================================
// Model Configuration (models/*.yaml)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub model: ModelMetadata,
    pub source: SourceConfig,
    pub grid: GridConfig,
    pub schedule: ScheduleConfig,
    pub retention: RetentionConfig,
    pub parameters: Vec<ParameterConfig>,
    #[serde(default)]
    pub composites: Vec<CompositeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    #[serde(rename = "type")]
    pub source_type: String,
    pub bucket: Option<String>,
    pub prefix_template: Option<String>,
    pub file_pattern: Option<String>,
    pub region: Option<String>,
    pub base_url: Option<String>,
    pub path_template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridConfig {
    pub projection: String,
    pub resolution: Option<String>,
    pub bbox: Option<BBoxConfig>,
    pub lon_convention: Option<String>,
    pub satellite_longitude: Option<f64>,
    pub satellite_height: Option<f64>,
    pub sweep_axis: Option<String>,
    pub nx: Option<u32>,
    pub ny: Option<u32>,
    pub dx: Option<f64>,
    pub dy: Option<f64>,
    pub lat1: Option<f64>,
    pub lon1: Option<f64>,
    pub latin1: Option<f64>,
    pub latin2: Option<f64>,
    pub lov: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BBoxConfig {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    pub cycles: Option<Vec<u8>>,
    pub forecast_hours: Option<ForecastHoursConfig>,
    pub poll_interval_secs: u64,
    pub delay_hours: Option<u64>,
    pub bands: Option<Vec<String>>,
    pub interval_minutes: Option<u64>,
    pub products: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastHoursConfig {
    pub start: u32,
    pub end: u32,
    pub step: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    pub hours: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterConfig {
    pub name: String,
    pub description: String,
    pub levels: Vec<LevelConfig>,
    pub style: String,
    pub units: String,
    pub display_units: Option<String>,
    pub conversion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelConfig {
    #[serde(rename = "type")]
    pub level_type: String,
    pub value: Option<f64>,
    pub values: Option<Vec<f64>>,
    pub display: Option<String>,
    pub display_template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeConfig {
    pub name: String,
    pub description: String,
    pub requires: Vec<String>,
    pub renderer: String,
    pub style: String,
}

// ============================================================================
// GRIB2 Parameter Tables (parameters/*.yaml)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grib2ParameterTable {
    pub table: TableMetadata,
    #[serde(flatten)]
    pub categories: HashMap<String, HashMap<String, Vec<ParameterDefinition>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableMetadata {
    pub source: String,
    pub discipline: u8,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDefinition {
    pub parameter: u8,
    pub name: String,
    pub description: String,
    pub units: String,
}

// ============================================================================
// Aggregated Configuration
// ============================================================================

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AllConfigs {
    pub ingestion: IngestionConfig,
    pub models: HashMap<String, ModelConfig>,
    pub parameter_tables: HashMap<String, Grib2ParameterTable>,
}

// ============================================================================
// Loading Functions
// ============================================================================

/// Load and parse ingestion.yaml with environment variable substitution
pub fn load_ingestion_config<P: AsRef<Path>>(path: P) -> Result<IngestionConfig> {
    let content = fs::read_to_string(path.as_ref())
        .with_context(|| format!("Failed to read ingestion config from {:?}", path.as_ref()))?;
    
    let expanded = expand_env_vars(&content)?;
    
    let config: IngestionConfig = serde_yaml::from_str(&expanded)
        .with_context(|| "Failed to parse ingestion config YAML")?;
    
    validate_ingestion_config(&config)?;
    
    Ok(config)
}

/// Load and parse a model configuration YAML file
pub fn load_model_config<P: AsRef<Path>>(path: P) -> Result<ModelConfig> {
    let content = fs::read_to_string(path.as_ref())
        .with_context(|| format!("Failed to read model config from {:?}", path.as_ref()))?;
    
    let expanded = expand_env_vars(&content)?;
    
    let config: ModelConfig = serde_yaml::from_str(&expanded)
        .with_context(|| format!("Failed to parse model config from {:?}", path.as_ref()))?;
    
    validate_model_config(&config)?;
    
    Ok(config)
}

/// Load and parse a GRIB2 parameter table YAML file
pub fn load_parameter_table<P: AsRef<Path>>(path: P) -> Result<Grib2ParameterTable> {
    let content = fs::read_to_string(path.as_ref())
        .with_context(|| format!("Failed to read parameter table from {:?}", path.as_ref()))?;
    
    let table: Grib2ParameterTable = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse parameter table from {:?}", path.as_ref()))?;
    
    Ok(table)
}

/// Load all configuration files from a base directory
pub fn load_all_configs<P: AsRef<Path>>(base_path: P) -> Result<AllConfigs> {
    let base = base_path.as_ref();
    
    // Load ingestion config
    let ingestion_path = base.join("config/ingestion.yaml");
    let ingestion = load_ingestion_config(&ingestion_path)?;
    
    // Resolve model configs directory (may be relative to base_path)
    let models_dir = if Path::new(&ingestion.models.config_dir).is_absolute() {
        PathBuf::from(&ingestion.models.config_dir)
    } else {
        base.join(&ingestion.models.config_dir)
    };
    let models = load_model_configs(&models_dir)?;
    
    // Resolve parameter tables directory (may be relative to base_path)
    let tables_dir = if Path::new(&ingestion.parameters.tables_dir).is_absolute() {
        PathBuf::from(&ingestion.parameters.tables_dir)
    } else {
        base.join(&ingestion.parameters.tables_dir)
    };
    let parameter_tables = load_parameter_tables(&tables_dir, &ingestion.parameters.tables)?;
    
    Ok(AllConfigs {
        ingestion,
        models,
        parameter_tables,
    })
}

/// Load all model configs from a directory
pub fn load_model_configs<P: AsRef<Path>>(models_dir: P) -> Result<HashMap<String, ModelConfig>> {
    let mut models = HashMap::new();
    
    let entries = fs::read_dir(models_dir.as_ref())
        .with_context(|| format!("Failed to read models directory {:?}", models_dir.as_ref()))?;
    
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") 
            || path.extension().and_then(|s| s.to_str()) == Some("yml") {
            let config = load_model_config(&path)?;
            let model_id = config.model.id.clone();
            models.insert(model_id, config);
        }
    }
    
    Ok(models)
}

/// Load specified parameter tables
pub fn load_parameter_tables<P: AsRef<Path>>(
    tables_dir: P,
    table_names: &[String],
) -> Result<HashMap<String, Grib2ParameterTable>> {
    let mut tables = HashMap::new();
    
    for name in table_names {
        let path = tables_dir.as_ref().join(format!("{}.yaml", name));
        let table = load_parameter_table(&path)?;
        tables.insert(name.clone(), table);
    }
    
    Ok(tables)
}

// ============================================================================
// Environment Variable Expansion
// ============================================================================

/// Expand environment variables in YAML content
/// Supports ${VAR} and ${VAR:-default} syntax
fn expand_env_vars(content: &str) -> Result<String> {
    let mut result = String::new();
    let mut chars = content.chars().peekable();
    
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            
            let mut var_expr = String::new();
            let mut brace_count = 1;
            
            while brace_count > 0 {
                match chars.next() {
                    Some('{') => {
                        brace_count += 1;
                        var_expr.push('{');
                    }
                    Some('}') => {
                        brace_count -= 1;
                        if brace_count > 0 {
                            var_expr.push('}');
                        }
                    }
                    Some(c) => var_expr.push(c),
                    None => anyhow::bail!("Unclosed variable substitution: ${{{}", var_expr),
                }
            }
            
            let value = resolve_var_expr(&var_expr)?;
            result.push_str(&value);
        } else {
            result.push(ch);
        }
    }
    
    Ok(result)
}

/// Resolve variable expression (supports VAR and VAR:-default syntax)
fn resolve_var_expr(expr: &str) -> Result<String> {
    if let Some((var_name, default)) = expr.split_once(":-") {
        match std::env::var(var_name.trim()) {
            Ok(val) if !val.is_empty() => Ok(val),
            _ => Ok(default.to_string()),
        }
    } else {
        std::env::var(expr.trim())
            .with_context(|| format!("Environment variable {} not set", expr))
    }
}

// ============================================================================
// Validation
// ============================================================================

fn validate_ingestion_config(config: &IngestionConfig) -> Result<()> {
    // Validate database config
    anyhow::ensure!(
        !config.database.host.is_empty(),
        "Database host cannot be empty"
    );
    anyhow::ensure!(
        config.database.port > 0,
        "Database port must be greater than 0"
    );
    
    // Validate cache config
    anyhow::ensure!(
        !config.cache.host.is_empty(),
        "Cache host cannot be empty"
    );
    
    // Validate logging level
    let valid_levels = ["trace", "debug", "info", "warn", "error"];
    anyhow::ensure!(
        valid_levels.contains(&config.logging.level.as_str()),
        "Invalid log level: {}. Must be one of: {:?}",
        config.logging.level,
        valid_levels
    );
    
    // Validate logging format
    let valid_formats = ["json", "pretty"];
    anyhow::ensure!(
        valid_formats.contains(&config.logging.format.as_str()),
        "Invalid log format: {}. Must be one of: {:?}",
        config.logging.format,
        valid_formats
    );
    
    Ok(())
}

fn validate_model_config(config: &ModelConfig) -> Result<()> {
    // Validate model ID
    anyhow::ensure!(
        !config.model.id.is_empty(),
        "Model ID cannot be empty"
    );
    
    // Validate source type
    let valid_sources = ["aws_s3", "aws_s3_goes", "aws_s3_grib2", "http", "local"];
    anyhow::ensure!(
        valid_sources.contains(&config.source.source_type.as_str()),
        "Invalid source type: {}. Must be one of: {:?}",
        config.source.source_type,
        valid_sources
    );
    
    // Validate projection
    let valid_projections = ["geographic", "latlon", "lambert_conformal", "geostationary"];
    anyhow::ensure!(
        valid_projections.contains(&config.grid.projection.as_str()),
        "Invalid projection: {}. Must be one of: {:?}",
        config.grid.projection,
        valid_projections
    );
    
    // Validate bbox if present
    if let Some(bbox) = &config.grid.bbox {
        anyhow::ensure!(
            bbox.min_lon < bbox.max_lon,
            "bbox.min_lon must be less than bbox.max_lon"
        );
        anyhow::ensure!(
            bbox.min_lat < bbox.max_lat,
            "bbox.min_lat must be less than bbox.max_lat"
        );
    }
    
    // Validate parameters
    anyhow::ensure!(
        !config.parameters.is_empty() || !config.composites.is_empty(),
        "Model must have at least one parameter or composite layer"
    );
    
    Ok(())
}

// ============================================================================
// Conversion to Runtime Config
// ============================================================================

use crate::config::{
    IngesterConfig as RuntimeIngesterConfig,
    ModelConfig as RuntimeModelConfig,
    DataSource as RuntimeDataSource,
    ParameterConfig as RuntimeParameterConfig,
    GribFilter,
};
use storage::ObjectStorageConfig;

impl AllConfigs {
    /// Convert to runtime ingester config
    pub fn to_runtime_config(&self) -> Result<RuntimeIngesterConfig> {
        let mut runtime_models = HashMap::new();
        
        // Convert each model config
        for (model_id, model_cfg) in &self.models {
            if model_cfg.model.enabled {
                runtime_models.insert(
                    model_id.clone(),
                    convert_model_config(model_cfg)?,
                );
            }
        }
        
        // Build storage config from ingestion config
        let storage = ObjectStorageConfig {
            endpoint: std::env::var("S3_ENDPOINT")
                .unwrap_or_else(|_| "http://minio:9000".to_string()),
            bucket: std::env::var("S3_BUCKET")
                .unwrap_or_else(|_| "weather-data".to_string()),
            access_key_id: std::env::var("S3_ACCESS_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            secret_access_key: std::env::var("S3_SECRET_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            region: std::env::var("S3_REGION")
                .unwrap_or_else(|_| "us-east-1".to_string()),
            allow_http: std::env::var("S3_ALLOW_HTTP")
                .map(|v| v == "true")
                .unwrap_or(true),
        };
        
        // Build database URL from config
        let database_url = format!(
            "postgresql://{}:{}@{}:{}/{}",
            self.ingestion.database.user,
            self.ingestion.database.password,
            self.ingestion.database.host,
            self.ingestion.database.port,
            self.ingestion.database.database,
        );
        
        // Build Redis URL from config
        let redis_url = if self.ingestion.cache.password.is_empty() {
            format!(
                "redis://{}:{}",
                self.ingestion.cache.host,
                self.ingestion.cache.port,
            )
        } else {
            format!(
                "redis://:{}@{}:{}",
                self.ingestion.cache.password,
                self.ingestion.cache.host,
                self.ingestion.cache.port,
            )
        };
        
        Ok(RuntimeIngesterConfig {
            storage,
            database_url,
            redis_url,
            models: runtime_models,
            poll_interval_secs: self.ingestion.ingestion.lookback_hours * 3600 / 24, // Default polling
            parallel_downloads: self.ingestion.download.parallel_downloads as usize,
            retention_hours: self.ingestion.ingestion.lookback_hours,
        })
    }
}

/// Convert a YAML model config to runtime model config
fn convert_model_config(cfg: &ModelConfig) -> Result<RuntimeModelConfig> {
    // Convert source
    let source = match cfg.source.source_type.as_str() {
        "aws_s3" | "aws_s3_grib2" => RuntimeDataSource::NoaaAws {
            bucket: cfg.source.bucket.clone()
                .ok_or_else(|| anyhow::anyhow!("aws_s3 source missing bucket"))?,
            // Use prefix_template or fall back to path_pattern for MRMS
            prefix_template: cfg.source.prefix_template.clone()
                .or_else(|| cfg.source.path_template.clone())
                .unwrap_or_default(),
        },
        "aws_s3_goes" => {
            // GOES satellite data on AWS
            let bucket = cfg.source.bucket.clone()
                .ok_or_else(|| anyhow::anyhow!("aws_s3_goes source missing bucket"))?;
            let product = cfg.source.file_pattern.clone()
                .ok_or_else(|| anyhow::anyhow!("aws_s3_goes source missing file_pattern (product)"))?;
            
            // Extract band numbers from schedule.bands
            let bands = cfg.schedule.bands.as_ref()
                .map(|b| b.iter().map(|s| {
                    // Parse band strings like "C02", "C13" to numbers
                    s.trim_start_matches('C').parse::<u8>().unwrap_or(2)
                }).collect())
                .unwrap_or_else(|| vec![2, 13]); // Default: visible red, clean IR
            
            RuntimeDataSource::GoesAws {
                bucket,
                product,
                bands,
            }
        },
        "http" => RuntimeDataSource::Nomads {
            base_url: cfg.source.base_url.clone()
                .ok_or_else(|| anyhow::anyhow!("http source missing base_url"))?,
            path_template: cfg.source.path_template.clone()
                .ok_or_else(|| anyhow::anyhow!("http source missing path_template"))?,
        },
        other => anyhow::bail!("Unsupported source type: {}", other),
    };
    
    // Convert parameters
    let mut runtime_params = Vec::new();
    for param in &cfg.parameters {
        for level in &param.levels {
            let level_str = if let Some(display) = &level.display {
                display.clone()
            } else if let Some(value) = level.value {
                format!("{} {}", value, level.level_type)
            } else {
                level.level_type.clone()
            };
            
            runtime_params.push(RuntimeParameterConfig {
                name: format!("{}_{}", param.name, sanitize_level(&level_str)),
                grib_filter: GribFilter {
                    level: level_str,
                    parameter: param.name.clone(),
                },
            });
        }
    }
    
    // Extract cycles (convert u8 to u32)
    let cycles = cfg.schedule.cycles.as_ref()
        .map(|c| c.iter().map(|&x| x as u32).collect())
        .unwrap_or_else(|| (0..24).collect());
    
    // Extract forecast hours
    let forecast_hours = if let Some(ref fh_cfg) = cfg.schedule.forecast_hours {
        (fh_cfg.start..=fh_cfg.end).step_by(fh_cfg.step as usize).collect()
    } else {
        vec![0]
    };
    
    Ok(RuntimeModelConfig {
        name: cfg.model.name.clone(),
        source,
        parameters: runtime_params,
        cycles,
        forecast_hours,
        resolution: cfg.grid.resolution.clone()
            .unwrap_or_else(|| "default".to_string()),
        poll_interval_secs: cfg.schedule.poll_interval_secs,
    })
}

/// Sanitize level string for use in parameter name
fn sanitize_level(level: &str) -> String {
    level
        .to_lowercase()
        .replace(" ", "_")
        .replace(".", "")
        .replace("-", "_")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_expand_env_vars_simple() {
        std::env::set_var("TEST_VAR", "test_value");
        let result = expand_env_vars("prefix_${TEST_VAR}_suffix").unwrap();
        assert_eq!(result, "prefix_test_value_suffix");
    }
    
    #[test]
    fn test_expand_env_vars_with_default() {
        std::env::remove_var("NONEXISTENT_VAR");
        let result = expand_env_vars("value_${NONEXISTENT_VAR:-default}_end").unwrap();
        assert_eq!(result, "value_default_end");
    }
    
    #[test]
    fn test_expand_env_vars_missing_required() {
        std::env::remove_var("REQUIRED_VAR");
        let result = expand_env_vars("${REQUIRED_VAR}");
        assert!(result.is_err());
    }
    
    #[test]
    fn test_resolve_var_expr_with_default() {
        std::env::remove_var("UNSET_VAR");
        let result = resolve_var_expr("UNSET_VAR:-5432").unwrap();
        assert_eq!(result, "5432");
    }
    
    #[test]
    fn test_resolve_var_expr_override_default() {
        std::env::set_var("SET_VAR", "custom");
        let result = resolve_var_expr("SET_VAR:-default").unwrap();
        assert_eq!(result, "custom");
    }
    
    #[test]
    #[ignore] // Run with: cargo test --package ingester -- --ignored --nocapture
    fn test_load_real_yaml_configs() {
        use std::env;
        
        // Set required env vars
        env::set_var("POSTGRES_HOST", "localhost");
        env::set_var("POSTGRES_PORT", "5432");
        env::set_var("POSTGRES_DB", "weather");
        env::set_var("POSTGRES_USER", "test");
        env::set_var("POSTGRES_PASSWORD", "testpass");
        env::set_var("REDIS_HOST", "localhost");
        env::set_var("REDIS_PORT", "6379");
        env::set_var("REDIS_PASSWORD", "");
        env::set_var("DATA_DIR", "/tmp/data");
        env::set_var("S3_ENDPOINT", "http://minio:9000");
        env::set_var("S3_BUCKET", "weather");
        env::set_var("S3_ACCESS_KEY", "test");
        env::set_var("S3_SECRET_KEY", "test");
        
        // Load all configs (from workspace root)
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let workspace_root = std::path::Path::new(&manifest_dir).parent().unwrap().parent().unwrap();
        let all_configs = load_all_configs(workspace_root).expect("Failed to load configs");
        
        println!("\nLoaded configs:");
        println!("  Models: {}", all_configs.models.len());
        println!("  Parameter tables: {}", all_configs.parameter_tables.len());
        
        assert!(all_configs.models.contains_key("gfs"), "GFS model not loaded");
        assert!(all_configs.models.contains_key("hrrr"), "HRRR model not loaded");
        
        // Convert to runtime config
        let runtime_cfg = all_configs.to_runtime_config()
            .expect("Failed to convert to runtime config");
        
        println!("\nRuntime config:");
        println!("  Models: {}", runtime_cfg.models.len());
        println!("  Parallel downloads: {}", runtime_cfg.parallel_downloads);
        
        assert!(!runtime_cfg.models.is_empty(), "No runtime models loaded");
        assert!(runtime_cfg.models.contains_key("gfs"), "GFS not in runtime models");
    }
}
