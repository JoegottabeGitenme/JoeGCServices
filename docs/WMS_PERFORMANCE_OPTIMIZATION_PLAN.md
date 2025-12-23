# WMS Performance Optimization Plan

**Date**: December 23, 2024  
**Status**: Phase 3b Complete - 3.6x Pipeline Speedup Achieved  
**Goal**: Improve WMS/WMTS rendering throughput and latency through targeted optimizations

---

## Executive Summary

This document outlines potential performance optimizations for the WMS rendering pipeline, based on analysis of the current implementation and industry best practices. The optimizations focus on:

1. **Parallelization** - Utilize multiple cores for I/O and computation
2. **Encoding efficiency** - Reduce PNG encoding overhead
3. **Memory management** - Reduce allocation pressure
4. **Format alternatives** - Consider WebP for modern clients

---

## Current State Analysis

| Optimization Area | Current Status | Implementation |
|-------------------|----------------|----------------|
| Parallel chunk fetching | ✅ DONE | `futures::future::join_all` in `zarr.rs:604-616` |
| Parallel pixel rendering | ✅ DONE | `rayon::par_chunks_mut` in `gradient.rs` and `style.rs` |
| Paletted PNG (8-bit) | ✅ DONE | `create_png_indexed()`, `create_png_auto()` in `png.rs` |
| Pre-computed palette | ✅ DONE | `PrecomputedPalette`, `apply_style_gradient_indexed()` in `style.rs` |
| WebP support | ❌ NOT DONE | URL parsing exists, no actual encoding |
| Buffer reuse / pooling | ❌ NOT DONE | Fresh `Vec` allocations per request |
| SIMD color ramping | ❌ NOT DONE | Scalar f32 arithmetic |
| Meta-tiling (WMTS) | ❌ NOT DONE | Each tile rendered independently |
| LRU chunk caching | ✅ DONE | `ChunkCache` with memory bounds |

### Key Files

| Component | Location | Purpose |
|-----------|----------|---------|
| Chunk fetching | `crates/grid-processor/src/processor/zarr.rs:604-609` | Zarr chunk I/O |
| Pixel rendering | `crates/renderer/src/gradient.rs:256-284` | Color mapping loops |
| PNG encoding | `crates/renderer/src/png.rs` | Custom PNG encoder (RGBA + indexed) |
| Pre-computed palette | `crates/renderer/src/style.rs` | `PrecomputedPalette`, `compute_palette()` |
| Indexed rendering | `crates/renderer/src/style.rs` | `apply_style_gradient_indexed()` |
| Color interpolation | `crates/renderer/src/gradient.rs:216-226` | Linear color blending |
| Chunk cache | `crates/grid-processor/src/cache/chunk_cache.rs` | LRU decompressed chunks |

---

## Optimization Details

### 1. Parallel Chunk Fetching

**Priority**: High  
**Effort**: Low (1-2 hours)  
**Impact**: Significant latency reduction when multiple chunks needed

#### Problem

When a WMS request spans multiple Zarr chunks, we currently fetch them sequentially:

```rust
// crates/grid-processor/src/processor/zarr.rs:604-609
// Note: For better performance, we could use parallel futures here
let mut chunk_data = Vec::with_capacity(chunks.len());
for (cx, cy) in &chunks {
    chunk_data.push(self.read_chunk(*cx, *cy).await?);
}
```

#### Impact Analysis

| Scenario | Sequential | Parallel |
|----------|-----------|----------|
| 4 chunks @ 50ms latency each | 200ms | 50ms |
| 9 chunks @ 50ms latency each | 450ms | 50ms |

#### Solution

Replace sequential loop with `futures::future::join_all`:

```rust
use futures::future::join_all;

let chunk_futures: Vec<_> = chunks
    .iter()
    .map(|(cx, cy)| self.read_chunk(*cx, *cy))
    .collect();

let chunk_results = join_all(chunk_futures).await;
let chunk_data: Result<Vec<_>, _> = chunk_results.into_iter().collect();
let chunk_data = chunk_data?;
```

#### Considerations

- Error handling: If one chunk fails, all fail (acceptable for our use case)
- Memory: All chunks loaded simultaneously (already the case post-assembly)
- Network saturation: Could overwhelm MinIO with many concurrent requests (mitigated by chunk cache)

---

### 2. Parallel Pixel Rendering (rayon)

**Priority**: High  
**Effort**: Medium (2-4 hours)  
**Impact**: Utilize all CPU cores for pixel coloring

#### Problem

A 256×256 tile has 65,536 pixels. Currently we color them sequentially:

```rust
// crates/renderer/src/gradient.rs:256-284
for y in 0..height {
    for x in 0..width {
        let idx = y * width + x;
        // ... color mapping for each pixel
    }
}
```

#### Solution

Add `rayon` dependency and parallelize:

```rust
use rayon::prelude::*;

// Process rows in parallel
pixels
    .par_chunks_mut(width * 4)
    .enumerate()
    .for_each(|(y, row)| {
        for x in 0..width {
            let data_idx = y * width + x;
            let pixel_idx = x * 4;
            let value = data[data_idx];
            let color = color_fn(normalize(value));
            row[pixel_idx] = color.r;
            row[pixel_idx + 1] = color.g;
            row[pixel_idx + 2] = color.b;
            row[pixel_idx + 3] = color.a;
        }
    });
```

#### Where to Apply

1. `crates/renderer/src/gradient.rs` - `render_grid()` function
2. `crates/renderer/src/style.rs` - `apply_style_gradient()` function
3. `services/wms-api/src/rendering/resampling.rs` - Resampling functions

#### Considerations

- Rayon has minimal overhead for work-stealing
- Row-based parallelism keeps memory access patterns cache-friendly
- May want to add threshold (e.g., only parallelize for tiles > 128×128)

---

### 3. Paletted PNG (8-bit Indexed)

**Priority**: High  
**Effort**: Medium (4-6 hours)  
**Impact**: 3-4x faster encoding, 5-10x smaller file size

#### Problem

Current PNG encoder outputs RGBA (color type 6, 4 bytes per pixel):

```rust
// crates/renderer/src/png.rs:21-22
ihdr_data.push(8); // bit depth
ihdr_data.push(6); // color type (RGBA)
```

Weather data typically uses limited color palettes (temperature scale: ~20-30 distinct colors).

#### Solution

Implement indexed PNG (color type 3):

```rust
pub fn create_indexed_png(
    pixels: &[u8],      // RGBA input
    width: usize,
    height: usize,
) -> Result<Vec<u8>, String> {
    // 1. Build palette from unique colors (max 256)
    let palette = extract_palette(pixels);
    
    if palette.len() > 256 {
        // Fall back to RGBA
        return create_png(pixels, width, height);
    }
    
    // 2. Create index buffer (1 byte per pixel instead of 4)
    let indices = map_to_indices(pixels, &palette);
    
    // 3. Write PNG with PLTE chunk
    let mut png = Vec::new();
    write_signature(&mut png);
    write_ihdr(&mut png, width, height, 8, 3); // color type 3 = indexed
    write_plte(&mut png, &palette);
    write_trns(&mut png, &palette); // transparency for alpha < 255
    write_idat(&mut png, &indices, width);
    write_iend(&mut png);
    
    Ok(png)
}
```

#### File Size Comparison

| Format | Bytes/Pixel | 256×256 Tile | Compression |
|--------|-------------|--------------|-------------|
| RGBA PNG | 4 | ~30-50 KB | Good |
| Indexed PNG | 1 + palette | ~8-15 KB | Better |
| Improvement | 4x raw | 2-5x compressed | - |

#### Considerations

- Weather visualizations typically have < 100 unique colors
- Transparent pixels (NaN values) handled via tRNS chunk
- Auto-detect: Use indexed if unique colors ≤ 256, else RGBA

---

### 4. WebP Support

**Priority**: Medium  
**Effort**: Medium (3-4 hours)  
**Impact**: Faster encoding, smaller files, modern format

#### Problem

PNG encoding is slower than necessary for web delivery. WebP is:
- Faster to encode at equivalent quality
- Smaller file size (typically 25-35% smaller than PNG)
- Supports transparency

#### Current State

URL parsing already recognizes `.webp`:
```rust
// crates/wms-protocol/src/wmts.rs:317
"webp" => "image/webp"
```

But no actual encoding exists.

#### Solution

Add `webp` crate and encode conditionally:

```rust
// Cargo.toml
webp = "0.2"

// renderer/src/webp.rs
pub fn create_webp(
    pixels: &[u8],
    width: usize,
    height: usize,
    quality: f32,  // 0-100, use ~80-90 for weather
) -> Result<Vec<u8>, String> {
    let encoder = webp::Encoder::from_rgba(pixels, width as u32, height as u32);
    let webp = encoder.encode(quality);
    Ok(webp.to_vec())
}
```

#### Considerations

- Client compatibility: Most modern browsers support WebP
- Lossless option available for exact color preservation
- Content negotiation: Check `Accept` header or URL extension

---

### 5. Buffer Reuse (Object Pooling)

