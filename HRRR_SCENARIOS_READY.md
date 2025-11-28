# HRRR Temporal Test Scenarios - Ready

## Summary

Successfully created 4 temporal test scenarios for HRRR (High-Resolution Rapid Refresh) forecast model data with 9 GRIB2 files covering 3 model cycles and 3 forecast hours each.

## Available Data

**Downloaded**: 9 GRIB2 files from HRRR Surface Forecasts
- **Product**: wrfsfcf (Surface forecast files)
- **Resolution**: 3 km
- **File Size**: ~130-145 MB per file
- **Total Size**: 1.3 GB
- **Model Cycles**: 09Z, 10Z, 11Z (2025-11-28)
- **Forecast Hours**: +0h, +1h, +2h per cycle
- **Coverage**: CONUS
- **Location**: `./data/hrrr-temporal/`

**Time Structure**:
```
Cycle 09Z:
  - 09:00Z (+0h analysis)
  - 10:00Z (+1h forecast)
  - 11:00Z (+2h forecast)

Cycle 10Z:
  - 10:00Z (+0h analysis)
  - 11:00Z (+1h forecast)
  - 12:00Z (+2h forecast)

Cycle 11Z:
  - 11:00Z (+0h analysis)
  - 12:00Z (+1h forecast)
  - 13:00Z (+2h forecast)
```

**Unique Valid Times**: 5 (09:00Z, 10:00Z, 11:00Z, 12:00Z, 13:00Z)
**Total Files**: 9 (some valid times available from multiple cycles)

## HRRR Temporal Dimensions

HRRR has **two time dimensions**:

### 1. Reference Time (Model Cycle)
- When the forecast was run
- Updates: Hourly (00Z, 01Z, ..., 23Z)
- Our data: 09Z, 10Z, 11Z

### 2. Valid Time (Forecast Hour)
- When the forecast is valid for
- Range: +0h to +18h (48h for main cycles)
- Our data: +0h, +1h, +2h

### Example:
- **Reference**: 2025-11-28T11:00:00Z (11Z cycle)
- **Forecast**: +2h
- **Valid**: 2025-11-28T13:00:00Z

## Test Scenarios Created

### 1. `hrrr_single_tile_temporal.yaml` - Isolated Test
**Purpose**: Pure temporal testing with fixed tile, single model cycle.

**Configuration**:
- Duration: 60 seconds
- Concurrency: 5 workers
- Tile: Fixed (zoom 6, Kansas City)
- Model Cycle: 11Z only
- Times: 3 valid times (11:00Z, 12:00Z, 13:00Z)
- Layer: hrrr_TMP

**Expected Behavior**:
- First 3 requests: 0% GRIB cache hit (loading 3 files)
- Requests 4+: ~100% hit rate
- Memory: 3 files × 135 MB = ~405 MB
- **Key insight**: Tests large file cache (vs 24MB for MRMS)

**Best For**:
- Debugging HRRR GRIB2 cache
- Measuring large file overhead
- Establishing HRRR performance baseline
- Comparing to MRMS/GOES efficiency

**Run Command**:
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/hrrr_single_tile_temporal.yaml
```

---

### 2. `hrrr_forecast_animation.yaml` - Forecast Playback
**Purpose**: Animating through forecast hours from a single model run.

**Configuration**:
- Duration: 90 seconds
- Concurrency: 10 workers
- Zoom Range: 4-9
- Model Cycle: 11Z only
- Times: 3 forecast hours (f00, f01, f02)
- Layers: hrrr_TMP, hrrr_WIND

**Expected Behavior**:
- Simulates watching forecast animation
- Sequential through +0h → +1h → +2h
- Cache: 3 files × 135 MB = 405 MB
- Tests spatial + temporal variation

**Best For**:
- Forecast briefing workflows
- Short-term forecast animation
- Nowcast to forecast transition

**Run Command**:
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/hrrr_forecast_animation.yaml
```

---

