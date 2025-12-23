# HRRR Rendering Performance Analysis

> **Note (December 2024):** This document describes the architecture before GribCache removal. Data access now uses Zarr storage with chunk-level caching. Some implementation details (e.g., GribCache, GRIB_CACHE_SIZE) are historical.

## Executive Summary

HRRR (High-Resolution Rapid Refresh) is a **3km resolution regional forecast model** covering CONUS with **1,905,141 grid points** (1799 x 1059). While current performance is excellent for cached requests (**12,778 req/s**), HRRR presents unique caching challenges due to:

1. **Large file sizes**: 130-150 MB per forecast file (vs 400 KB for MRMS, 2.8 MB for GOES)
2. **Lambert Conformal projection**: Requires per-pixel coordinate transformation
3. **High cache memory pressure**: A single 3-hour forecast animation needs ~400 MB cache

**Key Finding**: HRRR performance is excellent when cached, but the current cache configuration assumes ~5 MB average file size. HRRR's 130 MB files mean the cache fills much faster than expected, leading to premature evictions under mixed workloads.

---

## Grid Specifications

### HRRR Grid Characteristics

| Property | Value | Notes |
|----------|-------|-------|
| **Model ID** | `hrrr` | High-Resolution Rapid Refresh |
| **Grid Size** | 1799 x 1059 | ~1.9 million points |
| **Resolution** | 3 km | Highest resolution NWP model |
| **Coverage** | CONUS | Continental United States |
| **Projection** | Lambert Conformal Conic | Requires coordinate transformation |
| **Update Frequency** | Hourly | 24 cycles/day |
| **Forecast Range** | 0-48 hours | Extended forecasts available |
| **File Size** | 130-150 MB | Per shredded parameter file |
| **Memory per Grid** | 7.6 MB | 1,905,141 x 4 bytes |

### Projection Parameters (from `config/models/hrrr.yaml`)

```yaml
projection_params:
  lat1: 21.138123       # First grid point latitude
  lon1: -122.719528     # First grid point longitude  
  lov: -97.5            # Central meridian
  latin1: 38.5          # Standard parallel 1
  latin2: 38.5          # Standard parallel 2 (tangent cone)
  dx: 3000.0            # X spacing (meters)
  dy: 3000.0            # Y spacing (meters)
```

### Model Comparison

| Model | Resolution | Grid Size | Total Points | File Size | Memory/Grid | Projection |
|-------|-----------|-----------|--------------|-----------|-------------|------------|
| **HRRR** | 3 km | 1799x1059 | **1,905,141** | **130-150 MB** | 7.6 MB | Lambert Conformal |
| GFS | 25 km | 1440x721 | 1,038,240 | 50-80 MB | 4.0 MB | Lat/Lon (trivial) |
| MRMS | 1 km | 7000x3500 | 24,500,000 | ~400 KB | 98 MB | Lat/Lon (trivial) |
| GOES-16 | 2 km | ~2500x1500 | ~3,750,000 | ~2.8 MB | 15 MB | Geostationary |

**Key Insight**: HRRR has the largest raw file size but moderate grid memory footprint. The 130 MB file size is the primary cache pressure driver.

---

## Current Performance Characteristics

### Load Test Results (from `docs/BASELINE_METRICS.md`)

**HRRR Single Tile Temporal Test (30s, 5 concurrent):**

| Metric | Value | Assessment |
|--------|-------|------------|
| Total Requests | 383,346 | Excellent |
| Success Rate | 100.0% | Perfect |
| Requests/sec | **12,778 req/s** | Outstanding |
| Latency p50 | 0.4 ms | Excellent |
| Latency p90 | 0.4 ms | Excellent |
| Latency p99 | 0.7 ms | Excellent |
| Latency max | 6.0 ms | Good |
| Cache Hit Rate | 100.0% | After warmup |
| Throughput | 622.6 MB/s | Outstanding |

### Pipeline Stage Breakdown (Cache Miss)

Based on profiling data and architecture analysis:

| Stage | Estimated Time | % of Total | Notes |
|-------|---------------|------------|-------|
| **Storage I/O** | ~800-1200 ms | 65-70% | 130 MB from MinIO |
| **GRIB2 Parse** | ~300-400 ms | 25-30% | Decompress + decode |
| **Lambert Resample** | ~2-5 ms | <1% | Per-pixel transform |
| **PNG Encode** | ~0.1-0.5 ms | <1% | Final tile output |
| **Total Render** | ~1100-1600 ms | 100% | First request |

