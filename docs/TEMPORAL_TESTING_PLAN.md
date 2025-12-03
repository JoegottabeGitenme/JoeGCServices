# Temporal Load Testing Plan

## Problem Statement

Current load testing only tests **spatial** tile requests (different x/y/z coordinates) but not **temporal** patterns. Real users loop through time for animation, which creates different cache and performance characteristics.

## Current Limitations

### Load Test Tool
- ✅ Generates random tile coordinates (x, y, z)
- ✅ Selects random layers
- ❌ **Does NOT vary time parameters** (forecast_hour, observation_time)
- ❌ No temporal looping simulation

### Data Availability
**GFS** (Global Forecast System):
- ✅ Single forecast run available
- ❌ Multiple forecast hours not ingested (0, 3, 6, 9, 12... up to 384h)
- **Impact**: Can't test temporal animation for forecasts

**HRRR** (High-Resolution Rapid Refresh):
- ✅ Single analysis available
- ❌ Multiple forecast hours not ingested (0-18h)
- **Impact**: Can't test short-term forecast loops

**MRMS** (Multi-Radar Multi-Sensor):
- ✅ Latest observation available
- ❌ Historical observations not archived (updates every 2 min)
- **Impact**: Can't test radar animation playback

**GOES** (Satellite):
- ❓ Single image available
- ❌ Time series not implemented
- **Impact**: Can't test satellite loops

## Real-World Usage Patterns

### Pattern 1: Forecast Animation (GFS/HRRR)
```
User wants to see 48-hour temperature forecast
- Loop through: f000, f003, f006, f009... f048
- Same tile (x, y, z), different forecast_hour
- Cache behavior: First loop = misses, repeat loops = hits
```

### Pattern 2: Radar Loop (MRMS)
```
User wants last 30 minutes of radar
- Loop through: Now, Now-2min, Now-4min... Now-30min
- Same tile (x, y, z), different observation_time
- Cache behavior: Recent data likely cached, older may be evicted
```

### Pattern 3: Satellite Animation (GOES)
```
User wants last 3 hours of IR imagery
- Loop through: recent 36 frames (5-min intervals)
- Same tile (x, y, z), different scan times
- Cache behavior: Limited by GRIB cache size
```

## Required Enhancements

### Phase 1: Data Ingestion (Priority: HIGH)

#### GFS Multi-Hour Download
Update `scripts/download_gfs.sh`:
```bash
# Current: Downloads only f000 (analysis)
# Needed: Download f000, f003, f006, ... f048 (or more)

FORECAST_HOURS="0 3 6 9 12 15 18 21 24 27 30 33 36 39 42 45 48"
for fhour in $FORECAST_HOURS; do
    fhour_padded=$(printf "%03d" $fhour)
    url="https://nomads.ncep.noaa.gov/.../gfs.${cycle}/atmos/gfs.t${cycle}z.pgrb2.0p25.f${fhour_padded}"
    # Download and extract needed parameters
done
```

**Estimated Impact**:
- Disk space: ~500MB per forecast hour × 17 hours = ~8.5GB
- Ingest time: ~5-10 minutes total
- Worth it: YES - enables forecast animation testing

#### HRRR Multi-Hour Download  
Similar approach for HRRR:
```bash
FORECAST_HOURS="0 1 2 3 4 5 6 7 8 9 10 11 12"
# HRRR updates hourly, forecasts out to 18h
```

**Estimated Impact**:
- Disk space: ~200MB per hour × 13 hours = ~2.6GB
- Worth it: YES - tests high-resolution short-term loops

#### MRMS Time Series (CHALLENGING)
MRMS historical data not easily accessible via HTTP:

**Option A**: Poll and archive latest (cron job)
```bash
# Run every 2 minutes
*/2 * * * * /scripts/download_mrms.sh >> /var/log/mrms_archive.log
# Keep last 30 minutes (15 files per product)
```

**Option B**: Use NOMADS archive (if available)
- Check: https://nomads.ncep.noaa.gov/pub/data/nccf/radar/mosaic/
- May be delayed or incomplete

**Option C**: Simulate with synthetic timestamps (testing only)
- Copy latest file with different observation_time metadata
- Good enough for cache behavior testing

### Phase 2: Load Test Enhancement (Priority: MEDIUM)

Add temporal parameters to test config:

```yaml
name: forecast_animation
description: Simulate 48-hour forecast loop
base_url: http://localhost:8080
duration_secs: 120
concurrency: 20

layers:
  - name: gfs_TMP
    style: temperature
    weight: 1.0

tile_selection:
  type: fixed  # Same tile for all requests
  zoom: 5
  tile_x: 10
  tile_y: 12

# NEW: Temporal selection
time_selection:
  type: sequential_loop
  forecast_hours: [0, 3, 6, 9, 12, 15, 18, 21, 24]  # 9 frames
  loop_count: 10  # Replay 10 times during test
  
# This will generate URLs like:
# /wmts?...&TIME=2024-11-27T00:00:00Z  (f000)
# /wmts?...&TIME=2024-11-27T03:00:00Z  (f003)
# ... loop back to f000 ...
```

**Implementation in load-test**:
```rust
// In generator.rs
pub enum TimeSelection {
    None,  // Current behavior
    SequentialLoop {
        forecast_hours: Vec<u32>,
        current_index: usize,
    },
    RandomRange {
        forecast_hour_range: (u32, u32),
    },
}

impl TileGenerator {
    pub fn next_url(&mut self) -> String {
        let (layer, z, x, y) = self.select_tile();
        let time_param = self.select_time();  // NEW
        
        format!(
            "{}/wmts?...&TIME={}",
            self.config.base_url,
            time_param
        )
    }
}
```

