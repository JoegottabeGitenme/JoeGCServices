# storage

Unified storage abstractions for PostgreSQL, MinIO/S3, Redis, and Zarr grid data access.

## Overview

**Location**: `crates/storage/`  
**Dependencies**: `sqlx`, `aws-sdk-s3`, `redis`, `object_store`  
**LOC**: ~2,000

## Components

### ObjectStorage

MinIO/S3 operations for weather data and Zarr arrays:

```rust
use storage::ObjectStorage;

let storage = ObjectStorage::new(&config)?;

// Put object (raw bytes)
storage.put_object("grids/gfs/data.bin", data).await?;

// Get object
let data = storage.get_object("grids/gfs/data.bin").await?;

// List objects with prefix
let keys = storage.list_objects("grids/gfs/").await?;

// Delete object
storage.delete_object("grids/gfs/old_data.bin").await?;

// Check if object exists
let exists = storage.exists("grids/gfs/data.bin").await?;
```

### Catalog

PostgreSQL metadata queries for grid data:

```rust
use storage::{Catalog, CatalogEntry};

let catalog = Catalog::connect(&database_url).await?;

// Find grid by model/parameter/time
let entry = catalog.find_grid(
    "gfs",                      // model
    "TMP",                      // parameter  
    "2 m above ground",         // level
    reference_time,             // model run time
    3,                          // forecast hour
).await?;

// List all parameters for a model
let params = catalog.list_parameters("gfs").await?;

// Get latest available run
let latest = catalog.get_latest_run("gfs").await?;

// Insert new entry
let entry = CatalogEntry {
    model: "gfs".to_string(),
    parameter: "TMP".to_string(),
    level: "2 m above ground".to_string(),
    reference_time,
    forecast_hour: 3,
    storage_path: "grids/gfs/20241217_12z/tmp_2_m_above_ground_f003.zarr".to_string(),
    bbox: serde_json::json!({"min_lon": 0, "max_lon": 360, ...}),
    grid_shape: serde_json::json!([1440, 721]),
    zarr_metadata: Some(zarr_meta.to_json()),
    created_at: Utc::now(),
};
catalog.insert(&entry).await?;

// Delete old data (retention)
catalog.delete_before(model, cutoff_time).await?;
```

### CatalogEntry

Structure representing a grid dataset in the catalog:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: Option<i64>,
    pub model: String,
    pub parameter: String,
    pub level: String,
    pub reference_time: DateTime<Utc>,
    pub forecast_hour: i32,
    pub storage_path: String,
    pub bbox: serde_json::Value,
    pub grid_shape: serde_json::Value,
    pub zarr_metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}
```

### TileCache

Redis tile caching with TTL:

```rust
use storage::TileCache;

let cache = TileCache::connect(&redis_url).await?;

// Generate cache key
let cache_key = CacheKey::new(
    "gfs_TMP",           // layer
    "temperature",       // style
    5, 8, 10,           // z, x, y
    Some(reference_time),
    Some(3),             // forecast hour
);

// Get tile from cache
if let Some(tile) = cache.get(&cache_key).await? {
    return Ok(tile);
}

// Set tile with TTL (1 hour)
cache.set(&cache_key, &tile_data, Some(3600)).await?;

// Invalidate tiles for a layer
cache.invalidate_pattern("wms:tile:gfs_TMP:*").await?;

// Get cache stats
let stats = cache.stats().await?;
println!("Keys: {}, Memory: {} MB", stats.keys, stats.memory_mb);
```

### CacheKey

Structured cache key generation:

```rust
use storage::CacheKey;

let key = CacheKey::new(
    "gfs_TMP",           // layer
    "temperature",       // style  
    5, 8, 10,           // z, x, y
    Some(reference_time),
    Some(3),             // forecast hour
);

// Converts to string like:
// "wms:tile:gfs_TMP:temperature:5:8:10:20241217T120000Z:f003"
let key_str = key.to_string();
```

### TileMemoryCache

In-memory LRU cache for hot tiles:

```rust
use storage::TileMemoryCache;

// Create cache with 10k entry capacity (~500 MB at 50KB/tile)
let cache = TileMemoryCache::new(10000);

// Check cache (returns clone of data)
if let Some(tile) = cache.get(&key).await {
    return Ok(tile);
}

// Insert (evicts LRU if full)
cache.set(key, tile_data).await;

