# Making Our EDR Service the Fastest in the World

This document consolidates developer feedback on performance optimizations for our Rust-based OGC EDR service.

## Guiding Principle

> "Your service should behave like an index server that happens to emit weather data."

The goal: **O(request size) work, never O(dataset size).**

Most time should be spent:
- Identifying which bytes to read
- Moving those bytes once
- Never touching unnecessary data

---

## Priority 1: Critical Path Optimizations

These have the highest impact and should be implemented first.

### 1.1 Streaming Responses (Latency Killer)

**Problem:** Building entire responses in memory adds `Time_to_Construct + Time_to_Send`.

**Solution:**
- Use chunked transfer encoding - stream responses immediately
- In Axum/Actix-web, return a `Stream` body or use `Body::from_stream`
- Write JSON header, then stream data rows as they're processed
- Keeps Time-To-First-Byte (TTFB) extremely low

```rust
async fn stream_coverage_json(data: DataStream) -> impl IntoResponse {
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(Bytes::from(r#"{"type":"Coverage","domain":{"#));
        // yield chunks as data becomes available
    };
    Body::from_stream(stream)
}
```

### 1.2 Zero-Copy I/O and Memory Mapping

**Problem:** I/O latency and copying data between kernel/user space is the biggest bottleneck.

**Solutions:**
- **Memory-map Zarr chunks** using `memmap2` crate for local NVMe storage
- **Use `object_store` crate** for cloud storage (S3/GCS) with non-blocking ranged GETs
- **Buffer pooling** with `bytes::Bytes` to prevent allocator thrash under high concurrency
- Use `Cow<'_, str>` and `&[u8]` with serde to avoid allocations in request/response handling

### 1.3 Chunk-Optimal Zarr Access

**Problem:** EDR implementations often decode entire chunks unnecessarily.

**Solutions:**

1. **Align chunking with query patterns:**
   ```
   # For global models (area/cube queries):
   chunks = [time=1, y=256, x=256]
   
   # For point-heavy use (position/trajectory):
   chunks = [time=24, y=64, x=64]
   ```

2. **Chunk pruning before decoding:**
   - Compute chunk index intersection first
   - Only read intersecting chunks
   - Slice inside chunk only after decode

3. **Key metric:** Measure "chunks touched per request" - this matters more than raw I/O.

---

## Priority 2: Processing Optimizations

### 2.1 SIMD Optimizations

Apply SIMD acceleration to:
- **JSON parsing:** Use `simd-json` for request parsing
- **JSON serialization:** Use `sonic-rs` or `simd-json` (2-5x faster than serde_json)
- **Decompression:** Ensure Blosc/Zstd compiled with AVX2 support
- **Coordinate transforms:**

```rust
use std::simd::f64x4;

fn transform_coords_simd(coords: &mut [f64], scale: f64, offset: f64) {
    let scale_v = f64x4::splat(scale);
    let offset_v = f64x4::splat(offset);
    for chunk in coords.chunks_exact_mut(4) {
        let v = f64x4::from_slice(chunk);
        let result = v * scale_v + offset_v;
        result.copy_to_slice(chunk);
    }
}
```

### 2.2 Parallel Processing

**Architecture:** Keep Tokio for I/O, offload CPU-bound work to Rayon.

```rust
use rayon::prelude::*;

let results: Vec<_> = parameters
    .par_iter()
    .map(|param| fetch_and_process(param, &bbox, time))
    .collect();
```

**Parallelization opportunities:**
- Multiple variables
- Multiple time steps  
- Multiple chunks (embarrassingly parallel)

**Pattern:** Parallel per variable, sequential per chunk within variable.

### 2.3 Lock-Free Data Access

All read-only shared state should be `Arc<SomethingImmutable>`:
- Zarr metadata
- Grid definitions
- Index mappings

**No locks on hot paths.**

---

## Priority 3: Spatial & Query Optimizations

### 3.1 Spatial Indexing

- **R-tree** (`rstar` crate) over Zarr chunk bounding boxes for query planning
- **For structured grids:** Direct index math beats trees every time
- **Precompute:** Grid cell lookups, lat/lon to index mappings

### 3.2 Coordinate Transform Optimization

- Store native model grid + precomputed lat/lon lookup arrays
- Cache transform objects aggressively
- Use precompiled transform pipelines when reprojection unavoidable

