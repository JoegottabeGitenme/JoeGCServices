# PNG Encoding Optimization Research

## Current Implementation

**Location**: `crates/renderer/src/png.rs`

**Current Setup**:
- Custom PNG encoder (not using `image` crate)
- Uses `flate2` crate for DEFLATE compression
- Compression level: `Compression::fast()` (level 1)
- Color type: RGBA (6) with 8-bit depth
- No filtering optimization
- Single IDAT chunk per image

**Current Performance** (from Phase 3 testing):
- **Bandwidth**: 80-455 MB/s depending on layer type
- **Latency**: Sub-millisecond for cached tiles
- **Not a bottleneck**: Cache hit rate is 100% for repeated tiles

## PNG Compression Levels (flate2)

The `flate2` crate offers compression levels 0-9:

| Level | Name | Speed | Compression | Use Case |
|-------|------|-------|-------------|----------|
| 0 | `none()` | Fastest | Worst | Debugging only |
| 1 | `fast()` | **Current** | Minimal | High-throughput servers |
| 2-5 | - | Moderate | Better | Balanced |
| 6 | `default()` | Slower | Good | General purpose |
| 9 | `best()` | Slowest | Best | Archival, CDN |

### Trade-offs Analysis

**Current (Level 1)**:
```rust
flate2::Compression::fast()
```
- ✅ Minimizes CPU time
- ✅ Low latency (good for real-time)
- ❌ Larger file sizes
- ❌ Higher bandwidth usage

**Level 6 (Default)**:
```rust
flate2::Compression::default()
```
- ✅ ~30-40% smaller files
- ✅ Reduced bandwidth costs
- ❌ ~2-3x slower encoding
- ❌ Higher CPU usage

## Optimization Options

### Option 1: Adaptive Compression
Use different levels based on layer type:

```rust
pub enum CompressionStrategy {
    Fast,      // For real-time/interactive (wind barbs, isolines)
    Balanced,  // For gradients
    Best,      // For satellite imagery (large, cacheable)
}

impl CompressionStrategy {
    fn to_flate2_level(&self) -> flate2::Compression {
        match self {
            Self::Fast => flate2::Compression::fast(),
            Self::Balanced => flate2::Compression::new(4),
            Self::Best => flate2::Compression::default(),
        }
    }
}
```

**Benefits**:
- Optimize for workload characteristics
- Wind barbs stay fast (already fastest at 17K req/sec)
- Satellite images compress better (static data, worth the CPU cost)

### Option 2: PNG Filtering
PNG supports 5 filter types that pre-process scanlines before compression:

| Filter | Value | Description | Best For |
|--------|-------|-------------|----------|
| None | 0 | No filtering | Random data |
| Sub | 1 | Difference from left pixel | Gradients |
| Up | 2 | Difference from above pixel | Vertical patterns |
| Average | 3 | Average of left and above | General |
| Paeth | 4 | Complex predictor | Photographic |

**Current Implementation**: No filtering (default to 0)

**Optimization**:
```rust
// Add filter selection per scanline
for y in 0..height {
    // Could use filter type 1 (Sub) for temperature gradients
    // Could use filter type 0 (None) for wind barbs
}
```

### Option 3: Use `oxipng` for Post-Processing
`oxipng` is a Rust PNG optimizer that can losslessly reduce file size:

```toml
[dependencies]
oxipng = "9.0"
```

```rust
use oxipng::{optimize_from_memory, Options};

let png_data = create_png(pixels, width, height)?;
let optimized = optimize_from_memory(&png_data, &Options::default())?;
```

**Trade-offs**:
- ✅ Can reduce size by 10-30%
- ❌ Adds significant CPU overhead
- ❌ Not suitable for real-time (use for pre-generated tiles only)

### Option 4: Switch to `image` Crate
Use the standard `image` crate instead of custom encoder:

```rust
use image::{ImageBuffer, Rgba, codecs::png::PngEncoder};

let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, pixels)?;
let mut output = Vec::new();
let encoder = PngEncoder::new(&mut output);
encoder.write_image(&img, width, height, image::ColorType::Rgba8)?;
```

**Trade-offs**:
- ✅ Well-tested, maintained library
- ✅ More PNG features (interlacing, etc.)
- ❌ Less control over compression
- ❌ Potentially slower than custom implementation

## Benchmark Requirements

To evaluate these options, we need to:

1. **Measure current encoding time**:
   - Add timing to `create_png()` function
   - Log encoding duration per tile
   
2. **Measure file size impact**:
   - Track average PNG size per layer type
   - Calculate bandwidth savings
   
3. **Test different compression levels**:
   - Level 1 (current): baseline
   - Level 4: balanced
   - Level 6: default
   - Level 9: maximum

4. **Profile under load**:
   - Run stress test with different settings
   - Measure impact on throughput and latency

## Recommendations

### Immediate (Phase 4)
**NO CHANGES RECOMMENDED**

Rationale:
- Current performance is excellent (14K-18K req/sec)
- PNG encoding is NOT the bottleneck
- 100-concurrent collapse is from connection pools, not PNG
- Cache hit rate is 100% for repeated tiles (PNG only encoded once)

### Future Optimization (Phase 5+)
If bandwidth becomes a concern:

1. **Add compression level configuration**:
   ```bash
   PNG_COMPRESSION_LEVEL=1  # Keep fast for now
   ```

2. **Implement adaptive compression**:
   - Fast (level 1): Wind barbs, isolines, real-time
   - Balanced (level 4): Temperature, pressure gradients
   - Default (level 6): Satellite imagery, radar

3. **Add metrics tracking**:
   - PNG encoding time per layer type
   - Average file size per layer type
   - Compression ratio achieved

## Performance Impact Estimates

| Change | Encoding Time | File Size | Throughput Impact | Bandwidth Savings |
|--------|---------------|-----------|-------------------|-------------------|
| Level 1→4 | +50-100% | -15-25% | -10-15% | -15-25% |
| Level 1→6 | +100-200% | -30-40% | -20-30% | -30-40% |
| Level 1→9 | +300-500% | -35-45% | -50-70% | -35-45% |
| Add filtering | +10-20% | -5-15% | -5-10% | -5-15% |
| oxipng post | +500-1000% | -10-30% | N/A (async) | -10-30% |

## Conclusion

**Phase 4 Decision**: **Keep current PNG implementation unchanged**

**Justification**:
1. Current bottleneck is 100-concurrent connection exhaustion, not PNG encoding
2. Performance is excellent up to 50 concurrent (14K-18K req/sec)
3. PNG encoding happens only on cache miss (rare with 100% hit rate in tests)
4. Changing compression would add CPU load without addressing actual bottleneck
5. Phase 4 priorities: worker threads, connection pools, GRIB cache

**Future Work**:
- Add PNG compression level as configurable parameter
- Implement layer-type-specific compression strategies
- Benchmark impact under real-world cache-miss scenarios
- Consider for Phase 5 after connection pool fixes prove successful

## References
- flate2 crate: https://docs.rs/flate2/
- PNG spec: http://www.libpng.org/pub/png/spec/1.2/PNG-Contents.html
- oxipng: https://github.com/shssoichiro/oxipng
- image crate: https://docs.rs/image/