**Priority**: Medium  
**Effort**: Medium (4-6 hours)  
**Impact**: Reduce allocation pressure, improve p99 latency

#### Problem

Each request allocates fresh buffers:

```rust
let mut pixels = vec![0u8; width * height * 4];  // 256KB per tile
let mut output = vec![0.0f32; dst_width * dst_height];  // 256KB
let mut png = Vec::new();  // grows during encoding
```

Under high load, allocator contention increases p99 latency.

#### Solution

Thread-local buffer pools:

```rust
use std::cell::RefCell;

thread_local! {
    static PIXEL_BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(256 * 256 * 4));
    static RESAMPLE_BUFFER: RefCell<Vec<f32>> = RefCell::new(Vec::with_capacity(256 * 256));
}

pub fn with_pixel_buffer<F, R>(width: usize, height: usize, f: F) -> R
where
    F: FnOnce(&mut [u8]) -> R,
{
    PIXEL_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        let size = width * height * 4;
        buf.resize(size, 0);
        buf.fill(0);  // Clear for transparency
        f(&mut buf[..size])
    })
}
```

#### Alternative: `crossbeam` Object Pool

```rust
use crossbeam::sync::ShardedLock;
use std::sync::Arc;

struct BufferPool {
    buffers: ShardedLock<Vec<Vec<u8>>>,
}

impl BufferPool {
    fn acquire(&self, size: usize) -> PooledBuffer { ... }
    fn release(&self, buffer: Vec<u8>) { ... }
}
```

#### Considerations

- Thread-local is simpler, works well with rayon
- Object pool better for async contexts (tokio)
- Clear buffers before reuse (security, transparency)

---

### 6. SIMD for Color Ramping

**Priority**: Low  
**Effort**: High (8-16 hours)  
**Impact**: Modest speedup for color interpolation

#### Problem

Color interpolation is scalar:

```rust
// crates/renderer/src/gradient.rs:216-226
fn interpolate_color(color1: Color, color2: Color, t: f32) -> Color {
    Color::new(
        ((color1.r as f32 * (1.0 - t)) + (color2.r as f32 * t)) as u8,
        ((color1.g as f32 * (1.0 - t)) + (color2.g as f32 * t)) as u8,
        ((color1.b as f32 * (1.0 - t)) + (color2.b as f32 * t)) as u8,
        ((color1.a as f32 * (1.0 - t)) + (color2.a as f32 * t)) as u8,
    )
}
```

#### Solution

Use `wide` crate for SIMD:

```rust
use wide::f32x8;

// Process 8 pixels at once
fn interpolate_colors_simd(
    c1_r: f32x8, c1_g: f32x8, c1_b: f32x8, c1_a: f32x8,
    c2_r: f32x8, c2_g: f32x8, c2_b: f32x8, c2_a: f32x8,
    t: f32x8,
) -> (f32x8, f32x8, f32x8, f32x8) {
    let t_inv = f32x8::splat(1.0) - t;
    (
        c1_r * t_inv + c2_r * t,
        c1_g * t_inv + c2_g * t,
        c1_b * t_inv + c2_b * t,
        c1_a * t_inv + c2_a * t,
    )
}
```

#### Considerations

- Color ramping is already fast (~1ms for 256×256)
- Complexity vs benefit may not justify
- Better ROI from parallel pixel rendering first
- May be useful if combined with bilinear interpolation

---

### 7. Meta-Tiling (WMTS)

**Priority**: Low  
**Effort**: High (8-16 hours)  
**Impact**: Reduce redundant edge pixel calculations

#### Problem

Adjacent tiles re-render overlapping edge pixels:

```
┌─────┬─────┐
│ A   │ B   │  Tiles A and B both render their shared edge
├─────┼─────┤  Same for the center cross of tiles
│ C   │ D   │
└─────┴─────┘
```

#### Solution

Render a 2×2 or 3×3 "metatile", then slice:

```rust
pub async fn render_metatile(
    z: u8, 
    x: u32, 
    y: u32,
    meta_size: usize,  // 2 for 2x2, 3 for 3x3
) -> Result<HashMap<(u32, u32), Vec<u8>>> {
    // 1. Calculate combined bbox for all tiles in metatile
    let meta_width = TILE_SIZE * meta_size;
    let meta_height = TILE_SIZE * meta_size;
    
    // 2. Render large image once
    let large_image = render_region(bbox, meta_width, meta_height).await?;
    
    // 3. Slice into individual tiles
    let mut tiles = HashMap::new();
    for dy in 0..meta_size {
        for dx in 0..meta_size {
            let tile = slice_tile(&large_image, dx, dy, TILE_SIZE);
            tiles.insert((x + dx as u32, y + dy as u32), tile);
        }
    }
    
    Ok(tiles)
}
```

