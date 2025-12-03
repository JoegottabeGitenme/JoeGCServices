# Comprehensive Load Test "Hammering" Scenarios

## Overview

This document outlines extreme load testing scenarios designed to find the breaking points of the Weather WMS system. These tests go beyond typical production loads to identify bottlenecks, memory leaks, and failure modes.

---

## Current Load Test Baseline

From existing scenarios in `validation/load-test/scenarios/`:

| Scenario | Duration | Concurrency | RPS Target | Purpose |
|----------|----------|-------------|------------|---------|
| quick | 30s | 10 | ~50 | Smoke test |
| cold_cache | 60s | 15 | ~75 | Cache miss performance |
| warm_cache | 60s | 15 | ~150 | Cache hit performance |
| stress | 120s | 50 | ~200 | Moderate stress |

---

## New Hammering Scenarios

### Category 1: Extreme Concurrency Tests

#### 1.1 Concurrency Ramp (concurrency_ramp.yaml)

Gradually increase concurrency to find breaking point.

```yaml
name: concurrency_ramp
description: "Ramp concurrency from 10 to 500 to find breaking point"
base_url: http://localhost:8080
duration_secs: 600  # 10 minutes

stages:
  - duration_secs: 60
    concurrency: 10
  - duration_secs: 60
    concurrency: 25
  - duration_secs: 60
    concurrency: 50
  - duration_secs: 60
    concurrency: 100
  - duration_secs: 60
    concurrency: 200
  - duration_secs: 60
    concurrency: 300
  - duration_secs: 60
    concurrency: 400
  - duration_secs: 60
    concurrency: 500
  - duration_secs: 60
    concurrency: 100  # Cool down

layers:
  - name: gfs_TMP
    style: temperature
    weight: 1.0

tile_selection:
  type: random
  zoom_range: [4, 10]
  bbox:
    min_lon: -130
    min_lat: 20
    max_lon: -60
    max_lat: 55

metrics:
  record_percentiles: [50, 90, 95, 99, 99.9]
  record_errors: true
  record_timeouts: true
```

#### 1.2 Spike Test (spike_test.yaml)

Sudden traffic spike to test system resilience.

```yaml
name: spike_test
description: "Sudden 10x traffic spike"
base_url: http://localhost:8080
duration_secs: 300

stages:
  - duration_secs: 60
    concurrency: 20
    description: "Normal load"
  - duration_secs: 30
    concurrency: 200
    description: "SPIKE: 10x traffic"
  - duration_secs: 60
    concurrency: 20
    description: "Return to normal"
  - duration_secs: 30
    concurrency: 200
    description: "Second spike"
  - duration_secs: 60
    concurrency: 20
    description: "Final cool down"

layers:
  - name: gfs_TMP
    weight: 0.4
  - name: hrrr_REFC
    weight: 0.4
  - name: goes16_CMI_C13
    weight: 0.2

tile_selection:
  type: random
  zoom_range: [5, 10]
  bbox:
    min_lon: -100
    min_lat: 30
    max_lon: -80
    max_lat: 45
```

### Category 2: Cache Stress Tests

#### 2.1 Cache Thrashing (cache_thrash.yaml)

Force continuous cache misses to stress the rendering pipeline.

```yaml
name: cache_thrash
description: "Force 100% cache misses by requesting unique tiles"
base_url: http://localhost:8080
duration_secs: 300
concurrency: 100

# Use current timestamp to ensure unique tiles
cache_buster: true
cache_buster_strategy: query_param  # Adds ?_cb={random} to each request

layers:
  - name: gfs_TMP
    style: temperature
    weight: 0.5
  - name: hrrr_TMP
    style: temperature_isolines
    weight: 0.5

tile_selection:
  type: sequential_sweep
  zoom_levels: [4, 5, 6, 7, 8, 9, 10]
  bbox:
    min_lon: -130
    min_lat: 20
    max_lon: -60
    max_lat: 55

# Request tiles we've never seen before
unique_tiles_only: true

warmup_secs: 0  # No warmup - cold cache from start
```

