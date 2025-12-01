//! Admin dashboard API endpoints.
//!
//! Provides endpoints for monitoring and managing the WMS/ingestion system.

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};
use storage::DatasetQuery;

use crate::state::AppState;

// ============================================================================
// Response Types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct IngestionStatusResponse {
    pub models: Vec<ModelStatus>,
    pub catalog_summary: CatalogSummary,
    pub system_info: SystemInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub name: String,
    pub status: String,
    pub enabled: bool,
    pub last_ingest: Option<String>,
    pub total_files: u64,
    pub parameters: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogSummary {
    pub total_datasets: u64,
    pub total_parameters: u64,
    /// Total size across all storage (raw + shredded)
    pub total_size_bytes: u64,
    /// Size of raw ingested files (raw/ prefix in MinIO)
    pub raw_size_bytes: u64,
    /// Size of shredded/processed files (shredded/ prefix in MinIO)
    pub shredded_size_bytes: u64,
    /// Number of raw files
    pub raw_object_count: u64,
    /// Number of shredded files
    pub shredded_object_count: u64,
    pub models: Vec<ModelCatalogInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalogInfo {
    pub model: String,
    pub parameter_count: u64,
    pub dataset_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    pub cache_enabled: bool,
    pub rendering_workers: usize,
    pub uptime_seconds: u64,
    pub cpu_cores: usize,
    pub worker_threads: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelConfigResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: SourceInfo,
    pub grid: GridInfo,
    pub schedule: ScheduleInfo,
    pub parameters: Vec<ParameterInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub source_type: String,
    pub bucket: Option<String>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GridInfo {
    pub projection: String,
    pub resolution: Option<String>,
    pub bbox: Option<BBoxInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BBoxInfo {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduleInfo {
    pub cycles: Vec<u8>,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParameterInfo {
    pub name: String,
    pub description: String,
    pub levels: Vec<String>,
    pub style: String,
    pub units: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelsListResponse {
    pub models: Vec<ModelSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelSummary {
    pub id: String,
    pub name: String,
    pub model_type: String,
    pub source_type: String,
    pub projection: String,
    pub parameter_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelConfigYamlResponse {
    pub id: String,
    pub yaml: String,
}

// ============================================================================
// Ingestion Log Types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct IngestionLogResponse {
    pub entries: Vec<IngestionLogEntry>,
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestionLogEntry {
    pub timestamp: String,
    pub model: String,
    pub parameter: String,
    pub level: String,
    pub reference_time: String,
    pub forecast_hour: u32,
    pub file_size: u64,
    pub storage_path: String,
}

#[derive(Debug, Deserialize)]
pub struct IngestionLogQuery {
    pub limit: Option<usize>,
    pub model: Option<String>,
}

// ============================================================================
// Shredding Preview Types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ShredPreviewResponse {
    pub model_id: String,
    pub model_name: String,
    pub source_type: String,
    pub parameters_to_extract: Vec<ShredParameter>,
    pub total_extractions: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShredParameter {
    pub name: String,
    pub description: String,
    pub levels: Vec<ShredLevel>,
    pub style: String,
    pub units: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShredLevel {
    pub level_type: String,
    pub value: Option<String>,
    pub display: String,
    pub storage_path_template: String,
}

#[derive(Debug, Deserialize)]
pub struct ShredPreviewQuery {
    pub model: String,
}

// ============================================================================
// Config Update Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    pub yaml: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateConfigResponse {
    pub success: bool,
    pub message: String,
    pub validation_errors: Vec<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /admin/ingestion/status - Get overall ingestion status
pub async fn ingestion_status_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Getting ingestion status");
    
    // For now, return mock data since we don't have real-time ingestion tracking yet
    // In a full implementation, this would query the catalog and cache
    let catalog = &state.catalog;
    
    // Get available layers from catalog
    let query = storage::DatasetQuery {
        model: None,
        parameter: None,
        level: None,
        time_range: None,
        bbox: None,
    };
    let datasets = catalog.find_datasets(&query).await.unwrap_or_default();
    
    // Group by model
    let mut models_map = std::collections::HashMap::new();
    for dataset in &datasets {
        let model_id = dataset.model.clone();
        models_map.entry(model_id).or_insert_with(Vec::new).push(dataset.clone());
    }
    
    let mut models = Vec::new();
    for (model_id, model_datasets) in models_map.iter() {
        let parameters: Vec<String> = model_datasets
            .iter()
            .map(|d| d.parameter.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        
        models.push(ModelStatus {
            id: model_id.clone(),
            name: format!("{} Model", model_id.to_uppercase()),
            status: "active".to_string(),
            enabled: true,
            last_ingest: model_datasets.first().map(|d| {
                d.reference_time.format("%Y-%m-%d %H:%M:%S UTC").to_string()
            }),
            total_files: model_datasets.len() as u64,
            parameters,
        });
    }
    
    // Get detailed storage stats from MinIO (raw vs shredded breakdown)
    let storage_stats = state.storage.detailed_stats().await.unwrap_or_else(|e| {
        warn!(error = %e, "Failed to get detailed storage stats, using defaults");
        storage::DetailedStorageStats {
            raw_size_bytes: 0,
            raw_object_count: 0,
            shredded_size_bytes: 0,
            shredded_object_count: 0,
            total_size_bytes: 0,
            total_object_count: 0,
            bucket: "unknown".to_string(),
        }
    });
    
    let catalog_summary = CatalogSummary {
        total_datasets: datasets.len() as u64,
        total_parameters: models.iter().map(|m| m.parameters.len() as u64).sum(),
        total_size_bytes: storage_stats.total_size_bytes,
        raw_size_bytes: storage_stats.raw_size_bytes,
        shredded_size_bytes: storage_stats.shredded_size_bytes,
        raw_object_count: storage_stats.raw_object_count,
        shredded_object_count: storage_stats.shredded_object_count,
        models: models.iter().map(|m| ModelCatalogInfo {
            model: m.id.clone(),
            parameter_count: m.parameters.len() as u64,
            dataset_count: m.total_files,
        }).collect(),
    };
    
    let cpu_cores = num_cpus::get();
    let system_info = SystemInfo {
        cache_enabled: true,
        rendering_workers: cpu_cores,
        uptime_seconds: 0, // TODO: track actual uptime
        cpu_cores,
        worker_threads: cpu_cores, // Default to CPU cores
    };
    
    let response = IngestionStatusResponse {
        models,
        catalog_summary,
        system_info,
    };
    
    Json(response)
}

/// GET /admin/config/models - List all model configurations
pub async fn list_models_handler(
    Extension(_state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Listing model configurations");
    
    // Try to load model configs from YAML
    match load_model_summaries_from_yaml().await {
        Ok(models) => Json(ModelsListResponse { models }).into_response(),
        Err(e) => {
            warn!("Failed to load model configs: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load configs: {}", e)).into_response()
        }
    }
}

/// GET /admin/config/models/:id - Get specific model configuration (returns raw YAML)
pub async fn get_model_config_handler(
    Extension(_state): Extension<Arc<AppState>>,
    Path(model_id): Path<String>,
) -> impl IntoResponse {
    info!("Admin: Getting config for model: {}", model_id);
    
    match load_model_yaml(&model_id).await {
        Ok(Some(yaml)) => Json(ModelConfigYamlResponse { 
            id: model_id, 
            yaml 
        }).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, format!("Model '{}' not found", model_id)).into_response(),
        Err(e) => {
            warn!("Failed to load model config: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load config: {}", e)).into_response()
        }
    }
}

/// GET /admin/ingestion/log - Get recent ingestion activity
pub async fn ingestion_log_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<IngestionLogQuery>,
) -> impl IntoResponse {
    info!("Admin: Getting ingestion log");
    
    let limit = params.limit.unwrap_or(50).min(500);
    let catalog = &state.catalog;
    
    // Get recent ingestions (last 60 minutes by default)
    match catalog.get_recent_ingestions(60).await {
        Ok(datasets) => {
            let mut entries: Vec<IngestionLogEntry> = datasets
                .into_iter()
                .filter(|d| {
                    // Filter by model if specified
                    params.model.as_ref().map_or(true, |m| &d.model == m)
                })
                .take(limit)
                .map(|d| IngestionLogEntry {
                    timestamp: d.reference_time.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                    model: d.model,
                    parameter: d.parameter,
                    level: d.level,
                    reference_time: d.reference_time.format("%Y-%m-%d %H:%M UTC").to_string(),
                    forecast_hour: d.forecast_hour,
                    file_size: d.file_size,
                    storage_path: d.storage_path,
                })
                .collect();
            
            // Sort by timestamp descending (most recent first)
            entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            
            let total_count = entries.len();
            Json(IngestionLogResponse { entries, total_count }).into_response()
        }
        Err(e) => {
            warn!("Failed to get ingestion log: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get log: {}", e)).into_response()
        }
    }
}

/// GET /admin/preview-shred - Preview what parameters will be extracted for a model
pub async fn preview_shred_handler(
    Extension(_state): Extension<Arc<AppState>>,
    Query(params): Query<ShredPreviewQuery>,
) -> impl IntoResponse {
    info!("Admin: Preview shredding for model: {}", params.model);
    
    match build_shred_preview(&params.model).await {
        Ok(Some(preview)) => Json(preview).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, format!("Model '{}' not found", params.model)).into_response(),
        Err(e) => {
            warn!("Failed to build shred preview: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to build preview: {}", e)).into_response()
        }
    }
}

/// PUT /admin/config/models/:id - Update model configuration
pub async fn update_model_config_handler(
    Extension(_state): Extension<Arc<AppState>>,
    Path(model_id): Path<String>,
    Json(payload): Json<UpdateConfigRequest>,
) -> impl IntoResponse {
    info!("Admin: Updating config for model: {}", model_id);
    
    // Validate YAML syntax
    let validation_errors = validate_model_yaml(&payload.yaml);
    if !validation_errors.is_empty() {
        return Json(UpdateConfigResponse {
            success: false,
            message: "Validation failed".to_string(),
            validation_errors,
        }).into_response();
    }
    
    // Save the config
    match save_model_yaml(&model_id, &payload.yaml).await {
        Ok(()) => Json(UpdateConfigResponse {
            success: true,
            message: format!("Configuration for '{}' saved successfully", model_id),
            validation_errors: vec![],
        }).into_response(),
        Err(e) => {
            warn!("Failed to save model config: {}", e);
            Json(UpdateConfigResponse {
                success: false,
                message: format!("Failed to save: {}", e),
                validation_errors: vec![],
            }).into_response()
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Load summary info for all models from YAML files
async fn load_model_summaries_from_yaml() -> anyhow::Result<Vec<ModelSummary>> {
    use std::fs;
    use std::path::Path;
    
    let models_dir = Path::new("config/models");
    if !models_dir.exists() {
        return Ok(Vec::new());
    }
    
    let mut summaries = Vec::new();
    
    for entry in fs::read_dir(models_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(Some(summary)) = load_model_summary(stem).await {
                    summaries.push(summary);
                }
            }
        }
    }
    
    Ok(summaries)
}

/// Load summary info for a single model
async fn load_model_summary(model_id: &str) -> anyhow::Result<Option<ModelSummary>> {
    use std::fs;
    use std::path::Path;
    
    let config_path = Path::new("config/models").join(format!("{}.yaml", model_id));
    
    if !config_path.exists() {
        return Ok(None);
    }
    
    let contents = fs::read_to_string(&config_path)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&contents)?;
    
    let model = yaml.get("model");
    let source = yaml.get("source");
    let grid = yaml.get("grid");
    let parameters = yaml.get("parameters");
    
    let summary = ModelSummary {
        id: model_id.to_string(),
        name: model
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(model_id)
            .to_string(),
        model_type: model
            .and_then(|m| m.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        source_type: source
            .and_then(|s| s.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        projection: grid
            .and_then(|g| g.get("projection"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        parameter_count: parameters
            .and_then(|p| p.as_sequence())
            .map(|s| s.len())
            .unwrap_or(0),
    };
    
    Ok(Some(summary))
}

/// Load raw YAML content for a model
async fn load_model_yaml(model_id: &str) -> anyhow::Result<Option<String>> {
    use std::fs;
    use std::path::Path;
    
    let config_path = Path::new("config/models").join(format!("{}.yaml", model_id));
    
    if !config_path.exists() {
        return Ok(None);
    }
    
    let contents = fs::read_to_string(&config_path)?;
    Ok(Some(contents))
}

/// Save YAML content for a model
async fn save_model_yaml(model_id: &str, yaml_content: &str) -> anyhow::Result<()> {
    use std::fs;
    use std::path::Path;
    
    let config_path = Path::new("config/models").join(format!("{}.yaml", model_id));
    
    // Create backup of existing file
    if config_path.exists() {
        let backup_path = Path::new("config/models").join(format!("{}.yaml.bak", model_id));
        fs::copy(&config_path, &backup_path)?;
    }
    
    fs::write(&config_path, yaml_content)?;
    Ok(())
}

/// Validate YAML content for a model configuration
fn validate_model_yaml(yaml_content: &str) -> Vec<String> {
    let mut errors = Vec::new();
    
    // Check YAML syntax
    let yaml: serde_yaml::Value = match serde_yaml::from_str(yaml_content) {
        Ok(v) => v,
        Err(e) => {
            errors.push(format!("YAML syntax error: {}", e));
            return errors;
        }
    };
    
    // Check required sections
    if yaml.get("model").is_none() {
        errors.push("Missing required section: 'model'".to_string());
    } else {
        let model = yaml.get("model").unwrap();
        if model.get("id").is_none() {
            errors.push("Missing required field: 'model.id'".to_string());
        }
        if model.get("name").is_none() {
            errors.push("Missing required field: 'model.name'".to_string());
        }
    }
    
    if yaml.get("source").is_none() {
        errors.push("Missing required section: 'source'".to_string());
    } else {
        let source = yaml.get("source").unwrap();
        if source.get("type").is_none() {
            errors.push("Missing required field: 'source.type'".to_string());
        }
    }
    
    if yaml.get("grid").is_none() {
        errors.push("Missing required section: 'grid'".to_string());
    } else {
        let grid = yaml.get("grid").unwrap();
        if grid.get("projection").is_none() {
            errors.push("Missing required field: 'grid.projection'".to_string());
        }
    }
    
    if yaml.get("schedule").is_none() {
        errors.push("Missing required section: 'schedule'".to_string());
    }
    
    // Check parameters array
    if let Some(params) = yaml.get("parameters") {
        if let Some(params_seq) = params.as_sequence() {
            for (i, param) in params_seq.iter().enumerate() {
                if param.get("name").is_none() {
                    errors.push(format!("Parameter {} missing required field: 'name'", i + 1));
                }
            }
        }
    }
    
    errors
}

/// Build shredding preview from model configuration
async fn build_shred_preview(model_id: &str) -> anyhow::Result<Option<ShredPreviewResponse>> {
    use std::fs;
    use std::path::Path;
    
    let config_path = Path::new("config/models").join(format!("{}.yaml", model_id));
    
    if !config_path.exists() {
        return Ok(None);
    }
    
    let contents = fs::read_to_string(&config_path)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&contents)?;
    
    let model = yaml.get("model");
    let source = yaml.get("source");
    
    let model_name = model
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(model_id)
        .to_string();
    
    let source_type = source
        .and_then(|s| s.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    
    let mut parameters_to_extract = Vec::new();
    let mut total_extractions = 0;
    
    if let Some(params) = yaml.get("parameters") {
        if let Some(params_seq) = params.as_sequence() {
            for param in params_seq {
                let name = param.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                
                let description = param.get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                
                let style = param.get("style")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();
                
                let units = param.get("units")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                
                let mut levels = Vec::new();
                
                if let Some(levels_val) = param.get("levels") {
                    if let Some(levels_seq) = levels_val.as_sequence() {
                        for level in levels_seq {
                            let level_type = level.get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("surface")
                                .to_string();
                            
                            // Handle single value or array of values
                            if let Some(value) = level.get("value") {
                                let display = level.get("display")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(&format!("{:?}", value))
                                    .to_string();
                                
                                let value_str = value.as_i64()
                                    .map(|v| v.to_string())
                                    .or_else(|| value.as_str().map(|s| s.to_string()));
                                
                                let storage_path = format!(
                                    "shredded/{}/{{}}/{}_{}/f{{}}.grib2",
                                    model_id, name, display.replace(' ', "_")
                                );
                                
                                levels.push(ShredLevel {
                                    level_type: level_type.clone(),
                                    value: value_str,
                                    display,
                                    storage_path_template: storage_path,
                                });
                                total_extractions += 1;
                            } else if let Some(values) = level.get("values") {
                                if let Some(values_seq) = values.as_sequence() {
                                    let display_template = level.get("display_template")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("{value}");
                                    
                                    for val in values_seq {
                                        if let Some(v) = val.as_i64() {
                                            let display = display_template.replace("{value}", &v.to_string());
                                            let storage_path = format!(
                                                "shredded/{}/{{}}/{}_{}/f{{}}.grib2",
                                                model_id, name, display.replace(' ', "_")
                                            );
                                            
                                            levels.push(ShredLevel {
                                                level_type: level_type.clone(),
                                                value: Some(v.to_string()),
                                                display,
                                                storage_path_template: storage_path,
                                            });
                                            total_extractions += 1;
                                        }
                                    }
                                }
                            } else {
                                // Level with no specific value (e.g., surface, MSL)
                                let display = level.get("display")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(&level_type)
                                    .to_string();
                                
                                let storage_path = format!(
                                    "shredded/{}/{{}}/{}_{}/f{{}}.grib2",
                                    model_id, name, display.replace(' ', "_")
                                );
                                
                                levels.push(ShredLevel {
                                    level_type: level_type.clone(),
                                    value: None,
                                    display,
                                    storage_path_template: storage_path,
                                });
                                total_extractions += 1;
                            }
                        }
                    }
                }
                
                parameters_to_extract.push(ShredParameter {
                    name,
                    description,
                    levels,
                    style,
                    units,
                });
            }
        }
    }
    
    Ok(Some(ShredPreviewResponse {
        model_id: model_id.to_string(),
        model_name,
        source_type,
        parameters_to_extract,
        total_extractions,
    }))
}

#[allow(dead_code)]
async fn load_model_configs_from_yaml() -> anyhow::Result<Vec<ModelConfigResponse>> {
    use std::fs;
    use std::path::Path;
    
    let models_dir = Path::new("config/models");
    if !models_dir.exists() {
        return Ok(Vec::new());
    }
    
    let mut configs = Vec::new();
    
    for entry in fs::read_dir(models_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(Some(config)) = load_model_config_from_yaml(stem).await {
                    configs.push(config);
                }
            }
        }
    }
    
    Ok(configs)
}

#[allow(dead_code)]
async fn load_model_config_from_yaml(model_id: &str) -> anyhow::Result<Option<ModelConfigResponse>> {
    use std::fs;
    use std::path::Path;
    
    let config_path = Path::new("config/models").join(format!("{}.yaml", model_id));
    
    if !config_path.exists() {
        return Ok(None);
    }
    
    let contents = fs::read_to_string(&config_path)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&contents)?;
    
    // Extract model info
    let model = yaml.get("model").ok_or_else(|| anyhow::anyhow!("Missing 'model' section"))?;
    let source = yaml.get("source").ok_or_else(|| anyhow::anyhow!("Missing 'source' section"))?;
    let grid = yaml.get("grid").ok_or_else(|| anyhow::anyhow!("Missing 'grid' section"))?;
    let schedule = yaml.get("schedule").ok_or_else(|| anyhow::anyhow!("Missing 'schedule' section"))?;
    
    let config = ModelConfigResponse {
        id: model.get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(model_id)
            .to_string(),
        name: model.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string(),
        description: model.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        source: SourceInfo {
            source_type: source.get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            bucket: source.get("bucket")
                .and_then(|v| v.as_str())
                .map(String::from),
            region: source.get("region")
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        grid: GridInfo {
            projection: grid.get("projection")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            resolution: grid.get("resolution")
                .and_then(|v| v.as_str())
                .map(String::from),
            bbox: grid.get("bbox").and_then(|bbox| {
                Some(BBoxInfo {
                    min_lon: bbox.get("min_lon")?.as_f64()?,
                    min_lat: bbox.get("min_lat")?.as_f64()?,
                    max_lon: bbox.get("max_lon")?.as_f64()?,
                    max_lat: bbox.get("max_lat")?.as_f64()?,
                })
            }),
        },
        schedule: ScheduleInfo {
            cycles: schedule.get("cycles")
                .and_then(|v| v.as_sequence())
                .map(|seq| {
                    seq.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                        .collect()
                })
                .unwrap_or_default(),
            poll_interval_secs: schedule.get("poll_interval_secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(3600),
        },
        parameters: yaml.get("parameters")
            .and_then(|v| v.as_sequence())
            .map(|params| {
                params.iter()
                    .filter_map(|param| {
                        Some(ParameterInfo {
                            name: param.get("name")?.as_str()?.to_string(),
                            description: param.get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            levels: param.get("levels")
                                .and_then(|v| v.as_sequence())
                                .map(|lvls| {
                                    lvls.iter()
                                        .filter_map(|l| {
                                            l.get("display")
                                                .and_then(|d| d.as_str())
                                                .map(String::from)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                            style: param.get("style")
                                .and_then(|v| v.as_str())
                                .unwrap_or("default")
                                .to_string(),
                            units: param.get("units")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    };
    
    Ok(Some(config))
}
