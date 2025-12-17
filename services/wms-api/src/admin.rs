//! Admin dashboard API endpoints.
//!
//! Provides endpoints for monitoring and managing the WMS/ingestion system.

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use bytes::Bytes;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use storage::CatalogEntry;
use wms_common::BoundingBox;
use grid_processor::{DownsampleMethod, GridProcessorConfig, PyramidConfig, ZarrWriter, BoundingBox as GpBoundingBox};
use zarrs_filesystem::FilesystemStore;
use projection::LambertConformal;

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

// Database details response types
#[derive(Debug, Clone, Serialize)]
pub struct DatabaseDetailsResponse {
    pub models: Vec<ModelDetail>,
    pub total_datasets: u64,
    pub total_parameters: u64,
    pub total_size_bytes: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelDetail {
    pub model: String,
    pub parameter_count: u64,
    pub dataset_count: u64,
    pub total_size_bytes: u64,
    pub parameters: Vec<ParameterDetail>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParameterDetail {
    pub parameter: String,
    pub count: u64,
    pub oldest: Option<String>,
    pub newest: Option<String>,
    pub total_size_bytes: u64,
}

// Dataset info response for drill-down
#[derive(Debug, Clone, Serialize)]
pub struct DatasetInfoResponse {
    pub model: String,
    pub parameter: String,
    pub level: String,
    pub reference_time: String,
    pub forecast_hour: u32,
    pub valid_time: String,
    pub storage_path: String,
    pub file_size: u64,
}

// Storage tree response types
#[derive(Debug, Clone, Serialize)]
pub struct StorageTreeResponse {
    pub nodes: Vec<StorageTreeNode>,
    pub total_size: u64,
    pub total_objects: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageTreeNode {
    pub name: String,
    pub path: String,
    pub node_type: String,  // "file" or "directory"
    pub size: u64,
    pub children: Option<Vec<StorageTreeNode>>,
    pub file_count: u64,
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
// Ingest Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    /// Path to the file to ingest (local filesystem path accessible to wms-api)
    pub file_path: String,
    /// Original source URL (for tracking)
    #[serde(default)]
    pub source_url: Option<String>,
    /// Model name override (if not auto-detected from filename)
    #[serde(default)]
    pub model: Option<String>,
    /// Forecast hour override (if not auto-detected from filename)
    #[serde(default)]
    pub forecast_hour: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestResponse {
    pub success: bool,
    pub message: String,
    pub datasets_registered: usize,
    pub model: Option<String>,
    pub reference_time: Option<String>,
    pub parameters: Vec<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /admin/ingestion/status - Get overall ingestion status
pub async fn ingestion_status_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Getting ingestion status");
    
    let catalog = &state.catalog;
    
    // Get aggregated model stats directly from database
    let model_stats = catalog.get_model_stats().await.unwrap_or_default();
    
    // Convert to API response format
    let mut models: Vec<ModelStatus> = model_stats
        .iter()
        .map(|s| ModelStatus {
            id: s.model.clone(),
            name: format!("{} Model", s.model.to_uppercase()),
            status: "active".to_string(),
            enabled: true,
            last_ingest: s.last_ingest.map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string()),
            total_files: s.dataset_count,
            parameters: s.parameters.clone(),
        })
        .collect();
    
    // Sort models by name for consistent ordering
    models.sort_by(|a, b| a.id.cmp(&b.id));
    
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
    
    // Calculate totals from model stats
    let total_datasets: u64 = model_stats.iter().map(|s| s.dataset_count).sum();
    let total_parameters: u64 = model_stats.iter().map(|s| s.parameter_count).sum();
    
    let catalog_summary = CatalogSummary {
        total_datasets,
        total_parameters,
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

/// GET /admin/database/details - Get detailed database ingestion stats
/// Returns per-parameter statistics including counts and time ranges
pub async fn database_details_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Getting detailed database stats");
    
    let catalog = &state.catalog;
    
    // Get detailed per-parameter stats
    let param_stats = match catalog.get_detailed_parameter_stats().await {
        Ok(stats) => {
            info!("Got {} parameter stats from database", stats.len());
            stats
        }
        Err(e) => {
            error!("Failed to get detailed parameter stats: {}", e);
            Vec::new()
        }
    };
    
    // Group by model for the response
    let mut models_map: std::collections::HashMap<String, Vec<ParameterDetail>> = std::collections::HashMap::new();
    
    for stat in &param_stats {
        models_map
            .entry(stat.model.clone())
            .or_default()
            .push(ParameterDetail {
                parameter: stat.parameter.clone(),
                count: stat.count,
                oldest: stat.oldest.map(|t| t.to_rfc3339()),
                newest: stat.newest.map(|t| t.to_rfc3339()),
                total_size_bytes: stat.total_size_bytes,
            });
    }
    
    // Convert to sorted list
    let mut models: Vec<ModelDetail> = models_map
        .into_iter()
        .map(|(model, parameters)| {
            let total_datasets: u64 = parameters.iter().map(|p| p.count).sum();
            let total_size: u64 = parameters.iter().map(|p| p.total_size_bytes).sum();
            ModelDetail {
                model,
                parameter_count: parameters.len() as u64,
                dataset_count: total_datasets,
                total_size_bytes: total_size,
                parameters,
            }
        })
        .collect();
    
    models.sort_by(|a, b| a.model.cmp(&b.model));
    
    // Get database-level totals
    let total_datasets: u64 = param_stats.iter().map(|s| s.count).sum();
    let total_size: u64 = param_stats.iter().map(|s| s.total_size_bytes).sum();
    
    Json(DatabaseDetailsResponse {
        models,
        total_datasets,
        total_parameters: param_stats.len() as u64,
        total_size_bytes: total_size,
        updated_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// GET /admin/database/datasets/:model/:parameter - Get all datasets for a parameter
pub async fn database_datasets_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((model, parameter)): Path<(String, String)>,
) -> impl IntoResponse {
    info!("Admin: Getting datasets for {}/{}", model, parameter);
    
    let catalog = &state.catalog;
    
    match catalog.get_datasets_for_parameter(&model, &parameter).await {
        Ok(datasets) => {
            let response: Vec<DatasetInfoResponse> = datasets
                .into_iter()
                .map(|d| DatasetInfoResponse {
                    model: d.model,
                    parameter: d.parameter,
                    level: d.level,
                    reference_time: d.reference_time.to_rfc3339(),
                    forecast_hour: d.forecast_hour,
                    valid_time: d.valid_time.to_rfc3339(),
                    storage_path: d.storage_path,
                    file_size: d.file_size,
                })
                .collect();
            Json(response).into_response()
        }
        Err(e) => {
            error!("Failed to get datasets: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get datasets: {}", e)).into_response()
        }
    }
}

/// GET /admin/storage/tree - Get MinIO storage as a tree structure
pub async fn storage_tree_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Getting storage tree");
    
    // List all objects with sizes in MinIO
    let all_objects = match state.storage.list_with_sizes("").await {
        Ok(paths) => paths,
        Err(e) => {
            error!("Failed to list storage: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list storage: {}", e)).into_response();
        }
    };
    
    // Build tree structure from paths
    let tree = build_storage_tree(&all_objects);
    
    Json(tree).into_response()
}

/// Build a tree structure from flat list of paths with sizes
fn build_storage_tree(objects: &[(String, u64)]) -> StorageTreeResponse {
    use std::collections::HashMap;
    
    // First pass: collect all directories and files
    let mut dir_sizes: HashMap<String, (u64, u64)> = HashMap::new(); // path -> (size, count)
    let mut files: Vec<(String, String, u64)> = Vec::new(); // (dir_path, filename, size)
    
    let mut total_size: u64 = 0;
    let mut total_objects: u64 = 0;
    
    for (path, file_size) in objects {
        total_size += file_size;
        total_objects += 1;
        
        let parts: Vec<&str> = path.split('/').collect();
        if parts.is_empty() {
            continue;
        }
        
        // Track file
        if parts.len() >= 2 {
            let dir_path = parts[..parts.len()-1].join("/");
            let filename = parts[parts.len()-1].to_string();
            files.push((dir_path, filename, *file_size));
        } else {
            files.push(("".to_string(), parts[0].to_string(), *file_size));
        }
        
        // Accumulate sizes for all parent directories
        let mut current = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                current.push('/');
            }
            current.push_str(part);
            
            // Only track directories (not the file itself)
            if i < parts.len() - 1 {
                let entry = dir_sizes.entry(current.clone()).or_insert((0, 0));
                entry.0 += file_size;
                entry.1 += 1;
            }
        }
    }
    
    // Build tree structure
    fn build_node(
        path: &str,
        name: &str,
        dir_sizes: &HashMap<String, (u64, u64)>,
        files: &[(String, String, u64)],
    ) -> StorageTreeNode {
        let (size, file_count) = dir_sizes.get(path).copied().unwrap_or((0, 0));
        
        // Get immediate children
        let mut children: Vec<StorageTreeNode> = Vec::new();
        
        // Add subdirectories
        let prefix = if path.is_empty() { String::new() } else { format!("{}/", path) };
        let mut seen_subdirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        
        for (dir_path, _) in dir_sizes.iter() {
            if dir_path.starts_with(&prefix) {
                let remainder = &dir_path[prefix.len()..];
                if let Some(subdir) = remainder.split('/').next() {
                    if !subdir.is_empty() && seen_subdirs.insert(subdir.to_string()) {
                        let child_path = if path.is_empty() {
                            subdir.to_string()
                        } else {
                            format!("{}/{}", path, subdir)
                        };
                        children.push(build_node(&child_path, subdir, dir_sizes, files));
                    }
                }
            }
        }
        
        // Add files in this directory
        for (file_dir, filename, fsize) in files {
            if file_dir == path {
                children.push(StorageTreeNode {
                    name: filename.clone(),
                    path: if path.is_empty() {
                        filename.clone()
                    } else {
                        format!("{}/{}", path, filename)
                    },
                    node_type: "file".to_string(),
                    size: *fsize,
                    children: None,
                    file_count: 1,
                });
            }
        }
        
        // Sort: directories first, then files, alphabetically
        children.sort_by(|a, b| {
            match (&a.node_type[..], &b.node_type[..]) {
                ("directory", "file") => std::cmp::Ordering::Less,
                ("file", "directory") => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });
        
        StorageTreeNode {
            name: name.to_string(),
            path: path.to_string(),
            node_type: "directory".to_string(),
            size,
            children: Some(children),
            file_count,
        }
    }
    
    // Build root nodes
    let mut root_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (dir_path, _) in &dir_sizes {
        if let Some(root) = dir_path.split('/').next() {
            root_names.insert(root.to_string());
        }
    }
    
    let mut nodes: Vec<StorageTreeNode> = root_names
        .into_iter()
        .map(|name| build_node(&name, &name, &dir_sizes, &files))
        .collect();
    
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    
    StorageTreeResponse {
        nodes,
        total_size,
        total_objects,
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
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
                    params.model.as_ref().is_none_or(|m| &d.model == m)
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

/// GET /api/admin/ingestion/active - Get currently active and recent ingestions
pub async fn ingestion_active_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Getting active ingestion status");
    let status = state.ingestion_tracker.get_status().await;
    Json(status)
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

/// POST /admin/ingest - Ingest a downloaded file into the catalog
/// 
/// This endpoint is called by the downloader service after successfully
/// downloading a weather data file. It parses the GRIB2/NetCDF file,
/// extracts parameters, stores them in object storage, and registers
/// them in the catalog.
pub async fn ingest_handler(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<IngestRequest>,
) -> impl IntoResponse {
    info!(
        file_path = %payload.file_path,
        source_url = ?payload.source_url,
        model = ?payload.model,
        "Admin: Ingesting file"
    );
    
    // Generate unique ID for tracking
    let ingestion_id = uuid::Uuid::new_v4().to_string();
    let model = payload.model.clone().unwrap_or_else(|| 
        extract_model_from_filename(&payload.file_path).unwrap_or_else(|| "unknown".to_string())
    );
    
    // Start tracking this ingestion
    state.ingestion_tracker.start(
        ingestion_id.clone(),
        payload.file_path.clone(),
        model.clone(),
    ).await;
    
    match ingest_file_tracked(&state, &payload, &ingestion_id).await {
        Ok(response) => {
            // Mark as completed successfully
            state.ingestion_tracker.complete(
                &ingestion_id,
                true,
                None,
                response.datasets_registered as u32,
            ).await;
            (StatusCode::OK, Json(response)).into_response()
        },
        Err(e) => {
            error!(error = %e, file = %payload.file_path, "Ingestion failed");
            // Mark as failed
            state.ingestion_tracker.complete(
                &ingestion_id,
                false,
                Some(e.to_string()),
                0,
            ).await;
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(IngestResponse {
                    success: false,
                    message: format!("Ingestion failed: {}", e),
                    datasets_registered: 0,
                    model: None,
                    reference_time: None,
                    parameters: vec![],
                }),
            ).into_response()
        }
    }
}

/// Ingest a file into the catalog with progress tracking
async fn ingest_file_tracked(
    state: &Arc<AppState>,
    request: &IngestRequest,
    ingestion_id: &str,
) -> anyhow::Result<IngestResponse> {
    use std::fs;
    
    let file_path = &request.file_path;
    
    // Check if file exists
    if !std::path::Path::new(file_path).exists() {
        anyhow::bail!("File not found: {}", file_path);
    }
    
    // Check file type by extension
    let is_netcdf = file_path.ends_with(".nc") || file_path.ends_with(".nc4");
    let is_grib2 = file_path.ends_with(".grib2") || file_path.ends_with(".grb2") || file_path.ends_with(".grib2.gz");
    
    if is_netcdf {
        // Update status for NetCDF
        state.ingestion_tracker.update(ingestion_id, "parsing_netcdf", 0, 0).await;
        return ingest_netcdf_file(state, request, file_path).await;
    }
    
    if !is_grib2 {
        anyhow::bail!("Unsupported file type. Expected .grib2, .grb2, or .nc");
    }
    
    // Update status: parsing
    state.ingestion_tracker.update(ingestion_id, "parsing", 0, 0).await;
    
    // Read file (decompress if needed)
    let data = if file_path.ends_with(".gz") {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let file = fs::File::open(file_path)?;
        let mut decoder = GzDecoder::new(file);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        Bytes::from(decompressed)
    } else {
        Bytes::from(fs::read(file_path)?)
    };
    
    // Determine model from filename or request
    let model = request.model.clone().or_else(|| {
        extract_model_from_filename(file_path)
    }).unwrap_or_else(|| "gfs".to_string());
    
    // Determine forecast hour from filename or request
    let forecast_hour = request.forecast_hour.or_else(|| {
        extract_forecast_hour_from_filename(file_path)
    }).unwrap_or(0);
    
    info!(model = %model, forecast_hour = forecast_hour, "Detected file metadata");
    
    // Update status: shredding
    state.ingestion_tracker.update(ingestion_id, "shredding", 0, 0).await;
    
    // Parse GRIB2 and extract parameters
    let mut reader = grib2_parser::Grib2Reader::new(data);
    let mut registered_params = HashSet::new();
    let mut grib_reference_time: Option<chrono::DateTime<Utc>> = None;
    let mut datasets_registered = 0;
    let mut params_found = 0u32;
    
    // Parameters to ingest with their accepted level types
    let pressure_levels: HashSet<u32> = [
        1000, 975, 950, 925, 900, 850, 800, 750, 700, 650, 
        600, 550, 500, 450, 400, 350, 300, 250, 200, 150, 
        100, 70, 50, 30, 20, 10
    ].into_iter().collect();
    
    // Target parameters and their level specs
    // Level types: 1=surface, 100=isobaric, 101=MSL, 103=height above ground,
    //              200=entire atmosphere, 212=low cloud, 222=middle cloud, 232=high cloud
    let target_params: Vec<(&str, Vec<(u8, Option<u32>)>)> = vec![
        // Pressure
        ("PRMSL", vec![(101, None)]),                    // Mean sea level pressure
        
        // Temperature
        ("TMP", vec![(103, Some(2)), (100, None)]),      // 2m temp + pressure levels
        ("DPT", vec![(103, Some(2))]),                   // Dew point at 2m
        
        // Wind
        ("UGRD", vec![(103, Some(10)), (100, None)]),    // 10m wind + pressure levels
        ("VGRD", vec![(103, Some(10)), (100, None)]),    // 10m wind + pressure levels
        ("GUST", vec![(1, None)]),                       // Surface wind gust
        
        // Moisture
        ("RH", vec![(103, Some(2)), (100, None)]),       // 2m RH + pressure levels
        ("PWAT", vec![(200, None)]),                     // Precipitable water (entire atmosphere)
        
        // Geopotential
        ("HGT", vec![(100, None)]),                      // Geopotential height at pressure levels
        
        // Precipitation
        ("APCP", vec![(1, None)]),                       // Total precipitation (surface)
        
        // Convective/Stability - surface-based CAPE/CIN
        ("CAPE", vec![(1, None), (180, None)]),          // CAPE: surface (1) and surface-based (180)
        ("CIN", vec![(1, None), (180, None)]),           // CIN: surface (1) and surface-based (180)
        
        // Cloud cover
        ("TCDC", vec![(200, None), (10, None)]),         // Total cloud cover (entire atmosphere)
        ("LCDC", vec![(212, None), (214, None)]),        // Low cloud cover (layer or top)
        ("MCDC", vec![(222, None), (224, None)]),        // Middle cloud cover (layer or top)
        ("HCDC", vec![(232, None), (234, None)]),        // High cloud cover (layer or top)
        
        // Visibility
        ("VIS", vec![(1, None)]),                        // Visibility (surface)
        
        // MRMS-specific (keep existing)
        ("REFL", vec![(200, None), (1, None)]),          // Reflectivity (MRMS)
        ("PRECIP_RATE", vec![(1, None)]),                // Precip rate (MRMS)
    ];
    
    // For MRMS, extract parameter name from filename
    let mrms_param_name: Option<String> = if model == "mrms" {
        extract_mrms_param_from_filename(file_path)
    } else {
        None
    };
    
    while let Some(message) = reader.next_message().ok().flatten() {
        // Extract reference time from first message
        if grib_reference_time.is_none() {
            grib_reference_time = Some(message.identification.reference_time);
            info!(reference_time = %message.identification.reference_time, "Extracted reference time");
        }
        
        let grib_param = &message.product_definition.parameter_short_name;
        let param = if model == "mrms" {
            mrms_param_name.as_ref().unwrap_or(grib_param)
        } else {
            grib_param
        };
        let level = &message.product_definition.level_description;
        let level_type = message.product_definition.level_type;
        let level_value = message.product_definition.level_value;
        
        let param_level_key = format!("{}:{}", param, level);
        
        // Check if we should register this parameter
        let should_register = if model == "mrms" {
            !registered_params.contains(&param_level_key)
        } else {
            target_params.iter().any(|(p, level_specs)| {
                if param != p || registered_params.contains(&param_level_key) {
                    return false;
                }
                level_specs.iter().any(|(lt, lv)| {
                    if level_type != *lt {
                        return false;
                    }
                    if level_type == 100 {
                        return pressure_levels.contains(&level_value);
                    }
                    if let Some(required_value) = lv {
                        level_value == *required_value
                    } else {
                        true
                    }
                })
            })
        };
        
        if should_register {
            params_found += 1;
            
            let reference_time = grib_reference_time.unwrap_or_else(Utc::now);
            
            // Sanitize level for path
            let level_sanitized = level
                .replace([' ', '/'], "_")
                .to_lowercase();
            
            // Storage path: grids/{model}/{run_date}/{param}_{level}_f{fhr:03}.zarr
            // For observation data like MRMS (updates every ~2 minutes), use minute-level paths
            // For forecast models like GFS/HRRR, use hourly paths (they have hourly forecast cycles)
            let run_date = if model == "mrms" {
                reference_time.format("%Y%m%d_%H%Mz").to_string()
            } else {
                reference_time.format("%Y%m%d_%Hz").to_string()
            };
            let zarr_storage_path = format!(
                "grids/{}/{}/{}_{}_f{:03}.zarr",
                model,
                run_date,
                param.to_lowercase(),
                level_sanitized,
                forecast_hour
            );
            
            // Update status: unpacking grid data
            state.ingestion_tracker.update(ingestion_id, "unpacking", params_found, datasets_registered as u32).await;
            
            // Extract grid dimensions
            let width = message.grid_definition.num_points_longitude as usize;
            let height = message.grid_definition.num_points_latitude as usize;
            
            // Unpack the grid data
            let grid_data = match message.unpack_data() {
                Ok(data) => data,
                Err(e) => {
                    warn!(error = %e, param = %param, "Failed to unpack GRIB2 data, skipping");
                    continue;
                }
            };
            
            if grid_data.len() != width * height {
                warn!(
                    expected = width * height,
                    actual = grid_data.len(),
                    param = %param,
                    "Grid data size mismatch, skipping"
                );
                continue;
            }
            
            // Update status: writing Zarr
            state.ingestion_tracker.update(ingestion_id, "writing_zarr", params_found, datasets_registered as u32).await;
            
            // Calculate bounding box from grid definition
            // For HRRR, use Lambert Conformal projection to calculate geographic bounds
            let gp_bbox = if model == "hrrr" {
                // HRRR uses Lambert Conformal projection
                let proj = LambertConformal::hrrr();
                // geographic_bounds() returns (min_lon, min_lat, max_lon, max_lat)
                let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();
                GpBoundingBox::new(min_lon, min_lat, max_lon, max_lat)
            } else {
                // Standard lat/lon grid (GFS, MRMS, etc.)
                let grib_bbox = get_bbox_from_grid(&message.grid_definition);
                GpBoundingBox::new(
                    grib_bbox.min_x, grib_bbox.min_y, grib_bbox.max_x, grib_bbox.max_y
                )
            };
            
            // Create a temporary directory for Zarr output
            let temp_dir = match tempfile::tempdir() {
                Ok(dir) => dir,
                Err(e) => {
                    warn!(error = %e, "Failed to create temp dir for Zarr, skipping");
                    continue;
                }
            };
            let zarr_path = temp_dir.path().join("grid.zarr");
            if let Err(e) = std::fs::create_dir_all(&zarr_path) {
                warn!(error = %e, "Failed to create Zarr directory, skipping");
                continue;
            }
            
            // Create Zarr writer with default config
            let config = GridProcessorConfig::default();
            let writer = ZarrWriter::new(config);
            
            // Create filesystem store for the temp directory
            let store = match FilesystemStore::new(&zarr_path) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = ?e, "Failed to create filesystem store, skipping");
                    continue;
                }
            };
            
            // Get units (default if not available)
            let units = "unknown"; // TODO: extract from GRIB2 metadata
            
            // Configure pyramid generation
            let pyramid_config = PyramidConfig::from_env();
            let downsample_method = DownsampleMethod::for_parameter(param);
            
            // Write Zarr data with multi-resolution pyramid levels
            let write_result = match writer.write_multiscale(
                store,
                "/",
                &grid_data,
                width,
                height,
                &gp_bbox,
                &model,
                param,
                level,
                units,
                reference_time,
                forecast_hour,
                &pyramid_config,
                downsample_method,
            ) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = ?e, param = %param, "Failed to write Zarr, skipping");
                    continue;
                }
            };
            
            debug!(
                param = %param,
                level = %level,
                width = width,
                height = height,
                pyramid_levels = write_result.num_levels,
                "Wrote Zarr grid with pyramid levels to temp directory"
            );
            
            // Update status: uploading to MinIO
            state.ingestion_tracker.update(ingestion_id, "uploading", params_found, datasets_registered as u32).await;
            
            // Upload Zarr files to object storage
            let zarr_file_size = match upload_zarr_directory(&state.storage, &zarr_path, &zarr_storage_path).await {
                Ok(size) => size,
                Err(e) => {
                    warn!(error = %e, param = %param, "Failed to upload Zarr, skipping");
                    continue;
                }
            };
            
            info!(
                param = %param,
                level = %level,
                path = %zarr_storage_path,
                size = zarr_file_size,
                width = width,
                height = height,
                pyramid_levels = write_result.num_levels,
                "Stored Zarr grid with pyramid levels"
            );
            
            // Update status: registering
            state.ingestion_tracker.update(ingestion_id, "registering", params_found, datasets_registered as u32).await;
            
            // Get model-specific bounding box for catalog
            let bbox = get_model_bbox(&model);
            
            let entry = CatalogEntry {
                model: model.clone(),
                parameter: param.to_string(),
                level: level.clone(),
                reference_time,
                forecast_hour,
                bbox,
                storage_path: zarr_storage_path,
                file_size: zarr_file_size,
                zarr_metadata: Some(write_result.zarr_metadata.to_json()),
            };
            
            match state.catalog.register_dataset(&entry).await {
                Ok(id) => {
                    debug!(id = %id, param = %param, level = %level, "Registered Zarr dataset");
                    registered_params.insert(param_level_key);
                    datasets_registered += 1;
                }
                Err(e) => {
                    debug!(param = %param, level = %level, error = %e, "Could not register (may already exist)");
                }
            }
        }
    }
    
    let parameters: Vec<String> = registered_params
        .iter()
        .map(|k| k.split(':').next().unwrap_or(k).to_string())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    
    info!(
        model = %model,
        datasets = datasets_registered,
        parameters = ?parameters,
        "Ingestion complete"
    );
    
    Ok(IngestResponse {
        success: true,
        message: format!("Ingested {} datasets", datasets_registered),
        datasets_registered,
        model: Some(model),
        reference_time: grib_reference_time.map(|t| t.to_rfc3339()),
        parameters,
    })
}