#### 2.2 Cache Fill Race (cache_race.yaml)

Multiple clients requesting same tiles simultaneously.

```yaml
name: cache_race
description: "Race condition test - 100 clients request same tile"
base_url: http://localhost:8080
duration_secs: 120
concurrency: 100

# All clients request the exact same tiles
tile_selection:
  type: fixed_set
  tiles:
    - { z: 5, x: 8, y: 12 }
    - { z: 5, x: 9, y: 12 }
    - { z: 5, x: 8, y: 13 }
    - { z: 5, x: 9, y: 13 }
    - { z: 6, x: 16, y: 24 }
    - { z: 6, x: 17, y: 24 }
    - { z: 6, x: 16, y: 25 }
    - { z: 6, x: 17, y: 25 }

layers:
  - name: gfs_TMP
    weight: 1.0

# Request same tile every 10ms from all clients
request_interval_ms: 10
```

### Category 3: Memory Stress Tests

#### 3.1 Large Tile Requests (large_tiles.yaml)

Request maximum resolution tiles to stress memory.

```yaml
name: large_tiles
description: "Request large format tiles to stress memory"
base_url: http://localhost:8080
duration_secs: 300
concurrency: 20

# Override default tile size
tile_size: 2048  # 4x normal size

layers:
  - name: gfs_TMP
    weight: 0.25
  - name: hrrr_TMP
    weight: 0.25
  - name: gfs_WIND_BARBS
    weight: 0.25
  - name: hrrr_REFC
    weight: 0.25

tile_selection:
  type: random
  zoom_range: [3, 8]  # Lower zoom = larger geographic area = more data
  bbox:
    min_lon: -180
    min_lat: -90
    max_lon: 180
    max_lat: 90

wms_params:
  WIDTH: 2048
  HEIGHT: 2048
  FORMAT: image/png
```

#### 3.2 Memory Leak Detection (memory_leak.yaml)

Long-running test to detect memory leaks.

```yaml
name: memory_leak
description: "Long-duration test to detect memory leaks"
base_url: http://localhost:8080
duration_secs: 3600  # 1 hour
concurrency: 50

# Mix of operations to exercise all code paths
layers:
  - name: gfs_TMP
    style: temperature
    weight: 0.15
  - name: gfs_TMP
    style: temperature_isolines
    weight: 0.15
  - name: gfs_WIND_BARBS
    weight: 0.15
  - name: hrrr_TMP
    weight: 0.15
  - name: hrrr_REFC
    weight: 0.15
  - name: goes16_CMI_C02
    weight: 0.15
  - name: mrms_REFL
    weight: 0.10

tile_selection:
  type: random
  zoom_range: [4, 12]
  bbox:
    min_lon: -130
    min_lat: 20
    max_lon: -60
    max_lat: 55

# Collect memory metrics every minute
metrics:
  collect_interval_secs: 60
  record_memory_usage: true
  record_gc_stats: true
  alert_memory_growth_mb: 500  # Alert if memory grows by 500MB
```

### Category 4: Complex Rendering Tests

#### 4.1 Multi-Layer Composition (multi_layer.yaml)

Request multiple layers simultaneously.

```yaml
name: multi_layer
description: "Request 5 layers in single GetMap"
base_url: http://localhost:8080
duration_secs: 300
concurrency: 30

request_type: wms_getmap

# Each request includes 5 layers
layers_per_request: 5
layer_combinations:
  - layers:
      - gfs_TMP
      - gfs_WIND_BARBS
      - goes16_CMI_C02
      - hrrr_REFC
      - mrms_PRECIP_RATE
    weight: 0.3
  - layers:
      - gfs_TMP
      - gfs_TMP:temperature_isolines
      - gfs_WIND_BARBS
      - gfs_PRMSL
      - hrrr_TMP
    weight: 0.3
  - layers:
      - hrrr_TMP
      - hrrr_WIND_BARBS
      - hrrr_REFC
      - goes16_CMI_C13
      - mrms_REFL
    weight: 0.4

tile_selection:
  type: random
  zoom_range: [5, 8]
  bbox:
    min_lon: -110
    min_lat: 30
    max_lon: -70
    max_lat: 50
```

