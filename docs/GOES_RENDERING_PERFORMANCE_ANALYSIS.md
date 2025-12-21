# GOES Rendering Performance Analysis

> **Note (December 2024):** This document describes the architecture before GribCache removal. Data access now uses Zarr storage with chunk-level caching. Some implementation details (e.g., GribCache, grib_cache.rs) are historical.

**Date:** December 4, 2025  
**Purpose:** Deep dive into GOES satellite image rendering performance bottlenecks

## Executive Summary

GOES (Geostationary Operational Environmental Satellite) tile rendering has significant performance concerns, particularly on cache misses. The primary bottlenecks are:

1. **NetCDF Parsing** - Temp file I/O + native library overhead
2. **Geostationary Projection** - Per-pixel coordinate transforms
3. **Memory Allocations** - Large buffers for full grid + resampled data
4. **Cache Miss Latency** - 120-140ms vs 0.8ms cache hit (128x difference)

Current measured performance:
- **Cache hit:** 0.8ms (33,219 req/s)
- **Cache miss:** 120-140ms (259 req/s)

---

## 1. Request Flow Overview

```
HTTP GET /wmts/rest/goes16_CMI_C13/default/WebMercatorQuad/5/8/12.png
    |
    v
+-------------------+
| handlers.rs       |  L1 Cache (TileMemoryCache)
| wmts_get_tile()   |  -----> HIT: Return PNG (sub-ms)
| lines 543-858     |
+-------------------+
    | MISS
    v
+-------------------+
| Redis L2 Cache    |  -----> HIT: Promote to L1, return (1-10ms)
+-------------------+
    | MISS
    v
+-------------------+
| rendering.rs      |
| render_weather_   |
| data_with_time()  |
| lines 132-334     |
+-------------------+
    |
    +---> Catalog lookup (PostgreSQL)
    |
    +---> load_grid_data() --> Grid Cache check
    |           |
    |           +---> MISS: load_netcdf_grid_data()
    |                       |
    |                       +---> GribCache.get() (MinIO/S3)
    |                       +---> netcdf_parser::load_goes_netcdf_from_bytes()
    |                       +---> Insert into Grid Cache
    |
    +---> resample_geostationary_to_mercator_with_proj()
    |           |
    |           +---> For each 256x256 pixel:
    |                   - mercator_y_to_lat()
    |                   - proj.geo_to_grid()
    |                   - bilinear_interpolate()
    |
    +---> render_goes_ir() or render_goes_visible()
    |
    +---> renderer::png::create_png()
    |
    +---> Store in L1 + L2 caches
    |
    v
Return PNG
```

---

## 2. Performance Bottleneck Analysis

### 2.1 NetCDF Parsing (CRITICAL - ~65% of cache miss time)

**Location:** `crates/netcdf-parser/src/lib.rs:531-599`

**Current Implementation:**
```rust
pub fn load_goes_netcdf_from_bytes(data: &[u8]) -> NetCdfResult<...> {
    // PROBLEM 1: Temp file I/O
    let temp_file = temp_dir.join(format!("goes_native_{}.nc", std::process::id()));
    let mut file = std::fs::File::create(&temp_file)?;
    file.write_all(data)?;           // Write ~2.8MB to disk
    drop(file);
    
    // PROBLEM 2: netcdf crate opens file again
    let nc_file = netcdf::open(&temp_file)?;
    
    // ... read data ...
    
    // PROBLEM 3: Cleanup temp file
    let _ = std::fs::remove_file(&temp_file);
}
```

**Problems Identified:**

| Issue | Impact | Measurement |
|-------|--------|-------------|
| Temp file write | Synchronous I/O blocking async runtime | ~5-15ms |
| Temp file read | Double read (write then open) | ~5-10ms |
| File cleanup | Delete operation | ~1-2ms |
| netcdf library | Native C library overhead | ~20-50ms |

**Baseline Measurement:** (from `docs/NETCDF_OPTIMIZATION_BASELINE.md`)
- GRIB/NetCDF Load: **415.5 ms** (65.1% of total)
- GRIB/NetCDF Parse: **221.3 ms** (34.7% of total)

**Why Temp File is Required:**
The `netcdf` Rust crate wraps the C `libnetcdf` library, which requires a file path. The underlying HDF5 library does not support reading from memory buffers directly without custom I/O drivers.

**Potential Optimizations:**

1. **Memory-mapped files:** Use `memfd_create()` on Linux or `/dev/shm/` for temp files
2. **HDF5 virtual file driver:** Implement custom HDF5 VFD for memory buffers
3. **Cache NetCDF file paths:** Keep temp files around, use reference counting
4. **Parallel parsing:** Parse while downloading (stream processing)

---