/// Ingest a NetCDF file (GOES satellite data) into the catalog
async fn ingest_netcdf_file(
    state: &Arc<AppState>,
    request: &IngestRequest,
    file_path: &str,
) -> anyhow::Result<IngestResponse> {
    use chrono::TimeZone;
    use std::fs;
    
    info!(file_path = %file_path, "Ingesting GOES NetCDF file");
    
    // Read file
    let data = fs::read(file_path)?;
    let data_bytes = Bytes::from(data);
    let file_size = data_bytes.len() as u64;
    
    // Parse filename to extract metadata
    let filename = std::path::Path::new(file_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown.nc");
    
    // Extract band number from filename (e.g., "C02" from "...M6C02_G18...")
    let band = filename
        .find("M6C")
        .or_else(|| filename.find("M3C"))
        .and_then(|pos| {
            let band_str = &filename[pos + 3..pos + 5];
            band_str.parse::<u8>().ok()
        })
        .unwrap_or(2); // Default to band 2 (visible red)
    
    // Determine model from filename or override
    let model = request.model.clone().unwrap_or_else(|| {
        if filename.contains("_G16_") || filename.to_lowercase().contains("goes16") {
            "goes16".to_string()
        } else if filename.contains("_G18_") || filename.to_lowercase().contains("goes18") {
            "goes18".to_string()
        } else {
            "goes16".to_string() // Default to GOES-16
        }
    });
    
    // Extract satellite ID for logging
    let satellite = if filename.contains("_G16_") {
        "G16"
    } else if filename.contains("_G18_") {
        "G18"
    } else {
        "GOES"
    };
    
    // Extract observation time from filename (format: s20250500001170)
    // Time is in format: YYYYDDDHHMMSSt (year, day-of-year, hour, min, sec, tenths)
    let observation_time = filename
        .find("_s")
        .and_then(|pos| {
            if pos + 15 > filename.len() {
                return None;
            }
            let time_str = &filename[pos + 2..pos + 15];
            // Parse YYYYDDDHHMMSS
            let year: i32 = time_str.get(0..4)?.parse().ok()?;
            let doy: u32 = time_str.get(4..7)?.parse().ok()?;
            let hour: u32 = time_str.get(7..9)?.parse().ok()?;
            let min: u32 = time_str.get(9..11)?.parse().ok()?;
            let sec: u32 = time_str.get(11..13)?.parse().ok()?;
            
            // Convert to DateTime
            let date = chrono::NaiveDate::from_yo_opt(year, doy)?;
            let time = chrono::NaiveTime::from_hms_opt(hour, min, sec)?;
            Some(Utc.from_utc_datetime(&date.and_time(time)))
        })
        .unwrap_or_else(Utc::now);
    
    // Determine parameter name based on band
    let (parameter, level) = match band {
        1 => ("CMI_C01", "visible_blue"),       // 0.47µm Blue
        2 => ("CMI_C02", "visible_red"),        // 0.64µm Red (most common visible)
        3 => ("CMI_C03", "visible_veggie"),     // 0.86µm Vegetation
        4 => ("CMI_C04", "cirrus"),             // 1.37µm Cirrus
        5 => ("CMI_C05", "snow_ice"),           // 1.6µm Snow/Ice
        6 => ("CMI_C06", "cloud_particle"),     // 2.2µm Cloud Particle Size
        7 => ("CMI_C07", "shortwave_ir"),       // 3.9µm Shortwave Window
        8 => ("CMI_C08", "upper_vapor"),        // 6.2µm Upper-Level Water Vapor
        9 => ("CMI_C09", "mid_vapor"),          // 6.9µm Mid-Level Water Vapor
        10 => ("CMI_C10", "low_vapor"),         // 7.3µm Lower-Level Water Vapor
        11 => ("CMI_C11", "cloud_phase"),       // 8.4µm Cloud-Top Phase
        12 => ("CMI_C12", "ozone"),             // 9.6µm Ozone
        13 => ("CMI_C13", "clean_ir"),          // 10.3µm "Clean" Longwave IR
        14 => ("CMI_C14", "ir"),                // 11.2µm Longwave IR
        15 => ("CMI_C15", "dirty_ir"),          // 12.3µm "Dirty" Longwave IR
        16 => ("CMI_C16", "co2"),               // 13.3µm CO2
        _ => ("CMI_C02", "visible_red"),        // Default to visible red
    };
    
    info!(
        band = band,
        satellite = satellite,
        parameter = parameter,
        model = %model,
        observation_time = %observation_time,
        file_size = file_size,
        "Parsed GOES file metadata"
    );
    
    // Create storage path - include hour and minute for GOES (5-minute intervals)
    let run_datetime = observation_time.format("%Y%m%d_%H%Mz").to_string();
    let storage_path = format!(
        "raw/{}/{}/{}.nc",
        model,
        run_datetime,
        parameter.to_lowercase()
    );
    
    // Store the NetCDF file in object storage
    state.storage.put(&storage_path, data_bytes).await?;
    info!(path = %storage_path, "Stored GOES NetCDF file");
    
    // Get model-specific bounding box
    let bbox = match model.as_str() {
        "goes16" => BoundingBox::new(-143.0, 14.5, -53.0, 55.5),
        "goes18" => BoundingBox::new(-165.0, 14.5, -90.0, 55.5),
        _ => BoundingBox::new(-143.0, 14.5, -53.0, 55.5),
    };
    
    // Create catalog entry
    let entry = CatalogEntry {
        model: model.clone(),
        parameter: parameter.to_string(),
        level: level.to_string(),
        reference_time: observation_time,
        forecast_hour: 0, // Observational data, no forecast
        bbox,
        storage_path: storage_path.clone(),
        file_size,
        zarr_metadata: None,
    };
    
    // Register in catalog
    match state.catalog.register_dataset(&entry).await {
        Ok(id) => {
            info!(id = %id, parameter = %parameter, model = %model, band = band, "Registered GOES dataset");
        }
        Err(e) => {
            // If registration fails (e.g., duplicate), still count as success since file is stored
            warn!(error = %e, "Could not register dataset (may already exist)");
        }
    }
    
    // Trigger cache warming if enabled for this model
    {
        let warmer_guard = state.grid_warmer.read().await;
        if let Some(warmer) = warmer_guard.as_ref() {
            if warmer.should_warm_on_ingest(&model) {
                let warmer = warmer.clone();
                let model_clone = model.clone();
                let parameter_clone = parameter.to_string();
                let storage_path_clone = storage_path.clone();
                let observation_time_clone = observation_time;
                
                // Spawn warming task in background so we don't block the response
                tokio::spawn(async move {
                    warmer.warm_dataset(
                        &model_clone,
                        &parameter_clone,
                        &storage_path_clone,
                        observation_time_clone,
                    ).await;
                });
            }
        }
    }
    
    Ok(IngestResponse {
        success: true,
        message: format!("Ingested GOES {} band {}", satellite, band),
        datasets_registered: 1,
        model: Some(model),
        reference_time: Some(observation_time.to_rfc3339()),
        parameters: vec![parameter.to_string()],
    })
}

/// Extract model name from filename
fn extract_model_from_filename(file_path: &str) -> Option<String> {
    let filename = std::path::Path::new(file_path)
        .file_name()
        .and_then(|s| s.to_str())?;
    
    if filename.contains("_G16_") || filename.contains("goes16") {
        Some("goes16".to_string())
    } else if filename.contains("_G18_") || filename.contains("goes18") {
        Some("goes18".to_string())
    } else if filename.starts_with("hrrr") || filename.contains("hrrr") {
        Some("hrrr".to_string())
    } else if filename.starts_with("gfs") || filename.contains("gfs") {
        Some("gfs".to_string())
    } else if filename.starts_with("MRMS_") || filename.contains("mrms") {
        Some("mrms".to_string())
    } else {
        None
    }
}

/// Extract forecast hour from filename
fn extract_forecast_hour_from_filename(file_path: &str) -> Option<u32> {
    let filename = std::path::Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())?;
    
    // Pattern: _f### (e.g., gfs_20241201_00z_f003.grib2)
    if let Some(pos) = filename.rfind("_f") {
        let rest = &filename[pos + 2..];
        if let Some(hour) = rest.get(..3).and_then(|s| s.parse::<u32>().ok()) {
            return Some(hour);
        }
    }
    
    // Pattern: wrfsfcf## (HRRR)
    if let Some(pos) = filename.find("wrfsfcf") {
        let rest = &filename[pos + 7..];
        if let Some(hour) = rest.get(..2).and_then(|s| s.parse::<u32>().ok()) {
            return Some(hour);
        }
    }
    
    // Pattern: z_f### at end (our download naming)
    if let Some(pos) = filename.find("z_f") {
        let rest = &filename[pos + 3..];
        if let Ok(hour) = rest.parse::<u32>() {
            return Some(hour);
        }
    }
    
    None
}