// Get statistics
let stats = cache.stats().await;
println!("Entries: {}, Hit rate: {:.1}%", 
    stats.entries, stats.hit_rate() * 100.0);

// Clear cache
cache.clear().await;
```

## Zarr Storage Integration

The storage crate integrates with the `grid-processor` crate for Zarr access:

```rust
use storage::ObjectStorage;
use grid_processor::{ZarrGridProcessor, GridProcessorConfig};

// Create S3-backed storage for Zarr
let storage = ObjectStorage::new(&config)?;
let zarr_store = storage.as_zarr_store("grids/gfs/tmp.zarr")?;

// Open Zarr array
let processor = ZarrGridProcessor::open(
    zarr_store,
    "/",
    GridProcessorConfig::default(),
).await?;

// Read region with automatic pyramid selection
let region = processor.read_region(&bbox, Some((256, 256))).await?;
```

## Database Schema

### grid_catalog table

```sql
CREATE TABLE grid_catalog (
    id SERIAL PRIMARY KEY,
    model VARCHAR(50) NOT NULL,
    parameter VARCHAR(50) NOT NULL,
    level VARCHAR(100) NOT NULL,
    reference_time TIMESTAMPTZ NOT NULL,
    forecast_hour INTEGER NOT NULL,
    storage_path VARCHAR(500) NOT NULL,
    bbox JSONB NOT NULL,
    grid_shape JSONB NOT NULL,
    zarr_metadata JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    
    UNIQUE(model, parameter, level, reference_time, forecast_hour)
);

-- Indexes for common queries
CREATE INDEX idx_catalog_model_param ON grid_catalog(model, parameter);
CREATE INDEX idx_catalog_reference_time ON grid_catalog(reference_time DESC);
CREATE INDEX idx_catalog_created_at ON grid_catalog(created_at DESC);
```

### Common Queries

```sql
-- Find latest data for a layer
SELECT * FROM grid_catalog
WHERE model = 'gfs' AND parameter = 'TMP' AND level = '2 m above ground'
ORDER BY reference_time DESC, forecast_hour ASC
LIMIT 1;

-- List all available parameters
SELECT DISTINCT model, parameter, level 
FROM grid_catalog
ORDER BY model, parameter, level;

-- Get storage usage
SELECT model,
       COUNT(*) as datasets,
       COUNT(DISTINCT parameter) as parameters,
       MAX(created_at) as last_update
FROM grid_catalog
GROUP BY model;

-- Clean up old data
DELETE FROM grid_catalog
WHERE model = 'gfs' 
  AND reference_time < NOW() - INTERVAL '6 hours';
```

## Configuration

```rust
pub struct StorageConfig {
    /// PostgreSQL connection URL
    pub database_url: String,
    
    /// PostgreSQL connection pool size
    pub database_pool_size: u32,
    
    /// MinIO/S3 endpoint
    pub s3_endpoint: String,
    
    /// S3 bucket name
    pub s3_bucket: String,
    
    /// S3 access key
    pub s3_access_key: String,
    
    /// S3 secret key
    pub s3_secret_key: String,
    
    /// Redis URL
    pub redis_url: String,
    
    /// Tile cache TTL in seconds
    pub cache_ttl: u64,
    
    /// Memory cache capacity
    pub memory_cache_size: usize,
}
```

## Error Handling

```rust
pub enum StorageError {
    /// Database error
    DatabaseError(sqlx::Error),
    
    /// S3/MinIO error
    ObjectStorageError(String),
    
    /// Redis error
    CacheError(String),
    
    /// Entry not found
    NotFound { model: String, parameter: String },
    
    /// Serialization error
    SerializationError(String),
}
```

## Performance

| Operation | Time | Notes |
|-----------|------|-------|
| Catalog query (indexed) | 1-5ms | PostgreSQL |
| Catalog insert | 2-10ms | With unique constraint check |
| S3 get object (1MB) | 10-30ms | MinIO local network |
| S3 list objects | 5-20ms | Depends on prefix cardinality |
| Redis get | 1-2ms | Network latency |
| Redis set with TTL | 1-3ms | Includes expiry setup |
| Memory cache hit | <0.1ms | In-process |

## See Also

- [grid-processor](./grid-processor.md) - Zarr reading/writing
- [Architecture: Caching](../architecture/caching.md) - Cache strategy
- [WMS API](../services/wms-api.md) - Uses all storage components
- [Data Flow](../architecture/data-flow.md) - End-to-end pipeline
