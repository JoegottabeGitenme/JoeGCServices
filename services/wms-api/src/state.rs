//! Application state and shared resources.

use anyhow::Result;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use grid_processor::{ChunkCache, GridProcessorConfig};
use projection::ProjectionLutCache;
use storage::{Catalog, ObjectStorage, ObjectStorageConfig, TileCache, TileMemoryCache};
use crate::capabilities_cache::CapabilitiesCache;
use crate::layer_config::LayerConfigRegistry;
use crate::metrics::MetricsCollector;
use crate::model_config::ModelDimensionRegistry;

/// Configuration for performance optimizations.
/// Each optimization can be toggled on/off via environment variables.
#[derive(Clone, Debug)]
pub struct OptimizationConfig {
    // L1 Cache
    pub l1_cache_enabled: bool,
    pub l1_cache_size: usize,
    pub l1_cache_ttl_secs: u64,
    
    // Zarr Chunk Cache (for chunked Zarr grid data)
    pub chunk_cache_enabled: bool,
    pub chunk_cache_size_mb: usize,
    
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
            
            // Zarr Chunk Cache (for chunked Zarr grid data)
            // This caches decompressed chunks from Zarr files for efficient partial reads
            chunk_cache_enabled: parse_bool("ENABLE_CHUNK_CACHE", true),
            chunk_cache_size_mb: parse_usize("CHUNK_CACHE_SIZE_MB", 1024), // Default 1GB
            
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

/// Factory for creating grid processors from Zarr-format data.
///
/// This factory manages a shared chunk cache for efficient partial reads
/// from Zarr-format grid data stored in MinIO.
///
/// # Data Flow
/// 
/// When a tile request comes in for a dataset with `zarr_metadata`:
/// 1. The catalog query returns the `CatalogEntry` with embedded `zarr_metadata`
/// 2. `GridProcessorFactory::create_processor()` creates a `ZarrGridProcessor`
///    using metadata from the catalog (no MinIO request needed for metadata)
/// 3. The processor fetches only the chunks needed for the tile via byte-range requests
/// 4. Decompressed chunks are cached in the shared `ChunkCache` for reuse
///
/// For datasets without `zarr_metadata`, the legacy GRIB2/NetCDF path is used.
pub struct GridProcessorFactory {
    /// Grid processor configuration.
    pub config: GridProcessorConfig,
    /// Shared chunk cache for decompressed Zarr chunks.
    pub chunk_cache: Arc<RwLock<ChunkCache>>,
}

impl GridProcessorFactory {
    /// Create a new GridProcessorFactory.
    pub fn new(_storage: Arc<ObjectStorage>, chunk_cache_size_mb: usize) -> Self {
        let chunk_cache = Arc::new(RwLock::new(ChunkCache::new(
            chunk_cache_size_mb * 1024 * 1024,
        )));
        
        let config = GridProcessorConfig::default();
        
        Self {
            config,
            chunk_cache,
        }
    }
    
    /// Get chunk cache statistics for monitoring.
    pub async fn cache_stats(&self) -> grid_processor::CacheStats {
        self.chunk_cache.read().await.stats()
    }
    
    /// Get the shared chunk cache reference (for creating processors).
    pub fn chunk_cache(&self) -> Arc<RwLock<ChunkCache>> {
        self.chunk_cache.clone()
    }
    
    /// Get the processor config.
    pub fn config(&self) -> &GridProcessorConfig {
        &self.config
    }
    
    /// Clear the chunk cache (for hot reload / cache invalidation).
    /// Returns the number of entries and bytes cleared.
    pub async fn clear_chunk_cache(&self) -> (usize, u64) {
        let mut cache = self.chunk_cache.write().await;
        let stats = cache.stats();
        let entries = stats.entries;
        let bytes = stats.memory_bytes;
        cache.clear();
        (entries, bytes)
    }
}

/// Shared application state.
pub struct AppState {
    pub catalog: Catalog,
    pub cache: Mutex<TileCache>,
    pub tile_memory_cache: TileMemoryCache,  // L1 cache for rendered tiles
    pub storage: Arc<ObjectStorage>,
    pub grid_processor_factory: GridProcessorFactory,  // Factory for Zarr-based grid processors
    pub projection_luts: ProjectionLuts,  // Pre-computed projection LUTs for GOES
    pub metrics: Arc<MetricsCollector>,
    pub prefetch_rings: u32,  // Number of rings to prefetch (1=8 tiles, 2=24 tiles)
    pub optimization_config: OptimizationConfig,  // Feature flags for optimizations
    pub chunk_warmer: tokio::sync::RwLock<Option<std::sync::Arc<crate::chunk_warming::ChunkWarmer>>>,  // Chunk cache warmer
    pub model_dimensions: ModelDimensionRegistry,  // Model dimension configurations (from YAML)
    pub layer_configs: tokio::sync::RwLock<LayerConfigRegistry>,  // Layer configurations (from YAML) - styles, units, levels
    pub capabilities_cache: CapabilitiesCache,  // Cache for WMS/WMTS capabilities documents
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
        let storage = Arc::new(ObjectStorage::new(&storage_config)?);
        let metrics = Arc::new(MetricsCollector::new());

        // Create L1 in-memory tile cache
        let tile_memory_cache = TileMemoryCache::new(tile_cache_size, tile_cache_ttl);
        
        // Create GridProcessorFactory for Zarr-based data access
        let grid_processor_factory = if optimization_config.chunk_cache_enabled {
            let factory = GridProcessorFactory::new(
                storage.clone(),
                optimization_config.chunk_cache_size_mb,
            );
            info!(
                chunk_cache_size_mb = optimization_config.chunk_cache_size_mb,
                "GridProcessorFactory initialized with chunk cache"
            );
            factory
        } else {
            info!("Chunk cache disabled (set ENABLE_CHUNK_CACHE=true to enable)");
            GridProcessorFactory::new(storage.clone(), 0) // 0 MB = minimal cache
        };

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
        
        // Load layer configurations from YAML files (styles, units, levels)
        let layer_configs = tokio::sync::RwLock::new(LayerConfigRegistry::load_from_directory(&config_dir));
        {
            let configs = layer_configs.read().await;
            info!(
                models = configs.models().len(),
                total_layers = configs.total_layers(),
                "Loaded layer configurations"
            );
        }

        // Initialize capabilities cache
        let capabilities_cache_ttl = env::var("CAPABILITIES_CACHE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);
        let capabilities_cache = CapabilitiesCache::new(capabilities_cache_ttl);

        Ok(Self {
            catalog,
            cache: Mutex::new(cache),
            tile_memory_cache,
            storage,
            grid_processor_factory,
            projection_luts,
            metrics,
            prefetch_rings,
            optimization_config,
            chunk_warmer: tokio::sync::RwLock::new(None),
            model_dimensions,
            layer_configs,
            capabilities_cache,
        })
    }
}
