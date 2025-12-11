//! Application state and shared resources.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::VecDeque;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use projection::ProjectionLutCache;
use storage::{Catalog, GribCache, GridDataCache, JobQueue, ObjectStorage, ObjectStorageConfig, TileCache, TileMemoryCache};
use crate::metrics::MetricsCollector;
use crate::model_config::ModelDimensionRegistry;

// ============================================================================
// Ingestion Tracking
// ============================================================================

/// Status of an active ingestion job
#[derive(Debug, Clone, Serialize)]
pub struct ActiveIngestion {
    pub id: String,
    pub file_path: String,
    pub model: String,
    pub started_at: DateTime<Utc>,
    pub status: String,  // "parsing", "shredding", "storing", "registering"
    pub parameters_found: u32,
    pub parameters_stored: u32,
}

/// A completed ingestion record
#[derive(Debug, Clone, Serialize)]
pub struct CompletedIngestion {
    pub id: String,
    pub file_path: String,
    pub model: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub parameters_registered: u32,
    pub success: bool,
    pub error_message: Option<String>,
}

/// Tracks ingestion activity for the dashboard
#[derive(Debug)]
pub struct IngestionTracker {
    /// Currently active ingestions (keyed by ID)
    pub active: Mutex<std::collections::HashMap<String, ActiveIngestion>>,
    /// Recently completed ingestions (ring buffer, max 50)
    pub completed: Mutex<VecDeque<CompletedIngestion>>,
    /// Maximum completed entries to keep
    max_completed: usize,
}

impl IngestionTracker {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(std::collections::HashMap::new()),
            completed: Mutex::new(VecDeque::new()),
            max_completed: 50,
        }
    }
    
    /// Start tracking an ingestion job
    pub async fn start(&self, id: String, file_path: String, model: String) {
        let ingestion = ActiveIngestion {
            id: id.clone(),
            file_path,
            model,
            started_at: Utc::now(),
            status: "parsing".to_string(),
            parameters_found: 0,
            parameters_stored: 0,
        };
        self.active.lock().await.insert(id, ingestion);
    }
    
    /// Update ingestion status
    pub async fn update(&self, id: &str, status: &str, found: u32, stored: u32) {
        if let Some(ingestion) = self.active.lock().await.get_mut(id) {
            ingestion.status = status.to_string();
            ingestion.parameters_found = found;
            ingestion.parameters_stored = stored;
        }
    }
    
    /// Complete an ingestion job (success or failure)
    pub async fn complete(&self, id: &str, success: bool, error_message: Option<String>, parameters_registered: u32) {
        let mut active = self.active.lock().await;
        if let Some(ingestion) = active.remove(id) {
            let completed_at = Utc::now();
            let duration_ms = (completed_at - ingestion.started_at).num_milliseconds() as u64;
            
            let completed = CompletedIngestion {
                id: ingestion.id,
                file_path: ingestion.file_path,
                model: ingestion.model,
                started_at: ingestion.started_at,
                completed_at,
                duration_ms,
                parameters_registered,
                success,
                error_message,
            };
            
            let mut completed_list = self.completed.lock().await;
            completed_list.push_front(completed);
            
            // Keep only the most recent entries
            while completed_list.len() > self.max_completed {
                completed_list.pop_back();
            }
        }
    }
    
    /// Get current ingestion status for the dashboard
    pub async fn get_status(&self) -> IngestionTrackerStatus {
        let active = self.active.lock().await;
        let completed = self.completed.lock().await;
        
        IngestionTrackerStatus {
            active: active.values().cloned().collect(),
            recent: completed.iter().take(10).cloned().collect(),
            total_completed: completed.len(),
        }
    }
}

/// Response for ingestion status endpoint
#[derive(Debug, Clone, Serialize)]
pub struct IngestionTrackerStatus {
    pub active: Vec<ActiveIngestion>,
    pub recent: Vec<CompletedIngestion>,
    pub total_completed: usize,
}

