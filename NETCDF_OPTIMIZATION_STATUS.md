# NetCDF Optimization - Status Update

**Date:** December 2, 2025

## Summary of Work Completed

### ✅ Load Test Infrastructure Improvements
1. **Dynamic Time Queries** - Load tests now query available times from WMS GetCapabilities instead of using hardcoded timestamps
   - Added `TimeSelection::QuerySequential` and `TimeSelection::QueryRandom` variants
   - Created `wms_client.rs` module with proper XML parsing using `quick-xml`
   - Updated `TileGenerator` with async constructor
   - Modified all 3 GOES scenario files to use dynamic queries

2. **Baseline Performance Measurements** - Documented current NetCDF parsing performance
   - Test 1 (100% cache hits): 33,219 req/s, 0.26ms p99
   - Test 2 (51% cache hits): 259 req/s, 140ms p99
   - **Identified bottleneck: 120-140ms latency on cache misses due to ncdump subprocess**

### ✅ Dependencies Added
- `netcdf = "0.9"` added to `crates/netcdf-parser/Cargo.toml`  
- `quick-xml` added to load-test for proper XML parsing
- System libraries verified: `libhdf5-dev 1.14.6`, `libnetcdf-dev 4.9.3`

### ✅ Documentation
- Created `NETCDF_OPTIMIZATION_BASELINE.md` with detailed baseline metrics
- Created `TODO.md` entry for fixing remaining hardcoded timestamps in scenarios
- Updated `validation/load-test/results/runs.jsonl` with baseline results

## 🚧 Blocking Issue: HDF5 Version Incompatibility

**Problem:**  
The `netcdf` Rust crate (v0.9) depends on `hdf5-sys` v0.8.1, which only supports HDF5 versions up to 1.10.x. The system has HDF5 1.14.6 installed.

**Error:**
```
thread 'main' panicked at hdf5-sys-0.8.1/build.rs:200:21:
Invalid H5_VERSION: "1.14.6"
```

**Impact:**  
Cannot proceed with native NetCDF library implementation until this is resolved.

## Alternative Solutions

### Option 1: Downgrade System HDF5 (NOT RECOMMENDED)
- Downgrade from HDF5 1.14.6 to 1.10.x  
- **Cons:** May break other system dependencies, not forward-compatible

### Option 2: Wait for hdf5-sys Update
- Check if newer versions of `hdf5-sys` or `netcdf` crates support HDF5 1.14
- **Status:** `hdf5-metno` v0.11.0 exists as an alternative fork
- **Action:** Try using `hdf5-metno` instead

### Option 3: Optimize ncdump Approach (INTERIM SOLUTION)
Instead of replacing ncdump entirely, optimize the current approach:
1. **Remove temp file I/O** - Parse NetCDF directly from memory using custom ncdump wrapper
2. **Parallel processing** - Run `ncdump -h` and `ncdump -v` concurrently
3. **Binary output** - Use `ncdump -b` for faster data extraction
4. **Incremental parsing** - Stream parse ncdump output instead of waiting for full completion

**Expected improvement:** 2-3x faster (not 5-10x like native library)

### Option 4: Use Rasterio/GDAL Bindings
- Use `gdal` Rust crate which has mature HDF5 support
- GDAL can read NetCDF files natively
- **Pros:** Battle-tested, handles many formats
- **Cons:** Heavier dependency, more complex API

## Recommended Next Steps

1. **Try hdf5-metno fork** (15 min)
   ```toml
   netcdf = { version = "0.9", features = [] }
   hdf5 = { package = "hdf5-metno", version = "0.11" }
   ```

2. **If that fails, implement Interim Solution (Option 3)** (1-2 hours)
   - Remove temp file creation
   - Use `std::process::Stdio::piped()` to stream ncdump output
   - Parse incrementally without waiting for full output

3. **Long-term: Monitor netcdf/hdf5 crate updates** (ongoing)
   - Watch for HDF5 1.14 support in upstream crates
   - Migrate when available

## Performance Targets (Unchanged)

**Current (ncdump subprocess):**
- Cold cache miss: 120-140ms p90/p99
- Throughput: 259 req/s @ 51% hit rate

**Target (after optimization):**
- Cold cache miss: 20-40ms p90/p99 (3-6x improvement)
- Throughput: 800-1500 req/s @ 51% hit rate (3-6x improvement)

## Files Modified

- `crates/netcdf-parser/Cargo.toml` - Added netcdf dependency
- `validation/load-test/src/config.rs` - Added QuerySequential/QueryRandom time selection
- `validation/load-test/src/wms_client.rs` - New WMS GetCapabilities client
- `validation/load-test/src/generator.rs` - Async constructor for dynamic times
- `validation/load-test/src/runner.rs` - Updated to use async generator
- `validation/load-test/scenarios/*.yaml` - Updated GOES scenarios to use dynamic times
- `validation/load-test/results/runs.jsonl` - Baseline test results
- `TODO.md` - Added entry for remaining scenario updates

## Next Session Goals

1. Resolve HDF5 compatibility issue
2. Implement optimized NetCDF loading (native library or improved ncdump)
3. Run post-optimization tests
4. Compare performance improvements
5. Document results and close out optimization task