/// Extract MRMS parameter name from filename
fn extract_mrms_param_from_filename(file_path: &str) -> Option<String> {
    let filename = std::path::Path::new(file_path)
        .file_name()
        .and_then(|s| s.to_str())?;
    
    let lower = filename.to_lowercase();
    if lower.contains("reflectivity") || lower.contains("refl") {
        Some("REFL".to_string())
    } else if lower.contains("preciprate") || lower.contains("precip_rate") {
        Some("PRECIP_RATE".to_string())
    } else if lower.contains("qpe_01h") {
        Some("QPE_01H".to_string())
    } else if lower.contains("qpe") {
        Some("QPE".to_string())
    } else if filename.starts_with("MRMS_") {
        filename.strip_prefix("MRMS_")
            .and_then(|rest| rest.split('_').next())
            .map(|p| p.to_uppercase())
    } else {
        None
    }
}

/// Get model-specific bounding box
fn get_model_bbox(model: &str) -> BoundingBox {
    match model {
        "hrrr" => BoundingBox::new(-122.719528, 21.138123, -60.917193, 47.842195),
        "mrms" => BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
        "gfs" => BoundingBox::new(0.0, -90.0, 360.0, 90.0),
        "goes16" => BoundingBox::new(-143.0, 14.5, -53.0, 55.5),
        "goes18" => BoundingBox::new(-165.0, 14.5, -90.0, 55.5),
        _ => BoundingBox::new(0.0, -90.0, 360.0, 90.0),
    }
}