**Note**: HRRR cache miss latency is higher than MRMS/GOES due to larger file size. After cache warm-up, performance is excellent.

---

## Caching Architecture Analysis

### Current Cache Configuration

```rust
// From services/wms-api/src/state.rs

// GRIB file cache (raw bytes)
chunk_cache_enabled: parse_bool("ENABLE_CHUNK_CACHE", true),
chunk_cache_size_mb: parse_usize("CHUNK_CACHE_SIZE_MB", 500),  // Entry count

// Grid data cache (parsed f32 arrays)  
grid_cache_enabled: parse_bool("ENABLE_GRID_CACHE", true),
grid_cache_size: parse_usize("GRID_CACHE_SIZE", 100),  // Entry count
```

### Cache Memory Analysis

**GRIB File Cache (500 entries default):**

| Model | Files Cached | Memory Used | % of "Expected" 2.5GB |
|-------|-------------|-------------|----------------------|
| MRMS | 500 | 200 MB | 8% |
| GOES | 500 | 1.4 GB | 56% |
| GFS | 500 | 2.5 GB | 100% |
| **HRRR** | **19** | **2.5 GB** | **100%** (only 19 files!) |

**Problem**: With default settings, only **19 HRRR files** fit in the 2.5 GB expected cache footprint, but the cache allows 500 entries. Under HRRR-heavy workloads, memory could balloon to **65 GB** (500 x 130 MB).

**Grid Data Cache (100 entries default):**

| Model | Grids Cached | Memory Used | Notes |
|-------|-------------|-------------|-------|
| MRMS | 100 | 9.8 GB | Huge grids (98 MB each) |
| GOES | 100 | 1.5 GB | Reasonable |
| GFS | 100 | 400 MB | Efficient |
| **HRRR** | **100** | **760 MB** | Reasonable |

**Good News**: HRRR grid cache memory is reasonable at 7.6 MB per grid.

### Cache Key Structure

```rust
// GRIB file cache key
"shredded/hrrr/20251204/TMP_2m/f000.grib2"
"shredded/hrrr/20251204/TMP_2m/f001.grib2"

// Grid data cache key (includes level)
"shredded/hrrr/20251204/TMP_2m/f000.grib2:2 m above ground"
```

---

## HRRR-Specific Code Paths

### Lambert Conformal Projection (`crates/projection/src/lambert.rs`)

HRRR uses Lambert Conformal Conic projection, requiring trigonometric transforms for each pixel:

```rust
impl LambertConformal {
    pub fn hrrr() -> Self {
        Self::from_grib2(
            21.138123,      // lat1 (first grid point)
            -122.719528,    // lon1 
            -97.5,          // LoV (central meridian)
            38.5,           // latin1 (standard parallel)
            38.5,           // latin2 (tangent cone)
            3000.0,         // dx (meters)
            3000.0,         // dy (meters)
            1799,           // nx
            1059,           // ny
        )
    }
    
    pub fn geo_to_grid(&self, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
        // Complex spherical trigonometry:
        // 1. Convert lat/lon to radians
        // 2. Compute cone constant (n)
        // 3. Compute rho (radial distance from pole)
        // 4. Compute theta (angle from central meridian)
        // 5. Transform to grid indices (i, j)
    }
}
```

### Resampling Path (`services/wms-api/src/rendering.rs`)

```rust
fn resample_grid_for_bbox_with_proj(..., model: &str, ...) {
    match model {
        "hrrr" => {
            if use_mercator {
                resample_lambert_to_mercator(data, ...)
            } else {
                resample_lambert_to_geographic(data, ...)
            }
        }
        // GFS uses simple lat/lon resampling
        // GOES uses geostationary projection
    }
}
```

### Projection Overhead Analysis

For a 256x256 tile:
- **65,536 coordinate transformations** per tile
- Each transformation: 2-4 trig operations (sin, cos, atan2)
- Estimated overhead: **2-5 ms per tile** (cache miss scenario)

This is small compared to I/O but adds up under high concurrency.

---

## Bottleneck Analysis

### Primary Bottleneck: Storage I/O (65-70%)

**Root Cause**: HRRR files are 130-150 MB, requiring significant network transfer from MinIO.

**Impact**:
- First request to a new forecast hour: 800-1200 ms just for I/O
- Network bandwidth becomes limiting under concurrent cache misses
- MinIO connection overhead (TCP handshake, TLS) per request