### 2.2 Geostationary Projection Transforms (MODERATE - ~0.2% but scales with zoom)

**Location:** `services/wms-api/src/rendering.rs:1163-1241`

**Current Implementation:**
```rust
fn resample_geostationary_to_mercator_with_proj(...) -> Vec<f32> {
    let mut output = vec![f32::NAN; output_width * output_height];
    
    // For 256x256 tile = 65,536 iterations
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Mercator to lat/lon conversion
            let merc_y = max_merc_y - y_ratio * (max_merc_y - min_merc_y);
            let lat = mercator_y_to_lat(merc_y);  // trig functions
            
            // Geographic to geostationary grid (EXPENSIVE)
            let grid_coords = proj.geo_to_grid(lat, lon);  // ~15 trig ops
            
            // Bilinear interpolation
            // ... 4 array lookups, 6 multiplications ...
        }
    }
}
```

**`proj.geo_to_grid()` Cost Analysis:**

From `crates/projection/src/geostationary.rs:178-224`:

```rust
pub fn geo_to_scan(&self, lon_deg: f64, lat_deg: f64) -> Option<(f64, f64)> {
    let lat_rad = lat_deg.to_radians();        // 1 trig
    let lon_rad = lon_deg.to_radians();        // 1 trig
    let dlon = lon_rad - self.lambda_0;
    let cos_c = lat_rad.cos() * dlon.cos();    // 2 trig + 1 mul
    let horizon_angle = (self.req / self.h).acos();  // 1 trig
    if cos_c.acos() > horizon_angle { ... }    // 1 trig
    
    let phi_c = ((self.rpol/self.req).powi(2) * lat_rad.tan()).atan();  // 3 trig
    let rc = self.rpol / (1.0 - e2 * phi_c.cos().powi(2)).sqrt();  // 2 trig, 1 sqrt
    
    // 3D coordinate computation
    let sx = self.h - rc * phi_c.cos() * (lon_rad - self.lambda_0).cos();  // 2 trig
    let sy = -rc * phi_c.cos() * (lon_rad - self.lambda_0).sin();          // 2 trig
    let sz = rc * phi_c.sin();                                               // 1 trig
    
    let s_xy = sx.hypot(sy);  // 1 hypot (sqrt)
    let y_rad = sz.atan2(s_xy);   // 1 atan2
    let x_rad = (-sy).atan2(sx);  // 1 atan2
    
    // Total: ~18 trig/sqrt operations per pixel
}
```

**Measured Impact:**
- Resample time: 1.1 ms (0.2% of total for cache miss)
- Per-tile: 65,536 pixels x 18 ops = ~1.2M floating point operations

**At higher zoom levels:**
- Zoom 8: More tiles, same per-tile cost
- Overall impact remains low (~1-2ms per tile)

**Potential Optimizations:**

1. **Lookup table (LUT) caching:** Pre-compute grid indices for common zoom/tile combinations
2. **SIMD vectorization:** Process 4/8 pixels simultaneously with AVX
3. **GPU acceleration:** Move projection to GPU shader
4. **Approximation:** Use simpler projection math at lower zoom levels

---

### 2.3 Memory Allocations (MODERATE)

**Per-Request Allocations:**

| Buffer | Size (GOES CONUS) | Purpose |
|--------|-------------------|---------|
| Raw NetCDF bytes | 2.8 MB | File from S3 |
| Parsed grid `Vec<f32>` | 15 MB (2500x1500x4) | Full resolution data |
| Resampled grid `Vec<f32>` | 262 KB (256x256x4) | Tile-sized data |
| RGBA output `Vec<u8>` | 262 KB (256x256x4) | Colored pixels |
| PNG compressed | ~30 KB | Final output |

**Total per cache miss: ~18 MB allocated + ~16 MB copied**

**Problem:** Every cache miss allocates ~18MB, with most being the full grid that's cached afterward.

**Potential Optimizations:**

1. **Buffer pooling:** Reuse Vec allocations across requests
2. **Streaming decompression:** Decompress directly to cached buffer
3. **Lazy grid loading:** Only load grid region needed for tile
4. **Zero-copy caching:** Use `Arc<[f32]>` instead of cloning

---

### 2.4 GribCache Mutex Contention (MINOR - potential under high load)

**Location:** `crates/storage/src/grib_cache.rs:81`

```rust
pub struct GribCache {
    cache: Arc<Mutex<LruCache<String, Bytes>>>,  // <-- Mutex, not RwLock
}

pub async fn get(&self, key: &str) -> ... {
    let mut cache = self.cache.lock().await;  // Exclusive lock for reads!
}
```

**Problem:** Uses `Mutex` instead of `RwLock`, serializing all cache access including reads.

**Impact:**
- Under low concurrency: Negligible
- Under high concurrency (50+ workers): Can become bottleneck

