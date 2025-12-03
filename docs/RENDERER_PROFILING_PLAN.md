# Renderer Crate Profiling Plan

This document outlines a comprehensive strategy for profiling the `renderer` crate and the full WMS/WMTS rendering pipeline.

## Overview

The renderer crate contains several performance-critical components:
- **Gradient rendering** (`gradient.rs`) - Color mapping and grid resampling
- **Wind barb rendering** (`barbs.rs`) - SVG rasterization and compositing
- **Contour generation** (`contour.rs`) - Marching squares algorithm
- **PNG encoding** (`png.rs`) - zlib compression

We'll use multiple profiling approaches to get both micro-level function performance and macro-level pipeline analysis.

---

## 1. Profiling Tools for Rust

### 1.1 Criterion (Micro-benchmarks)
**Purpose**: Precise, statistical benchmarking of individual functions

**Best for**:
- Comparing algorithm implementations
- Measuring impact of optimizations
- Regression testing performance

**Usage**:
```bash
# Run all benchmarks
cargo bench --package renderer

# Run specific benchmark group
cargo bench --package renderer -- gradient

# Generate HTML report
cargo bench --package renderer -- --save-baseline main
```

### 1.2 Flamegraph (CPU Profiling)
**Purpose**: Visualize where CPU time is spent across the call stack

**Best for**:
- Finding unexpected hotspots
- Understanding call relationships
- Identifying optimization opportunities

**Requirements**:
```bash
# Install flamegraph (requires perf on Linux)
cargo install flamegraph

# Linux: Install perf
sudo apt install linux-tools-common linux-tools-generic
```

**Usage**:
```bash
# Profile the WMS API server
./scripts/profile_flamegraph.sh

# Profile a specific benchmark
cargo flamegraph --package renderer --bench render_benchmarks
```

### 1.3 Tracing Spans (Request Pipeline)
**Purpose**: Measure time spent in each stage of a real request

**Best for**:
- End-to-end request analysis
- Identifying slow stages in production
- Correlating with logs

**Already implemented**: The WMS API has tracing spans (see `PROFILING_IMPLEMENTATION.md`). We'll add more granular spans to the renderer crate.

### 1.4 perf + perf-record (Low-level CPU Analysis)
**Purpose**: Hardware performance counter analysis

**Best for**:
- Cache miss analysis
- Branch prediction issues
- Instruction-level optimization

**Usage**:
```bash
# Record with hardware counters
perf record -g --call-graph dwarf ./target/release/wms-api

# Analyze results
perf report
```

### 1.5 Samply (Modern Sampling Profiler)
**Purpose**: Alternative to perf with better visualization

**Best for**:
- macOS and Linux
- Async Rust code
- Interactive exploration

```bash
cargo install samply
samply record ./target/release/wms-api
```

---

## 2. Renderer Functions to Profile

### 2.1 Gradient Module (`gradient.rs`)

| Function | Priority | Description |
|----------|----------|-------------|
| `resample_grid()` | **HIGH** | Bilinear interpolation - O(n) per pixel |
| `subset_grid()` | MEDIUM | Grid subsetting for bbox |
| `render_grid()` | **HIGH** | Main rendering loop |
| `render_temperature()` | MEDIUM | Temperature-specific color mapping |
| `temperature_color()` | LOW | Single color lookup |
| `interpolate_color()` | LOW | Linear interpolation |

### 2.2 Wind Barbs Module (`barbs.rs`)

| Function | Priority | Description |
|----------|----------|-------------|
| `render_wind_barbs_aligned()` | **HIGH** | Main barb rendering |
| `render_barb_at_position()` | **HIGH** | SVG parsing + rasterization per barb |
| `composite_barb_onto_canvas()` | MEDIUM | Alpha blending |
| `calculate_barb_positions_geographic()` | LOW | Position calculation |
| `uv_to_speed_direction()` | LOW | Trig calculations |

### 2.3 Contour Module (`contour.rs`)

| Function | Priority | Description |
|----------|----------|-------------|
| `march_squares()` | **HIGH** | Core algorithm - O(n) for grid |
| `connect_segments()` | MEDIUM | Segment linkage |
| `smooth_contour()` | MEDIUM | Chaikin smoothing |
| `render_contours_to_canvas()` | **HIGH** | tiny-skia path drawing |

### 2.4 PNG Module (`png.rs`)

| Function | Priority | Description |
|----------|----------|-------------|
| `create_png()` | **HIGH** | Full PNG encoding |
| `deflate_idat()` | **HIGH** | zlib compression (likely bottleneck) |

---

## 3. Benchmark Test Data

Create standard test datasets for consistent benchmarking:

### 3.1 Grid Sizes
- **Small**: 256x256 (single tile)
- **Medium**: 1440x721 (GFS global)
- **Large**: 7000x3500 (MRMS CONUS)
- **XL**: 10000x5000 (GOES full disk)

