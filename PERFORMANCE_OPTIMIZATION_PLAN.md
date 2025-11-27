# Performance Optimization Plan: "World's Fastest WMTS"

This is a comprehensive plan to benchmark, profile, and optimize the WMTS rendering pipeline.

---

## Phase 1: High-Resolution Test Datasets

### 1.1 MRMS (Multi-Radar Multi-Sensor) Data
- **Resolution**: ~1km (0.01° grid)
- **Source**: NOAA MRMS via AWS Open Data or NCEP
- **Parameters**: Precipitation rate, composite reflectivity, rotation tracks
- **Format**: GRIB2
- **URL Pattern**: `https://mrms.ncep.noaa.gov/data/2D/`

**Tasks:**
- Create `scripts/download_mrms.sh` to fetch MRMS GRIB2 files
- Update ingester to handle MRMS-specific GRIB2 templates (different product discipline)
- Add MRMS layer definitions to catalog

### 1.2 GOES Satellite Data
- **Resolution**: 0.5km-2km depending on band
- **Source**: NOAA GOES-16/17/18 via AWS `s3://noaa-goes18/`
- **Format**: NetCDF (requires netcdf-parser crate work)
- **Parameters**: Visible, IR, water vapor bands

**Tasks:**
- Implement NetCDF parsing in `crates/netcdf-parser/`
- Create satellite-specific color maps (grayscale, enhanced IR)
- Handle non-geographic projections (geostationary)

### 1.3 HRRR (High-Resolution Rapid Refresh)
- **Resolution**: 3km
- **Source**: NOAA NOMADS or AWS
- **Format**: GRIB2 (similar to GFS)
- **Advantage**: Already compatible with current GRIB2 parser

**Recommendation**: Start with **HRRR** as it's the easiest path - same format as GFS but higher resolution.

---

## Phase 2: Load Testing Framework

### 2.1 Test Harness Design

Create `validation/load-test/` directory with:

```
validation/load-test/
├── Cargo.toml              # Rust-based load tester
├── src/
│   ├── main.rs             # CLI entry point
│   ├── scenarios.rs        # Test scenario definitions
│   ├── tile_generator.rs   # Generate realistic tile request patterns
│   └── metrics.rs          # Collect and report metrics
├── scenarios/
│   ├── cold_cache.yaml     # Test from empty cache
│   ├── warm_cache.yaml     # Test with pre-warmed cache
│   ├── zoom_sweep.yaml     # Test all zoom levels
│   └── layer_comparison.yaml # Compare gradient vs barbs vs isolines
└── results/                # JSON/CSV output for analysis
```

### 2.2 Request Pattern Generation

```rust
// tile_generator.rs - Generate realistic tile request patterns
pub struct TileRequestGenerator {
    pub strategy: RequestStrategy,
    pub zoom_range: (u32, u32),
    pub bbox: Option<BoundingBox>,  // Geographic focus area
    pub layers: Vec<String>,
    pub styles: Vec<String>,
}

pub enum RequestStrategy {
    Random,              // Random tiles within bounds
    Sequential,          // Sweep through tiles systematically
    PanSimulation,       // Simulate user panning (adjacent tiles)
    ZoomSimulation,      // Simulate zoom in/out (parent/child tiles)
    Hotspot { center: (f64, f64), radius: f64 },  // Focus on area
}
```

### 2.3 Concurrency Levels to Test

| Scenario | Concurrent Requests | Duration |
|----------|---------------------|----------|
| Light    | 10                  | 60s      |
| Medium   | 50                  | 60s      |
| Heavy    | 200                 | 60s      |
| Burst    | 500                 | 10s      |
| Sustained| 100                 | 300s     |

---

## Phase 3: System State Management

### 3.1 Reset Script

Create `scripts/reset_test_state.sh`:

```bash
#!/bin/bash
# Reset system to consistent state for benchmarking

# 1. Flush Redis cache
docker compose exec redis redis-cli FLUSHALL

# 2. Clear any pending render jobs
docker compose exec redis redis-cli DEL render:queue

# 3. Reset metrics counters
curl -X POST http://localhost:8080/api/metrics/reset

# 4. Optional: restart services to clear in-memory state
docker compose restart wms-api

# 5. Wait for healthy
sleep 5
curl -f http://localhost:8080/health || exit 1

echo "System reset complete"
```

### 3.2 Cache Warming Script