**Fix:** Change to `RwLock` for read-heavy workload (most accesses are reads).

---

### 2.5 Cache Key Collision Risk (MINOR)

**Location:** `services/wms-api/src/rendering.rs:1543`

```rust
let cache_key = entry.storage_path.clone();  // Just the path
```

**Risk:** If same file path is used for different parameters (unlikely but possible with path reuse), cache could return wrong data.

**Fix:** Include model + parameter in cache key: `format!("{}:{}:{}", model, parameter, storage_path)`

---

## 3. Caching Architecture

### Multi-Level Cache Hierarchy

```
                    ┌─────────────────────────────────────────┐
                    │           Tile Memory Cache (L1)        │
                    │  - 10,000 entries, 300s TTL             │
                    │  - In-memory LRU                        │
                    │  - Access: <0.1ms                       │
                    │  - Key: layer:style:CRS:z_x_y:time      │
                    └─────────────────────────────────────────┘
                                        │ MISS
                                        v
                    ┌─────────────────────────────────────────┐
                    │           Redis Cache (L2)              │
                    │  - Persistent tile storage              │
                    │  - Access: 1-10ms                       │
                    │  - TTL: Configurable                    │
                    └─────────────────────────────────────────┘
                                        │ MISS
                                        v
                    ┌─────────────────────────────────────────┐
                    │           Grid Data Cache               │
                    │  - 100 entries (configurable)           │
                    │  - Parsed f32 arrays                    │
                    │  - ~15MB per GOES grid                  │
                    │  - Access: <0.1ms                       │
                    └─────────────────────────────────────────┘
                                        │ MISS
                                        v
                    ┌─────────────────────────────────────────┐
                    │           GRIB/File Cache               │
                    │  - 500 entries                          │
                    │  - Raw file bytes                       │
                    │  - Access: <1ms (memory) / 50ms (S3)    │
                    └─────────────────────────────────────────┘
                                        │ MISS
                                        v
                    ┌─────────────────────────────────────────┐
                    │           MinIO/S3 Storage              │
                    │  - Access: 50-200ms                     │
                    └─────────────────────────────────────────┘
```

### Cache Effectiveness

| Scenario | Cache Hit Rate | Latency | Throughput |
|----------|---------------|---------|------------|
| Animation playback (same tiles, cycling times) | 100% | 0.1-0.2ms | 33,000 req/s |
| Typical map usage (clustered spatial) | 70-90% | 5-20ms avg | 3,000-10,000 req/s |
| Cold start / random access | 40-60% | 50-80ms avg | 300-500 req/s |
| Worst case (all misses) | 0% | 120-140ms | 70-100 req/s |

---

## 4. Measured Performance Data

### Test 1: 100% Cache Hits (Best Case)

| Metric | Value |
|--------|-------|
| Requests/sec | 33,219 |
| p50 latency | 0.125 ms |
| p99 latency | 0.262 ms |
| Max latency | 5.0 ms |
| Throughput | 789 MB/s |

### Test 2: 51% Cache Hits (Realistic)

| Metric | Value |
|--------|-------|
| Requests/sec | 259 |
| p50 latency | 0.8 ms |
| p90 latency | 123.5 ms |
| p99 latency | 140.9 ms |
| Max latency | 169.9 ms |
| Throughput | 2.0 MB/s |

### Pipeline Timing Breakdown (Cache Miss)

| Stage | Time | % |
|-------|------|---|
| GRIB/NetCDF Load | 415.5 ms | 65.1% |
| GRIB/NetCDF Parse | 221.3 ms | 34.7% |
| Resample | 1.1 ms | 0.2% |
| PNG Encode | 0.1 ms | 0.0% |
| **Total** | **638 ms** | 100% |

---

## 5. Optimization Recommendations

### Priority 1: NetCDF Parsing (High Impact)

**Current:** Temp file + netcdf library = ~600ms  
**Target:** Memory-based parsing = ~50-100ms

**Options:**

1. **Memory-mapped temp file:**
   ```rust
   // Use /dev/shm on Linux for memory-backed file
   let temp_file = Path::new("/dev/shm").join(format!("goes_{}.nc", id));
   ```

2. **Custom HDF5 VFD:**
   - Implement HDF5 Virtual File Driver for memory buffers
   - Requires deep HDF5/netcdf integration

3. **Pre-parse and cache metadata:**
   - Parse projection params once per file
   - Cache as `GoesProjectionParams` struct
   - Only re-read data array on cache miss

### Priority 2: Grid Cache Improvements (Medium Impact)

**Current:** 100-entry cache, separate from tile cache  
**Target:** Larger cache, smarter eviction