### 3.2 Test Data Generation
```rust
// Generate test grid with realistic weather patterns
fn generate_temperature_grid(width: usize, height: usize) -> Vec<f32> {
    let mut data = vec![0.0; width * height];
    for y in 0..height {
        for x in 0..width {
            // Simulate temperature gradient with some noise
            let lat_factor = (y as f32 / height as f32) * 50.0 - 25.0;
            let noise = (x as f32 * 0.1).sin() * 5.0;
            data[y * width + x] = 273.15 + lat_factor + noise; // Kelvin
        }
    }
    data
}

// Generate U/V wind components
fn generate_wind_components(width: usize, height: usize) -> (Vec<f32>, Vec<f32>) {
    // ... realistic wind patterns
}
```

---

## 4. Benchmark Scenarios

### 4.1 Individual Function Benchmarks
```rust
// crates/renderer/benches/render_benchmarks.rs
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};

fn benchmark_resample_grid(c: &mut Criterion) {
    let mut group = c.benchmark_group("resample_grid");
    
    for (src_size, dst_size) in [
        ((1440, 721), (256, 256)),    // GFS to tile
        ((7000, 3500), (256, 256)),   // MRMS to tile
        ((256, 256), (512, 512)),     // Upscale
    ] {
        let data = generate_test_grid(src_size.0, src_size.1);
        
        group.throughput(Throughput::Elements((dst_size.0 * dst_size.1) as u64));
        group.bench_with_input(
            BenchmarkId::new("bilinear", format!("{}x{}_to_{}x{}", 
                src_size.0, src_size.1, dst_size.0, dst_size.1)),
            &data,
            |b, data| {
                b.iter(|| {
                    gradient::resample_grid(data, src_size.0, src_size.1, dst_size.0, dst_size.1)
                });
            },
        );
    }
    group.finish();
}
```

### 4.2 Pipeline Benchmarks
```rust
fn benchmark_full_render_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");
    
    // Temperature gradient render
    let temp_data = generate_temperature_grid(1440, 721);
    group.bench_function("temperature_256x256", |b| {
        b.iter(|| {
            let resampled = gradient::resample_grid(&temp_data, 1440, 721, 256, 256);
            let rgba = gradient::render_temperature(&resampled, 256, 256, -40.0, 40.0);
            png::create_png(&rgba, 256, 256)
        });
    });
    
    group.finish();
}
```

### 4.3 Comparison Benchmarks
```rust
fn benchmark_png_compression_levels(c: &mut Criterion) {
    let mut group = c.benchmark_group("png_compression");
    
    let rgba = generate_rgba_data(256, 256);
    
    // Compare compression strategies
    // (Requires modifying png.rs to accept compression level)
    for level in [1, 6, 9] {
        group.bench_with_input(
            BenchmarkId::new("zlib_level", level),
            &level,
            |b, &level| {
                b.iter(|| create_png_with_level(&rgba, 256, 256, level));
            },
        );
    }
    group.finish();
}
```

---

## 5. Scripts

### 5.1 Run All Benchmarks
```bash
#!/bin/bash
# scripts/run_benchmarks.sh

set -e

echo "=== Running Renderer Benchmarks ==="

# Build in release mode first
cargo build --release --package renderer

# Run criterion benchmarks
cargo bench --package renderer -- --save-baseline current

# Compare with baseline if it exists
if [ -d "target/criterion/baseline" ]; then
    echo "Comparing with baseline..."
    cargo bench --package renderer -- --baseline main
fi

echo "Benchmark report: target/criterion/report/index.html"
```

### 5.2 Generate Flamegraph
```bash
#!/bin/bash
# scripts/profile_flamegraph.sh

set -e

DURATION=${1:-30}
SCENARIO=${2:-"quick"}

echo "=== Generating Flamegraph ==="
echo "Duration: ${DURATION}s"
echo "Scenario: ${SCENARIO}"

# Build with debug symbols but optimized
RUSTFLAGS="-C debuginfo=2" cargo build --release --package wms-api

# Start the server in background
./target/release/wms-api &
WMS_PID=$!
sleep 3

# Start profiling
echo "Recording performance data..."
if command -v flamegraph &> /dev/null; then
    # Use cargo flamegraph wrapper
    timeout ${DURATION}s ./scripts/run_load_test.sh ${SCENARIO} &
    LOAD_PID=$!
    
    sudo perf record -g --call-graph dwarf -p $WMS_PID -o perf.data -- sleep ${DURATION}
    
    wait $LOAD_PID 2>/dev/null || true
    
    # Generate flamegraph
    sudo perf script -i perf.data | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg
    echo "Flamegraph saved to: flamegraph.svg"
else
    echo "flamegraph not installed. Install with: cargo install flamegraph"
fi

# Cleanup
kill $WMS_PID 2>/dev/null || true
rm -f perf.data perf.data.old

echo "Done!"
```

### 5.3 Profile Specific Function
```bash
#!/bin/bash
# scripts/profile_function.sh

FUNCTION=${1:-"resample_grid"}

echo "=== Profiling function: ${FUNCTION} ==="

# Build benchmark binary
cargo build --release --package renderer --bench render_benchmarks

# Profile with perf
sudo perf record -g --call-graph dwarf \
    ./target/release/deps/render_benchmarks-* --bench "${FUNCTION}" --profile-time 10

# Generate report
sudo perf report --no-children --sort comm,dso,symbol
```