/// Extract bounding box from GRIB2 grid definition.
fn get_bbox_from_grid(grid: &grib2_parser::sections::GridDefinition) -> BoundingBox {
    // Convert millidegrees to degrees (grib2-parser already divides by 1000 to get millidegrees)
    let first_lat = grid.first_latitude_millidegrees as f64 / 1_000.0;
    let first_lon = grid.first_longitude_millidegrees as f64 / 1_000.0;
    let last_lat = grid.last_latitude_millidegrees as f64 / 1_000.0;
    let last_lon = grid.last_longitude_millidegrees as f64 / 1_000.0;
    
    // Determine min/max (grid might scan in different directions)
    let min_lat = first_lat.min(last_lat);
    let max_lat = first_lat.max(last_lat);
    let min_lon = first_lon.min(last_lon);
    let max_lon = first_lon.max(last_lon);
    
    // Handle longitude wrapping (GRIB2 may use 0-360 instead of -180-180)
    let (min_lon, max_lon) = if min_lon > 180.0 {
        (min_lon - 360.0, max_lon - 360.0)
    } else {
        (min_lon, max_lon)
    };
    
    BoundingBox::new(min_lon, min_lat, max_lon, max_lat)
}

/// Upload a Zarr directory to object storage.
async fn upload_zarr_directory(
    storage: &storage::ObjectStorage,
    local_path: &std::path::Path,
    storage_prefix: &str,
) -> anyhow::Result<u64> {
    let mut total_size = 0u64;
    
    for entry in walkdir::WalkDir::new(local_path) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let relative_path = entry.path().strip_prefix(local_path)?;
            let storage_path = format!("{}/{}", storage_prefix, relative_path.display());
            
            let file_data = tokio::fs::read(entry.path()).await?;
            let file_size = file_data.len() as u64;
            total_size += file_size;
            
            storage.put(&storage_path, Bytes::from(file_data)).await?;
            debug!(path = %storage_path, size = file_size, "Uploaded Zarr file");
        }
    }
    
    Ok(total_size)
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