### 3.3 Request Validation

Validate before any I/O:
- Query geometry
- CRS
- Time range
- Variable existence

**Fail fast = save I/O.**

---

## Priority 4: Caching Strategy

### Multi-Level Caching

| Level | What to Cache | Implementation |
|-------|---------------|----------------|
| HTTP | Full responses | Cache-Control headers, reverse proxy |
| Application | Query results | `moka` or `quick_cache` |
| Computation | Interpolation weights, grid indices | Per-thread LRU |

```rust
use moka::future::Cache;

let cache: Cache<QueryHash, Arc<CoverageJson>> = Cache::builder()
    .max_capacity(10_000)
    .time_to_live(Duration::from_secs(300))
    .build();
```

### Cache Design

- **Good cache:** `(variable, time, tile)` results, common EDR queries
- **Bad cache:** Entire datasets, entire chunks blindly
- **Metadata:** Serve `/collections`, `/coverage`, `/instances` entirely from memory
- **Normalize queries** before hashing; consider snapping coordinates to grid

---

## Priority 5: HTTP & Serialization

### 5.1 HTTP Optimization

- **HTTP/2** for multiplexing (reduces connection overhead)
- Consider **HTTP/3/QUIC** for latency-critical scenarios
- Long-lived connections with keep-alive tuning
- Use `tower` middleware for timeouts, concurrency limits, load shedding

### 5.2 Compression

- Use **zstd** (better ratio and speed than gzip)
- Pre-compress static metadata responses
- Consider delta encoding for numeric arrays before compression

### 5.3 Binary Formats

OGC EDR allows multiple encodings. Performance ranking:
1. Zarr references (fastest)
2. NetCDF (fast)
3. MessagePack/CBOR (faster than JSON)
4. JSON (slow)
5. GeoJSON (slowest)

Consider offering Arrow IPC or FlatGeobuf for 10-50x size reduction.

---

## Priority 6: Advanced Optimizations

### 6.1 Chunk-Level Computation

Instead of: Extract values → compute stats/interpolation

Do: Compute inside chunk → return only final values

Applies to: point interpolation, time aggregation, vertical slicing

**Reduces:** Memory movement, serialization cost, cache pressure

### 6.2 Request Coalescing

Batch similar concurrent requests:
```rust
// Multiple requests for temperature at nearby points
// → single fetch of encompassing tile, then split results
```

### 6.3 Query Parsing

Avoid regex in hot paths:
```rust
fn parse_point(s: &str) -> Option<(f64, f64)> {
    let s = s.strip_prefix("POINT(")?.strip_suffix(")")?;
    let (x, y) = s.split_once(' ')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}
```

### 6.4 Memory Optimization

- Use `bumpalo` arena allocators for request-scoped allocations
- Reuse buffers via object pooling in hot paths

---

## Benchmarking Requirements

To be "world's fastest", measure adversarially:

**Metrics:**
- p50 / p95 / p99 latency
- Chunks touched per request
- Bytes read vs bytes returned
- Time-to-first-byte

**Test Scenarios:**
- 1 point, 10k time steps
- 10k points, 1 timestep
- Many concurrent small queries
- One massive cube query

**Tools:**
- `cargo flamegraph` / `samply` for CPU profiling
- `dhat` / `bytehound` for heap profiling
- `criterion` for microbenchmarks
- `tracing` with `tracing-timing` for span analysis

---

## Quick Wins Checklist

Start with these for immediate impact:

- [ ] Switch to `sonic-rs` or `simd-json` for serialization
- [ ] Add response streaming for large queries
- [ ] Implement application-level caching with `moka`
- [ ] Memory-map local Zarr chunks with `memmap2`
- [ ] Profile with flamegraphs to find actual bottlenecks
- [ ] Validate requests before touching any data
- [ ] Serve metadata endpoints from memory

---

## Recommended Crates

| Purpose | Crate |
|---------|-------|
| Zarr access | `zarrs` |
| Memory mapping | `memmap2` |
| Cloud storage | `object_store` |
| Caching | `moka`, `quick_cache` |
| Spatial index | `rstar` |
| JSON (fast) | `simd-json`, `sonic-rs` |
| Parallelism | `rayon` |
| Async runtime | `tokio` |
| HTTP framework | `axum`, `actix-web` |
| Compression | `zstd` |
| Profiling | `tracing`, `criterion` |