#### 4.2 Style Intensive (style_intensive.yaml)

Focus on computationally expensive rendering styles.

```yaml
name: style_intensive
description: "Test expensive rendering: isolines, wind barbs, contours"
base_url: http://localhost:8080
duration_secs: 300
concurrency: 50

layers:
  # Contour generation is CPU intensive
  - name: gfs_TMP
    style: temperature_isolines
    weight: 0.3
  - name: gfs_TMP
    style: temperature_isolines:fine  # 2°C intervals
    weight: 0.2
  # Wind barb rendering is complex
  - name: gfs_WIND_BARBS
    weight: 0.25
  - name: hrrr_WIND_BARBS
    weight: 0.25

tile_selection:
  type: random
  zoom_range: [6, 10]
  bbox:
    min_lon: -100
    min_lat: 30
    max_lon: -80
    max_lat: 45
```

### Category 5: Protocol Edge Cases

#### 5.1 Invalid Request Storm (invalid_requests.yaml)

Test error handling under load.

```yaml
name: invalid_requests
description: "Mix of valid and invalid requests to test error handling"
base_url: http://localhost:8080
duration_secs: 180
concurrency: 50

request_mix:
  # 50% valid requests
  - type: valid
    weight: 0.5
    layers: [gfs_TMP, hrrr_REFC]
  
  # 10% requests for non-existent layers
  - type: invalid_layer
    weight: 0.1
    layers: [nonexistent_LAYER, fake_MODEL_param]
  
  # 10% invalid bounding boxes
  - type: invalid_bbox
    weight: 0.1
    bbox_errors:
      - { min_lon: 200, min_lat: 0, max_lon: 250, max_lat: 50 }  # Out of range
      - { min_lon: -80, min_lat: 50, max_lon: -100, max_lat: 30 }  # Inverted
  
  # 10% missing required parameters
  - type: missing_params
    weight: 0.1
    missing: [LAYERS, CRS, BBOX]
  
  # 10% invalid CRS
  - type: invalid_crs
    weight: 0.1
    crs_values: [EPSG:99999, CRS:INVALID, WGS84]
  
  # 10% malformed requests
  - type: malformed
    weight: 0.1
    malformations:
      - double_question_mark: true
      - null_bytes: true
      - unicode_garbage: true
```

#### 5.2 Large Response Test (large_response.yaml)

Request maximum data to stress response handling.

```yaml
name: large_response
description: "Request very large tiles to test response buffering"
base_url: http://localhost:8080
duration_secs: 300
concurrency: 10

wms_params:
  WIDTH: 4096
  HEIGHT: 4096
  FORMAT: image/png

layers:
  - name: gfs_TMP
    weight: 0.5
  - name: hrrr_REFC
    weight: 0.5

tile_selection:
  type: random
  zoom_range: [2, 5]  # Zoom 2-5 covers large areas
  bbox:
    min_lon: -180
    min_lat: -90
    max_lon: 180
    max_lat: 90

# Expect responses up to 10MB
expected_max_response_size_mb: 10
timeout_secs: 120
```

### Category 6: Endurance Tests

#### 6.1 24-Hour Soak Test (soak_24h.yaml)

Full day test to find long-term issues.

