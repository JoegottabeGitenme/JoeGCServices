# Tile Data Caching Analysis

## Overview

This document analyzes how the WMS tile rendering pipeline handles data caching and whether data is reused across multiple tile requests. The goal is to understand if requesting tiles at higher zoom levels efficiently shares underlying data or if each tile request independently fetches data from MinIO.

## Summary

**The system IS designed to cache and reuse data across tile requests**, but there is a gap for GRIB2 files where parsed grid data is not cached.

## Caching Architecture

The system implements a **4-layer caching architecture**:

| Layer | What's Cached | Location | Capacity | TTL | Shared Across Tiles? |
|-------|--------------|----------|----------|-----|---------------------|
| **L1** (TileMemoryCache) | Rendered PNG tiles | `storage/src/tile_memory_cache.rs` | ~10K tiles | 5 min | No - same tile only |
| **L2** (Redis) | Rendered PNG tiles | `storage/src/cache.rs` | Unlimited | 1 hour | No - same tile only |
| **GribCache** | Raw GRIB2/NetCDF bytes | `storage/src/grib_cache.rs` | ~500 files | None | **YES** |
| **GridDataCache** | Parsed `Vec<f32>` grids | `storage/src/grid_cache.rs` | ~100 grids | None | **YES** (NetCDF only) |

## Data Flow

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           HTTP Tile Request                              │
└────────────────────────────────────┬────────────────────────────────────┘
                                     │
┌────────────────────────────────────▼────────────────────────────────────┐
│                    L1 Cache (TileMemoryCache)                            │
│              In-memory LRU, ~10K tiles, 5 min TTL                        │
│                    ⚡ Sub-millisecond access                             │
└────────────────────────────────────┬────────────────────────────────────┘
                             MISS    │    HIT → Return immediately
┌────────────────────────────────────▼────────────────────────────────────┐
│                    L2 Cache (Redis TileCache)                            │
│              Persistent, 1 hour TTL, network latency                     │
└────────────────────────────────────┬────────────────────────────────────┘
                             MISS    │    HIT → Promote to L1, Return
┌────────────────────────────────────▼────────────────────────────────────┐
│                    PostgreSQL Catalog                                    │
│          Find dataset entry by model/param/time/level                    │
└────────────────────────────────────┬────────────────────────────────────┘
                                     │
┌────────────────────────────────────▼────────────────────────────────────┐
│                    GribCache (Raw Data)                                  │
│          In-memory LRU, ~500 files, stores Bytes                         │
│                    Shared across all tiles!                              │
└────────────────────────────────────┬────────────────────────────────────┘
                             MISS    │    HIT → Skip MinIO fetch
┌────────────────────────────────────▼────────────────────────────────────┐
│                    MinIO/S3 Object Storage                               │
│              Loads ENTIRE GRIB2/NetCDF file                              │
└────────────────────────────────────┬────────────────────────────────────┘
                                     │
┌────────────────────────────────────▼────────────────────────────────────┐
│                    GridDataCache (Parsed Grids)                          │
│          For GOES/NetCDF: caches parsed Vec<f32>                         │
│              Avoids repeated parsing (NetCDF only!)                      │
└────────────────────────────────────┬────────────────────────────────────┘
                                     │
┌────────────────────────────────────▼────────────────────────────────────┐
│                    Parse & Resample                                      │
│      GRIB2 → unpack_data() → bilinear interpolation                      │
│      Projection-aware (Lambert, Geostationary, etc)                      │
└────────────────────────────────────┬────────────────────────────────────┘
                                     │
┌────────────────────────────────────▼────────────────────────────────────┐
│                    Render → PNG Encode → Return                          │
└─────────────────────────────────────────────────────────────────────────┘
```

## How Data Sharing Works

### Scenario 1: Adjacent tiles from same GRIB2 file

```
Tile request 1: gfs_TMP at z=4, x=5, y=6, time=0
Tile request 2: gfs_TMP at z=4, x=5, y=7, time=0  (adjacent tile)

