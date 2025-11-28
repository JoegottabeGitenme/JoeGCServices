# GOES-19 Temporal Test Scenarios - Ready

## Summary

Successfully created 3 temporal test scenarios for GOES-19 satellite data with 5 time steps at 5-minute intervals.

## Available Data

**Downloaded**: 5 NetCDF files from GOES-19 Band 13 (Clean IR)
- **Product**: ABI-L1b-RadC (Level 1b Radiances, CONUS)
- **Band**: 13 (Clean IR, 10.35 µm)
- **Resolution**: 2 km
- **File Size**: ~2.8 MB each (~14 MB total)
- **Temporal Coverage**: 2025-11-28 15:01:17Z to 15:21:17Z (20 minutes)
- **Update Frequency**: 5-minute intervals
- **Location**: `./data/goes-temporal/band13/`

**Timestamps Extracted**:
```
2025-11-28T15:01:17Z
2025-11-28T15:06:17Z
2025-11-28T15:11:17Z
2025-11-28T15:16:17Z
2025-11-28T15:21:17Z
```

## Test Scenarios Created

### 1. `goes_single_tile_temporal.yaml` - Isolated Temporal Test
**Purpose**: Pure temporal testing without spatial variation.

**Configuration**:
- Duration: 60 seconds
- Concurrency: 5 workers
- Tile: Fixed single tile (zoom 5, central CONUS)
- Time: Sequential through 5 time steps
- Layer: goes19_IR (Clean IR)

**Expected Behavior**:
- First 5 requests: 0% NetCDF cache hit rate (loading files)
- Requests 6+: ~100% cache hit rate (all files in memory)
- Memory: 5 files × 2.8 MB = 14 MB
- Rendered tiles: 5 unique tiles (one per time step)

**Best For**:
- Verifying NetCDF cache works
- Debugging temporal lookup logic
- Measuring pure temporal overhead
- Comparing GOES vs MRMS efficiency

**Run Command**:
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_single_tile_temporal.yaml
```

---

### 2. `goes_temporal_5min.yaml` - Moderate Temporal Stress
**Purpose**: Realistic temporal + spatial testing.

**Configuration**:
- Duration: 120 seconds (2 minutes)
- Concurrency: 15 workers
- Zoom Range: 3-8 (appropriate for 2 km resolution)
- Time: Sequential through 5 time steps
- Coverage: Full CONUS

**Expected Behavior**:
- First pass: 5 NetCDF cache misses
- Subsequent passes: High cache hit rate
- Spatial randomness tests rendered tile cache
- Zoom 3-8 appropriate for GOES resolution

**Best For**:
- Simulating animation playback
- Testing spatial + temporal cache interaction
- Realistic user workload

**Run Command**:
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_temporal_5min.yaml
```

---

### 3. `goes_random_temporal.yaml` - Unpredictable Access
**Purpose**: Worst-case temporal access pattern.

**Configuration**:
- Duration: 90 seconds (1.5 minutes)
- Concurrency: 10 workers
- Zoom Range: 4-7 (focused)
- Time: Random from 5 time steps
- Coverage: Western CONUS

**Expected Behavior**:
- Random time selection per request
- Unpredictable NetCDF file access
- Tests LRU cache robustness
- Should still achieve good hit rate (only 5 files)

**Best For**:
- Testing cache eviction logic
- Simulating time-comparison workflows
- Stress testing with unpredictable access

