# Benchmarking

Performance testing and profiling for Weather WMS.

## Benchmarking Tools

### Criterion (Micro-benchmarks)

```bash
# Install criterion
# Already in Cargo.toml dev-dependencies

# Run all benchmarks
cargo bench --workspace

# Run specific benchmark
cargo bench -p renderer -- gradient

# Save baseline
cargo bench -- --save-baseline before

# Compare with baseline
cargo bench -- --baseline before
```

### Example Benchmark

```rust
// crates/renderer/benches/rendering_benchmark.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use renderer::{Renderer, Style};

fn gradient_rendering(c: &mut Criterion) {
    let mut group = c.benchmark_group("gradient");
    
    for size in [256, 512, 1024].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            size,
            |b, &size| {
                let grid = create_test_grid(size, size);
                let style = Style::default_temperature();
                let renderer = Renderer::new(style);
                
                b.iter(|| {
                    renderer.render(black_box(&grid), size, size)
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(benches, gradient_rendering);
criterion_main!(benches);
```

## Profiling

### Flamegraph

```bash
# Install flamegraph
cargo install flamegraph

# Generate flamegraph for service
sudo cargo flamegraph --bin wms-api

# Open flamegraph.svg in browser
open flamegraph.svg
```

### Using perf (Linux)

```bash
# Record performance data
perf record -F 99 -g target/release/wms-api

# Generate report
perf report

# Generate flamegraph
perf script | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg
```

### Using Instruments (macOS)

```bash
# Build with symbols
cargo build --release

# Open in Instruments
instruments -t "Time Profiler" target/release/wms-api
```

## Load Testing

### Built-in Load Test Suite

```bash
cd validation/load-test

# Run all scenarios
./run_all_load_tests.sh

# Results in: validation/load-test/results/
```

### Load Test Scenarios

```yaml
# scenarios/cache_test.yaml
name: "Cache Performance Test"
duration: 60
concurrent_users: 20

requests:
  # Request same tile repeatedly (tests cache)
  - name: "Cached Tile"
    weight: 100
    endpoint: "/tiles/gfs_TMP_2m/temperature/4/3/5.png"
```

```yaml
# scenarios/realistic.yaml
name: "Realistic Traffic"
duration: 300
concurrent_users: 50

requests:
  # Mix of different layers and zoom levels
  - name: "Temperature"
    weight: 40
    endpoint: "/tiles/gfs_TMP_2m/temperature/{{random_int 0 8}}/{{random_int 0 255}}/{{random_int 0 255}}.png"
  
  - name: "Wind"
    weight: 30
    endpoint: "/tiles/gfs_UGRD_10m/wind/{{random_int 0 8}}/{{random_int 0 255}}/{{random_int 0 255}}.png"
  
  - name: "GetCapabilities"
    weight: 5
    endpoint: "/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0"
```

### Analyzing Results

```bash
# View HTML report
open validation/load-test/visualize.html

# Or parse JSON results
jq '.summary' validation/load-test/results/realistic_*.json
```

## Performance Metrics

### Key Metrics to Track

| Metric | Target | Critical |
|--------|--------|----------|
| Request latency (p50) | <20ms | >100ms |
| Request latency (p99) | <100ms | >500ms |
| Throughput | >500 req/s | <100 req/s |
| Cache hit rate | >85% | <70% |
| Memory usage | <4GB | >8GB |
| CPU usage | <70% | >90% |

### Renderer Benchmarks

The renderer crate has comprehensive benchmarks for all rendering operations:

```bash
# All renderer benchmarks
cargo bench --package renderer

# Specific benchmark groups
cargo bench --package renderer -- resample_grid     # Grid resampling
cargo bench --package renderer -- style_rendering   # Color mapping
cargo bench --package renderer -- png_encoding      # PNG creation
cargo bench --package renderer -- precomputed       # Pre-computed palette
cargo bench --package renderer -- full_pipeline     # End-to-end
```

#### Current Performance Baselines

**Full Pipeline (Resample → Render → PNG)**:

| Pipeline | 256×256 | 512×512 | Notes |
|----------|---------|---------|-------|
| RGBA | 1.54 ms | 5.66 ms | Traditional approach |
| **Pre-computed** | **430 µs** | **1.46 ms** | **3.6-4x faster** |

**PNG Encoding Only**:

| Method | 256×256 | 512×512 | 1024×1024 |
|--------|---------|---------|-----------|
| RGBA direct | 87 µs | 672 µs | 2.97 ms |
| Auto extract | 349 µs | 803 µs | 3.56 ms |
| **Pre-computed** | **22 µs** | **63 µs** | **210 µs** |

**File Size**:

| Format | 256×256 | 512×512 | Savings |
|--------|---------|---------|---------|
| RGBA PNG | 6.4 KB | 18.4 KB | - |
| Indexed PNG | 4.0 KB | 10.6 KB | ~40% |

### Baseline Performance

Run baseline tests to track improvements/regressions:

```bash
# Save current performance
cargo bench -- --save-baseline v1.0.0

# After changes
cargo bench -- --baseline v1.0.0

# Criterion will show % change
```

## Memory Profiling

