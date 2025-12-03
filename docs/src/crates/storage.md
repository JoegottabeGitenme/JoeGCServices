# storage

Unified storage abstractions for PostgreSQL, MinIO/S3, and Redis.

## Overview

**Location**: `crates/storage/`  
**Dependencies**: `sqlx`, `aws-sdk-s3`, `redis`  
**LOC**: ~1,500

## Components

### ObjectStorage

MinIO/S3 operations:

```rust
use storage::ObjectStorage;

let storage = ObjectStorage::new(&config)?;

// Put object
storage.put_object("gfs/TMP_2m/2024120300_f000_shard_0000.bin", data).await?;

// Get object
let data = storage.get_object("gfs/TMP_2m/...").await?;

// List objects
let keys = storage.list_objects("gfs/TMP_2m/").await?;
```

### Catalog

PostgreSQL metadata queries:

```rust
use storage::Catalog;

let catalog = Catalog::connect(&database_url).await?;

// Find grid
let grid = catalog.find_grid("gfs", "TMP_2m", time).await?;

// List parameters
let params = catalog.list_parameters("gfs").await?;
```

### TileCache

Redis tile caching:

```rust
use storage::TileCache;

let cache = TileCache::connect(&redis_url).await?;

// Get tile
if let Some(tile) = cache.get(&cache_key).await? {
    return Ok(tile);
}

// Set tile with TTL
cache.set(&cache_key, &tile_data, Some(3600)).await?;
```

### TileMemoryCache

In-memory LRU cache:

```rust
use storage::TileMemoryCache;

let cache = TileMemoryCache::new(10000);  // 10k entries

// Check cache
if let Some(tile) = cache.get(&key).await {
    return Ok(tile);
}

// Insert
cache.set(key, tile_data).await;
```

## See Also

- [Architecture: Caching](../architecture/caching.md) - Cache strategy
- [WMS API](../services/wms-api.md) - Uses all storage components