### 3. `hrrr_multi_cycle.yaml` - Model Cycle Comparison
**Purpose**: Comparing analysis times across different model runs.

**Configuration**:
- Duration: 120 seconds
- Concurrency: 15 workers
- Zoom Range: 5-8
- Model Cycles: 09Z, 10Z, 11Z (all at +0h)
- Times: 3 analysis times
- Layer: hrrr_TMP

**Expected Behavior**:
- Cycles through different reference times
- Each cycle = independent model run
- Cache: 3 files × 130 MB = 390 MB
- Tests reference time lookups

**Best For**:
- Model consistency checking
- Nowcast updates
- Model verification workflows

**Run Command**:
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/hrrr_multi_cycle.yaml
```

---

### 4. `hrrr_comprehensive_temporal.yaml` - Full Coverage
**Purpose**: All available times - maximum cache pressure test.

**Configuration**:
- Duration: 180 seconds (3 minutes)
- Concurrency: 20 workers
- Zoom Range: 4-10
- Times: All 5 unique valid times
- Layers: hrrr_TMP, hrrr_WIND, hrrr_REFC

**Expected Behavior**:
- Up to 9 GRIB files loaded
- **Cache pressure: ~1.2 GB** (largest of all scenarios!)
- Tests cache under maximum HRRR load
- May exceed default cache sizes

**Best For**:
- Stress testing cache limits
- Multi-panel forecaster workstations
- Cache sizing decisions
- Performance tuning

**Run Command**:
```bash
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/hrrr_comprehensive_temporal.yaml
```

---

## Comparison: MRMS vs GOES vs HRRR

| Aspect | MRMS | GOES | HRRR |
|--------|------|------|------|
| **Data Type** | Radar | Satellite | Model |
| **Files** | 59 | 5 | 9 |
| **File Size** | 400 KB | 2.8 MB | 135 MB |
| **Total Cache** | 24 MB | 14 MB | **1.2 GB** |
| **Resolution** | 1 km | 2 km | 3 km |
| **Update** | ~2 min | ~5 min | Hourly |
| **Temporal Type** | Observation | Observation | Forecast |
| **Time Dimensions** | 1 (obs time) | 1 (scan time) | **2 (ref + valid)** |
| **Zoom Range** | 3-13 | 3-8 | 4-10 |
| **Cache Pressure** | Low | Low | **Very High** |
| **Best Test For** | Animation | Satellite | Large files |

**Key Insight**: HRRR is the ultimate cache stress test - 50x larger files than MRMS!

---

## Next Steps to Execute Tests

### Prerequisites

1. **Start Services**:
   ```bash
   ./scripts/start.sh
   docker-compose ps  # Wait for healthy
   ```

2. **Ingest HRRR Data**:
   ```bash
   # Ingest all 9 GRIB2 files
   for grib_file in ./data/hrrr-temporal/*/*/*Z/*.grib2; do
     cargo run --package ingester -- --test-file "$grib_file"
   done
   ```

3. **Verify Ingestion**:
   ```bash
   psql -h localhost -U postgres -d weather_data -c \
     "SELECT reference_time, forecast_time, COUNT(*) \
      FROM grib_files \
      WHERE dataset = 'hrrr' \
      GROUP BY reference_time, forecast_time \
      ORDER BY reference_time DESC, forecast_time ASC \
      LIMIT 20;"
   ```
   Expected: 9 rows (3 cycles × 3 forecast hours)

4. **Configure HRRR Layers** (if not exists):
   ```sql
   -- Temperature
   INSERT INTO layers (name, title, abstract, source_dataset, variable_name, bbox, styles)
   VALUES (
     'hrrr_TMP',
     'HRRR Temperature',
     'HRRR 2m Temperature forecast',
     'hrrr',
     'TMP',
     ST_MakeEnvelope(-130, 20, -60, 55, 4326),
     ARRAY['temperature']
   );
   
   -- Wind
   INSERT INTO layers (name, title, abstract, source_dataset, variable_name, bbox, styles)
   VALUES (
     'hrrr_WIND',
     'HRRR Wind',
     'HRRR 10m Wind forecast',
     'hrrr',
     'WIND',
     ST_MakeEnvelope(-130, 20, -60, 55, 4326),
     ARRAY['wind']
   );
   ```

### Test Execution Order

**Recommended progression:**

1. **Single Tile** (simplest - pure temporal):
   ```bash
   cargo run --package load-test -- run \
     --scenario validation/load-test/scenarios/hrrr_single_tile_temporal.yaml
   ```
   
   **What to observe**:
   - First 3 requests: Slow (loading 135 MB files)
   - Subsequent: Fast (cache hits)
   - Compare load time to MRMS (400 KB files)

2. **Forecast Animation** (medium complexity):
   ```bash
   cargo run --package load-test -- run \
     --scenario validation/load-test/scenarios/hrrr_forecast_animation.yaml
   ```
   
   **What to observe**:
   - Sequential forecast hour access
   - Spatial + temporal cache interaction
   - Realistic forecast workflow

3. **Multi-Cycle** (different temporal pattern):
   ```bash
   cargo run --package load-test -- run \
     --scenario validation/load-test/scenarios/hrrr_multi_cycle.yaml
   ```
   
   **What to observe**:
   - Reference time vs valid time caching
   - Model comparison workflows

4. **Comprehensive** (maximum stress):
   ```bash
   cargo run --package load-test -- run \
     --scenario validation/load-test/scenarios/hrrr_comprehensive_temporal.yaml
   ```
   
   **What to observe**:
   - Cache pressure at 1.2 GB
   - Cache eviction behavior
   - Performance degradation (if any)
   - Memory usage trends

---

## Monitoring & Analysis

### Check Cache Metrics
```bash
# GRIB cache stats
curl http://localhost:8080/metrics | grep -E "grib_cache"

# Watch real-time
watch -n 2 'curl -s http://localhost:8080/metrics | grep -E "cache_(hit|miss|size|evict)"'
```

### Monitor Memory Usage
```bash
# Container memory
docker stats wms-api --no-stream

# Expected for HRRR:
# Single tile test: ~405 MB (3 files)
# Comprehensive: ~1.2 GB (9 files)
# Plus rendered tiles
# Plus service overhead
```

### Analyze Performance
Key metrics to track:
- **Load Time**: First request latency (cache miss)
- **Hit Time**: Subsequent request latency (cache hit)
- **Cache Hit Rate**: Should stabilize at 80-100%
- **Memory Growth**: Should plateau after warmup
- **Eviction Rate**: Should be low (all files fit)

### Compare to MRMS/GOES
Expected comparisons:
- **HRRR load time**: Higher (135 MB vs 400 KB)
- **HRRR hit rate**: Similar (if cache sized correctly)
- **HRRR memory**: Much higher (1.2 GB vs 24 MB)
- **HRRR throughput**: May be lower (larger files)

---

## Cache Sizing Recommendations

### Current Setup (from Phase 4)
- **GRIB Cache Size**: 500 entries
- **Sufficient for MRMS**: Yes (59 files)
- **Sufficient for GOES**: Yes (5 files)
- **Sufficient for HRRR**: Yes (9 files), but...

### HRRR-Specific Considerations

**Memory Pressure**:
```
MRMS:  59 files × 400 KB  = 24 MB
GOES:   5 files × 2.8 MB  = 14 MB
HRRR:   9 files × 135 MB  = 1.2 GB  ← 50x larger!
Mixed: 59 + 5 + 9 = 73 files, ~1.24 GB
```

**Recommendations**:
1. **Separate cache tiers**: Consider separate caches for HRRR vs MRMS/GOES
2. **Memory limits**: Set max cache memory (e.g., 2 GB) not just entry count
3. **Eviction policy**: LRU is good, but consider:
   - Priority by dataset (keep MRMS/GOES longer)
   - Time-based eviction (older forecasts less useful)
   - Size-aware eviction (evict large HRRR files first)

4. **Cache configuration**:
   ```yaml
   GRIB_CACHE_SIZE: "500"           # Entry count
   GRIB_CACHE_MAX_MB: "2048"        # Max memory (2 GB)
   GRIB_CACHE_HRRR_PRIORITY: "low"  # Evict HRRR first
   ```

---

## Troubleshooting

### Issue: Out of memory errors
**Cause**: HRRR files too large for cache
**Fix**:
1. Reduce concurrent tests
2. Increase Docker memory limits
3. Implement memory-based cache limits
4. Use smaller test scenarios first

### Issue: Slow performance
**Cause**: Loading 135 MB files takes time
**Fix**:
1. Verify SSD storage for MinIO
2. Check network between WMS and MinIO
3. Monitor GRIB decode time
4. Consider file format optimization

### Issue: Cache thrashing
**Cause**: Too many HRRR files for cache size
**Fix**:
1. Increase cache size
2. Reduce test concurrency
3. Use sequential scenarios instead of random
4. Implement smarter eviction

---

## Success Criteria

✅ **Scenarios Created**:
- [x] 4 HRRR temporal scenarios
- [x] Scenarios load without errors
- [x] TIME parameters generated correctly
- [x] Both reference time and valid time handled

✅ **Data Available**:
- [x] 9 GRIB2 files downloaded (1.3 GB)
- [x] 3 model cycles covered
- [x] 3 forecast hours per cycle
- [x] Files verified (exist and correct size)

✅ **Infrastructure**:
- [x] Download script working
- [x] Timestamp extraction working
- [x] Test scenarios validated
- [x] Documentation complete

🔲 **Pending Execution**:
- [ ] Ingest HRRR data
- [ ] Configure HRRR layers
- [ ] Run single-tile test
- [ ] Run forecast animation test
- [ ] Run multi-cycle test
- [ ] Run comprehensive test
- [ ] Analyze cache performance
- [ ] Compare with MRMS/GOES

---

## Files Created

**Test Scenarios** (4 files):
1. `validation/load-test/scenarios/hrrr_single_tile_temporal.yaml`
2. `validation/load-test/scenarios/hrrr_forecast_animation.yaml`
3. `validation/load-test/scenarios/hrrr_multi_cycle.yaml`
4. `validation/load-test/scenarios/hrrr_comprehensive_temporal.yaml`

**Scripts**:
- `scripts/download_hrrr_temporal.sh` - Download HRRR temporal data
- `scripts/extract_hrrr_timestamps.sh` - Extract reference/valid times

**Documentation**:
- `HRRR_SCENARIOS_READY.md` - This document

---

## Quick Reference

```bash
# Download more HRRR data
MAX_CYCLES=5 FORECAST_HOURS="0 1 2 3 6" ./scripts/download_hrrr_temporal.sh

# Extract timestamps
./scripts/extract_hrrr_timestamps.sh ./data/hrrr-temporal

# Ingest data
for f in ./data/hrrr-temporal/*/*/*Z/*.grib2; do
  cargo run --package ingester -- --test-file "$f"
done

# Run tests (in order of complexity)
cargo run --package load-test -- run --scenario \
  validation/load-test/scenarios/hrrr_single_tile_temporal.yaml

cargo run --package load-test -- run --scenario \
  validation/load-test/scenarios/hrrr_forecast_animation.yaml

cargo run --package load-test -- run --scenario \
  validation/load-test/scenarios/hrrr_multi_cycle.yaml

cargo run --package load-test -- run --scenario \
  validation/load-test/scenarios/hrrr_comprehensive_temporal.yaml

# Monitor
curl http://localhost:8080/metrics | grep grib_cache
docker stats wms-api --no-stream
```

---

**Status**: HRRR temporal test scenarios ready. 9 GRIB2 files (1.3 GB) downloaded and ready for ingestion. Cache stress test ready!
