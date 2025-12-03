# Rendering Pipeline Profiling - Implementation

## Overview

Comprehensive profiling has been added to the Weather WMS rendering pipeline to identify performance bottlenecks. This implements the metrics defined in Phase 4.1 of the performance optimization plan.

## Metrics Tracked

### Request-Level Metrics
- `request_latency_ms` - Total end-to-end request time
- `cache_lookup_time_ms` - Redis cache check duration
- `cache_hit_rate` - Percentage of requests served from cache
- `tiles_per_second` - Overall throughput

### Rendering Pipeline Breakdown
- `grib_load_time_ms` - Time to load GRIB/NetCDF from MinIO/Object Storage
- `grib_parse_time_ms` - Time to parse and decompress GRIB2/NetCDF
- `resample_time_ms` - Grid resampling and projection time
- `png_encode_time_ms` - PNG encoding duration
- `render_time_ms` - Total rendering time (all above combined)

### Per-Layer-Type Metrics
- Gradient layers (temperature, pressure, satellite, radar)
- Wind barb layers (vector rendering)
- Isoline layers (contour rendering)

## Implementation Details

### 1. Enhanced MetricsCollector

**File**: `services/wms-api/src/metrics.rs`

Added new timing collectors:
```rust
pub struct MetricsCollector {
    // ... existing fields ...
    
    // Pipeline timing stats
    grib_load_times: RwLock<TimingStats>,
    grib_parse_times: RwLock<TimingStats>,
    resample_times: RwLock<TimingStats>,
    png_encode_times: RwLock<TimingStats>,
    cache_lookup_times: RwLock<TimingStats>,
}
```

New recording methods:
- `record_grib_load(duration_us)` - Track file loading from storage
- `record_grib_parse(duration_us)` - Track GRIB2 parsing/decompression
- `record_resample(duration_us)` - Track grid resampling
- `record_png_encode(duration_us)` - Track PNG encoding
- `record_cache_lookup(duration_us)` - Track cache operations

### 2. Rendering Pipeline Instrumentation

**File**: `services/wms-api/src/rendering.rs`

Added tracing spans at each pipeline stage:

```rust
pub async fn render_weather_data_with_time(...) -> Result<Vec<u8>, String> {
    let _span = tracing::info_span!("render_weather_data",
        model = model,
        parameter = parameter,
        width = width,
        height = height,
        has_bbox = bbox.is_some()
    ).entered();
    
    // Catalog lookup
    let entry = {
        let _lookup_span = tracing::info_span!("catalog_lookup").entered();
        // ... catalog query ...
    };
    
    // Load grid data (GRIB/NetCDF)
    let grid_result = {
        let _load_span = tracing::info_span!("load_grid_data",
            storage_path = %entry.storage_path,
            file_size_estimate = "unknown"
        ).entered();
        load_grid_data(storage, &entry, parameter).await?
    };
    
    // Resample grid
    let resampled_data = {
        let _span = tracing::info_span!("resample_grid",
            from_width = grid_width,
            from_height = grid_height,
            to_width = rendered_width,
            to_height = rendered_height,
            has_bbox = bbox.is_some(),
            use_mercator = use_mercator
        ).entered();
        // ... resampling logic ...
    };
    
    // Apply color rendering
    let rgba_data = {
        let _span = tracing::info_span!("apply_color_rendering",
            parameter = parameter,
            style = style_name.unwrap_or("auto")
        ).entered();
        // ... color application ...
    };
    
    // PNG encoding
    let png = {
        let _span = tracing::info_span!("png_encode",
            width = rendered_width,
            height = rendered_height,
            rgba_bytes = rgba_data.len()
        ).entered();
        renderer::png::create_png(&rgba_data, rendered_width, rendered_height)?
    };
    
    Ok(png)
}
```

### 3. Storage and Parsing Instrumentation

**File**: `services/wms-api/src/rendering.rs` (load_grid_data function)

```rust
async fn load_grid_data(...) -> Result<GridData, String> {
    // Track file load from storage
    let file_data = {
        let _span = tracing::debug_span!("storage_get",
            path = %entry.storage_path
        ).entered();
        storage.get(&entry.storage_path).await?
    };
    
    // Track GRIB parsing
    let msg = {
        let _span = tracing::debug_span!("grib_parse",
            file_size = file_data.len(),
            is_shredded = is_shredded
        ).entered();
        // ... GRIB parsing ...
    };
}
```

### 4. Metrics Snapshot

Enhanced `MetricsSnapshot` to include pipeline breakdown:

```rust
pub struct MetricsSnapshot {
    // ... existing fields ...
    
    // Pipeline timing breakdown
    pub grib_load_avg_ms: f64,
    pub grib_load_last_ms: f64,
    pub grib_load_count: u64,
    
    pub grib_parse_avg_ms: f64,
    pub grib_parse_last_ms: f64,
    pub grib_parse_count: u64,
    
    pub resample_avg_ms: f64,
    pub resample_last_ms: f64,
    pub resample_count: u64,
    
    pub png_encode_avg_ms: f64,
    pub png_encode_last_ms: f64,
    pub png_encode_count: u64,
    
    pub cache_lookup_avg_ms: f64,
    pub cache_lookup_last_ms: f64,
    pub cache_lookup_count: u64,
}
```

## Metrics Endpoints

### Prometheus Metrics
**Endpoint**: `GET /metrics`

Histograms available:
- `render_duration_ms` - Overall render time
- `render_duration_by_type_ms{layer_type="gradient|wind_barbs|isolines"}` - By layer type
- `grib_load_duration_ms` - File loading time
- `grib_parse_duration_ms` - GRIB parsing time
- `resample_duration_ms` - Resampling time
- `png_encode_duration_ms` - PNG encoding time
- `cache_lookup_duration_ms` - Cache operations
- `minio_read_duration_ms` - MinIO read operations

Counters available:
- `renders_total` - Total renders
- `renders_by_type_total{layer_type="..."}` - By layer type
- `cache_hits_total` - Cache hits
- `cache_misses_total` - Cache misses
- `minio_reads_total` - MinIO reads

### JSON Metrics
**Endpoint**: `GET /api/metrics`

Returns detailed JSON with:
```json
{
  "uptime_secs": 3600,
  "renders_total": 10000,
  "render_avg_ms": 45.2,
  "render_min_ms": 12.3,
  "render_max_ms": 234.5,
  "cache_hit_rate": 85.5,
  
  "grib_load_avg_ms": 15.2,
  "grib_parse_avg_ms": 8.3,
  "resample_avg_ms": 12.1,
  "png_encode_avg_ms": 9.6,
  "cache_lookup_avg_ms": 0.5,
  
  "layer_type_stats": {
    "Gradient": {
      "count": 8000,
      "avg_ms": 42.1,
      "min_ms": 11.2,
      "max_ms": 201.3
    },
    "WindBarbs": {
      "count": 1500,
      "avg_ms": 65.4,
      "min_ms": 35.2,
      "max_ms": 312.7
    },
    "Isolines": {
      "count": 500,
      "avg_ms": 52.3,
      "min_ms": 25.6,
      "max_ms": 189.4
    }
  }
}
```

## Usage

### View Metrics in Real-Time
```bash
# Watch Prometheus metrics
watch -n 1 'curl -s http://localhost:8080/metrics | grep -E "(render|grib|resample|png)_duration"'

# JSON metrics with pretty print
curl -s http://localhost:8080/api/metrics | jq .

# Pipeline breakdown
curl -s http://localhost:8080/api/metrics | jq '{
  grib_load_ms: .grib_load_avg_ms,
  grib_parse_ms: .grib_parse_avg_ms,
  resample_ms: .resample_avg_ms,
  png_encode_ms: .png_encode_avg_ms,
  total_render_ms: .render_avg_ms
}'
```

### Analyze Bottlenecks
```bash
# Create analysis script
cat > analyze_pipeline.sh << 'EOF'
#!/bin/bash
echo "=== Rendering Pipeline Breakdown ==="
METRICS=$(curl -s http://localhost:8080/api/metrics)

echo "Total Renders: $(echo $METRICS | jq -r .renders_total)"
echo "Avg Render Time: $(echo $METRICS | jq -r .render_avg_ms) ms"
echo ""
echo "Pipeline Stages:"
echo "  1. GRIB Load:   $(echo $METRICS | jq -r .grib_load_avg_ms) ms"
echo "  2. GRIB Parse:  $(echo $METRICS | jq -r .grib_parse_avg_ms) ms"
echo "  3. Resample:    $(echo $METRICS | jq -r .resample_avg_ms) ms"
echo "  4. PNG Encode:  $(echo $METRICS | jq -r .png_encode_avg_ms) ms"
echo ""
echo "Cache Performance:"
echo "  Hit Rate: $(echo $METRICS | jq -r .cache_hit_rate)%"
echo "  Lookup Time: $(echo $METRICS | jq -r .cache_lookup_avg_ms) ms"
EOF
chmod +x analyze_pipeline.sh
./analyze_pipeline.sh
```