// ============================================================================
// Cleanup/Retention Types and Handlers
// ============================================================================

/// Response for cleanup status endpoint
#[derive(Debug, Clone, Serialize)]
pub struct CleanupStatusResponse {
    pub enabled: bool,
    pub interval_secs: u64,
    pub next_run_in_secs: Option<u64>,
    pub last_run: Option<String>,
    pub model_retentions: Vec<ModelRetentionInfo>,
    pub purge_preview: Vec<ModelPurgePreview>,
    pub expired_count: i64,
    pub total_purge_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelRetentionInfo {
    pub model: String,
    pub retention_hours: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelPurgePreview {
    pub model: String,
    pub retention_hours: u32,
    pub cutoff_time: String,
    pub dataset_count: u64,
    pub total_size_bytes: u64,
    pub oldest_data: Option<String>,
    pub next_purge_in: Option<String>,
}

/// Response for manual cleanup trigger
#[derive(Debug, Clone, Serialize)]
pub struct CleanupRunResponse {
    pub success: bool,
    pub message: String,
    pub marked_expired: u64,
    pub files_deleted: u64,
    pub records_deleted: u64,
    pub errors: u64,
}

/// GET /api/admin/cleanup/status - Get cleanup/retention status
pub async fn cleanup_status_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Getting cleanup status");
    
    let config_dir = std::env::var("CONFIG_DIR").unwrap_or_else(|_| "/app/config".to_string());
    let config = crate::cleanup::CleanupConfig::from_env_and_configs(&config_dir);
    
    let expired_count = state.catalog.count_expired().await.unwrap_or(0);
    
    // Get list of models from the database
    let models = state.catalog.list_models().await.unwrap_or_default();
    
    let mut model_retentions: Vec<ModelRetentionInfo> = Vec::new();
    let mut purge_preview: Vec<ModelPurgePreview> = Vec::new();
    let mut total_purge_size_bytes: u64 = 0;
    
    let now = Utc::now();
    
    for model in &models {
        let retention_hours = config.get_retention_hours(model);
        let cutoff = now - chrono::Duration::hours(retention_hours as i64);
        
        model_retentions.push(ModelRetentionInfo {
            model: model.clone(),
            retention_hours,
        });
        
        // Get preview of what would be purged
        let preview = state.catalog
            .preview_model_expiration(model, cutoff)
            .await
            .unwrap_or_default();
        
        // Get oldest dataset time to calculate when next purge will happen
        let oldest_time = state.catalog
            .get_oldest_dataset_time(model)
            .await
            .ok()
            .flatten();
        
        let (oldest_data, next_purge_in) = if let Some(oldest) = oldest_time {
            let oldest_str = oldest.format("%Y-%m-%d %H:%M UTC").to_string();
            
            // Calculate when the oldest data will be purged
            let purge_time = oldest + chrono::Duration::hours(retention_hours as i64);
            let time_until_purge = purge_time - now;
            
            let next_purge_str = if time_until_purge.num_seconds() <= 0 {
                Some("Now (next cleanup cycle)".to_string())
            } else if time_until_purge.num_hours() < 1 {
                Some(format!("{} minutes", time_until_purge.num_minutes()))
            } else if time_until_purge.num_hours() < 24 {
                Some(format!("{} hours", time_until_purge.num_hours()))
            } else {
                Some(format!("{} days", time_until_purge.num_days()))
            };
            
            (Some(oldest_str), next_purge_str)
        } else {
            (None, None)
        };
        
        total_purge_size_bytes += preview.total_size_bytes;
        
        purge_preview.push(ModelPurgePreview {
            model: model.clone(),
            retention_hours,
            cutoff_time: cutoff.format("%Y-%m-%d %H:%M UTC").to_string(),
            dataset_count: preview.dataset_count,
            total_size_bytes: preview.total_size_bytes,
            oldest_data,
            next_purge_in,
        });
    }
    
    // Also add models from config that might not have data yet
    for (model, hours) in &config.model_retentions {
        if !models.contains(model) {
            model_retentions.push(ModelRetentionInfo {
                model: model.clone(),
                retention_hours: *hours,
            });
        }
    }
    
    Json(CleanupStatusResponse {
        enabled: config.enabled,
        interval_secs: config.interval_secs,
        next_run_in_secs: Some(config.interval_secs), // Approximate - could track actual last run
        last_run: None, // Would need to track this in state
        model_retentions,
        purge_preview,
        expired_count,
        total_purge_size_bytes,
    })
}

/// POST /api/admin/cleanup/run - Manually trigger cleanup
pub async fn cleanup_run_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Manual cleanup triggered");
    