```yaml
name: soak_24h
description: "24-hour endurance test at moderate load"
base_url: http://localhost:8080
duration_secs: 86400  # 24 hours
concurrency: 30

# Simulate realistic daily traffic pattern
traffic_pattern:
  type: diurnal
  peak_hour: 14  # 2 PM
  peak_multiplier: 2.0  # 2x traffic at peak
  trough_hour: 4  # 4 AM
  trough_multiplier: 0.3  # 30% traffic at trough

layers:
  - name: gfs_TMP
    weight: 0.2
  - name: hrrr_TMP
    weight: 0.2
  - name: gfs_WIND_BARBS
    weight: 0.15
  - name: hrrr_WIND_BARBS
    weight: 0.15
  - name: hrrr_REFC
    weight: 0.15
  - name: goes16_CMI_C02
    weight: 0.15

tile_selection:
  type: random
  zoom_range: [4, 12]
  bbox:
    min_lon: -130
    min_lat: 20
    max_lon: -60
    max_lat: 55

# Collect comprehensive metrics
metrics:
  collect_interval_secs: 300  # Every 5 minutes
  record_histograms: true
  record_system_stats: true
  alert_thresholds:
    p99_latency_ms: 5000
    error_rate_percent: 1.0
    memory_growth_percent: 50
```

#### 6.2 Rolling Restart Test (rolling_restart.yaml)

Test behavior during service restarts.

```yaml
name: rolling_restart
description: "Maintain load during simulated rolling restarts"
base_url: http://localhost:8080
duration_secs: 600
concurrency: 50

# Load generator continues during service events
service_events:
  enabled: true
  events:
    - at_secs: 120
      action: restart_api
      description: "First API restart"
    - at_secs: 240
      action: restart_api
      description: "Second API restart"
    - at_secs: 360
      action: restart_redis
      description: "Redis restart (cache flush)"
    - at_secs: 480
      action: restart_api
      description: "Final API restart"

layers:
  - name: gfs_TMP
    weight: 0.5
  - name: hrrr_REFC
    weight: 0.5

tile_selection:
  type: random
  zoom_range: [5, 10]
  bbox:
    min_lon: -100
    min_lat: 30
    max_lon: -80
    max_lat: 45

# Track error spikes during restarts
metrics:
  track_error_windows: true
  acceptable_error_spike_duration_secs: 10
```

---

## Implementation Requirements

### Load Test Tool Enhancements

Add these capabilities to `validation/load-test/`:

```rust
// src/stages.rs
pub struct TestStage {
    pub duration_secs: u64,
    pub concurrency: u32,
    pub description: Option<String>,
}

pub fn run_staged_test(config: &StageConfig) -> Result<TestResults> {
    let mut results = Vec::new();
    
    for stage in &config.stages {
        info!(
            "Starting stage: {} ({} concurrent for {}s)",
            stage.description.as_deref().unwrap_or("unnamed"),
            stage.concurrency,
            stage.duration_secs
        );
        
        let stage_result = run_stage(stage, &config.common)?;
        results.push(stage_result);
    }
    
    Ok(combine_results(results))
}
```

```rust
// src/metrics.rs
pub struct ExtendedMetrics {
    // Latency histograms
    pub latency_p50_ms: f64,
    pub latency_p90_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub latency_p999_ms: f64,
    
    // Error tracking
    pub error_count: u64,
    pub error_rate: f64,
    pub errors_by_type: HashMap<String, u64>,
    
    // Memory tracking
    pub memory_start_mb: u64,
    pub memory_end_mb: u64,
    pub memory_peak_mb: u64,
    
    // System stats
    pub cpu_avg_percent: f64,
    pub cpu_peak_percent: f64,
}
```

### Alerting Integration