Create `scripts/warm_cache.sh`:

```bash
#!/bin/bash
# Pre-warm cache with common tiles

LAYER=$1
ZOOM_MIN=${2:-3}
ZOOM_MAX=${3:-8}

for z in $(seq $ZOOM_MIN $ZOOM_MAX); do
    # Generate tile coordinates for zoom level
    # Focus on populated areas (US, Europe)
    ...
done
```

---

## Phase 4: Metrics Collection

### 4.1 Metrics to Track

| Metric | Description | Granularity |
|--------|-------------|-------------|
| `request_latency_ms` | Total request time | Per request |
| `render_time_ms` | Time spent rendering | Per tile |
| `cache_lookup_time_ms` | Redis cache check | Per request |
| `grib_load_time_ms` | Time to load GRIB from MinIO | Per render |
| `grib_parse_time_ms` | Time to parse GRIB2 | Per render |
| `resample_time_ms` | Grid resampling time | Per render |
| `png_encode_time_ms` | PNG encoding time | Per render |
| `cache_hit_rate` | % requests served from cache | Aggregate |
| `tiles_per_second` | Throughput | Aggregate |

### 4.2 Instrumented Metrics in Code

Current state: Basic metrics exist in `services/wms-api/src/metrics.rs`

**Enhancements needed:**

```rust
// metrics.rs - Add detailed timing breakdowns
pub struct RenderMetrics {
    pub layer_type: LayerType,  // Gradient, WindBarbs, Isolines
    pub zoom_level: u32,
    pub cache_hit: bool,
    
    // Timing breakdown (microseconds)
    pub catalog_lookup_us: u64,
    pub storage_fetch_us: u64,
    pub grib_parse_us: u64,
    pub data_unpack_us: u64,
    pub resample_us: u64,
    pub render_us: u64,
    pub png_encode_us: u64,
    pub total_us: u64,
}

pub enum LayerType {
    Gradient,
    WindBarbs,
    Isolines,
    Satellite,
}
```

### 4.3 Metrics Export

Add Prometheus-compatible endpoint:

```
GET /metrics
```

Output format:
```prometheus
# HELP wmts_request_duration_seconds Request latency histogram
# TYPE wmts_request_duration_seconds histogram
wmts_request_duration_seconds_bucket{layer="gfs_TMP",style="default",le="0.1"} 1523
wmts_request_duration_seconds_bucket{layer="gfs_TMP",style="default",le="0.5"} 2891
...

# HELP wmts_render_duration_seconds Render time by layer type
wmts_render_duration_seconds{layer_type="gradient"} 0.045
wmts_render_duration_seconds{layer_type="wind_barbs"} 0.123
wmts_render_duration_seconds{layer_type="isolines"} 0.089
```

### 4.4 Results Dashboard

Options:
1. **Grafana** - Connect to Prometheus metrics
2. **Custom HTML dashboard** - Extend existing `web/index.html`
3. **CLI summary** - Output after load test run

---

## Phase 5: Profiling Strategy

### 5.1 Rust Profiling Tools

| Tool | Purpose | Usage |
|------|---------|-------|
| `perf` | CPU profiling | `perf record -g ./target/release/wms-api` |
| `flamegraph` | Visualize CPU time | `cargo flamegraph --bin wms-api` |
| `criterion` | Micro-benchmarks | Unit test specific functions |
| `tracing` | Distributed tracing | Already in use, add spans |
| `tokio-console` | Async runtime analysis | Debug async bottlenecks |

### 5.2 Benchmark Suite

Create `crates/renderer/benches/`:

```rust
// render_benchmarks.rs
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};

fn benchmark_gradient_render(c: &mut Criterion) {
    let data = load_test_grid();  // 1440x721 GFS grid
    
    let mut group = c.benchmark_group("gradient_render");
    for size in [256, 512, 1024].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            size,
            |b, &size| {
                b.iter(|| render_temperature(&data, size, size, -40.0, 40.0))
            },
        );
    }
    group.finish();
}

fn benchmark_resample(c: &mut Criterion) {
    // Test resampling at different resolutions
}

fn benchmark_png_encode(c: &mut Criterion) {
    // Test PNG encoding at different sizes
}

criterion_group!(benches, 
    benchmark_gradient_render,
    benchmark_resample,
    benchmark_png_encode
);
criterion_main!(benches);
```

### 5.3 Profiling Workflow