**Mitigation Opportunities**:
1. **Memory-based cache limits** - Prevent cache thrashing
2. **Predictive prefetching** - Load next forecast hour proactively
3. **Connection pooling** - Reduce per-request overhead

### Secondary Bottleneck: GRIB2 Parsing (25-30%)

**Root Cause**: GRIB2 files use PNG/JPEG2000 compression requiring CPU-intensive decompression.

**Impact**:
- 1.9 million values to decompress and decode
- Single-threaded decompression in grib2-parser

**Mitigation Opportunities**:
1. **Grid cache is critical** - Already implemented, avoids re-parsing
2. **Increase grid cache capacity** for forecast animations
3. **Consider parallel decompression** (future enhancement)

### Minor Bottleneck: Lambert Projection (<1%)

**Root Cause**: Per-pixel coordinate transformation.

**Impact**: 2-5 ms per tile, negligible for most workloads.

**Mitigation Opportunities**:
1. Pre-compute lookup tables for common zoom levels
2. SIMD vectorization of projection math
3. Tile-level projection result caching

---

## Optimization Recommendations

### High Priority: Memory-Aware GRIB Cache

**Problem**: Current cache limits by entry count, not memory. HRRR's 130 MB files can cause memory explosion.

**Current Configuration**:
```yaml
CHUNK_CACHE_SIZE_MB: 500  # Entry count only
# With all HRRR: 500 x 130 MB = 65 GB (!)
```

**Recommended Implementation**:

```rust
// Enhanced GribCache with memory limits
pub struct GribCache {
    cache: Arc<RwLock<LruCache<String, Bytes>>>,
    storage: Arc<ObjectStorage>,
    stats: Arc<RwLock<CacheStats>>,
    capacity: usize,
    max_memory_bytes: usize,  // NEW: Memory limit
    current_memory_bytes: AtomicUsize,  // NEW: Current usage
}

impl GribCache {
    pub fn new_with_memory_limit(
        capacity: usize, 
        max_memory_mb: usize,
        storage: Arc<ObjectStorage>
    ) -> Self {
        // ...
    }
    
    async fn maybe_evict_for_memory(&self, incoming_size: usize) {
        // Evict entries until we have room for the new entry
        while self.current_memory_bytes.load(Ordering::Relaxed) + incoming_size 
              > self.max_memory_bytes {
            if let Some((_, evicted)) = self.cache.write().await.pop_lru() {
                self.current_memory_bytes.fetch_sub(evicted.len(), Ordering::Relaxed);
            } else {
                break;
            }
        }
    }
}
```

**Configuration**:
```yaml
CHUNK_CACHE_SIZE_MB: 500           # Max entries
GRIB_CACHE_MAX_MB: 3072        # 3 GB memory limit
```

### High Priority: Increase Grid Cache for HRRR Workloads

**Problem**: Default 100 entries may be insufficient for HRRR forecast animations.

**Scenario**: 24-hour forecast animation with 3 layers
- 24 forecast hours x 3 layers = 72 grid entries
- Plus other models in mixed workloads

**Recommendation**:
```yaml
# For HRRR-heavy workloads
GRID_CACHE_SIZE: 200  # Up from 100

# Memory impact: 200 x 7.6 MB = 1.5 GB for HRRR grids
# This is reasonable and provides good animation support
```

### Medium Priority: Model-Specific Cache Sizing

**Concept**: Different models have different cache characteristics. Consider model-aware eviction.

```rust
// Future: Model-aware cache eviction
fn eviction_priority(key: &str) -> u8 {
    if key.contains("/hrrr/") {
        2  // Lower priority (evict first) - large files
    } else if key.contains("/mrms/") {
        0  // High priority (keep) - small files, frequent updates
    } else {
        1  // Normal priority
    }
}
```

### Medium Priority: Predictive Prefetching for Forecast Animation

**Observation**: Users often request sequential forecast hours (f000, f001, f002...).

**Implementation**:
```rust
// After serving f000, prefetch f001 in background
async fn prefetch_next_forecast_hour(
    cache: &GribCache,
    current_path: &str,
) {
    if let Some(next_path) = increment_forecast_hour(current_path) {
        // Fire-and-forget background fetch
        tokio::spawn(async move {
            let _ = cache.get(&next_path).await;
        });
    }
}
```

### Low Priority: Lambert Projection Lookup Tables

**Concept**: Pre-compute grid-to-geographic transforms for common tile coordinates.