#### Considerations

- Most beneficial when tiles requested in clusters (map panning)
- Adds complexity to caching (cache metatiles or individual tiles?)
- May not help for random access patterns
- Useful for cache warming scenarios

---

## Implementation Phases

### Phase 1: Quick Wins (1-2 days)

1. **Parallel chunk fetching** - COMPLETED (2024-12-23)
   - [x] Update `zarr.rs` to use `join_all`
   - [x] Test with multi-chunk requests (all 61 tests pass)
   - [ ] Benchmark latency improvement (optional)

### Phase 2: Parallel Rendering (2-3 days)

2. **Add rayon for pixel rendering** - COMPLETED (2024-12-23)
   - [x] Add `rayon` to workspace and `renderer/Cargo.toml`
   - [x] Update `render_grid()` in `gradient.rs` with `par_chunks_mut`
   - [x] Update `resample_grid()` in `gradient.rs` with `par_chunks_mut`
   - [x] Update `apply_style_gradient()` in `style.rs` with `par_chunks_mut`
   - [ ] Benchmark throughput improvement (optional)

### Phase 3: PNG Optimization (3-4 days)

qq3. **Implement paletted PNG** - COMPLETED (2024-12-23)
   - [x] Add palette extraction function (`extract_palette_sequential()`, `extract_palette_parallel()`)
   - [x] Implement indexed PNG encoding (`create_png_indexed()`)
   - [x] Add auto-detection (`create_png_auto()`)
   - [x] Benchmark file size and encoding speed
   - [x] Parallel palette extraction with rayon for large images
   
   **Result:** Indexed PNG produces **37-42% smaller files** for weather tiles with
   quantized color palettes. However, RGBA encoding is faster due to palette extraction
   overhead. Parallel extraction improves auto_weather performance by 2.5-4x.
   
   **File Size:** 256×256: 6.4KB→4.0KB (37% smaller), 512×512: 18.4KB→10.6KB (42% smaller)

### Phase 3b: Pre-computed Palette (Best of Both Worlds)

4. **Pre-computed palette rendering** - COMPLETED (2024-12-23)
   - [x] Add `PrecomputedPalette` struct to `style.rs`
   - [x] Add `StyleDefinition::compute_palette()` method
   - [x] Add `apply_style_gradient_indexed()` function (outputs palette indices directly)
   - [x] Add `create_png_from_precomputed()` function (skips palette extraction)
   - [x] Benchmark full pipeline with pre-computed palette
   
   **Key Innovation:** Pre-compute all possible colors from style JSON at load time.
   Rendering outputs 1-byte palette indices instead of 4-byte RGBA. PNG encoding
   uses the pre-computed palette directly, eliminating runtime extraction.
   
   #### PNG Encoding Benchmarks (indices already generated)
   
   | Tile Size | Pre-computed | Auto Extract | RGBA Direct | Speedup |
   |-----------|-------------|--------------|-------------|---------|
   | 256×256 | **22 µs** | 349 µs | 87 µs | **16x vs auto, 4x vs RGBA** |
   | 512×512 | **63 µs** | 803 µs | 672 µs | **13x vs auto, 11x vs RGBA** |
   | 1024×1024 | **210 µs** | 3.56 ms | 2.97 ms | **17x vs auto, 14x vs RGBA** |
   
   #### Full Pipeline Benchmarks (Resample → Render → PNG)
   
   | Tile Size | RGBA Pipeline | Auto Pipeline | Pre-computed | Speedup |
   |-----------|--------------|---------------|--------------|---------|
   | 256×256 | 1.54 ms | 1.75 ms | **430 µs** | **3.6x faster** |
   | 512×512 | 5.66 ms | - | **1.46 ms** | **3.9x faster** |
   
   **Benefits:**
   - **Fast encoding** like RGBA (no palette extraction overhead)
   - **Small files** like indexed PNG (~40% smaller)
   - **Less memory** during rendering (1 byte/pixel instead of 4)
   
   **Implementation:**
   ```rust
   // At style load time (once):
   let palette = style.compute_palette().unwrap();
   
   // At render time (per request):
   let indices = apply_style_gradient_indexed(&data, w, h, &palette, &style);
   let png = create_png_from_precomputed(&indices, w, h, &palette)?;
   ```
   
   **Recommendation:** Use pre-computed palette for all weather tile rendering.
   This is now the fastest path with smallest output files.