/// Configuration for performance optimizations.
/// Each optimization can be toggled on/off via environment variables.
#[derive(Clone, Debug)]
pub struct OptimizationConfig {
    // L1 Cache
    pub l1_cache_enabled: bool,
    pub l1_cache_size: usize,
    pub l1_cache_ttl_secs: u64,
    
    // GRIB Cache
    pub grib_cache_enabled: bool,
    pub grib_cache_size: usize,
    
    // Grid Data Cache (for parsed GOES/NetCDF data)
    pub grid_cache_enabled: bool,
    pub grid_cache_size: usize,
    
    // GRIB2 Grid Cache (extends grid cache to also cache parsed GRIB2 data)
    pub grib_grid_cache_enabled: bool,
    
    // Prefetch
    pub prefetch_enabled: bool,
    pub prefetch_rings: u32,
    pub prefetch_min_zoom: u32,
    pub prefetch_max_zoom: u32,
    
    // Cache Warming
    pub cache_warming_enabled: bool,
    
    // Projection LUT
    pub projection_lut_enabled: bool,
    pub projection_lut_dir: String,
    
    // Memory Pressure Management
    pub memory_pressure_enabled: bool,
    pub memory_limit_mb: usize,           // Hard limit for total memory (0 = auto-detect from cgroup)
    pub memory_pressure_threshold: f64,   // Percentage (0.0-1.0) at which to start evicting (default 0.80)
    pub memory_pressure_target: f64,      // Target percentage after eviction (default 0.70)
    pub memory_check_interval_secs: u64,  // How often to check memory pressure
}

impl OptimizationConfig {
    /// Parse optimization configuration from environment variables.
    pub fn from_env() -> Self {
        fn parse_bool(key: &str, default: bool) -> bool {
            env::var(key)
                .ok()
                .map(|v| v.to_lowercase() == "true" || v == "1")
                .unwrap_or(default)
        }
        
        fn parse_usize(key: &str, default: usize) -> usize {
            env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        
        fn parse_u64(key: &str, default: u64) -> u64 {
            env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        
        fn parse_u32(key: &str, default: u32) -> u32 {
            env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        
        Self {
            // L1 Cache
            l1_cache_enabled: parse_bool("ENABLE_L1_CACHE", true),
            l1_cache_size: parse_usize("TILE_CACHE_SIZE", 10000),
            l1_cache_ttl_secs: parse_u64("TILE_CACHE_TTL_SECS", 300),
            
            // GRIB Cache
            grib_cache_enabled: parse_bool("ENABLE_GRIB_CACHE", true),
            grib_cache_size: parse_usize("GRIB_CACHE_SIZE", 500),
            
            // Grid Data Cache (for parsed GOES/NetCDF data)
            grid_cache_enabled: parse_bool("ENABLE_GRID_CACHE", true),
            grid_cache_size: parse_usize("GRID_CACHE_SIZE", 100),
            
            // GRIB2 Grid Cache (extends grid cache to also cache parsed GRIB2 data)
            // This can significantly reduce CPU usage for adjacent tile requests
            grib_grid_cache_enabled: parse_bool("ENABLE_GRIB_GRID_CACHE", true),
            
            // Prefetch
            prefetch_enabled: parse_bool("ENABLE_PREFETCH", true),
            prefetch_rings: parse_u32("PREFETCH_RINGS", 2),
            prefetch_min_zoom: parse_u32("PREFETCH_MIN_ZOOM", 3),
            prefetch_max_zoom: parse_u32("PREFETCH_MAX_ZOOM", 12),
            
            // Cache Warming
            cache_warming_enabled: parse_bool("ENABLE_CACHE_WARMING", true),
            
            // Projection LUT
            projection_lut_enabled: parse_bool("ENABLE_PROJECTION_LUT", true),
            projection_lut_dir: env::var("PROJECTION_LUT_DIR")
                .unwrap_or_else(|_| "/app/data/luts".to_string()),
            
            // Memory Pressure Management
            memory_pressure_enabled: parse_bool("ENABLE_MEMORY_PRESSURE", true),
            memory_limit_mb: parse_usize("MEMORY_LIMIT_MB", 0), // 0 = auto-detect
            memory_pressure_threshold: {
                let val = env::var("MEMORY_PRESSURE_THRESHOLD")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.80);
                val.clamp(0.0, 1.0)
            },
            memory_pressure_target: {
                let val = env::var("MEMORY_PRESSURE_TARGET")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.70);
                val.clamp(0.0, 1.0)
            },
            memory_check_interval_secs: parse_u64("MEMORY_CHECK_INTERVAL_SECS", 30),
        }
    }
}

/// Holds pre-computed projection LUTs for fast GOES tile rendering.
pub struct ProjectionLuts {
    pub goes16: Option<ProjectionLutCache>,
    pub goes18: Option<ProjectionLutCache>,
}

impl ProjectionLuts {
    /// Load LUTs from the specified directory.
    pub fn load(lut_dir: &str) -> Self {
        let dir = PathBuf::from(lut_dir);
        
        let goes16 = Self::load_satellite_lut(&dir, "goes16");
        let goes18 = Self::load_satellite_lut(&dir, "goes18");
        
        Self { goes16, goes18 }
    }
    
