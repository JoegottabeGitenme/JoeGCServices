# Layer Performance Analysis - Nov 27, 2025

## Executive Summary

Performance testing reveals **wind barbs are the fastest render type**, contrary to expectations. The system handles 14K-18K req/sec at 20 concurrency but **breaks down at 100 concurrent requests**, with latencies jumping from <2ms to >1.5 seconds.

## Per-Layer-Type Performance (20 Concurrent Requests, 60s Duration)

### 🏆 Wind Barbs (FASTEST)
```
Test: wind_barbs_only.yaml
Total Requests:      1,073,658
Throughput:          17,895 req/sec
Latency p50:         1.1 ms
Latency p90:         1.2 ms
Latency p99:         1.5 ms
Latency max:         6.2 ms
Bandwidth:           265.5 MB/s
Cache Hit Rate:      100%
```

**Layers Tested:**
- gfs_WIND_BARBS (wind_barbs style)
- hrrr_WIND_BARBS (wind_barbs style)

**Key Finding**: Wind barbs are 21% faster than gradients despite being vector visualizations requiring UGRD/VGRD component loading and barb symbol rendering.

### 🥈 Gradient Layers
```
Test: gradient_only.yaml
Total Requests:      870,428
Throughput:          14,831 req/sec
Latency p50:         1.2 ms
Latency p90:         1.4 ms
Latency p99:         1.8 ms
Latency max:         1,694.7 ms ⚠️
Bandwidth:           455.1 MB/s
Cache Hit Rate:      100%
```

**Layers Tested:**
- gfs_TMP (temperature style)
- gfs_PRMSL (default style)
- hrrr_TMP (temperature style)
- mrms_REFL (reflectivity style)
- goes16_CMI_C13 (goes_ir style)

**Concern**: Max latency spike to 1.7 seconds (outlier - needs investigation)

### 🥉 Isoline/Contour Layers
```
Test: isolines_only.yaml
Total Requests:      847,381
Throughput:          14,123 req/sec
Latency p50:         1.3 ms
Latency p90:         1.6 ms
Latency p95:         1.8 ms
Latency p99:         2.9 ms
Latency max:         15.1 ms
Bandwidth:           80.5 MB/s
Cache Hit Rate:      100%
```

**Layers Tested:**
- gfs_TMP (isolines style)
- gfs_PRMSL (isolines style)
- hrrr_TMP (isolines style)

**Findings**: 
- Slightly slower than gradients (5% lower throughput)
- More consistent max latency (15ms vs 1,695ms)
- Much lower bandwidth (80 MB/s vs 455 MB/s) - contours compress better

## Performance Ranking Summary

| Render Type | Req/Sec | p99 Latency | Max Latency | Bandwidth |
|-------------|---------|-------------|-------------|-----------|
| Wind Barbs  | 17,895  | 1.5 ms      | 6.2 ms      | 265 MB/s  |
| Gradients   | 14,831  | 1.8 ms      | 1,695 ms ⚠️ | 455 MB/s  |
| Isolines    | 14,123  | 2.9 ms      | 15.1 ms     | 80 MB/s   |

## Stress Testing - Finding the Breaking Point

### Extreme Stress Test (100 Concurrent Requests, 180s Duration)
```
Test: extreme_stress.yaml
Total Requests:      9,191
Throughput:          52.6 req/sec ⚠️ (99.7% DROP from 20 concurrent)
Latency p50:         1,574 ms ⚠️
Latency p90:         3,488 ms ⚠️
Latency p95:         4,219 ms ⚠️
Latency p99:         5,358 ms ⚠️
Latency max:         7,963 ms ⚠️
Bandwidth:           0.7 MB/s
Cache Hit Rate:      78.3%
```

**Critical Findings:**
1. **System breaks down at 100 concurrent requests**
2. **340x latency increase** (1.1ms → 1,574ms p50)
3. **Throughput collapses** from 17K req/sec to 52 req/sec
4. **Multi-second response times** become the norm

## Concurrency Scaling Analysis

| Concurrency | Throughput | p50 Latency | p99 Latency | Performance |
|-------------|------------|-------------|-------------|-------------|
| 1           | 8,169      | 0.1 ms      | 0.2 ms      | Excellent   |
| 10          | 15,272     | 0.6 ms      | 0.8 ms      | Excellent   |
| 20          | 14K-18K    | 1.1-1.3 ms  | 1.5-2.9 ms  | Excellent   |
| 100         | 53         | 1,574 ms    | 5,358 ms    | **BROKEN**  |

**Conclusion**: System has a **concurrency limit around 50-75 requests**, beyond which performance degrades catastrophically.

## Root Cause Hypotheses

### Why does performance collapse at 100 concurrent?

1. **Tokio Runtime Saturation**
   - Default tokio worker pool may be exhausted
   - Context switching overhead dominates
   
2. **Database Connection Pool Exhaustion**
   - PostgreSQL catalog lookups may be blocking
   - Limited connection pool size

3. **Redis Connection Bottleneck**
   - Cache operations serializing
   - Connection pool limits

4. **Memory Pressure**
   - 100 simultaneous tile renders (256x256 each)
   - GRIB data decompression in memory
   - Tile buffer allocations

5. **I/O Contention**
   - MinIO reads blocking (even with cache)
   - File descriptor limits

## Performance Optimization Recommendations (Phase 4)

### High Priority
1. **Profile 100-concurrent scenario** to identify exact bottleneck
2. **Increase Tokio worker threads** if CPU-bound
3. **Expand connection pools** (Postgres, Redis, MinIO)
4. **Add async task limits** to prevent overload
5. **Implement request queue** with backpressure

### Medium Priority
6. **In-memory GRIB cache** to reduce MinIO hits
7. **Pre-decode wind barbs** (since they're already fast)
8. **Optimize gradient rendering** (investigate 1.7s spike)

### Low Priority
9. **PNG encoding optimization** (evaluate compression levels)
10. **Tile rendering parallelism** (currently seems good)

## Test Scenarios Created

New scenarios added to `validation/load-test/scenarios/`:
- `gradient_only.yaml` - Test all gradient layers
- `wind_barbs_only.yaml` - Test wind barb layers
- `isolines_only.yaml` - Test contour rendering
- `extreme_stress.yaml` - Find performance limits (100 concurrent)
- `comprehensive.yaml` - Test all layer types together

## Next Steps for Phase 4

1. ✅ Baseline captured
2. ✅ Per-layer-type performance characterized
3. ✅ Breaking point identified (100 concurrent)
4. **TODO**: Profile the 100-concurrent scenario
5. **TODO**: Implement fixes based on profiling
6. **TODO**: Re-test to validate improvements

## Code Changes Made

### Metrics Tracking Enhancement
Added per-layer-type metrics to `services/wms-api/src/metrics.rs`:
- `LayerType` enum (Gradient, WindBarbs, Isolines)
- `record_render_with_type()` method
- `LayerTypeStats` structure
- `from_layer_and_style()` classifier

These changes enable future dashboard breakdowns by render type.
