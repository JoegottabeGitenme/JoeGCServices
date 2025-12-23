# Tile Buffer Rendering Implementation Plan

*Efficient edge-artifact prevention for contours and wind barbs*

**Author:** Claude (AI Assistant)  
**Date:** December 12, 2024  
**Status:** ✅ IMPLEMENTED (December 23, 2024)  
**Related:** [GRID_PROCESSOR_IMPLEMENTATION_PLAN.md](./GRID_PROCESSOR_IMPLEMENTATION_PLAN.md)

---

## Implementation Summary (December 23, 2024)

The pixel buffer approach has been implemented, replacing the inefficient 3x3 tile expansion:

### Benchmark Results

| Approach | Render Size | Time | Speedup |
|----------|-------------|------|---------|
| No expansion | 256×256 | 1.7 ms | 9.9x (edge artifacts) |
| **3x3 expansion (old)** | **768×768** | **16.8 ms** | **baseline** |
| 60px buffer | 376×376 | 4.0 ms | 4.2x faster |
| **120px buffer (new default)** | **496×496** | **6.9 ms** | **2.4x faster** |

The 120px buffer was chosen as the default because 60px still showed minor edge clipping with 108px wind barbs in some cases.

### Files Changed

1. **`crates/wms-common/src/tile.rs`**
   - Added `TileBufferConfig` struct with `expanded_bbox()` and `crop_to_tile()` methods
   - Deprecated `ExpandedTileConfig::tiles_3x3()` in favor of `TileBufferConfig`

2. **`services/wms-api/src/rendering/wind.rs`**
   - Updated `render_wind_barbs_tile()` to use `TileBufferConfig`
   - Updated `render_wind_barbs_tile_with_level()` to use `TileBufferConfig`

3. **`services/wms-api/src/rendering/mod.rs`**
   - Updated `render_numbers_tile_with_buffer()` to use `TileBufferConfig`

4. **`crates/renderer/benches/barbs_benchmarks.rs`**
   - Added benchmarks for tile expansion comparison

### Configuration

```bash
# Default is 120px buffer (ensures no clipping for 108px wind barbs)
TILE_RENDER_BUFFER_PIXELS=120
```

---

## Executive Summary

This plan introduces a **configurable pixel buffer** for tile rendering to eliminate edge artifacts in contour lines and wind barbs. Instead of the current approach of rendering a full 3×3 tile grid (9× the work), we render the tile plus a small buffer margin and crop to the final size.

**Key Benefits:**
- **Eliminate tile edge artifacts** for contours and wind barbs
- **Reduce computation** from 9× (3×3 tiles) to ~1.4× (with 50px buffer)
- **Configurable per use case** via environment variable
- **Works with existing GridProcessor** - no changes needed to data access layer

---

## Table of Contents