    let config_dir = std::env::var("CONFIG_DIR").unwrap_or_else(|_| "/app/config".to_string());
    let config = crate::cleanup::CleanupConfig::from_env_and_configs(&config_dir);
    
    let cleanup_task = crate::cleanup::CleanupTask::new(state.clone(), config);
    
    match cleanup_task.run_once().await {
        Ok(stats) => {
            Json(CleanupRunResponse {
                success: true,
                message: "Cleanup completed successfully".to_string(),
                marked_expired: stats.marked_expired,
                files_deleted: stats.files_deleted,
                records_deleted: stats.records_deleted,
                errors: stats.delete_errors,
            })
        }
        Err(e) => {
            error!(error = %e, "Manual cleanup failed");
            Json(CleanupRunResponse {
                success: false,
                message: format!("Cleanup failed: {}", e),
                marked_expired: 0,
                files_deleted: 0,
                records_deleted: 0,
                errors: 1,
            })
        }
    }
}

// ============================================================================
// Database/Storage Sync Types and Handlers
// ============================================================================

/// Response for sync status/dry-run endpoint
#[derive(Debug, Clone, Serialize)]
pub struct SyncStatusResponse {
    pub db_records_checked: u64,
    pub minio_objects_checked: u64,
    pub orphan_db_records: u64,
    pub orphan_minio_objects: u64,
    pub orphan_db_deleted: u64,
    pub orphan_minio_deleted: u64,
    pub errors: Vec<String>,
    pub dry_run: bool,
    pub message: String,
}