1. **Increase cache size:**
   - Current: 100 grids x 15MB = 1.5GB max
   - Proposed: 200 grids x 15MB = 3GB max
   - Memory available: Check deployment constraints

2. **Time-aware eviction:**
   - Keep most recent 2-3 observation times in cache
   - Evict oldest times first (animation typically moves forward)

3. **Predictive prefetch:**
   - When time T is requested, prefetch T+5min
   - Pre-warm cache for common zoom levels

### Priority 3: Projection Optimization (Low Impact, Future)

**Current:** 65,536 x 18 trig ops = ~1ms  
**Target:** <0.5ms via LUT or SIMD

1. **Lookup table for common tiles:**
   ```rust
   // Pre-compute for zoom levels 3-8, cache grid indices
   static GOES16_GRID_LUT: LazyLock<HashMap<TileKey, Vec<(f32, f32)>>> = ...;
   ```

2. **SIMD vectorization:**
   - Process 8 pixels at once with AVX2
   - ~4x theoretical speedup

### Priority 4: Lock Contention (Low Impact)

1. **Change GribCache Mutex to RwLock:**
   ```rust
   cache: Arc<RwLock<LruCache<String, Bytes>>>
   ```

2. **Consider sharded cache:**
   - 8-16 separate LRU caches
   - Hash key to shard
   - Reduces lock contention

---

## 6. Quick Wins (Implement Now)

### 6.1 Use /dev/shm for Temp Files

**File:** `crates/netcdf-parser/src/lib.rs`

```rust
pub fn load_goes_netcdf_from_bytes(data: &[u8]) -> NetCdfResult<...> {
    // Use memory-backed filesystem on Linux
    #[cfg(target_os = "linux")]
    let temp_dir = Path::new("/dev/shm");
    #[cfg(not(target_os = "linux"))]
    let temp_dir = std::env::temp_dir();
    
    let temp_file = temp_dir.join(format!("goes_native_{}.nc", std::process::id()));
    // ... rest unchanged ...
}
```

**Expected improvement:** 5-15ms per parse (disk I/O elimination)

### 6.2 Increase Grid Cache Size

**File:** `services/wms-api/src/main.rs` (or config)

```rust
// Increase from 100 to 200 (requires ~3GB RAM)
let grid_cache = GridDataCache::new(200);
```

**Expected improvement:** Higher hit rate for temporal animations

### 6.3 Add Cache Key Specificity

**File:** `services/wms-api/src/rendering.rs`

```rust
// Before:
let cache_key = entry.storage_path.clone();

// After:
let cache_key = format!("{}:{}:{}", entry.model, entry.parameter, entry.storage_path);
```

**Expected improvement:** Prevents potential cache collisions

---

## 7. Testing Recommendations

### Run Existing Load Tests

```bash
# Cache hit performance (baseline)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_single_tile_temporal.yaml

# Cache miss performance (optimization target)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_random_temporal.yaml

# Realistic usage pattern
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_temporal_5min.yaml
```

### Profiling

```bash
# Generate flamegraph
./scripts/profile_flamegraph.sh 30 goes_random_temporal

# Pipeline timing
./scripts/profile_request_pipeline.sh
```

### Metrics Endpoint

```bash
# Check pipeline timing
curl http://localhost:8080/api/metrics | jq '.pipeline_timing'

# Check cache stats
curl http://localhost:8080/api/metrics | jq '.grid_cache'
```

---

## 8. Conclusion

GOES rendering performance is **cache-bound**. When tiles are cached, performance is excellent (33K req/s). The challenge is minimizing cache miss latency and maximizing cache hit rate.

**Key findings:**
1. NetCDF parsing is 99% of cache miss latency
2. Projection math is well-optimized (~1ms)
3. Cache hierarchy works well but could be tuned

**Recommended next steps:**
1. Implement `/dev/shm` temp file optimization (quick win)
2. Increase grid cache size if memory permits
3. Consider time-aware cache prefetching
4. Long-term: Investigate HDF5 memory VFD or alternative NetCDF libraries

---

## Appendix: Key Files Reference

| File | Purpose | Key Lines |
|------|---------|-----------|
| `services/wms-api/src/handlers.rs` | WMS/WMTS entry points | 543-858 |
| `services/wms-api/src/rendering.rs` | Main rendering logic | 132-334, 1163-1241, 1533-1654 |
| `crates/netcdf-parser/src/lib.rs` | NetCDF parsing | 531-599 |
| `crates/projection/src/geostationary.rs` | GOES projection math | 143-224 |
| `crates/storage/src/grid_cache.rs` | Parsed grid caching | Full file |
| `crates/storage/src/grib_cache.rs` | File byte caching | Full file |
| `config/models/goes16.yaml` | GOES-16 configuration | Full file |
| `config/models/goes18.yaml` | GOES-18 configuration | Full file |