1. [Problem Statement](#1-problem-statement)
2. [Current Approach Analysis](#2-current-approach-analysis)
3. [Proposed Solution](#3-proposed-solution)
4. [Implementation Details](#4-implementation-details)
5. [Configuration](#5-configuration)
6. [Integration with GridProcessor](#6-integration-with-gridprocessor)
7. [Testing Plan](#7-testing-plan)
8. [Implementation Phases](#8-implementation-phases)

---

## 1. Problem Statement

### 1.1 The Edge Artifact Problem

When rendering tiles independently, features that span tile boundaries can exhibit visual discontinuities:

**Contour Lines:**
```
┌─────────────┬─────────────┐
│             │             │
│      ───────┤             │  ← Contour stops at tile edge
│             │             │
│             │     ────────│  ← Continues in adjacent tile, but may not align
│             │             │
└─────────────┴─────────────┘
```

**Wind Barbs:**
```
┌─────────────┬─────────────┐
│             │             │
│    ↗        │  ↗          │  ← Barbs may be clipped at edges
│       ↗    ↗│             │  ← Or positioned inconsistently
│             │   ↗         │
│    ↗        │             │
└─────────────┴─────────────┘
```

### 1.2 Why This Happens

1. **Contours (Marching Squares)**: The algorithm needs to know values at neighboring cells to determine if a contour line crosses a cell edge. At tile boundaries, there's no neighboring data.

2. **Wind Barbs**: Barbs near tile edges may be clipped, or their placement may not align with adjacent tiles if the grid isn't globally aligned.

### 1.3 Impact

- Visual seams between tiles in map viewers
- Unprofessional appearance
- Contour lines that should be continuous appear broken
- Wind barbs may be missing or duplicated at boundaries

---

## 2. Current Approach Analysis

### 2.1 Wind Barbs: 3×3 Expanded Rendering

The current wind barb implementation uses a **3×3 tile expansion**:

```rust
// In rendering.rs
let config = ExpandedTileConfig::tiles_3x3();  // expansion = 1
let expanded_bbox = expanded_tile_bbox(&coord, &config);
let (exp_w, exp_h) = actual_expanded_dimensions(&coord, &config);
// Render at 768×768 (3×3 tiles)
// ... render wind barbs ...
// Crop center 256×256 tile
let final_pixels = crop_center_tile(&barb_pixels, render_width, &coord, &config);
```

**Problems:**
- **9× the rendering area** (768×768 vs 256×256)
- **9× the grid data** fetched and processed
- At low zoom levels, expanded bbox can span huge geographic areas
- Computationally expensive

### 2.2 Contours: No Buffer (Known Limitation)

Contours currently render with **no expansion**:

```rust
// In rendering.rs (lines 3374-3377 comment)
// "For isolines, we don't use expanded rendering because:
//  1. Contours are continuous and don't need alignment across tiles like wind barbs
//  2. Expanded rendering at low zoom can cause the bbox to span the entire world"
```

**Result:** Contour lines terminate at tile edges, creating visible seams.

### 2.3 Cost Analysis

| Approach | Render Area | Data Fetched | Relative Cost |
|----------|-------------|--------------|---------------|
| No buffer (current contours) | 256×256 | 1× | 1.0× |
| 50px buffer (proposed) | 356×356 | ~1.9× | ~1.4× |
| 3×3 tiles (current barbs) | 768×768 | 9× | ~4-5× |

The 50px buffer approach is **3-4× more efficient** than 3×3 expansion while still providing sufficient context.

---

## 3. Proposed Solution

### 3.1 Concept: Pixel Buffer Margin

Instead of rendering entire adjacent tiles, render the requested tile plus a **configurable pixel buffer** on all sides:

```
                    Buffer (50px)
              ┌─────────────────────────┐
              │                         │
              │   ┌─────────────────┐   │
              │   │                 │   │
   Buffer     │   │   Output Tile   │   │   Buffer
   (50px)     │   │   (256×256)     │   │   (50px)
              │   │                 │   │
              │   └─────────────────┘   │
              │                         │
              └─────────────────────────┘
                    Buffer (50px)

        Render area: 356×356 pixels
        Output area: 256×256 pixels
```

### 3.2 How It Works

1. **Expand the bounding box** by the buffer amount (in degrees)
2. **Request grid data** for the expanded bbox from GridProcessor
3. **Render** at the expanded size (e.g., 356×356)
4. **Crop** to the center tile (256×256)

### 3.3 Buffer Size Considerations

| Feature | Required Context | Recommended Buffer |
|---------|------------------|-------------------|
| Contour lines | 2-3 grid cells | 20-30 pixels |
| Smoothed contours | 5-10 grid cells | 40-50 pixels |
| Wind barbs | 1 barb length | 30-50 pixels |
| Contour labels | Label width/2 | 50-100 pixels |

**Default recommendation: 50 pixels** - sufficient for most use cases while keeping overhead low (~40% extra area).

---

## 4. Implementation Details

### 4.1 New Types (`crates/wms-common/src/tile.rs`)

```rust
/// Configuration for rendering tiles with a buffer margin.
/// 
/// The buffer allows features like contour lines and wind barbs to have
/// context beyond the tile edge, preventing artifacts at tile boundaries.
#[derive(Debug, Clone, Copy)]
pub struct TileBufferConfig {
    /// Buffer size in pixels on each side of the tile
    pub buffer_pixels: u32,
    /// Base tile size (typically 256)
    pub tile_size: u32,
}

impl TileBufferConfig {
    /// Create a new buffer configuration
    pub fn new(buffer_pixels: u32, tile_size: u32) -> Self {
        Self { buffer_pixels, tile_size }
    }
    
    /// Create from environment variable (TILE_RENDER_BUFFER_PIXELS)
    pub fn from_env() -> Self {
        let buffer = std::env::var("TILE_RENDER_BUFFER_PIXELS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50);
        
        let tile_size = std::env::var("TILE_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(256);
            
        Self { buffer_pixels: buffer, tile_size }
    }
    
    /// No buffer (for gradient/raster rendering that doesn't need it)
    pub fn no_buffer() -> Self {
        Self { buffer_pixels: 0, tile_size: 256 }
    }
    
    /// Total render width including buffer on both sides
    pub fn render_width(&self) -> u32 {
        self.tile_size + 2 * self.buffer_pixels
    }
    
    /// Total render height including buffer on both sides
    pub fn render_height(&self) -> u32 {
        self.tile_size + 2 * self.buffer_pixels
    }
    
    /// Calculate the expanded bounding box for a tile
    pub fn expanded_bbox(&self, tile_bbox: &BoundingBox) -> BoundingBox {
        if self.buffer_pixels == 0 {
            return tile_bbox.clone();
        }
        
        // Calculate degrees per pixel for this tile
        let tile_width_deg = tile_bbox.max_lon - tile_bbox.min_lon;
        let tile_height_deg = tile_bbox.max_lat - tile_bbox.min_lat;
        let deg_per_pixel_lon = tile_width_deg / self.tile_size as f64;
        let deg_per_pixel_lat = tile_height_deg / self.tile_size as f64;
        
        // Expand by buffer amount
        let buffer_lon = self.buffer_pixels as f64 * deg_per_pixel_lon;
        let buffer_lat = self.buffer_pixels as f64 * deg_per_pixel_lat;
        
        BoundingBox::new(
            tile_bbox.min_lon - buffer_lon,
            tile_bbox.min_lat - buffer_lat,
            tile_bbox.max_lon + buffer_lon,
            tile_bbox.max_lat + buffer_lat,
        )
    }
    
    /// Crop the center tile from an expanded RGBA pixel buffer
    pub fn crop_to_tile(&self, expanded_pixels: &[u8]) -> Vec<u8> {
        if self.buffer_pixels == 0 {
            return expanded_pixels.to_vec();
        }
        
        let render_width = self.render_width() as usize;
        let tile_size = self.tile_size as usize;
        let buffer = self.buffer_pixels as usize;
        
        let mut result = vec![0u8; tile_size * tile_size * 4];
        
        for row in 0..tile_size {
            let src_y = buffer + row;
            let src_start = (src_y * render_width + buffer) * 4;
            let src_end = src_start + tile_size * 4;
            
            let dst_start = row * tile_size * 4;
            let dst_end = dst_start + tile_size * 4;
            
            if src_end <= expanded_pixels.len() {
                result[dst_start..dst_end].copy_from_slice(&expanded_pixels[src_start..src_end]);
            }
        }
        
        result
    }
}

impl Default for TileBufferConfig {
    fn default() -> Self {
        Self::from_env()
    }
}
```

### 4.2 Updated Contour Rendering (`services/wms-api/src/rendering.rs`)

```rust
/// Render contour lines (isolines) for a tile with buffer for edge continuity
pub async fn render_isolines_tile_with_buffer(
    state: &AppState,
    entry: &CatalogEntry,
    tile_bbox: &BoundingBox,
    style: &ContourStyle,
    tile_coord: Option<&TileCoord>,
) -> Result<Vec<u8>, RenderError> {
    let buffer_config = TileBufferConfig::from_env();
    
    // Calculate expanded bbox and dimensions
    let render_bbox = buffer_config.expanded_bbox(tile_bbox);
    let render_width = buffer_config.render_width() as usize;
    let render_height = buffer_config.render_height() as usize;
    
    // Load grid data for expanded region
    let grid_region = state.grid_processor_factory
        .get_processor(&entry.storage_path)
        .await?
        .read_region(&render_bbox)
        .await?;
    
    // Resample to render dimensions
    let resampled = resample_grid(
        &grid_region.data,
        grid_region.width,
        grid_region.height,
        render_width,
        render_height,
        state.config.interpolation,
    );
    
    // Generate contours on expanded grid
    let contours = generate_contours(
        &resampled,
        render_width,
        render_height,
        &style.levels,
    );
    
    // Render contours to RGBA buffer
    let mut pixels = vec![0u8; render_width * render_height * 4];
    render_contours_to_canvas(
        &mut pixels,
        render_width,
        render_height,
        &contours,
        &style.config,
    );
    
    // Crop to final tile size
    let final_pixels = buffer_config.crop_to_tile(&pixels);
    
    // Encode to PNG
    encode_png(&final_pixels, buffer_config.tile_size as usize, buffer_config.tile_size as usize)
}
```

### 4.3 Updated Wind Barb Rendering

Replace the 3×3 expansion with pixel buffer:

```rust
/// Render wind barbs for a tile with buffer for edge continuity
pub async fn render_wind_barbs_tile_with_buffer(
    state: &AppState,
    u_entry: &CatalogEntry,
    v_entry: &CatalogEntry,
    tile_bbox: &BoundingBox,
    style: &WindBarbStyle,
    tile_coord: Option<&TileCoord>,
) -> Result<Vec<u8>, RenderError> {
    let buffer_config = TileBufferConfig::from_env();
    
    // Calculate expanded bbox and dimensions
    let render_bbox = buffer_config.expanded_bbox(tile_bbox);
    let render_width = buffer_config.render_width() as usize;
    let render_height = buffer_config.render_height() as usize;
    
    // Load U and V wind components for expanded region
    let u_region = state.grid_processor_factory
        .get_processor(&u_entry.storage_path)
        .await?
        .read_region(&render_bbox)
        .await?;
        
    let v_region = state.grid_processor_factory
        .get_processor(&v_entry.storage_path)
        .await?
        .read_region(&render_bbox)
        .await?;
    
    // Resample both components
    let u_resampled = resample_grid(&u_region.data, ...);
    let v_resampled = resample_grid(&v_region.data, ...);
    
    // Calculate barb positions using geographic alignment
    // (This ensures barbs align across tiles globally)
    let barb_positions = calculate_barb_positions_geographic(
        &render_bbox,
        render_width,
        render_height,
        style.spacing_degrees,
    );
    
    // Render barbs
    let mut pixels = vec![0u8; render_width * render_height * 4];
    render_wind_barbs_to_canvas(
        &mut pixels,
        render_width,
        render_height,
        &u_resampled,
        &v_resampled,
        &barb_positions,
        style,
    );
    
    // Crop to final tile size
    let final_pixels = buffer_config.crop_to_tile(&pixels);
    
    // Encode to PNG
    encode_png(&final_pixels, buffer_config.tile_size as usize, buffer_config.tile_size as usize)
}
```

### 4.4 Feature-Specific Buffer Overrides

Some features may need different buffer sizes. Support per-feature overrides:

```rust
/// Get the appropriate buffer size for a rendering feature
pub fn buffer_for_feature(feature: RenderFeature) -> u32 {
    let base_buffer = std::env::var("TILE_RENDER_BUFFER_PIXELS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    
    match feature {
        RenderFeature::Gradient => 0,           // No buffer needed
        RenderFeature::Contours => base_buffer,
        RenderFeature::ContoursWithLabels => base_buffer + 50,  // Extra for labels
        RenderFeature::WindBarbs => base_buffer,
        RenderFeature::Numbers => base_buffer / 2,  // Less context needed
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RenderFeature {
    Gradient,
    Contours,
    ContoursWithLabels,
    WindBarbs,
    Numbers,
}
```

---

## 5. Configuration

### 5.1 Environment Variables

```bash
# ============================================================================
# TILE RENDERING BUFFER CONFIGURATION
# ============================================================================

# Buffer size in pixels around each tile for features that need edge context
# (contours, wind barbs, labels). Set to 0 to disable buffering.
#
# Recommended values:
#   0     - No buffer (for gradient/raster only)
#   30    - Minimal buffer (basic contours)
#   50    - Default (contours + wind barbs)
#   100   - Extended (contours with labels)
#
# Performance impact: (buffer + 256 + buffer)² / 256²
#   30px buffer: ~1.27× render area
#   50px buffer: ~1.93× render area
#   100px buffer: ~3.24× render area
#
TILE_RENDER_BUFFER_PIXELS=50

# Per-feature buffer overrides (optional)
# If set, these override the base buffer for specific features
CONTOUR_BUFFER_PIXELS=50
WIND_BARB_BUFFER_PIXELS=50
LABEL_BUFFER_PIXELS=100
```

### 5.2 Configuration in `.env.example`

```bash
# ============================================================================
# TILE RENDERING
# ============================================================================

# Buffer margin for tile rendering (prevents edge artifacts in contours/barbs)
# See docs/TILE_BUFFER_RENDERING_PLAN.md for details
TILE_RENDER_BUFFER_PIXELS=50       # Default: 50 pixels on each side
```

### 5.3 Runtime Configuration Access

```rust
// In services/wms-api/src/state.rs

/// Configuration for tile rendering
#[derive(Debug, Clone)]
pub struct TileRenderConfig {
    /// Default buffer size in pixels
    pub buffer_pixels: u32,
    /// Per-feature overrides
    pub contour_buffer: Option<u32>,
    pub wind_barb_buffer: Option<u32>,
    pub label_buffer: Option<u32>,
}

impl TileRenderConfig {
    pub fn from_env() -> Self {
        fn parse_u32(key: &str) -> Option<u32> {
            std::env::var(key).ok().and_then(|s| s.parse().ok())
        }
        
        Self {
            buffer_pixels: parse_u32("TILE_RENDER_BUFFER_PIXELS").unwrap_or(50),
            contour_buffer: parse_u32("CONTOUR_BUFFER_PIXELS"),
            wind_barb_buffer: parse_u32("WIND_BARB_BUFFER_PIXELS"),
            label_buffer: parse_u32("LABEL_BUFFER_PIXELS"),
        }
    }
    
    pub fn buffer_for(&self, feature: RenderFeature) -> u32 {
        match feature {
            RenderFeature::Gradient => 0,
            RenderFeature::Contours => self.contour_buffer.unwrap_or(self.buffer_pixels),
            RenderFeature::ContoursWithLabels => self.label_buffer.unwrap_or(self.buffer_pixels + 50),
            RenderFeature::WindBarbs => self.wind_barb_buffer.unwrap_or(self.buffer_pixels),
            RenderFeature::Numbers => self.buffer_pixels / 2,
        }
    }
}
```

---

## 6. Integration with GridProcessor

### 6.1 No Changes Required to GridProcessor

The `GridProcessor` abstraction from the [Grid Processor Implementation Plan](./GRID_PROCESSOR_IMPLEMENTATION_PLAN.md) already supports this use case perfectly:

```rust
// GridProcessor trait
async fn read_region(&self, bbox: &BoundingBox) -> Result<GridRegion, GridProcessorError>;
```

The rendering code simply passes an **expanded bbox** to `read_region()`. The GridProcessor:
1. Calculates which chunks intersect the expanded bbox
2. Fetches only those chunks (usually the same chunks as the original bbox)
3. Returns the requested region

### 6.2 Chunk Efficiency

With 512×512 chunks and a 50px buffer:

```
Original tile bbox:     Might need 1-2 chunks
Expanded bbox (+50px):  Usually needs same 1-2 chunks

Why? The buffer is small relative to chunk size.
A 256px tile at typical zoom spans ~0.5-2° longitude.
A 50px buffer adds ~0.1-0.4° on each side.
Chunks are 512 grid cells, often covering 10°+ at GFS resolution.
```

**The buffer rarely requires additional chunks to be fetched.**

### 6.3 Cache Benefits

The chunk cache in GridProcessor will naturally cache the expanded region data:

```
Request 1: Tile (5, 3) with buffer → Fetches chunk (0, 0)
Request 2: Tile (5, 4) with buffer → Chunk (0, 0) already cached!
```

Adjacent tiles share chunks, and the buffer regions overlap, maximizing cache hits.

---

## 7. Testing Plan

### 7.1 Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_buffer_config_expanded_bbox() {
        let config = TileBufferConfig::new(50, 256);
        let tile_bbox = BoundingBox::new(-100.0, 35.0, -99.0, 36.0);
        
        let expanded = config.expanded_bbox(&tile_bbox);
        
        // Buffer should add ~0.195° on each side (50/256 * 1°)
        assert!((expanded.min_lon - (-100.195)).abs() < 0.01);
        assert!((expanded.max_lon - (-98.805)).abs() < 0.01);
        assert!((expanded.min_lat - 34.805).abs() < 0.01);
        assert!((expanded.max_lat - 36.195).abs() < 0.01);
    }
    
    #[test]
    fn test_buffer_config_crop() {
        let config = TileBufferConfig::new(50, 256);
        
        // Create test pattern: expanded area with distinct border
        let render_w = config.render_width() as usize;  // 356
        let render_h = config.render_height() as usize; // 356
        let mut expanded = vec![0u8; render_w * render_h * 4];
        
        // Fill center (tile area) with white
        for y in 50..(50 + 256) {
            for x in 50..(50 + 256) {
                let idx = (y * render_w + x) * 4;
                expanded[idx..idx+4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        
        // Crop
        let cropped = config.crop_to_tile(&expanded);
        
        // Verify all pixels are white
        assert_eq!(cropped.len(), 256 * 256 * 4);
        for chunk in cropped.chunks(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }
    
    #[test]
    fn test_no_buffer_passthrough() {
        let config = TileBufferConfig::no_buffer();
        let tile_bbox = BoundingBox::new(-100.0, 35.0, -99.0, 36.0);
        
        let expanded = config.expanded_bbox(&tile_bbox);
        
        assert_eq!(expanded.min_lon, tile_bbox.min_lon);
        assert_eq!(expanded.max_lon, tile_bbox.max_lon);
    }
}
```

### 7.2 Visual Regression Tests

Create test cases that verify edge continuity:

```rust
#[tokio::test]
async fn test_contour_edge_continuity() {
    // Render two adjacent tiles
    let tile_a = render_contours_tile(TileCoord { z: 5, x: 10, y: 12 }).await;
    let tile_b = render_contours_tile(TileCoord { z: 5, x: 11, y: 12 }).await;
    
    // Extract right edge of tile_a and left edge of tile_b
    let edge_a = extract_right_edge(&tile_a);
    let edge_b = extract_left_edge(&tile_b);
    
    // Verify contour lines connect (same color at boundary)
    assert_edges_match(&edge_a, &edge_b, tolerance: 5);  // Allow 5px difference
}
```

### 7.3 Performance Benchmarks

```rust
#[bench]
fn bench_contour_no_buffer(b: &mut Bencher) {
    let config = TileBufferConfig::no_buffer();
    b.iter(|| render_contours_with_config(&config));
}

#[bench]
fn bench_contour_50px_buffer(b: &mut Bencher) {
    let config = TileBufferConfig::new(50, 256);
    b.iter(|| render_contours_with_config(&config));
}

#[bench]
fn bench_contour_3x3_tiles(b: &mut Bencher) {
    // Current approach for comparison
    let config = ExpandedTileConfig::tiles_3x3();
    b.iter(|| render_contours_expanded(&config));
}
```

Expected results:
- 50px buffer: ~1.5× slower than no buffer
- 3×3 tiles: ~4-5× slower than no buffer
- **50px buffer is ~3× faster than 3×3 tiles**

---

## 8. Implementation Phases

### Phase 1: Core Buffer Infrastructure (Day 1-2)

**Tasks:**
1. Add `TileBufferConfig` to `crates/wms-common/src/tile.rs`
2. Implement `expanded_bbox()` and `crop_to_tile()` methods
3. Add environment variable parsing
4. Unit tests for buffer calculations

**Deliverable:** Working buffer config with tests.

### Phase 2: Contour Rendering Integration (Day 3-4)

**Tasks:**
1. Update `render_isolines_tile()` to use buffer config
2. Ensure contours are generated on expanded grid
3. Verify label positioning respects buffer (labels outside crop area are OK)
4. Visual testing with adjacent tiles

**Deliverable:** Contour rendering with configurable buffer.

### Phase 3: Wind Barb Rendering Migration (Day 5-6)

**Tasks:**
1. Replace 3×3 expansion with buffer-based approach
2. Ensure geographic alignment still works
3. Verify barb clipping at buffer boundary
4. Performance comparison vs. old approach

**Deliverable:** Wind barb rendering with buffer (replacing 3×3).

### Phase 4: Documentation & Cleanup (Day 7)

**Tasks:**
1. Update `.env.example` with new variables
2. Add configuration to admin dashboard
3. Remove old `ExpandedTileConfig` code (if not needed elsewhere)
4. Update rendering documentation

**Deliverable:** Complete feature with documentation.

---

## Summary

This plan introduces a **pixel-based buffer system** for tile rendering that:

1. **Replaces the inefficient 3×3 tile expansion** with a targeted buffer margin
2. **Reduces rendering overhead** from 9× to ~1.4× for a 50px buffer
3. **Is fully configurable** via `TILE_RENDER_BUFFER_PIXELS` environment variable
4. **Integrates seamlessly** with the GridProcessor abstraction
5. **Maintains visual quality** by providing sufficient context for contours and wind barbs

**Key Configuration:**
```bash
TILE_RENDER_BUFFER_PIXELS=50  # 50 pixels on each side, ~1.4× render area
```

**Comparison:**
| Approach | Render Area | Relative Cost | Edge Quality |
|----------|-------------|---------------|--------------|
| No buffer | 256×256 | 1.0× | Poor (artifacts) |
| 50px buffer | 356×356 | 1.4× | Good |
| 3×3 tiles | 768×768 | 4-5× | Good |

The buffer approach achieves the same visual quality as 3×3 expansion at **3× lower cost**.

---

## Appendix A: Mathematical Details

### Buffer Size to Degrees Conversion

For a tile at zoom level `z`:
```
tile_width_degrees = 360 / 2^z
degrees_per_pixel = tile_width_degrees / 256
buffer_degrees = buffer_pixels × degrees_per_pixel
```

Examples:
| Zoom | Tile Width (°) | 50px Buffer (°) |
|------|----------------|-----------------|
| 4 | 22.5° | 4.39° |
| 6 | 5.625° | 1.10° |
| 8 | 1.406° | 0.27° |
| 10 | 0.352° | 0.069° |

### Render Area Overhead

```
overhead_ratio = (tile_size + 2×buffer)² / tile_size²

For 256px tile:
  30px buffer: 316² / 256² = 1.52×
  50px buffer: 356² / 256² = 1.93×
  100px buffer: 456² / 256² = 3.17×
```

---

## Appendix B: Related Files

| File | Purpose |
|------|---------|
| `crates/wms-common/src/tile.rs` | Tile utilities, buffer config |
| `crates/renderer/src/contour.rs` | Contour generation |
| `crates/renderer/src/barbs.rs` | Wind barb rendering |
| `services/wms-api/src/rendering.rs` | Rendering pipeline |
| `services/wms-api/src/state.rs` | Configuration state |

---

*End of Plan*
