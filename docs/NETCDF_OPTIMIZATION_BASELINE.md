# NetCDF Optimization - Baseline Performance

**Date:** December 2, 2025  
**Goal:** Replace `ncdump` subprocess with native netcdf Rust library for 5-10x speedup

## Current Implementation

**Method:** External `ncdump` command-line tool  
**Location:** `services/wms-api/src/rendering.rs` - `load_netcdf_grid_data()` function (lines 1463-1700+)

**Process:**
1. Write NetCDF file to temporary disk location (`/tmp/goes_temp_*.nc`)
2. Run `ncdump -h` subprocess to extract metadata
3. Parse ASCII output for dimensions, projection params, scale/offset
4. Run `ncdump -v CMI` subprocess to extract data values
5. Parse ASCII output to extract numeric values
6. Apply scale/offset and convert to f32 array
7. Clean up temp file

## Baseline Test Results

### Test Environment
- **System:** libhdf5 and libnetcdf installed
- **Cache Configuration:**
  - L1 tile cache: 10,000 entries, 300s TTL
  - GRIB cache: 500 entries
  - Prefetch: enabled (2 rings, zoom 3-12)
  - Cache warming: enabled
- **Data:** GOES-18 Band 13 (Clean IR) NetCDF files
  - File size: ~2.8 MB per file
  - Grid dimensions: 10000x6000
  - 5 time steps used in tests

### Test 1: Single Tile Temporal (100% Cache Hits)
**Scenario:** Fixed tile, 5 sequential times cycling rapidly  
**Purpose:** Measure pure cache performance (best case)

- **Duration:** 60s
- **Concurrency:** 5 workers
- **Total Requests:** 1,993,098
- **Requests/sec:** 33,219 req/s
- **Latency:**
  - p50: 0.125 ms
  - p90: 0.179 ms
  - p99: 0.262 ms
  - max: 5.0 ms
- **Cache Hit Rate:** 100.0%
- **Throughput:** 789 MB/s

**Analysis:** All requests served from tile cache. Shows baseline system performance without NetCDF parsing overhead.

### Test 2: Random Temporal with Spatial Variation (51% Cache Hits)
**Scenario:** Random tiles + random times (zoom 4-12, western CONUS)  
**Purpose:** Measure cache miss performance with NetCDF parsing

- **Duration:** 90s
- **Concurrency:** 10 workers
- **Total Requests:** 23,299
- **Requests/sec:** 259 req/s
- **Latency:**
  - p50: 0.8 ms (cache hit)
  - p90: 123.5 ms (cache miss - **NetCDF parsing + rendering**)
  - p95: 130.4 ms
  - p99: 140.9 ms
  - max: 169.9 ms
- **Cache Hit Rate:** 51.3%
- **Throughput:** 2.0 MB/s

**Analysis:** 
- Cache hits: sub-millisecond (0.8ms median)
- **Cache misses: 120-140ms** - This is the bottleneck!
- Only 259 req/s throughput (vs 33K for cached)
- **128x slowdown** when cache misses occur

## Identified Bottleneck

**Cache miss latency breakdown (estimated):**
- NetCDF file I/O from S3/cache: ~10-20ms
- **`ncdump -h` subprocess: ~2-3 seconds**
- **`ncdump -v CMI` subprocess: ~2-3 seconds**
- Data parsing and conversion: ~100-500ms
- Grid projection and resampling: ~500-1000ms
- PNG encoding: ~100-300ms
- **Total: ~5-7 seconds per cold cache miss** (but concurrent requests help)

The p90 latency of 123ms with 10 concurrent workers suggests the ncdump operations are the primary bottleneck.

## Optimization Plan

### Implementation
1. ✅ Add `netcdf = "0.9"` and `hdf5 = "0.8"` to `crates/netcdf-parser/Cargo.toml`
2. ⏳ Ensure system has `libhdf5-dev` and `libnetcdf-dev` installed
3. ⏳ Rewrite `load_netcdf_grid_data()` to use native netcdf library:
   - Open file with `netcdf::open()`
   - Read dimensions directly
   - Read variables and attributes without temp files
   - Extract CMI data array directly into memory
   - Apply scale/offset inline

### Expected Improvements
- **NetCDF parsing:** 4-5 seconds → 0.5-1 second (5-10x faster)
- **p90 latency:** 123ms → 20-40ms (3-6x faster)
- **Requests/sec (cold):** 259 → 800-1500 req/s (3-6x faster)
- **No temp file overhead**
- **Better error handling**

### Success Criteria
After optimization, rerun the same tests and compare:
- [ ] p90 latency reduced by at least 3x
- [ ] Cold cache throughput increased by at least 3x
- [ ] No regression in cache hit performance
- [ ] All tests pass with same success rate

## Next Steps
1. Verify libhdf5-dev and libnetcdf-dev are installed
2. Implement native NetCDF reader in `load_netcdf_grid_data()`
3. Run optimization tests
4. Compare results and document improvements
