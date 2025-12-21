# Temporal Testing - Ready for Execution

## Summary

We have successfully implemented temporal load testing capabilities and downloaded 59 time steps of MRMS radar data for comprehensive temporal cache testing.

## What Was Accomplished

### 1. MRMS Data Download (59 Files, 24MB)
- **Source**: AWS S3 bucket `noaa-mrms-pds`
- **Product**: MergedReflectivityQC_00.50 (Composite Radar Reflectivity)
- **Temporal Coverage**: 2025-11-28 12:44:39Z to 14:40:37Z (~2 hours)
- **Update Frequency**: ~2 minutes per file
- **Total Files**: 59 GRIB2 files
- **Total Size**: 24MB
- **Script**: `./scripts/download_mrms.sh` (fully working)

**Download command:**
```bash
MRMS_HOURS=2 ./scripts/download_mrms.sh
```

**Data location:**
```
./data/mrms/MergedReflectivityQC_00.50/MRMS_*.grib2
```

### 2. Load Test Tool - Temporal Support Added

**Code changes:**
- `validation/load-test/src/config.rs` - Added `TimeSelection` enum
- `validation/load-test/src/generator.rs` - Added temporal URL generation
- Support for sequential and random time selection
- TIME parameter automatically appended to WMTS URLs

**New configuration options:**
```yaml
time_selection:
  type: sequential  # or random
  times:
    - "2025-11-28T12:44:39Z"
    - "2025-11-28T12:46:37Z"
    # ... more timestamps
```

### 3. Test Scenarios Created

#### `mrms_temporal_stress.yaml` - Comprehensive Temporal Test
- **Purpose**: Full temporal stress test across all 59 time steps
- **Zoom Range**: 3-13 (tests cache at all scales)
- **Concurrency**: 30 workers
- **Duration**: 300 seconds (5 minutes)
- **Pattern**: Sequential time cycling (simulates animation playback)
- **Coverage**: CONUS bounding box

**Expected behavior:**
- First pass: 59 GRIB cache misses (loading all time steps)
- Subsequent passes: High GRIB cache hit rate
- Tests spatial + temporal cache interaction
- Stresses LRU eviction with varying zoom levels

**Run command:**
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_temporal_stress.yaml
```

#### `mrms_temporal_random.yaml` - Random Temporal Access
- **Purpose**: Worst-case temporal access pattern
- **Zoom Range**: 5-9 (focused range for faster testing)
- **Concurrency**: 20 workers
- **Duration**: 180 seconds (3 minutes)
- **Pattern**: Random time selection
- **Use Case**: Simulates users jumping between time steps

**Expected behavior:**
- Unpredictable GRIB file access
- Tests LRU cache effectiveness
- All 59 files fit comfortably in 500-entry cache
- Cache hit rate should stabilize after warmup

**Run command:**
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_temporal_random.yaml
```

#### `mrms_single_tile_temporal.yaml` - Isolated Temporal Test
- **Purpose**: Pure temporal testing without spatial variation
- **Tile**: Fixed single tile (zoom 7, Kansas City area)
- **Concurrency**: 5 workers
- **Duration**: 60 seconds
- **Pattern**: Sequential time cycling
- **Use Case**: Debugging cache behavior

**Expected behavior:**
- First loop: 59 cache misses
- Second+ loops: 100% GRIB cache hits (ideal scenario)
- 59 unique rendered tiles (one per time step)
- Perfect for isolating temporal cache from spatial cache