### 5.4 Request Pipeline Analysis
```bash
#!/bin/bash
# scripts/profile_request_pipeline.sh

echo "=== Request Pipeline Profiling ==="

# Enable detailed tracing
export RUST_LOG="wms_api=debug,renderer=debug"

# Start server with tracing
cargo run --release --package wms-api 2>&1 | tee pipeline.log &
WMS_PID=$!
sleep 3

# Make sample requests
for tile in "5/10/12" "6/20/25" "7/40/50"; do
    echo "Requesting tile: ${tile}"
    curl -s -o /dev/null -w "Time: %{time_total}s\n" \
        "http://localhost:8080/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=gfs_TMP&STYLE=temperature&FORMAT=image/png&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=${tile%%/*}&TILEROW=${tile#*/}&TILECOL=${tile##*/}"
done

# Extract timing info from logs
echo ""
echo "=== Pipeline Breakdown ==="
grep -E "(catalog_lookup|load_grid_data|resample|render|png_encode)" pipeline.log | tail -20

# Cleanup
kill $WMS_PID 2>/dev/null || true
rm -f pipeline.log
```

---

## 6. Implementation Steps

### Phase 1: Criterion Benchmarks (2-3 hours)
1. Add `criterion` to `renderer/Cargo.toml` dev-dependencies
2. Create `benches/render_benchmarks.rs` with:
   - Grid generation helpers
   - Resample benchmarks
   - Render benchmarks
   - PNG encoding benchmarks
3. Create `benches/barbs_benchmarks.rs` for wind barb profiling
4. Create `benches/contour_benchmarks.rs` for contour profiling

### Phase 2: Profiling Scripts (1-2 hours)
1. Create `scripts/run_benchmarks.sh`
2. Create `scripts/profile_flamegraph.sh`
3. Create `scripts/profile_function.sh`
4. Create `scripts/profile_request_pipeline.sh`

### Phase 3: Enhanced Tracing (1-2 hours)
1. Add fine-grained tracing spans to renderer functions
2. Add timing macros for function-level profiling
3. Update wms-api to collect renderer timing

### Phase 4: Continuous Benchmarking (Optional)
1. Add benchmark run to CI
2. Track performance regressions
3. Create performance dashboard

---

## 7. Expected Optimization Opportunities

Based on code analysis, likely bottlenecks are:

### 7.1 Resample Grid (gradient.rs:93-140)
- **Current**: O(n) bilinear interpolation per output pixel
- **Opportunities**:
  - SIMD vectorization for bulk interpolation
  - Parallel processing with rayon
  - Pre-computed lookup tables for common transforms

### 7.2 Wind Barb Rendering (barbs.rs:382-460)
- **Current**: SVG parse → rasterize → composite per barb
- **Opportunities**:
  - Pre-rasterize barb sprites at common sizes (sprite cache)
  - Batch composite operations
  - Reduce SVG complexity

### 7.3 PNG Encoding (png.rs:57-73)
- **Current**: Row-by-row zlib compression
- **Opportunities**:
  - Use faster compression level (already using `Compression::fast()`)
  - Parallel row filtering
  - Consider using `png` crate with optimized encoder

### 7.4 Contour Generation (contour.rs:87-128)
- **Current**: Sequential marching squares
- **Opportunities**:
  - Parallel row processing
  - Spatial indexing for segment connection
  - Skip empty regions

---

## 8. Metrics to Track

| Metric | Target | Current (est.) |
|--------|--------|----------------|
| `resample_grid` 256x256 | <5ms | ~15ms |
| `render_temperature` 256x256 | <2ms | ~5ms |
| `create_png` 256x256 | <10ms | ~20ms |
| `render_wind_barbs` tile | <50ms | ~100ms |
| `march_squares` 256x256 | <10ms | ~30ms |
| Full tile render | <50ms | ~150ms |

---

## 9. Running the Profiling

### Quick Start
```bash
# 1. Build and run benchmarks
cargo bench --package renderer

# 2. Generate flamegraph during load test
./scripts/profile_flamegraph.sh 30 cold_cache

# 3. Analyze specific function
./scripts/profile_function.sh resample_grid

# 4. View pipeline breakdown
./scripts/profile_request_pipeline.sh
```

### Full Profiling Session
```bash
# 1. Establish baseline
cargo bench --package renderer -- --save-baseline baseline

# 2. Run load test with profiling
./scripts/profile_flamegraph.sh 60 stress

# 3. Analyze and make optimizations
# ... (edit code)

# 4. Compare with baseline
cargo bench --package renderer -- --baseline baseline

# 5. Repeat until targets met
```

---

## References

- [Criterion User Guide](https://bheisler.github.io/criterion.rs/book/)
- [Flamegraph](https://github.com/flamegraph-rs/flamegraph)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [PROFILING_IMPLEMENTATION.md](PROFILING_IMPLEMENTATION.md) - Existing metrics setup