    fn load_satellite_lut(dir: &PathBuf, satellite: &str) -> Option<ProjectionLutCache> {
        // Try different file name patterns
        let patterns = [
            format!("{}_conus_z0-7.lut", satellite),
            format!("{}_conus_z0-8.lut", satellite),
            format!("{}.lut", satellite),
        ];
        
        for pattern in &patterns {
            let path = dir.join(pattern);
            if path.exists() {
                match File::open(&path) {
                    Ok(file) => {
                        let reader = BufReader::new(file);
                        match ProjectionLutCache::load(reader) {
                            Ok(cache) => {
                                info!(
                                    satellite = satellite,
                                    path = %path.display(),
                                    tiles = cache.len(),
                                    max_zoom = cache.max_zoom,
                                    memory_mb = cache.memory_usage() as f64 / 1024.0 / 1024.0,
                                    "Loaded projection LUT"
                                );
                                return Some(cache);
                            }
                            Err(e) => {
                                warn!(
                                    satellite = satellite,
                                    path = %path.display(),
                                    error = %e,
                                    "Failed to parse projection LUT"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            satellite = satellite,
                            path = %path.display(),
                            error = %e,
                            "Failed to open projection LUT file"
                        );
                    }
                }
            }
        }
        
        info!(
            satellite = satellite,
            dir = %dir.display(),
            "No projection LUT found (will compute transforms on-the-fly)"
        );
        None
    }
    
    /// Get the LUT for a satellite, if available.
    // pub fn get(&self, satellite: &str) -> Option<&ProjectionLutCache> {
    //     match satellite {
    //         "goes16" | "goes" => self.goes16.as_ref(),
    //         "goes18" => self.goes18.as_ref(),
    //         _ => None,
    //     }
    // }
    
    /// Check if any LUTs are loaded.
    pub fn is_empty(&self) -> bool {
        self.goes16.is_none() && self.goes18.is_none()
    }
    
    /// Get total memory usage of all loaded LUTs.
    pub fn memory_usage(&self) -> usize {
        self.goes16.as_ref().map(|c| c.memory_usage()).unwrap_or(0)
            + self.goes18.as_ref().map(|c| c.memory_usage()).unwrap_or(0)
    }
}

/// Shared application state.
pub struct AppState {
    pub catalog: Catalog,
    pub cache: Mutex<TileCache>,
    pub tile_memory_cache: TileMemoryCache,  // L1 cache for rendered tiles
    #[allow(dead_code)]
    pub queue: JobQueue,
    pub storage: Arc<ObjectStorage>,
    pub grib_cache: GribCache,
    pub grid_cache: GridDataCache,  // Cache for parsed grid data (GOES/NetCDF)
    pub projection_luts: ProjectionLuts,  // Pre-computed projection LUTs for GOES
    pub metrics: Arc<MetricsCollector>,
    pub prefetch_rings: u32,  // Number of rings to prefetch (1=8 tiles, 2=24 tiles)
    pub optimization_config: OptimizationConfig,  // Feature flags for optimizations
    pub grid_warmer: tokio::sync::RwLock<Option<std::sync::Arc<crate::grid_warming::GridWarmer>>>,  // Grid cache warmer
    pub ingestion_tracker: IngestionTracker,  // Track active/recent ingestions for dashboard
    pub model_dimensions: ModelDimensionRegistry,  // Model dimension configurations (from YAML)
}

impl AppState {
    pub async fn new() -> Result<Self> {
        // Load optimization configuration from environment
        let optimization_config = OptimizationConfig::from_env();
        
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@postgres:5432/weatherwms".to_string()
        });

        let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());
        