### Load Testing with Profiling
```bash
# Run load test and monitor metrics
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_single_tile_temporal.yaml &

# Watch metrics during test
watch -n 2 './analyze_pipeline.sh'

# After test, check breakdown
curl -s http://localhost:8080/api/metrics | jq '{
  pipeline: {
    load: .grib_load_avg_ms,
    parse: .grib_parse_avg_ms,
    resample: .resample_avg_ms,
    encode: .png_encode_avg_ms
  },
  by_layer_type: .layer_type_stats
}'
```

## Tracing Integration

### View Traces
If using a tracing backend (Jaeger, etc.):

```bash
# Export traces to stdout (set in environment)
export RUST_LOG=wms_api=info,trace

# Run service and see detailed spans
docker-compose logs -f wms-api | grep -E "render|grib|resample|png"
```

### Trace Example Output
```
[INFO render_weather_data{model="mrms" parameter="REFL" width=256 height=256}]
  [INFO catalog_lookup] Found entry in 2.3ms
  [INFO load_grid_data{storage_path="mrms/..."}]
    [DEBUG storage_get] Retrieved 425KB in 12.5ms
    [DEBUG grib_parse{file_size=425984}] Parsed in 8.2ms
  [INFO resample_grid{from_width=7000 from_height=3500 to_width=256 to_height=256}] Complete in 11.8ms
  [INFO apply_color_rendering{parameter="REFL"}] Applied in 3.2ms
  [INFO png_encode{width=256 height=256}] Encoded in 9.1ms
[INFO] Total render: 47.1ms
```

## Expected Bottleneck Analysis

Based on the metrics, you can identify where time is spent:

### Scenario 1: GRIB Load Dominant
```
GRIB Load:   85.2 ms  ← BOTTLENECK
GRIB Parse:   8.3 ms
Resample:    12.1 ms
PNG Encode:   9.6 ms
Total:      115.2 ms
```
**Solution**: Implement GRIB cache (already in Phase 4), optimize MinIO network, use faster storage.

### Scenario 2: Resample Dominant
```
GRIB Load:   15.2 ms
GRIB Parse:   8.3 ms
Resample:   102.4 ms  ← BOTTLENECK
PNG Encode:   9.6 ms
Total:      135.5 ms
```
**Solution**: Optimize resampling algorithm, use SIMD, reduce zoom complexity.

### Scenario 3: PNG Encode Dominant
```
GRIB Load:   15.2 ms
GRIB Parse:   8.3 ms
Resample:    12.1 ms
PNG Encode:  78.3 ms  ← BOTTLENECK
Total:      113.9 ms
```
**Solution**: Already optimal (Phase 4 analysis), but could parallelize for large images.

### Scenario 4: Balanced
```
GRIB Load:   15.2 ms
GRIB Parse:   8.3 ms
Resample:    12.1 ms
PNG Encode:   9.6 ms
Total:       45.2 ms
```
**Result**: Well-optimized pipeline, focus on caching to reduce frequency.

## Next Steps

1. **Collect Baseline Metrics**:
   ```bash
   # Run test and save baseline
   ./analyze_pipeline.sh > baseline_metrics.txt
   ```

2. **Run Load Tests**:
   ```bash
   # MRMS (small files)
   cargo run --package load-test -- run --scenario \
     validation/load-test/scenarios/mrms_temporal_stress.yaml
   
   # HRRR (large files)
   cargo run --package load-test -- run --scenario \
     validation/load-test/scenarios/hrrr_single_tile_temporal.yaml
   ```

3. **Compare Datasets**:
   - MRMS (400 KB): Expect resample/PNG to dominate
   - HRRR (135 MB): Expect GRIB load/parse to dominate
   - GOES (2.8 MB): Balanced or resample-heavy

4. **Optimize Based on Data**:
   - If GRIB load > 50% of time: Focus on caching, storage speed
   - If resample > 50% of time: Focus on algorithm optimization
   - If PNG encode > 30% of time: Already optimal per Phase 4 analysis

## Files Modified

- `services/wms-api/src/metrics.rs` - Added pipeline metrics collection
- `services/wms-api/src/rendering.rs` - Added tracing instrumentation
- `PROFILING_IMPLEMENTATION.md` - This document

## Status

✅ Metrics collection implemented
✅ Tracing spans added to pipeline
✅ Prometheus histograms configured
✅ JSON metrics endpoint enhanced
🔲 Baseline metrics collection (next: run tests)
🔲 Bottleneck identification (next: analyze results)
🔲 Optimization based on findings (next: Phase 5+)

---

**Ready for profiling!** Run load tests and analyze the breakdown to identify optimization opportunities.