```bash
# 1. Build release with debug symbols
RUSTFLAGS="-C debuginfo=2" cargo build --release

# 2. Run with profiling
perf record -g --call-graph dwarf ./target/release/wms-api &
PID=$!

# 3. Run load test
./load-test --scenario heavy --duration 60s

# 4. Stop and analyze
kill $PID
perf report

# 5. Generate flamegraph
perf script | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg
```

---

## Phase 6: Known Optimization Opportunities

Based on code review, these are likely bottlenecks:

### 6.1 GRIB2 Parsing (`crates/grib2-parser/`)
- **Issue**: Re-parsing GRIB2 for every tile request
- **Opportunity**: Cache parsed grid data in memory (LRU cache)
- **Impact**: High - GRIB parsing is expensive

### 6.2 Grid Resampling (`services/wms-api/src/rendering.rs`)
- **Issue**: Bilinear interpolation per-pixel
- **Opportunity**: SIMD optimization, pre-computed lookup tables
- **Impact**: Medium - scales with tile size

### 6.3 PNG Encoding (`crates/renderer/src/png.rs`)
- **Issue**: Encoding from scratch each time
- **Opportunity**: Use faster PNG encoder (e.g., `png` crate with `deflate` optimization)
- **Impact**: Medium - ~20-30% of render time typically

### 6.4 Wind Barb Rendering (`crates/renderer/src/barbs.rs`)
- **Issue**: SVG rasterization per barb
- **Opportunity**: Pre-rasterize barb sprites at common sizes
- **Impact**: High for wind barb layers

### 6.5 Contour Generation (`crates/renderer/src/contour.rs`)
- **Issue**: Marching squares on full grid
- **Opportunity**: Spatial indexing, parallel contour tracing
- **Impact**: High for isoline layers

### 6.6 Storage I/O
- **Issue**: Network round-trip to MinIO for each GRIB file
- **Opportunity**: Local file cache, memory-mapped files
- **Impact**: High for cache misses

---

## Implementation Roadmap

### Phase 1: Foundation (COMPLETED - Nov 2024)

#### Completed Items:
- [x] **Download scripts for all data sources** (Nov 26-27, 2024)
  - `scripts/download_gfs.sh` - GFS global forecast (1° resolution, ~40MB/file)
  - `scripts/download_hrrr.sh` - HRRR high-res CONUS (3km)
  - `scripts/download_mrms.sh` - MRMS radar/precip (~1km)
  - `scripts/download_goes.sh` - GOES satellite imagery
  
- [x] **Data ingestion pipeline** (Nov 26-27, 2024)
  - Auto-download sample data if none exists
  - Ingest GFS, HRRR, GOES, MRMS into catalog
  - `scripts/ingest_test_data.sh` handles all data sources
  
- [x] **System state management** (Nov 26, 2024)
  - `scripts/reset_test_state.sh` - Flush Redis, clear queue, optional restart
  
- [x] **Basic metrics collection** (existing)
  - `services/wms-api/src/metrics.rs` - Request counts, cache hits, render times
  - Timer utility for measuring operation duration
  
- [x] **Rendering fixes for all projections** (Nov 27, 2024)
  - Fixed GFS longitude normalization (0-360° to -180-180°)
  - Fixed HRRR Lambert Conformal to Mercator projection
  - Fixed GOES Geostationary to Mercator projection

### Phase 2: Load Testing Framework (IN PROGRESS)

See detailed task breakdown below in "Phase 2 Implementation Details" section.

### Phase 3: Metrics & Profiling
- [ ] Add Prometheus-compatible `/metrics` endpoint
- [ ] Implement detailed timing breakdown per render stage
- [ ] Set up `criterion` benchmarks for hot paths
- [ ] Run initial profiling to identify top bottlenecks

### Phase 4: First Optimizations
- [ ] Implement in-memory GRIB data cache (LRU)
- [ ] Optimize PNG encoding (evaluate `png` crate options)
- [ ] Pre-rasterize wind barb sprites at common sizes
- [ ] Local file cache for GRIB data (reduce MinIO round-trips)

### Ongoing
- [ ] Iterate: profile -> optimize -> benchmark -> repeat
- [ ] Track performance regression with CI benchmarks

---

## Phase 2 Implementation Details

### Overview

Build a Rust-based load testing tool that can generate realistic WMTS tile request patterns,
measure performance under various conditions, and output detailed metrics for analysis.