        // Parse connection pool sizes from environment
        let db_pool_size = env::var("DATABASE_POOL_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(20); // Increased from 10 to 20 default

        // Use optimization config for cache sizes
        let grib_cache_size = optimization_config.grib_cache_size;
        let grid_cache_size = optimization_config.grid_cache_size;
        let tile_cache_size = optimization_config.l1_cache_size;
        let tile_cache_ttl = optimization_config.l1_cache_ttl_secs;
        let prefetch_rings = optimization_config.prefetch_rings;

        let storage_config = ObjectStorageConfig {
            endpoint: env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://minio:9000".to_string()),
            bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
            access_key_id: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
            secret_access_key: env::var("S3_SECRET_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            region: "us-east-1".to_string(),
            allow_http: true,
        };

        let catalog = Catalog::connect_with_pool_size(&database_url, db_pool_size).await?;
        let cache = TileCache::connect(&redis_url).await?;
        let queue = JobQueue::connect(&redis_url).await?;
        let storage = Arc::new(ObjectStorage::new(&storage_config)?);
        let metrics = Arc::new(MetricsCollector::new());
        
        // Create GRIB cache with shared storage reference
        let grib_cache = GribCache::new(grib_cache_size, storage.clone());
        
        // Create grid data cache for parsed GOES/NetCDF data
        let grid_cache = GridDataCache::new(grid_cache_size);

        // Create L1 in-memory tile cache
        let tile_memory_cache = TileMemoryCache::new(tile_cache_size, tile_cache_ttl);

        // Load projection LUTs for fast GOES rendering
        let projection_luts = if optimization_config.projection_lut_enabled {
            info!(
                lut_dir = %optimization_config.projection_lut_dir,
                "Loading projection LUTs for GOES rendering..."
            );
            let luts = ProjectionLuts::load(&optimization_config.projection_lut_dir);
            if luts.is_empty() {
                info!("No projection LUTs loaded - GOES rendering will use on-the-fly projection");
            } else {
                info!(
                    memory_mb = luts.memory_usage() as f64 / 1024.0 / 1024.0,
                    "Projection LUTs loaded successfully"
                );
            }
            luts
        } else {
            info!("Projection LUTs disabled (set ENABLE_PROJECTION_LUT=true to enable)");
            ProjectionLuts { goes16: None, goes18: None }
        };
        
        // Load model dimension configurations from YAML files
        let config_dir = env::var("CONFIG_DIR").unwrap_or_else(|_| "config".to_string());
        let model_dimensions = ModelDimensionRegistry::load_from_directory(&config_dir);
        info!(
            models = model_dimensions.models().len(),
            "Loaded model dimension configurations"
        );

        Ok(Self {
            catalog,
            cache: Mutex::new(cache),
            tile_memory_cache,
            queue,
            storage,
            grib_cache,
            grid_cache,
            projection_luts,
            metrics,
            prefetch_rings,
            optimization_config,
            grid_warmer: tokio::sync::RwLock::new(None),
            ingestion_tracker: IngestionTracker::new(),
            model_dimensions,
        })
    }
    
    /// Get the grid cache reference if GRIB grid caching is enabled.
    /// Returns None if caching is disabled via ENABLE_GRIB_GRID_CACHE=false
    pub fn grid_cache_if_enabled(&self) -> Option<&GridDataCache> {
        if self.optimization_config.grib_grid_cache_enabled {
            Some(&self.grid_cache)
        } else {
            None
        }
    }
}