/// GET /api/admin/sync/status - Preview sync status (dry run)
pub async fn sync_status_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Getting sync status (dry run)");
    
    let sync_task = crate::cleanup::SyncTask::new(state.clone());
    
    match sync_task.dry_run().await {
        Ok(stats) => {
            let message = if stats.orphan_db_records == 0 && stats.orphan_minio_objects == 0 {
                "Database and storage are in sync".to_string()
            } else {
                format!(
                    "Found {} orphan DB records and {} orphan MinIO objects",
                    stats.orphan_db_records, stats.orphan_minio_objects
                )
            };
            
            Json(SyncStatusResponse {
                db_records_checked: stats.db_records_checked,
                minio_objects_checked: stats.minio_objects_checked,
                orphan_db_records: stats.orphan_db_records,
                orphan_minio_objects: stats.orphan_minio_objects,
                orphan_db_deleted: 0,
                orphan_minio_deleted: 0,
                errors: stats.errors,
                dry_run: true,
                message,
            })
        }
        Err(e) => {
            error!(error = %e, "Sync status check failed");
            Json(SyncStatusResponse {
                db_records_checked: 0,
                minio_objects_checked: 0,
                orphan_db_records: 0,
                orphan_minio_objects: 0,
                orphan_db_deleted: 0,
                orphan_minio_deleted: 0,
                errors: vec![format!("Sync check failed: {}", e)],
                dry_run: true,
                message: format!("Failed to check sync status: {}", e),
            })
        }
    }
}

/// Response for sync preview endpoint - shows what will be deleted
#[derive(Debug, Clone, Serialize)]
pub struct SyncPreviewResponse {
    pub orphan_db_paths: Vec<String>,
    pub orphan_minio_paths: Vec<String>,
    pub db_records_checked: u64,
    pub minio_objects_checked: u64,
}

/// GET /api/admin/sync/preview - Get detailed list of what will be synced
pub async fn sync_preview_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Getting sync preview (detailed orphan list)");
    
    let sync_task = crate::cleanup::SyncTask::new(state.clone());
    
    match sync_task.preview().await {
        Ok(preview) => {
            Json(SyncPreviewResponse {
                orphan_db_paths: preview.orphan_db_paths,
                orphan_minio_paths: preview.orphan_minio_paths,
                db_records_checked: preview.db_records_checked,
                minio_objects_checked: preview.minio_objects_checked,
            }).into_response()
        }
        Err(e) => {
            error!(error = %e, "Sync preview failed");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to generate preview: {}", e)).into_response()
        }
    }
}

/// POST /api/admin/sync/run - Run sync and delete orphans
pub async fn sync_run_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Admin: Running sync to clean up orphans");
    
    let sync_task = crate::cleanup::SyncTask::new(state.clone());
    
    match sync_task.run().await {
        Ok(stats) => {
            let message = format!(
                "Sync complete: deleted {} orphan DB records and {} orphan MinIO objects",
                stats.orphan_db_deleted, stats.orphan_minio_deleted
            );
            
            Json(SyncStatusResponse {
                db_records_checked: stats.db_records_checked,
                minio_objects_checked: stats.minio_objects_checked,
                orphan_db_records: stats.orphan_db_records,
                orphan_minio_objects: stats.orphan_minio_objects,
                orphan_db_deleted: stats.orphan_db_deleted,
                orphan_minio_deleted: stats.orphan_minio_deleted,
                errors: stats.errors,
                dry_run: false,
                message,
            })
        }
        Err(e) => {
            error!(error = %e, "Sync run failed");
            Json(SyncStatusResponse {
                db_records_checked: 0,
                minio_objects_checked: 0,
                orphan_db_records: 0,
                orphan_minio_objects: 0,
                orphan_db_deleted: 0,
                orphan_minio_deleted: 0,
                errors: vec![format!("Sync failed: {}", e)],
                dry_run: false,
                message: format!("Sync failed: {}", e),
            })
        }
    }
}

// ============================================================================
// Full Configuration Endpoint (for dashboard widget)
// ============================================================================

/// Response for the full configuration endpoint
#[derive(Debug, Clone, Serialize)]
pub struct FullConfigurationResponse {
    pub models: Vec<ModelConfigSummary>,
    pub styles: Vec<StyleConfigSummary>,
    pub ingestion: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelConfigSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub source_type: String,
    pub source_bucket: Option<String>,
    pub projection: String,
    pub resolution: Option<String>,
    pub bbox: Option<BBoxInfo>,
    pub schedule_type: String,  // "forecast" or "observation"
    pub cycles: Vec<u8>,        // For forecast models
    pub forecast_hours: Option<ForecastHoursInfo>,
    pub poll_interval_secs: u64,
    pub retention_hours: u32,
    pub precaching_enabled: bool,
    pub precache_keep_recent: Option<u32>,
    pub precache_warm_on_ingest: Option<bool>,
    pub precache_parameters: Option<Vec<String>>,
    pub parameter_count: usize,
    pub parameters: Vec<ParameterSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForecastHoursInfo {
    pub start: u32,
    pub end: u32,
    pub step: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParameterSummary {
    pub name: String,
    pub description: String,
    pub style: String,
    pub units: String,
    pub level_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StyleConfigSummary {
    pub filename: String,
    pub name: String,
    pub description: String,
    pub style_count: usize,
    pub styles: Vec<StyleInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StyleInfo {
    pub id: String,
    pub name: String,
    pub style_type: String,
    pub units: String,
    pub range_min: Option<f64>,
    pub range_max: Option<f64>,
    pub stop_count: usize,
}

/// GET /api/admin/config/full - Get complete configuration for dashboard
pub async fn full_config_handler() -> impl IntoResponse {
    info!("Admin: Getting full configuration for dashboard");
    
    match load_full_configuration().await {
        Ok(config) => Json(config).into_response(),
        Err(e) => {
            error!(error = %e, "Failed to load full configuration");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load config: {}", e)).into_response()
        }
    }
}

async fn load_full_configuration() -> anyhow::Result<FullConfigurationResponse> {
    // Load model configurations
    let models = load_all_model_configs().await?;
    
    // Load style configurations
    let styles = load_all_style_configs().await?;
    
    // Load ingestion config
    let ingestion = load_ingestion_config().await?;
    
    Ok(FullConfigurationResponse {
        models,
        styles,
        ingestion,
    })
}

async fn load_all_model_configs() -> anyhow::Result<Vec<ModelConfigSummary>> {
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
                if let Ok(Some(config)) = load_model_config_summary(stem).await {
                    configs.push(config);
                }
            }
        }
    }
    
    // Sort by model ID
    configs.sort_by(|a, b| a.id.cmp(&b.id));
    
    Ok(configs)
}

async fn load_model_config_summary(model_id: &str) -> anyhow::Result<Option<ModelConfigSummary>> {
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
    let schedule = yaml.get("schedule");
    let retention = yaml.get("retention");
    let precaching = yaml.get("precaching");
    let parameters = yaml.get("parameters");
    
    // Determine schedule type
    let schedule_type = schedule
        .and_then(|s| s.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            // If no type specified but has cycles, it's a forecast model
            if schedule.and_then(|s| s.get("cycles")).is_some() {
                "forecast"
            } else {
                "observation"
            }
        })
        .to_string();
    