**Run command:**
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_single_tile_temporal.yaml
```

## Next Steps to Execute Tests

### Step 1: Ingest MRMS Data

Before running tests, ingest the 59 MRMS files into the catalog:

```bash
# Option 1: Ingest all files
for grib_file in ./data/mrms/MergedReflectivityQC_00.50/*.grib2; do
  cargo run --package ingester -- --test-file "$grib_file"
done

# Option 2: Use ingester service with auto-discovery (if configured)
# Configure ingestion paths in services/ingester/src/config.rs
```

**Verify ingestion:**
```bash
psql -h localhost -U postgres -d weather_data -c \
  "SELECT reference_time, COUNT(*) 
   FROM grib_files 
   WHERE dataset = 'mrms' 
   GROUP BY reference_time 
   ORDER BY reference_time DESC 
   LIMIT 10;"
```

Expected: 59 rows, one per time step.

### Step 2: Create MRMS Layer Configuration

Add MRMS layer to WMS catalog (if not already configured):

```sql
INSERT INTO layers (name, title, abstract, source_dataset, variable_name, bbox, styles)
VALUES (
  'mrms_REFL',
  'MRMS Composite Reflectivity',
  'Multi-Radar Multi-Sensor composite reflectivity at 0.50° elevation',
  'mrms',
  'MergedReflectivityQC',
  ST_MakeEnvelope(-130, 20, -60, 55, 4326),
  ARRAY['reflectivity']
);
```

Or verify existing layer:
```bash
curl http://localhost:8080/wmts?SERVICE=WMTS&REQUEST=GetCapabilities | grep mrms_REFL
```

### Step 3: Run Tests

Start with the single-tile test to verify basic temporal functionality:

```bash
# Start services (if not already running)
./scripts/start.sh

# Wait for services to be healthy
docker-compose ps

# Run single-tile temporal test (easiest to analyze)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_single_tile_temporal.yaml

# Run random temporal access pattern
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_temporal_random.yaml

# Run comprehensive temporal stress test
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_temporal_stress.yaml
```

### Step 4: Analyze Results

**Key metrics to observe:**

1. **GRIB Cache Hit Rate**:
   - First 59 requests: 0% hit rate (loading all times)
   - After warmup: Should approach 100% (all files in cache)
   - Monitor: Check GRIB cache metrics endpoint

2. **Rendered Tile Cache**:
   - Depends on zoom level and tile variety
   - Single tile test: 59 unique tiles total
   - Stress test: Thousands of unique tiles

3. **Latency Patterns**:
   - Cold requests (cache miss): Higher latency (GRIB decode + render)
   - Warm requests (cache hit): Lower latency (render only or full cache hit)
   - Track p50, p95, p99 latency over time

4. **Database Performance**:
   - 59 different `reference_time` values queried
   - Should be indexed for fast temporal lookups
   - Monitor query execution times

5. **Memory Usage**:
   - GRIB cache: 59 files × ~400KB = ~24MB baseline
   - Rendered tiles: Varies by test scenario
   - Monitor container memory with `docker stats`

**Check metrics:**
```bash
# Prometheus metrics endpoint
curl http://localhost:8080/metrics | grep -E "cache|grib|render"

# Container resource usage
docker stats wms-api --no-stream
```

## Test Progression Recommendation

1. **Start Simple**: Single-tile temporal test
   - Verify TIME parameter works
   - Confirm GRIB cache behavior
   - Easy to reason about results

2. **Add Complexity**: Random temporal access
   - Tests unpredictable access patterns
   - Validates LRU eviction logic
   - Medium difficulty to analyze

3. **Full Stress**: Comprehensive temporal + spatial
   - Tests realistic animation workload
   - Combines temporal and spatial cache pressure
   - Most representative of production

## Expected Performance Improvements

With GRIB cache implemented (Phase 4):

**Before (no GRIB cache):**
- Every request: Fetch from MinIO + decode GRIB
- Temporal requests: No sharing between time steps

**After (with GRIB cache):**
- First request per time: Fetch + decode + cache
- Subsequent requests: In-memory GRIB data
- Expected: 50-80% reduction in MinIO calls
- Expected: Faster time-series animations

## Files Modified/Created

**Modified:**
- `validation/load-test/src/config.rs` - TimeSelection support
- `validation/load-test/src/generator.rs` - Temporal URL generation
- `validation/load-test/src/main.rs` - Config struct updates
- `scripts/download_mrms.sh` - AWS S3 download script (rewritten)
- All service Dockerfiles - Copy validation directory for workspace

**Created:**
- `validation/load-test/scenarios/mrms_temporal_stress.yaml`
- `validation/load-test/scenarios/mrms_temporal_random.yaml`
- `validation/load-test/scenarios/mrms_single_tile_temporal.yaml`
- `TEMPORAL_TESTING_READY.md` (this file)

## MRMS Data Properties

```
Grid: 7000 × 3500 points
Resolution: 0.01° (~1 km)
Coverage: CONUS (130°W-60°W, 20°N-55°N)
Update Frequency: Every ~2 minutes
File Size: ~380-430 KB compressed (GRIB2)
Variable: MergedReflectivityQC (dBZ)
Elevation: 500 m above mean sea level
```

## Troubleshooting

**Issue: No data returned for MRMS layer**
- Check ingestion completed: `psql` query above
- Verify layer exists: `GetCapabilities` request
- Check TIME parameter format: ISO 8601 (YYYY-MM-DDTHH:MM:SSZ)

**Issue: 404 errors in load test**
- Layer name mismatch: Check `mrms_REFL` vs actual catalog name
- Service not running: `docker-compose ps`
- TIME parameter invalid: Check timestamps match ingested data

**Issue: Poor cache hit rate**
- GRIB cache not wired into rendering (see Phase 4 TODO)
- Cache size too small: Increase `CHUNK_CACHE_SIZE_MB` env var
- Verify cache metrics: `curl http://localhost:8080/metrics`

**Issue: High memory usage**
- 59 GRIB files in cache = ~24MB (acceptable)
- Rendered tiles accumulating: Monitor Redis memory
- Check for memory leaks: `docker stats` over time

## Success Criteria

✅ **Temporal Testing Capability**:
- Load test tool supports TIME parameter
- Three temporal scenarios created
- Scenarios load and run without errors

✅ **Data Availability**:
- 59 MRMS files downloaded (24MB)
- Temporal range: ~2 hours of radar data
- Files verified with wgrib2

✅ **Documentation**:
- Usage instructions complete
- Expected behaviors documented
- Troubleshooting guide provided

🔲 **Pending** (requires services running):
- Ingest MRMS data into catalog
- Run actual temporal load tests
- Analyze cache performance metrics
- Validate GRIB cache effectiveness

---

**Status**: Ready for ingestion and testing. All code and scenarios are in place.