### Directory Structure

```
validation/load-test/
├── Cargo.toml
├── src/
│   ├── main.rs              # CLI entry point
│   ├── lib.rs               # Library exports
│   ├── config.rs            # Configuration loading
│   ├── generator.rs         # Tile request URL generation
│   ├── runner.rs            # HTTP request execution & concurrency
│   ├── metrics.rs           # Results collection & statistics
│   └── report.rs            # Output formatting (JSON, table, CSV)
├── scenarios/
│   ├── quick.yaml           # Fast smoke test (10 req, 1 concurrent)
│   ├── cold_cache.yaml      # Cache miss testing
│   ├── warm_cache.yaml      # Cache hit testing  
│   ├── zoom_sweep.yaml      # All zoom levels 0-12
│   ├── stress.yaml          # High concurrency stress test
│   └── layer_comparison.yaml # Compare different layer types
└── results/                 # Output directory for test results
```

### Task Breakdown

#### Task 2.1: Project Setup (Est: 1 hour)
**Goal**: Create the load-test crate structure and dependencies

**Files to create**:
- `validation/load-test/Cargo.toml`
- `validation/load-test/src/lib.rs`
- `validation/load-test/src/main.rs`

**Dependencies**:
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.11", features = ["json"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
serde_json = "1"
hdrhistogram = "7"           # High-precision latency histograms
comfy-table = "7"            # CLI table output
indicatif = "0.17"           # Progress bars
chrono = "0.4"
rand = "0.8"
```

**Acceptance criteria**:
- `cargo build` succeeds in validation/load-test/
- `cargo run -- --help` shows usage information

---

#### Task 2.2: Configuration System (Est: 1.5 hours)
**Goal**: Load test scenarios from YAML config files

**File**: `validation/load-test/src/config.rs`

**Data structures**:
```rust
pub struct TestConfig {
    pub name: String,
    pub description: String,
    pub base_url: String,            // e.g., "http://localhost:8080"
    pub duration_secs: u64,          // How long to run
    pub concurrency: u32,            // Concurrent requests
    pub requests_per_second: Option<f64>,  // Rate limiting (optional)
    pub warmup_secs: u64,            // Warmup period (excluded from stats)
    pub layers: Vec<LayerConfig>,
    pub tile_selection: TileSelection,
}

pub struct LayerConfig {
    pub name: String,                // e.g., "gfs_TMP"
    pub style: Option<String>,       // e.g., "temperature"
    pub weight: f64,                 // Relative request frequency (1.0 = normal)
}

pub enum TileSelection {
    Random { zoom_range: (u32, u32), bbox: Option<BBox> },
    Sequential { zoom: u32, bbox: BBox },
    Fixed { tiles: Vec<(u32, u32, u32)> },  // (z, x, y)
    PanSimulation { start: (u32, u32, u32), steps: u32 },
}

pub struct BBox {
    pub min_lon: f64,
    pub min_lat: f64, 
    pub max_lon: f64,
    pub max_lat: f64,
}
```

**Acceptance criteria**:
- Can load scenario from YAML file
- Validates required fields
- Provides sensible defaults

---

#### Task 2.3: Tile URL Generator (Est: 2 hours)
**Goal**: Generate valid WMTS tile request URLs based on configuration

**File**: `validation/load-test/src/generator.rs`

**Key functions**:
```rust
pub struct TileGenerator {
    config: TestConfig,
    rng: StdRng,
}

impl TileGenerator {
    /// Generate next tile request URL
    pub fn next_url(&mut self) -> String;
    
    /// Convert lat/lon to tile coordinates at zoom level
    fn latlon_to_tile(lat: f64, lon: f64, zoom: u32) -> (u32, u32);
    
    /// Get valid tile range for zoom level
    fn tile_range_for_zoom(zoom: u32) -> (u32, u32);
    
    /// Get tiles within bounding box at zoom level
    fn tiles_in_bbox(bbox: &BBox, zoom: u32) -> Vec<(u32, u32)>;
}
```

**URL format**: 
```
{base_url}/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0
  &LAYER={layer}&STYLE={style}&FORMAT=image/png
  &TILEMATRIXSET=WebMercatorQuad&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}
