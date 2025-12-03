//! Application state and shared resources.

use anyhow::Result;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;

use storage::{Catalog, GribCache, GridDataCache, JobQueue, ObjectStorage, ObjectStorageConfig, TileCache, TileMemoryCache};
use crate::metrics::MetricsCollector;

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
    
    // Prefetch
    pub prefetch_enabled: bool,
    pub prefetch_rings: u32,
    pub prefetch_min_zoom: u32,
    pub prefetch_max_zoom: u32,
    
    // Cache Warming
    pub cache_warming_enabled: bool,
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
            
            // Prefetch
            prefetch_enabled: parse_bool("ENABLE_PREFETCH", true),
            prefetch_rings: parse_u32("PREFETCH_RINGS", 2),
            prefetch_min_zoom: parse_u32("PREFETCH_MIN_ZOOM", 3),
            prefetch_max_zoom: parse_u32("PREFETCH_MAX_ZOOM", 12),
            
            // Cache Warming
            cache_warming_enabled: parse_bool("ENABLE_CACHE_WARMING", true),
        }
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
    pub metrics: Arc<MetricsCollector>,
    pub prefetch_rings: u32,  // Number of rings to prefetch (1=8 tiles, 2=24 tiles)
    pub optimization_config: OptimizationConfig,  // Feature flags for optimizations
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

        Ok(Self {
            catalog,
            cache: Mutex::new(cache),
            tile_memory_cache,
            queue,
            storage,
            grib_cache,
            grid_cache,
            metrics,
            prefetch_rings,
            optimization_config,
        })
    }
}
