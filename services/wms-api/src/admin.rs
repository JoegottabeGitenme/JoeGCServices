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
    
    match ingest_file(&state, &payload).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(e) => {
            error!(error = %e, file = %payload.file_path, "Ingestion failed");
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

/// Ingest a file into the catalog
async fn ingest_file(
    state: &Arc<AppState>,
    request: &IngestRequest,
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
        return ingest_netcdf_file(state, request, file_path).await;
    }
    
    if !is_grib2 {
        anyhow::bail!("Unsupported file type. Expected .grib2, .grb2, or .nc");
    }
    
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
    
    // Parse GRIB2 and extract parameters
    let mut reader = grib2_parser::Grib2Reader::new(data);
    let mut registered_params = HashSet::new();
    let mut grib_reference_time: Option<chrono::DateTime<Utc>> = None;
    let mut datasets_registered = 0;
    
    // Parameters to ingest with their accepted level types
    let pressure_levels: HashSet<u32> = [
        1000, 975, 950, 925, 900, 850, 800, 750, 700, 650, 
        600, 550, 500, 450, 400, 350, 300, 250, 200, 150, 
        100, 70, 50, 30, 20, 10
    ].into_iter().collect();
    
    // Target parameters and their level specs
    let target_params: Vec<(&str, Vec<(u8, Option<u32>)>)> = vec![
        ("PRMSL", vec![(101, None)]),                    // Mean sea level pressure
        ("TMP", vec![(103, Some(2)), (100, None)]),      // 2m temp + pressure levels
        ("UGRD", vec![(103, Some(10)), (100, None)]),    // 10m wind + pressure levels
        ("VGRD", vec![(103, Some(10)), (100, None)]),    // 10m wind + pressure levels
        ("RH", vec![(103, Some(2)), (100, None)]),       // 2m RH + pressure levels
        ("HGT", vec![(100, None)]),                      // Geopotential height
        ("GUST", vec![(1, None)]),                       // Surface wind gust
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
            let reference_time = grib_reference_time.unwrap_or_else(Utc::now);
            
            // Sanitize level for path
            let level_sanitized = level
                .replace([' ', '/'], "_")
                .to_lowercase();
            
            // Storage path: shredded/{model}/{run_date}/{param}_{level}/f{fhr:03}.grib2
            let run_date = reference_time.format("%Y%m%d_%Hz").to_string();
            let storage_path = format!(
                "shredded/{}/{}/{}_{}/f{:03}.grib2",
                model,
                run_date,
                param.to_lowercase(),
                level_sanitized,
                forecast_hour
            );
            
            // Store shredded GRIB message
            let shredded_data = message.raw_data.clone();
            let shredded_size = shredded_data.len() as u64;
            
            state.storage.put(&storage_path, shredded_data).await?;
            
            debug!(
                param = %param,
                level = %level,
                path = %storage_path,
                size = shredded_size,
                "Stored shredded GRIB message"
            );
            
            // Get model-specific bounding box
            let bbox = get_model_bbox(&model);
            
            let entry = CatalogEntry {
                model: model.clone(),
                parameter: param.to_string(),
                level: level.clone(),
                reference_time,
                forecast_hour,
                bbox,
                storage_path,
                file_size: shredded_size,
            };
            
            match state.catalog.register_dataset(&entry).await {
                Ok(id) => {
                    debug!(id = %id, param = %param, level = %level, "Registered dataset");
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