### Valgrind (Linux)

```bash
# Install valgrind
sudo apt-get install valgrind

# Run with massif
valgrind --tool=massif target/release/wms-api

# Visualize
ms_print massif.out.* | less
```

### Heaptrack (Linux)

```bash
# Install heaptrack
sudo apt-get install heaptrack

# Profile
heaptrack target/release/wms-api

# Analyze
heaptrack_gui heaptrack.wms-api.*.gz
```

### Instruments (macOS)

```bash
# Use Allocations instrument
instruments -t Allocations target/release/wms-api
```

## CPU Profiling

### Sampling Profiler

```bash
# Linux: perf
sudo perf record -F 99 -g -p $(pgrep wms-api)
sudo perf report

# macOS: sample
sample wms-api 10 -f sample.txt
```

### Continuous Profiling

```rust
// Enable profiling in production
use pprof::ProfilerGuard;

#[tokio::main]
async fn main() {
    let guard = ProfilerGuard::new(100).unwrap();
    
    // Run application
    run_server().await;
    
    // Save profile
    if let Ok(report) = guard.report().build() {
        let file = File::create("flamegraph.svg").unwrap();
        report.flamegraph(file).unwrap();
    }
}
```

## Stress Testing

### High Concurrency

```bash
# 1000 concurrent connections
ab -n 10000 -c 1000 \
  "http://localhost:8080/tiles/gfs_TMP_2m/temperature/4/3/5.png"
```

### Sustained Load

```bash
# Run for 1 hour
cd validation/load-test
cargo run --release -- \
  --duration 3600 \
  --concurrent 100 \
  scenarios/realistic.yaml
```

### Memory Stress

```bash
# Clear caches and stress test
curl -X POST http://localhost:8080/api/cache/clear

# Then hammer with unique requests (cache misses)
for i in {0..1000}; do
  curl "http://localhost:8080/tiles/gfs_TMP_2m/temperature/8/$((RANDOM%256))/$((RANDOM%256)).png" &
done
```

## Optimization Workflow

1. **Measure**: Establish baseline with benchmarks
2. **Profile**: Identify bottlenecks with flamegraph
3. **Optimize**: Make targeted improvements
4. **Verify**: Re-run benchmarks to confirm improvement
5. **Repeat**: Focus on next bottleneck

### Example: Optimizing Tile Rendering

```bash
# 1. Baseline
cargo bench -p renderer -- gradient > before.txt

# 2. Profile
sudo cargo flamegraph --bin wms-api
# Identify slow function: interpolate_grid()

# 3. Optimize (add SIMD, better algorithm, etc.)
# ... make changes ...

# 4. Verify
cargo bench -p renderer -- gradient > after.txt

# 5. Compare
# before.txt: 25.3ms
# after.txt: 10.1ms
# Improvement: 60% faster!
```

### Recent Optimizations (December 2024)

Three major optimizations were implemented:

#### 1. Parallel Chunk Fetching
- **Change**: Use `futures::join_all` for concurrent Zarr chunk reads
- **Impact**: 4x faster for multi-chunk requests
- **Location**: `crates/grid-processor/src/processor/zarr.rs`

#### 2. Parallel Pixel Rendering
- **Change**: Use rayon `par_chunks_mut` for row-parallel color mapping
- **Impact**: Near-linear CPU scaling
- **Location**: `crates/renderer/src/style.rs`, `crates/renderer/src/gradient.rs`

#### 3. Pre-computed Palette
- **Change**: Pre-compute color palettes at style load time
- **Impact**: 3-4x faster full pipeline, 40% smaller files
- **Location**: `crates/renderer/src/style.rs`, `crates/renderer/src/png.rs`

#### Key APIs Added

```rust
// Pre-compute palette (once per style)
let palette = style.compute_palette()?;

// Fast indexed rendering (1 byte/pixel)
let indices = apply_style_gradient_indexed(&data, w, h, &palette, &style);

// Fast indexed PNG (no palette extraction)
let png = create_png_from_precomputed(&indices, w, h, &palette)?;
```

## Continuous Performance Testing

### GitHub Actions

```yaml
name: Benchmarks

on: [push]

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
    - uses: actions/checkout@v4
    
    - name: Install Rust
      uses: actions-rs/toolchain@v1
      with:
        toolchain: stable
    
    - name: Run benchmarks
      run: cargo bench --workspace | tee output.txt
    
    - name: Store benchmark result
      uses: benchmark-action/github-action-benchmark@v1
      with:
        tool: 'cargo'
        output-file-path: output.txt
        github-token: ${{ secrets.GITHUB_TOKEN }}
        auto-push: true
```

## Performance Regression Detection

### Automated Alerts

```bash
# Set thresholds in benchmark
cargo bench -- --baseline main

# Fail if regression > 10%
if [ "$(jq '.mean.estimate' target/criterion/*/new/estimates.json)" -gt 1.1 ]; then
  echo "Performance regression detected!"
  exit 1
fi
```

## Next Steps

- [Testing](./testing.md) - Functional testing
- [Contributing](./contributing.md) - Submit optimizations
- [Architecture: Caching](../architecture/caching.md) - Cache tuning