```

**Acceptance criteria**:
- Generates valid WMTS URLs
- Random selection respects bbox constraints
- Sequential mode iterates through all tiles in order

---

#### Task 2.4: HTTP Request Runner (Est: 2.5 hours)
**Goal**: Execute HTTP requests with controlled concurrency and measure timing

**File**: `validation/load-test/src/runner.rs`

**Key components**:
```rust
pub struct LoadRunner {
    client: reqwest::Client,
    config: TestConfig,
    generator: TileGenerator,
    metrics: Arc<Mutex<MetricsCollector>>,
}

pub struct RequestResult {
    pub url: String,
    pub status: u16,
    pub latency_us: u64,
    pub bytes: usize,
    pub cache_hit: bool,        // From X-Cache header
    pub timestamp: Instant,
    pub error: Option<String>,
}

impl LoadRunner {
    /// Run the load test
    pub async fn run(&mut self) -> TestResults;
    
    /// Execute single request and record metrics
    async fn execute_request(&self, url: &str) -> RequestResult;
}
```

**Concurrency model**:
- Use `tokio::sync::Semaphore` to limit concurrent requests
- Spawn tasks for each request up to concurrency limit
- Optional rate limiting with token bucket

**Acceptance criteria**:
- Respects concurrency limit
- Accurately measures request latency
- Handles errors gracefully (timeouts, connection failures)
- Detects cache hits from response headers

---

#### Task 2.5: Metrics Collection (Est: 1.5 hours)
**Goal**: Collect and compute statistics from request results

**File**: `validation/load-test/src/metrics.rs`

**Key components**:
```rust
pub struct MetricsCollector {
    histogram: Histogram<u64>,      // HDR histogram for latencies
    requests_total: u64,
    requests_success: u64,
    requests_failed: u64,
    cache_hits: u64,
    cache_misses: u64,
    bytes_total: u64,
    start_time: Instant,
    first_request_time: Option<Instant>,
    last_request_time: Option<Instant>,
}

pub struct TestResults {
    pub config_name: String,
    pub duration_secs: f64,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub requests_per_second: f64,
    
    // Latency percentiles (ms)
    pub latency_p50: f64,
    pub latency_p75: f64,
    pub latency_p90: f64,
    pub latency_p95: f64,
    pub latency_p99: f64,
    pub latency_min: f64,
    pub latency_max: f64,
    pub latency_avg: f64,
    
    // Cache stats
    pub cache_hit_rate: f64,
    
    // Throughput
    pub bytes_per_second: f64,
    pub tiles_per_second: f64,
}
```

**Acceptance criteria**:
- Accurate percentile calculations using HDR histogram
- Excludes warmup period from final stats
- Tracks cache hit rate

---

#### Task 2.6: Report Output (Est: 1 hour)
**Goal**: Format and output test results

**File**: `validation/load-test/src/report.rs`

**Output formats**:

1. **Console table** (default):
```
┌─────────────────────────────────────────────────────────┐
│ Load Test Results: cold_cache                           │
├─────────────────────────────────────────────────────────┤
│ Duration:        60.0s                                  │
│ Total Requests:  5,234                                  │
│ Success Rate:    99.8%                                  │
│ Requests/sec:    87.2                                   │
├─────────────────────────────────────────────────────────┤
│ Latency (ms)     p50    p90    p95    p99    max        │
│                  45.2   89.3   123.4  234.5  567.8      │
├─────────────────────────────────────────────────────────┤
│ Cache Hit Rate:  0.0%                                   │
│ Throughput:      12.3 MB/s                              │
└─────────────────────────────────────────────────────────┘
```

2. **JSON** (for automation):
```json
{
  "config_name": "cold_cache",
  "timestamp": "2024-11-27T15:30:00Z",
  "results": { ... }
}
```

3. **CSV** (for spreadsheet analysis):
```csv
timestamp,config,duration,requests,rps,p50,p90,p99,cache_hit_rate
2024-11-27T15:30:00Z,cold_cache,60.0,5234,87.2,45.2,89.3,234.5,0.0
```

**Acceptance criteria**:
- Clean, readable console output
- JSON output for CI integration
- CSV append mode for tracking over time

---

#### Task 2.7: CLI Interface (Est: 1 hour)
**Goal**: User-friendly command-line interface

**File**: `validation/load-test/src/main.rs`

**Commands**:
```bash
# Run a scenario
load-test run --scenario scenarios/cold_cache.yaml

# Run with overrides
load-test run --scenario scenarios/cold_cache.yaml \
  --concurrency 100 \
  --duration 120