    // Parse forecast hours if present
    let forecast_hours = schedule
        .and_then(|s| s.get("forecast_hours"))
        .and_then(|fh| {
            Some(ForecastHoursInfo {
                start: fh.get("start")?.as_u64()? as u32,
                end: fh.get("end")?.as_u64()? as u32,
                step: fh.get("step")?.as_u64()? as u32,
            })
        });
    
    // Parse parameters
    let params_list: Vec<ParameterSummary> = parameters
        .and_then(|p| p.as_sequence())
        .map(|params| {
            params.iter()
                .filter_map(|param| {
                    let name = param.get("name")?.as_str()?.to_string();
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
                    
                    // Count levels
                    let level_count = param.get("levels")
                        .and_then(|l| l.as_sequence())
                        .map(|levels| {
                            levels.iter().map(|level| {
                                // Count values array if present, otherwise 1
                                level.get("values")
                                    .and_then(|v| v.as_sequence())
                                    .map(|vals| vals.len())
                                    .unwrap_or(1)
                            }).sum()
                        })
                        .unwrap_or(0);
                    
                    Some(ParameterSummary {
                        name,
                        description,
                        style,
                        units,
                        level_count,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    
    let config = ModelConfigSummary {
        id: model_id.to_string(),
        name: model
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(model_id)
            .to_string(),
        description: model
            .and_then(|m| m.get("description"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        enabled: model
            .and_then(|m| m.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        source_type: source
            .and_then(|s| s.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        source_bucket: source
            .and_then(|s| s.get("bucket"))
            .and_then(|v| v.as_str())
            .map(String::from),
        projection: grid
            .and_then(|g| g.get("projection"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        resolution: grid
            .and_then(|g| g.get("resolution"))
            .and_then(|v| v.as_str())
            .map(String::from),
        bbox: grid.and_then(|g| g.get("bbox")).and_then(|bbox| {
            Some(BBoxInfo {
                min_lon: bbox.get("min_lon")?.as_f64()?,
                min_lat: bbox.get("min_lat")?.as_f64()?,
                max_lon: bbox.get("max_lon")?.as_f64()?,
                max_lat: bbox.get("max_lat")?.as_f64()?,
            })
        }),
        schedule_type,
        cycles: schedule
            .and_then(|s| s.get("cycles"))
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect()
            })
            .unwrap_or_default(),
        forecast_hours,
        poll_interval_secs: schedule
            .and_then(|s| s.get("poll_interval_secs"))
            .and_then(|v| v.as_u64())
            .unwrap_or(3600),
        retention_hours: retention
            .and_then(|r| r.get("hours"))
            .and_then(|v| v.as_u64())
            .unwrap_or(24) as u32,
        precaching_enabled: precaching
            .and_then(|p| p.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        precache_keep_recent: precaching
            .and_then(|p| p.get("keep_recent"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32),
        precache_warm_on_ingest: precaching
            .and_then(|p| p.get("warm_on_ingest"))
            .and_then(|v| v.as_bool()),
        precache_parameters: precaching
            .and_then(|p| p.get("parameters"))
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            }),
        parameter_count: params_list.len(),
        parameters: params_list,
    };
    
    Ok(Some(config))
}

async fn load_all_style_configs() -> anyhow::Result<Vec<StyleConfigSummary>> {
    use std::fs;
    use std::path::Path;
    
    let styles_dir = Path::new("config/styles");
    if !styles_dir.exists() {
        return Ok(Vec::new());
    }
    
    let mut configs = Vec::new();
    
    for entry in fs::read_dir(styles_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                // Skip schema example
                if stem == "schema.example" {
                    continue;
                }
                if let Ok(Some(config)) = load_style_config_summary(&path).await {
                    configs.push(config);
                }
            }
        }
    }
    
    // Sort by filename
    configs.sort_by(|a, b| a.filename.cmp(&b.filename));
    
    Ok(configs)
}

async fn load_style_config_summary(path: &std::path::Path) -> anyhow::Result<Option<StyleConfigSummary>> {
    use std::fs;
    
    let contents = fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&contents)?;
    
    let filename = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    
    let metadata = json.get("metadata");
    let styles_obj = json.get("styles");
    
    let mut styles = Vec::new();
    
    if let Some(styles_map) = styles_obj.and_then(|s| s.as_object()) {
        for (id, style) in styles_map {
            let name = style.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(id)
                .to_string();
            let style_type = style.get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("gradient")
                .to_string();
            let units = style.get("units")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let range = style.get("range");
            let range_min = range.and_then(|r| r.get("min")).and_then(|v| v.as_f64());
            let range_max = range.and_then(|r| r.get("max")).and_then(|v| v.as_f64());
            let stop_count = style.get("stops")
                .and_then(|s| s.as_array())
                .map(|arr| arr.len())
                .unwrap_or(0);
            
            styles.push(StyleInfo {
                id: id.clone(),
                name,
                style_type,
                units,
                range_min,
                range_max,
                stop_count,
            });
        }
    }
    
    let name = metadata
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(&filename)
        .to_string();
    
    Ok(Some(StyleConfigSummary {
        filename,
        name,
        description: metadata
            .and_then(|m| m.get("description"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        style_count: styles.len(),
        styles,
    }))
}

async fn load_ingestion_config() -> anyhow::Result<serde_json::Value> {
    use std::fs;
    use std::path::Path;
    
    let config_path = Path::new("config/ingestion.yaml");
    
    if !config_path.exists() {
        return Ok(serde_json::json!({}));
    }
    
    let contents = fs::read_to_string(config_path)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&contents)?;
    
    // Convert YAML to JSON for easier frontend handling
    let json_str = serde_json::to_string(&yaml)?;
    let json: serde_json::Value = serde_json::from_str(&json_str)?;
    
    Ok(json)
}
