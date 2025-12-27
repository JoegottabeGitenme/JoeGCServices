//! Application state and shared resources.

use anyhow::Result;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::capabilities_cache::CapabilitiesCache;
use crate::layer_config::LayerConfigRegistry;
use crate::metrics::MetricsCollector;
use crate::model_config::ModelDimensionRegistry;
use grid_processor::{GridProcessorFactory, MinioConfig};
use storage::{Catalog, ObjectStorage, ObjectStorageConfig, TileCache, TileMemoryCache};

/// Configuration for performance optimizations.
/// Each optimization can be toggled on/off via environment variables.
#[derive(Clone, Debug)]
pub struct OptimizationConfig {
    // L1 Cache (memory-based)
    pub l1_cache_enabled: bool,
    pub l1_cache_size_mb: usize, // Max memory in MB (default: 1024 = 1GB)
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

    // Memory Pressure Management
    pub memory_pressure_enabled: bool,
    pub memory_limit_mb: usize, // Hard limit for total memory (0 = auto-detect from cgroup)
    pub memory_pressure_threshold: f64, // Percentage (0.0-1.0) at which to start evicting (default 0.80)
    pub memory_pressure_target: f64,    // Target percentage after eviction (default 0.70)
    pub memory_check_interval_secs: u64, // How often to check memory pressure
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
            // L1 Cache (memory-based)
            l1_cache_enabled: parse_bool("ENABLE_L1_CACHE", true),
            l1_cache_size_mb: parse_usize("TILE_CACHE_SIZE_MB", 1024), // Default: 1GB
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

// GridProcessorFactory is now imported from grid_processor crate.
// It provides:
// - Shared ChunkCache for decompressed Zarr chunks
// - MinIO configuration for creating storage on demand
// - Common GridProcessorConfig settings
//
// See grid_processor::GridProcessorFactory for documentation.

/// Shared application state.
pub struct AppState {
    pub catalog: Catalog,
    pub cache: Mutex<TileCache>,
    pub tile_memory_cache: TileMemoryCache, // L1 cache for rendered tiles
    pub storage: Arc<ObjectStorage>,
    pub grid_processor_factory: GridProcessorFactory, // Factory for Zarr-based grid processors
    pub metrics: Arc<MetricsCollector>,
    pub prefetch_rings: u32, // Number of rings to prefetch (1=8 tiles, 2=24 tiles)
    pub optimization_config: OptimizationConfig, // Feature flags for optimizations
    pub chunk_warmer:
        tokio::sync::RwLock<Option<std::sync::Arc<crate::chunk_warming::ChunkWarmer>>>, // Chunk cache warmer
    pub model_dimensions: ModelDimensionRegistry, // Model dimension configurations (from YAML)
    pub layer_configs: tokio::sync::RwLock<LayerConfigRegistry>, // Layer configurations (from YAML) - styles, units, levels
    pub capabilities_cache: CapabilitiesCache, // Cache for WMS/WMTS capabilities documents
}

impl AppState {
    pub async fn new() -> Result<Self> {
        // Load optimization configuration from environment
        let optimization_config = OptimizationConfig::from_env();

        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@postgres:5432/weatherwms".to_string()
        });

        let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());
        let redis_tile_ttl_secs = env::var("REDIS_TILE_TTL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(3600); // Default: 1 hour

        // Parse connection pool sizes from environment
        let db_pool_size = env::var("DATABASE_POOL_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(20); // Increased from 10 to 20 default

        // Use optimization config for cache sizes
        let tile_cache_size_mb = optimization_config.l1_cache_size_mb;
        let tile_cache_ttl = optimization_config.l1_cache_ttl_secs;
        let prefetch_rings = optimization_config.prefetch_rings;

        let storage_config = ObjectStorageConfig {
            endpoint: env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://minio:9000".to_string()),
            bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
            access_key_id: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
            secret_access_key: env::var("S3_SECRET_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            region: env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
            allow_http: env::var("S3_ALLOW_HTTP")
                .map(|v| v.to_lowercase() == "true" || v == "1")
                .unwrap_or(true),
        };

        let catalog = Catalog::connect_with_pool_size(&database_url, db_pool_size).await?;
        let cache = TileCache::connect(&redis_url, redis_tile_ttl_secs).await?;
        let storage = Arc::new(ObjectStorage::new(&storage_config)?);
        let metrics = Arc::new(MetricsCollector::new());

        // Create L1 in-memory tile cache (memory-based, size in MB)
        let tile_memory_cache = TileMemoryCache::new(tile_cache_size_mb, tile_cache_ttl);
        info!(
            l1_cache_size_mb = tile_cache_size_mb,
            l1_cache_ttl_secs = tile_cache_ttl,
            "L1 tile memory cache initialized"
        );

        // Create GridProcessorFactory for Zarr-based data access
        // Now using the factory from grid-processor crate which manages MinIO config internally
        let grid_processor_factory = if optimization_config.chunk_cache_enabled {
            let minio_config = MinioConfig::from_env();
            let factory =
                GridProcessorFactory::new(minio_config, optimization_config.chunk_cache_size_mb);
            info!(
                chunk_cache_size_mb = optimization_config.chunk_cache_size_mb,
                "GridProcessorFactory initialized with chunk cache"
            );
            factory
        } else {
            info!("Chunk cache disabled (set ENABLE_CHUNK_CACHE=true to enable)");
            let minio_config = MinioConfig::from_env();
            GridProcessorFactory::new(minio_config, 0) // 0 MB = minimal cache
        };

        // Load model dimension configurations from YAML files
        let config_dir = env::var("CONFIG_DIR").unwrap_or_else(|_| "config".to_string());
        let model_dimensions = ModelDimensionRegistry::load_from_directory(&config_dir);
        info!(
            models = model_dimensions.models().len(),
            "Loaded model dimension configurations"
        );

        // Load layer configurations from YAML files (styles, units, levels)
        let layer_configs =
            tokio::sync::RwLock::new(LayerConfigRegistry::load_from_directory(&config_dir));
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