Both requests:
1. Query catalog → same catalog entry (same model/param/time)
2. GribCache.get("shredded/gfs/TMP/...") → SHARED raw bytes
3. Parse GRIB2 → ⚠️ DUPLICATED work (not cached for GRIB2)
4. Resample to different bboxes → different work
5. Render → different output
```

### Scenario 2: Same tile requested twice

```
Tile request 1: gfs_TMP at z=4, x=5, y=6
Tile request 2: gfs_TMP at z=4, x=5, y=6 (same tile, 30 seconds later)

Request 2 hits:
1. L1 Cache → HIT (if within 5 min TTL)
   OR
2. L2 Redis Cache → HIT (if L1 miss but within 1 hour TTL)

Zero data loading or rendering needed!
```

### Scenario 3: GOES NetCDF with GridDataCache

```
Tile request 1: goes16_CMI_C13 at z=4, x=5, y=6
Tile request 2: goes16_CMI_C13 at z=4, x=5, y=7 (adjacent)

Both requests:
1. Query catalog → same NetCDF file
2. GribCache.get() → shared raw bytes
3. GridDataCache.get() → HIT for request 2!
   - Parsed Vec<f32> is shared
   - Skips expensive NetCDF parsing
4. Resample to different bboxes
5. Render different output
```

## Previously Identified Gap: GRIB2 Parsing (NOW FIXED)

### Previous Behavior (Before Fix)

Previously, GRIB2 parsing was not cached:

```rust
// Raw bytes ARE cached (good!)
let file_data = grib_cache.get(&entry.storage_path).await?;

// But parsing happened EVERY TIME for GRIB2 (not cached!)
let msg = find_parameter_in_grib(file_data, parameter, ...)?;
let grid_data = msg.unpack_data()?;  // <-- Repeated for every tile!
```

### Current Behavior (After Fix)

Now GRIB2 parsed grids are also cached:

```rust
// Check grid cache first (works for both GRIB2 and NetCDF now!)
if let Some(cached) = cache.get(&cache_key).await {
    metrics.record_data_source_cache_hit(&source_type).await;
    return Ok(cached_grid);  // Cache HIT - skip decompression!
}

// Cache miss - parse and store for next time
let grid_data = msg.unpack_data()?;
cache.insert(cache_key, CachedGridData { ... }).await;
```

### Caching Status by Data Format

| Data Format | Raw Bytes Cached? | Parsed Grid Cached? | Impact |
|-------------|-------------------|---------------------|--------|
| NetCDF (GOES) | ✅ Yes (GribCache) | ✅ Yes (GridDataCache) | Optimal |
| GRIB2 (GFS/HRRR/MRMS) | ✅ Yes (GribCache) | ✅ Yes (GridDataCache) | **Optimal** |

### Performance Impact

GRIB2 `unpack_data()` involves decompression which can be expensive:
- JPEG2000 decompression (common in GFS)
- PNG decompression (some HRRR data)
- Simple packing (faster, but still repeated)

With the fix, a user panning across a map at zoom level 8 will now benefit from cached parsed grids. The first tile pays the decompression cost, and subsequent adjacent tiles reuse the cached grid data.

## Monitoring Cache Behavior

### Prometheus Metrics

Available at `http://localhost:8080/metrics`:

```
# GRIB raw file cache
grib_cache_hits
grib_cache_misses  
grib_cache_hit_rate_percent
grib_cache_size
grib_cache_capacity

# L1 tile cache
tile_memory_cache_hits
tile_memory_cache_misses
tile_memory_cache_hit_rate_percent
tile_memory_cache_evictions_total
tile_memory_cache_size_bytes
```

### JSON Metrics API

Available at `http://localhost:8080/api/metrics`:

```json
{
  "l1_cache": {
    "hits": 1234,
    "misses": 56,
    "hit_rate": 95.7,
    "evictions": 12,
    "size_bytes": 52428800
  },
  "l2_cache": {
    "connected": true,
    "key_count": 500,
    "memory_used": 104857600
  }
}
```

### Log Messages

Watch for these log patterns:

```
# GridDataCache hit for GOES (good - parsed data reused)
Grid cache HIT for GOES data

# GridDataCache hit for GRIB2 (good - parsed data reused)
Grid cache HIT for GRIB2 data

# GridDataCache miss (parsing required)
Loaded NetCDF file from cache/storage (grid cache MISS)

# GRIB2 grid caching
Cached parsed GRIB2 grid data storage_path=shredded/gfs/TMP_2m/...

# GribCache activity
Loading grid data storage_path=shredded/gfs/TMP_2m/...
```

## Configuration

### Environment Variables

```bash
# L1 Tile Memory Cache
ENABLE_L1_CACHE=true
TILE_CACHE_SIZE=10000        # Max entries
TILE_CACHE_TTL_SECS=300      # 5 minutes

# L2 Redis Cache
REDIS_URL=redis://redis:6379
TILE_CACHE_TTL=3600          # 1 hour

# GRIB Raw File Cache
ENABLE_GRIB_CACHE=true
GRIB_CACHE_SIZE=500          # Max files (~2.5GB)

# Grid Data Cache (for NetCDF/GOES and GRIB2)
ENABLE_GRID_CACHE=true
GRID_CACHE_SIZE=100          # Max grids (~1.5GB for GOES CONUS)
ENABLE_GRIB_GRID_CACHE=true  # Also cache parsed GRIB2 grids

# Prefetch (proactive caching)
ENABLE_PREFETCH=true
PREFETCH_RINGS=2             # Render 24 surrounding tiles
PREFETCH_MIN_ZOOM=3
PREFETCH_MAX_ZOOM=12
```

## Implementation Status

### Completed Optimizations

#### 1. Per-Data-Source Parsing Metrics

Added comprehensive metrics to track parsing performance by data source (GFS, HRRR, MRMS, GOES):

- Parse count, cache hits, cache misses per data source
- Average, min, max, and last parse times
- Cache hit rate percentage
- Available in admin dashboard at `/admin.html`
- Available in JSON API at `/api/metrics`

#### 2. Extended GridDataCache to GRIB2

The grid cache now also caches parsed GRIB2 grids (not just NetCDF):

```rust
// Cache key format for GRIB2: "storage_path:level"
let cache_key = format!("{}:{}", entry.storage_path, entry.level);

// Check cache first
if let Some(cached) = cache.get(&cache_key).await {
    metrics.record_data_source_cache_hit(&source_type).await;
    return Ok(cached_grid);
}

// On cache miss, parse and store
let grid_data = msg.unpack_data()?;
cache.insert(cache_key, CachedGridData { ... }).await;
```

#### 3. Configuration Options

New environment variable to control GRIB2 grid caching:

```bash
ENABLE_GRIB_GRID_CACHE=true  # Also cache parsed GRIB2 grids
```

### Memory Considerations

Caching parsed GRIB2 grids increases memory usage:
- GFS global (0.25°): 1440 x 721 = ~4MB per grid
- HRRR CONUS (3km): 1799 x 1059 = ~7.6MB per grid
- With 100 cached grids: ~400-760MB additional RAM

This is worthwhile because it eliminates redundant JPEG2000/PNG decompression when rendering adjacent tiles from the same weather model data.

## Key Files

| Component | File Path |
|-----------|-----------|
| App State & Cache Setup | `services/wms-api/src/state.rs` |
| HTTP Handlers | `services/wms-api/src/handlers.rs` |
| Rendering Logic | `services/wms-api/src/rendering.rs` |
| L1 Tile Cache | `crates/storage/src/tile_memory_cache.rs` |
| L2 Redis Cache | `crates/storage/src/cache.rs` |
| GRIB Raw Cache | `crates/storage/src/grib_cache.rs` |
| Grid Data Cache | `crates/storage/src/grid_cache.rs` |
| MinIO Client | `crates/storage/src/object_store.rs` |

## Conclusion

The caching architecture is well-designed with multiple layers. The main optimization opportunity is extending `GridDataCache` to also cache parsed GRIB2 grids, which would eliminate redundant decompression when rendering adjacent tiles from the same underlying weather model data.