**Memory Cost**: 256x256 x 2 x 8 bytes = 1 MB per zoom level/region combo

**Benefit**: Eliminates trig operations for cached transforms

---

## Cache Sizing Recommendations

### Development Environment

```yaml
# Low memory, single-model testing
CHUNK_CACHE_SIZE_MB: 50
GRIB_CACHE_MAX_MB: 512
GRID_CACHE_SIZE: 50
```

### Production - Standard Workload

```yaml
# Mixed model usage, moderate memory
CHUNK_CACHE_SIZE_MB: 500
GRIB_CACHE_MAX_MB: 3072   # 3 GB
GRID_CACHE_SIZE: 150
```

### Production - HRRR-Heavy Workload

```yaml
# Forecast animation, high memory available
CHUNK_CACHE_SIZE_MB: 500
GRIB_CACHE_MAX_MB: 6144   # 6 GB
GRID_CACHE_SIZE: 250
```

### Expected Memory Footprint by Configuration

| Config | GRIB Cache | Grid Cache | Total |
|--------|-----------|------------|-------|
| Development | 512 MB | 400 MB | ~1 GB |
| Standard | 3 GB | 1.2 GB | ~4.2 GB |
| HRRR-Heavy | 6 GB | 2 GB | ~8 GB |

---

## HRRR Load Test Scenarios

Four specialized scenarios in `validation/load-test/scenarios/`:

### 1. `hrrr_single_tile_temporal.yaml`
- **Purpose**: Baseline temporal caching test
- **Config**: Fixed tile (Kansas City Z6), 3 forecast hours
- **Cache Need**: ~405 MB (3 x 135 MB)

### 2. `hrrr_forecast_animation.yaml`
- **Purpose**: Forecast playback simulation
- **Config**: Z4-9 tiles, 3 forecast hours
- **Cache Need**: ~405 MB

### 3. `hrrr_multi_cycle.yaml`
- **Purpose**: Cross-cycle comparison
- **Config**: 3 model cycles (09Z, 10Z, 11Z) at +0h
- **Cache Need**: ~405 MB

### 4. `hrrr_comprehensive_temporal.yaml`
- **Purpose**: Maximum stress test
- **Config**: 9 time combinations, 3 layers
- **Cache Need**: ~1.2 GB

---

## Implementation Checklist

### Phase 1: Configuration Updates (Low Risk)

- [ ] Increase default `GRID_CACHE_SIZE` from 100 to 150
- [ ] Document HRRR memory requirements in env configuration
- [ ] Add HRRR-specific cache sizing examples to docs

### Phase 2: Memory-Aware GRIB Cache (Medium Risk)

- [ ] Add `GRIB_CACHE_MAX_MB` environment variable
- [ ] Implement memory tracking in `GribCache`
- [ ] Add memory-based eviction logic
- [ ] Update cache stats to include memory usage
- [ ] Add memory metrics to `/metrics` endpoint

### Phase 3: Advanced Optimizations (Future)

- [ ] Predictive forecast hour prefetching
- [ ] Model-aware eviction priorities
- [ ] Lambert projection lookup table caching
- [ ] Parallel GRIB2 decompression

---

## Summary

| Aspect | Current Status | After Phase 1 | After Phase 2 |
|--------|---------------|---------------|---------------|
| Cache Hit Performance | 12,778 req/s | Same | Same |
| Cache Miss Latency | 1100-1600 ms | Same | Same |
| Memory Predictability | Poor | Better | Excellent |
| HRRR Animation Support | Limited | Good | Excellent |
| Mixed Workload Stability | Fair | Good | Excellent |

**Bottom Line**: HRRR rendering performance is already excellent when cached. The main improvement opportunity is **memory-aware caching** to prevent cache thrashing under mixed workloads and support longer forecast animations without memory explosion.

---

## Files Referenced

- `config/models/hrrr.yaml` - HRRR model configuration
- `services/wms-api/src/state.rs` - Cache initialization (lines 79-88, 158-161)
- `services/wms-api/src/rendering.rs` - HRRR rendering paths
- `crates/projection/src/lambert.rs` - Lambert Conformal projection
- `crates/storage/src/grib_cache.rs` - GRIB file cache implementation
- `crates/storage/src/grid_cache.rs` - Parsed grid cache implementation
- `docs/BASELINE_METRICS.md` - Performance baselines
- `docs/HRRR_SCENARIOS_READY.md` - Load test scenarios
- `validation/load-test/scenarios/hrrr_*.yaml` - Load test configurations