```rust
// src/alerts.rs
pub struct AlertConfig {
    pub p99_latency_threshold_ms: u64,
    pub error_rate_threshold: f64,
    pub memory_growth_threshold_mb: u64,
}

pub fn check_alerts(metrics: &ExtendedMetrics, config: &AlertConfig) -> Vec<Alert> {
    let mut alerts = Vec::new();
    
    if metrics.latency_p99_ms > config.p99_latency_threshold_ms as f64 {
        alerts.push(Alert::LatencyExceeded {
            actual: metrics.latency_p99_ms,
            threshold: config.p99_latency_threshold_ms,
        });
    }
    
    if metrics.error_rate > config.error_rate_threshold {
        alerts.push(Alert::ErrorRateExceeded {
            actual: metrics.error_rate,
            threshold: config.error_rate_threshold,
        });
    }
    
    let memory_growth = metrics.memory_end_mb.saturating_sub(metrics.memory_start_mb);
    if memory_growth > config.memory_growth_threshold_mb {
        alerts.push(Alert::MemoryGrowth {
            growth_mb: memory_growth,
            threshold: config.memory_growth_threshold_mb,
        });
    }
    
    alerts
}
```

---

## Running the Tests

### Quick Hammer Test

```bash
# 5-minute aggressive test
./scripts/run_load_test.sh concurrency_ramp --duration 300
```

### Full Stress Suite

```bash
#!/bin/bash
# scripts/run_stress_suite.sh

echo "Starting comprehensive stress test suite"

# Reset state
./scripts/reset_test_state.sh --restart

# Run each test with results
tests=(
    "concurrency_ramp"
    "spike_test"
    "cache_thrash"
    "cache_race"
    "large_tiles"
    "multi_layer"
    "style_intensive"
    "invalid_requests"
)

for test in "${tests[@]}"; do
    echo "Running: $test"
    ./scripts/run_load_test.sh "$test" --save --output json
    
    # Brief cooldown between tests
    sleep 30
    ./scripts/reset_test_state.sh
done

echo "Stress suite complete. Results in validation/load-test/results/"
```

### Soak Test

```bash
# Start 24-hour soak test in background
nohup ./scripts/run_load_test.sh soak_24h --save > soak_test.log 2>&1 &
echo "Soak test started. PID: $!"
echo "Monitor with: tail -f soak_test.log"
```

---

## Success Criteria

### Performance Thresholds

| Metric | Acceptable | Warning | Critical |
|--------|-----------|---------|----------|
| P50 Latency | < 50ms | 50-100ms | > 100ms |
| P99 Latency | < 500ms | 500-2000ms | > 2000ms |
| Error Rate | < 0.1% | 0.1-1% | > 1% |
| Memory Growth/hr | < 50MB | 50-200MB | > 200MB |
| CPU @ 100 conc | < 70% | 70-90% | > 90% |

### Breaking Point Targets

- **Concurrency**: System should handle 200+ concurrent users
- **Throughput**: 500+ requests/second sustained
- **Spike Recovery**: Return to normal latency within 30s after 10x spike
- **Memory Stability**: No growth over 24-hour soak test
- **Error Handling**: 0% valid request failures under load

---

## Reporting

### Test Report Template

```markdown
# Load Test Report: {scenario_name}

**Date**: {date}
**Duration**: {duration}
**Environment**: {environment}

## Summary
- Total Requests: {total_requests}
- Success Rate: {success_rate}%
- Throughput: {rps} req/s

## Latency
| Percentile | Value |
|------------|-------|
| P50 | {p50}ms |
| P90 | {p90}ms |
| P95 | {p95}ms |
| P99 | {p99}ms |

## Errors
| Error Type | Count |
|------------|-------|
| Timeout | {timeout_count} |
| 5xx | {5xx_count} |
| Connection | {conn_count} |

## System Resources
- CPU Peak: {cpu_peak}%
- Memory Start: {mem_start}MB
- Memory End: {mem_end}MB
- Memory Growth: {mem_growth}MB

## Alerts Triggered
{alerts}

## Recommendations
{recommendations}
```

---

## Next Steps

1. **Implement staged testing** in load-test tool
2. **Create all scenario YAML files**
3. **Add memory tracking** to metrics collection
4. **Integrate with monitoring** (Grafana dashboards)
5. **Run baseline tests** before any optimization
6. **Schedule regular soak tests** (weekly 24h tests)