# Quick test (built-in defaults)
load-test quick --layer gfs_TMP --requests 100

# List available scenarios
load-test list

# Output formats
load-test run --scenario cold_cache.yaml --output json > results.json
load-test run --scenario cold_cache.yaml --output csv >> results.csv
```

**Acceptance criteria**:
- Clear help text
- Validates inputs
- Shows progress during test
- Returns non-zero exit code on failure

---

#### Task 2.8: Test Scenarios (Est: 1 hour)
**Goal**: Create useful test scenario configurations

**Files in `scenarios/`**:

**quick.yaml** - Fast smoke test:
```yaml
name: quick
description: Quick smoke test (10 requests)
base_url: http://localhost:8080
duration_secs: 10
concurrency: 1
warmup_secs: 0
layers:
  - name: gfs_TMP
tile_selection:
  type: random
  zoom_range: [4, 6]
```

**cold_cache.yaml** - Test cache-miss performance:
```yaml
name: cold_cache
description: Test rendering performance without cache
base_url: http://localhost:8080
duration_secs: 60
concurrency: 10
warmup_secs: 5
layers:
  - name: gfs_TMP
    weight: 1.0
  - name: gfs_PRMSL
    weight: 0.5
tile_selection:
  type: random
  zoom_range: [3, 10]
  bbox: { min_lon: -130, min_lat: 20, max_lon: -60, max_lat: 55 }
```

**stress.yaml** - High concurrency:
```yaml
name: stress
description: Stress test with high concurrency
base_url: http://localhost:8080
duration_secs: 60
concurrency: 200
warmup_secs: 10
layers:
  - name: gfs_TMP
tile_selection:
  type: random
  zoom_range: [5, 8]
```

**layer_comparison.yaml** - Compare layer types:
```yaml
name: layer_comparison
description: Compare performance of different layer types
base_url: http://localhost:8080
duration_secs: 120
concurrency: 20
warmup_secs: 10
layers:
  - name: gfs_TMP
    style: temperature
    weight: 1.0
  - name: gfs_WIND_BARBS
    weight: 1.0
  - name: gfs_PRMSL
    style: temperature_isolines
    weight: 1.0
tile_selection:
  type: random
  zoom_range: [4, 8]
```

---

#### Task 2.9: Integration & Documentation (Est: 1 hour)
**Goal**: Integrate with project workflow and document usage

**Updates**:
- Add to `AGENTS.md`: How to run load tests
- Update `start.sh --help` to mention load testing
- Create `validation/load-test/README.md`

**README content**:
- Installation/build instructions
- Quick start guide
- Scenario configuration reference
- Interpreting results
- Tips for performance testing

---

### Total Estimated Time: ~12 hours

### Suggested Implementation Order:
1. Task 2.1 (Setup) + Task 2.2 (Config) - Get project compiling
2. Task 2.3 (Generator) - Core tile URL generation
3. Task 2.4 (Runner) - HTTP execution (most complex)
4. Task 2.5 (Metrics) - Statistics collection
5. Task 2.6 (Report) + Task 2.7 (CLI) - User interface
6. Task 2.8 (Scenarios) - Test configurations
7. Task 2.9 (Docs) - Polish and documentation

---

## Success Metrics

| Metric | Current (est.) | Target | Stretch Goal |
|--------|----------------|--------|--------------|
| P50 latency (cache miss) | ~200ms | <50ms | <20ms |
| P50 latency (cache hit) | ~10ms | <5ms | <2ms |
| Throughput (tiles/sec) | ~50 | >500 | >1000 |
| Cache hit rate | Variable | >80% | >95% |

---

## Questions to Resolve

1. **Dataset priority**: Start with HRRR (easiest) or MRMS (most challenging)?
2. **Load test tooling**: Build custom Rust tool or use existing (k6, wrk, vegeta)?
3. **Profiling depth**: Focus on CPU time or also memory/IO?
4. **Optimization order**: Cache layer first or rendering hot paths?

---

## References

- [MRMS Data Access](https://mrms.ncep.noaa.gov/)
- [HRRR on AWS](https://registry.opendata.aws/noaa-hrrr-pds/)
- [GOES on AWS](https://registry.opendata.aws/noaa-goes/)
- [Rust Flamegraph](https://github.com/flamegraph-rs/flamegraph)
- [Criterion.rs](https://github.com/bheisler/criterion.rs)