**Run Command**:
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_random_temporal.yaml
```

---

## Comparison: GOES vs MRMS Test Scenarios

| Aspect | MRMS | GOES-19 |
|--------|------|---------|
| **Time Steps** | 59 files | 5 files |
| **Temporal Range** | ~2 hours | ~20 minutes |
| **Update Interval** | ~2 minutes | ~5 minutes |
| **File Size** | 400 KB | 2.8 MB |
| **Total Cache Size** | 24 MB | 14 MB |
| **Resolution** | 1 km | 2 km |
| **Zoom Range** | 3-13 | 3-8 |
| **Projection** | Lat-Lon | Geostationary |
| **Cache Pressure** | Lower | Higher per file |
| **Test Duration** | Longer animations | Shorter loops |

**Key Insight**: GOES has fewer time steps but larger files, making it a good test for cache efficiency with larger datasets.

---

## Next Steps to Execute Tests

### Prerequisites
1. **Start Services**:
   ```bash
   ./scripts/start.sh
   # Wait for services to be healthy
   docker-compose ps
   ```

2. **Ingest GOES Data**:
   ```bash
   # Ingest the 5 NetCDF files
   for nc_file in ./data/goes-temporal/band13/*.nc; do
     cargo run --package ingester -- --test-file "$nc_file"
   done
   ```

3. **Verify Ingestion**:
   ```bash
   psql -h localhost -U postgres -d weather_data -c \
     "SELECT reference_time, COUNT(*) FROM grib_files \
      WHERE dataset = 'goes19' GROUP BY reference_time \
      ORDER BY reference_time DESC LIMIT 10;"
   ```
   Expected: 5 rows, one per time step.

4. **Configure GOES Layer** (if not already exists):
   ```sql
   INSERT INTO layers (name, title, abstract, source_dataset, variable_name, bbox, styles)
   VALUES (
     'goes19_IR',
     'GOES-19 Clean IR (Band 13)',
     'GOES-19 ABI Band 13: Clean IR window (10.35 µm) for cloud-top temperatures',
     'goes19',
     'Rad',  -- Or appropriate variable name from NetCDF
     ST_MakeEnvelope(-130, 20, -60, 55, 4326),
     ARRAY['goes_ir']
   );
   ```

### Test Execution Order

**Start with the simplest test first:**

1. **Single Tile Test** (easiest to analyze):
   ```bash
   cargo run --package load-test -- run \
     --scenario validation/load-test/scenarios/goes_single_tile_temporal.yaml
   ```
   
   **What to observe**:
   - First 5 requests should be slow (cache misses)
   - Subsequent requests should be fast (cache hits)
   - Latency metrics: p50, p95, p99
   - Cache hit rate approaching 100%

2. **Random Temporal Test** (medium complexity):
   ```bash
   cargo run --package load-test -- run \
     --scenario validation/load-test/scenarios/goes_random_temporal.yaml
   ```
   
   **What to observe**:
   - Cache hit rate should stabilize quickly (only 5 files)
   - Random access shouldn't cause thrashing
   - Compare latencies to sequential test

3. **Full Temporal + Spatial Test**:
   ```bash
   cargo run --package load-test -- run \
     --scenario validation/load-test/scenarios/goes_temporal_5min.yaml
   ```
   
   **What to observe**:
   - Combination of temporal and spatial cache pressure
   - Higher concurrency (15 workers)
   - Rendered tile cache diversity

---

## Monitoring Cache Performance

### Check NetCDF Cache Metrics
```bash
# View cache statistics
curl http://localhost:8080/metrics | grep -E "netcdf_cache|grib_cache"

# Watch in real-time
watch -n 2 'curl -s http://localhost:8080/metrics | grep -E "cache_(hit|miss|size)"'
```

### Monitor Memory Usage
```bash
# Container memory
docker stats wms-api --no-stream

# Expected memory for GOES cache
# 5 files × 2.8 MB = ~14 MB baseline
# Plus rendered tiles (varies by zoom)
```

### Check Request Latencies
```bash
# From load test output
# Look for:
# - p50 (median latency)
# - p95 (95th percentile)
# - p99 (99th percentile)
# - max (worst case)

# Compare:
# - Cold requests (cache miss): Higher latency
# - Warm requests (cache hit): Lower latency
```

---

## Expected Performance Metrics

### Single Tile Test
With 5 files in cache and fixed tile:
- **Cache Hit Rate**: Should approach 100% after warmup
- **Latency (cold)**: Higher for first 5 unique requests
- **Latency (warm)**: Should be consistent and low
- **Memory**: Stable at ~14 MB for NetCDF cache

### Random Temporal Test
With unpredictable time access:
- **Cache Hit Rate**: Should stabilize at 80-95%
- **Latency**: More variable due to occasional misses
- **Cache Eviction**: Should be minimal (5 files fit easily)

### Full Stress Test
With spatial + temporal variation:
- **Cache Hit Rate**: Lower due to tile cache churn
- **Throughput**: Depends on zoom level complexity
- **Memory**: Higher due to rendered tiles

---

## Troubleshooting

### Issue: 500 errors in load test
**Causes**:
- Services not running (`docker-compose ps`)
- GOES data not ingested
- Layer `goes19_IR` doesn't exist
- NetCDF parser doesn't support GOES projection

**Fix**:
1. Verify services: `curl http://localhost:8080/health`
2. Check ingestion: `psql` query above
3. Verify layer: `curl http://localhost:8080/wmts?SERVICE=WMTS&REQUEST=GetCapabilities`

### Issue: All cache misses
**Causes**:
- NetCDF cache not wired into rendering
- Cache size too small
- Cache key mismatch (time format)

**Fix**:
1. Check metrics: `curl http://localhost:8080/metrics | grep cache`
2. Verify cache enabled in config
3. Check logs for cache key debugging

### Issue: High memory usage
**Causes**:
- NetCDF files not being evicted
- Memory leak in parser
- Rendered tiles accumulating

**Fix**:
1. Monitor over time: `docker stats`
2. Check cache size limits
3. Verify LRU eviction working

---

## Success Criteria

✅ **Scenario Validation**:
- [x] 3 GOES scenarios created
- [x] Scenarios load without errors
- [x] TIME parameters generated correctly
- [x] URLs properly formatted

✅ **Data Availability**:
- [x] 5 GOES NetCDF files downloaded
- [x] Timestamps extracted successfully
- [x] Files verified (exist and correct size)

🔲 **Pending Execution**:
- [ ] Ingest GOES data into catalog
- [ ] Configure GOES layer in WMS
- [ ] Run single-tile test
- [ ] Run random temporal test
- [ ] Run full stress test
- [ ] Analyze cache performance
- [ ] Compare with MRMS results

---

## Files Created

**Test Scenarios** (3 files):
1. `validation/load-test/scenarios/goes_single_tile_temporal.yaml`
2. `validation/load-test/scenarios/goes_temporal_5min.yaml`
3. `validation/load-test/scenarios/goes_random_temporal.yaml`

**Scripts** (already created):
- `scripts/download_goes_temporal.sh` - Download GOES data
- `scripts/extract_goes_timestamps.sh` - Extract timestamps (fixed for octal issue)

**Documentation**:
- `GOES_SCENARIOS_READY.md` - This document

---

## Quick Reference

```bash
# Download more GOES data
GOES_HOURS=6 MAX_FILES=50 ./scripts/download_goes_temporal.sh

# Extract timestamps
./scripts/extract_goes_timestamps.sh ./data/goes-temporal/band13

# Ingest data
for f in ./data/goes-temporal/band13/*.nc; do
  cargo run --package ingester -- --test-file "$f"
done

# Run tests (in order)
cargo run --package load-test -- run --scenario \
  validation/load-test/scenarios/goes_single_tile_temporal.yaml

cargo run --package load-test -- run --scenario \
  validation/load-test/scenarios/goes_random_temporal.yaml

cargo run --package load-test -- run --scenario \
  validation/load-test/scenarios/goes_temporal_5min.yaml

# Monitor cache
curl http://localhost:8080/metrics | grep cache
docker stats wms-api --no-stream
```

---

**Status**: GOES temporal test scenarios ready. Waiting for data ingestion and service configuration to execute tests.