### Phase 3: New Test Scenarios (Priority: MEDIUM)

Create specialized temporal scenarios:

**1. `forecast_loop_cold_cache.yaml`**
```yaml
# Simulate first-time forecast animation view
# Expected: 100% cache miss on first loop
# Expected: 100% cache hit on subsequent loops (if cache holds all frames)
```

**2. `forecast_loop_warm_cache.yaml`**
```yaml
# Simulate repeated forecast viewing
# Multiple users viewing same time range
# Expected: High cache hit rate from start
```

**3. `radar_recent_30min.yaml`**
```yaml
# Simulate radar loop (last 30 minutes)
# 15 frames × 2-minute intervals
# Expected: Recent frames cached, older frames may be evicted
```

**4. `temporal_cache_stress.yaml`**
```yaml
# Test GRIB cache under temporal load
# Multiple users looping through different time ranges
# Expected: High GRIB cache eviction rate, test LRU behavior
```

## Cache Behavior Analysis

### Current System (No Temporal Variation)
```
Request: /wmts?LAYER=gfs_TMP&Z=5&X=10&Y=12
Cache key: gfs_TMP:5:10:12:default_time

Result: 100% hit rate after first request
```

### With Temporal Variation
```
Request 1: /wmts?LAYER=gfs_TMP&Z=5&X=10&Y=12&TIME=...f000
Request 2: /wmts?LAYER=gfs_TMP&Z=5&X=10&Y=12&TIME=...f003
...
Request 9: /wmts?LAYER=gfs_TMP&Z=5&X=10&Y=12&TIME=...f024

Cache keys: 
- gfs_TMP:5:10:12:f000
- gfs_TMP:5:10:12:f003
- ...
- gfs_TMP:5:10:12:f024

Result: 9 unique cache entries, first loop = 9 misses
Repeat loop: 9 hits (if all fit in cache)
```

### GRIB Cache Impact
```
Without temporal data:
- 1 GRIB file per layer
- GRIB cache: Low churn, high hit rate

With temporal data (48h forecast):
- 17 GRIB files per layer (f000, f003, ... f048)
- Multiple concurrent users looping
- GRIB cache: High churn with 500-entry limit
- Eviction analysis: LRU vs FIFO vs time-aware
```

## Metrics to Track

### New Metrics Needed
1. **Cache hit rate by time dimension**
   - Hit rate for f000 vs f048
   - Hit rate for recent vs old observations
   
2. **GRIB cache churn**
   - Evictions per minute under temporal load
   - Average GRIB age at eviction
   
3. **Temporal request patterns**
   - Most common time ranges requested
   - Sequential vs random time access
   
4. **Animation playback performance**
   - Average latency for 24-frame loop
   - Variability between first and subsequent loops

## Implementation Priority

### Must Have (Phase 5)
1. ✅ **GFS multi-hour download script** - Enables testing
2. ✅ **Update ingester to handle forecast_hour** - May already work
3. ✅ **Basic temporal test scenario** - Document expected behavior

### Should Have (Phase 6)
4. **Load test tool temporal support** - Sequential time looping
5. **HRRR multi-hour download** - Short-range forecast testing
6. **Temporal metrics tracking** - Measure cache by time

### Nice to Have (Future)
7. **MRMS archival system** - Radar animation testing
8. **GOES time series** - Satellite loops
9. **Adaptive GRIB cache** - Time-aware eviction policy

## Quick Start for Testing

### Minimal Viable Temporal Test
Even without load test tool changes, you can test manually:

```bash
# 1. Download multiple GFS forecast hours (manual)
./scripts/download_gfs.sh  # Modify to download f000, f003, f006

# 2. Ingest all hours
cargo run --package ingester -- --test-file data/gfs/gfs_f000_TMP.grib2
cargo run --package ingester -- --test-file data/gfs/gfs_f003_TMP.grib2
cargo run --package ingester -- --test-file data/gfs/gfs_f006_TMP.grib2

# 3. Manual temporal requests
for fhour in 0 3 6; do
  curl "http://localhost:8080/wmts?...&forecast_hour=$fhour" -o tile_f${fhour}.png
done

# 4. Check cache behavior
curl http://localhost:8080/api/metrics | jq '.cache'

# 5. Repeat requests - should see cache hits
for fhour in 0 3 6; do
  curl "http://localhost:8080/wmts?...&forecast_hour=$fhour" -o tile_f${fhour}_2.png
done
```

## Success Criteria

Temporal testing is successful when:
1. ✅ Can request same tile at different forecast hours
2. ✅ Each time gets unique cache key
3. ✅ First loop shows cache misses
4. ✅ Second loop shows cache hits (if all fit)
5. ✅ GRIB cache shows expected eviction under load
6. ✅ Load test can simulate animation playback patterns

## Conclusion

**Immediate Action**: 
- Update GFS download to fetch multiple forecast hours
- Test with manual requests
- Document cache behavior

**Next Phase**:
- Enhance load test tool for temporal patterns
- Create temporal-specific test scenarios  
- Measure and optimize time-dimension caching

This will reveal whether our GRIB cache size (500 entries) is adequate for multi-user temporal animation workloads.