### Phase 4: Memory Optimization (2-3 days)

4. **Buffer pooling**
   - [ ] Implement thread-local buffer pools
   - [ ] Update render functions to use pools
   - [ ] Benchmark p99 latency under load

### Phase 5: Optional Enhancements (as needed)

5. **WebP support**
   - [ ] Add `webp` crate
   - [ ] Implement WebP encoding
   - [ ] Add content negotiation

6. **SIMD / Meta-tiling**
   - Only if benchmarks show need

---

## Benchmarking Plan

### Baseline Measurements (Captured 2024-12-23)

#### PNG Encoding - Before Optimization (RGBA only)

| Tile Size | Encoding Time | Throughput | Notes |
|-----------|---------------|------------|-------|
| 256×256 | 1.02 ms | 243.88 MiB/s | Standard WMTS tile |
| 512×512 | 3.57 ms | 280.11 MiB/s | Large tile |
| 1024×1024 | 13.99 ms | 285.95 MiB/s | Very large tile |

#### PNG Encoding - After Phase 3 (Indexed PNG support)

| Benchmark | 256×256 | 512×512 | 1024×1024 | Notes |
|-----------|---------|---------|-----------|-------|
| `rgba_random` | 894 µs | 3.52 ms | 14.1 ms | Random data, many colors |
| `auto_weather` | 884 µs | 3.49 ms | 13.9 ms | Weather data, auto-detect |
| `rgba_weather` | **87 µs** | **228 µs** | **733 µs** | Weather data, forced RGBA |

**Key Finding:** Weather-like data with limited unique colors compresses 10-20x faster 
with RGBA because zlib exploits the redundancy extremely well. The indexed PNG palette 
extraction overhead (HashMap per pixel) currently negates the benefits of smaller 
uncompressed size.

#### File Size Comparison (Quantized Weather Palette, 19 colors)

| Tile Size | RGBA PNG | Indexed PNG | Savings |
|-----------|----------|-------------|---------|
| 256×256 | 6.4 KB | 4.0 KB | **36.9%** |
| 512×512 | 18.4 KB | 10.6 KB | **42.4%** |

**Updated Finding:** When using realistic quantized weather palettes (discrete color stops
like real style JSON configurations), indexed PNG achieves **37-42% file size reduction**.
However, the encoding speed tradeoff remains: RGBA encodes faster due to the palette
extraction overhead.

**Recommendation:** Use pre-computed palette (Phase 3b) for best performance.

**Benchmark commands:**
```bash
cargo bench --package renderer --bench render_benchmarks -- png_encoding
cargo bench --package renderer --bench render_benchmarks -- precomputed
cargo bench --package renderer --bench render_benchmarks -- full_pipeline
```

### Metrics to Track

| Metric | Baseline | After Phase 3b | Target | Status |
|--------|----------|----------------|--------|--------|
| Full pipeline 256×256 | 1.54 ms | **430 µs** | <0.5ms | ✅ ACHIEVED |
| Full pipeline 512×512 | 5.66 ms | **1.46 ms** | <2ms | ✅ ACHIEVED |
| PNG encoding 256×256 | 1.02 ms | **22 µs** | <0.5ms | ✅ ACHIEVED |
| PNG encoding 512×512 | 3.57 ms | **63 µs** | <1ms | ✅ ACHIEVED |
| PNG file size 256×256 | ~6.4 KB | **~4.0 KB** | <5KB | ✅ ACHIEVED |
| Throughput improvement | - | **3.6-4x** | 2x+ | ✅ ACHIEVED |

### After Each Phase

1. Run benchmarks
2. Compare to baseline
3. Document improvement (or lack thereof)
4. Decide whether to continue to next phase

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Parallel chunk fetching overwhelms MinIO | Chunk cache already reduces concurrent fetches; add semaphore if needed |
| Rayon overhead for small tiles | Add threshold, only parallelize for tiles > 128×128 |
| Indexed PNG loses color fidelity | Weather data has discrete colors; visually identical |
| Buffer pool complexity | Start with simple thread-local; only add pooling if needed |
| WebP client compatibility | Keep PNG as default, WebP opt-in via extension |

---

## References

- [PNG Specification](https://www.w3.org/TR/PNG/)
- [Rayon Documentation](https://docs.rs/rayon/latest/rayon/)
- [WebP Container Specification](https://developers.google.com/speed/webp/docs/riff_container)
- [GOES Rendering Performance Analysis](./GOES_RENDERING_PERFORMANCE_ANALYSIS.md)
- [Rendering Pipeline Analysis](./RENDERING_PIPELINE_ANALYSIS.md)
