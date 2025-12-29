# renderer

Weather data visualization engine that converts numeric grids to styled PNG images.

## Overview

**Location**: `crates/renderer/`  
**Dependencies**: `rayon`, `flate2`, `crc32fast`, `image`, `imageproc`, `rusttype`  
**LOC**: ~4,000

## Key Features

- **Parallel rendering** using rayon for multi-core utilization
- **Pre-computed palettes** for optimal indexed PNG encoding
- **Multiple rendering styles**: gradients, contours, wind barbs
- **Custom PNG encoder** supporting both RGBA and indexed (palette) formats

## Modules

| Module | Description |
|--------|-------------|
| `style` | Style configuration, color mapping, pre-computed palettes |
| `gradient` | Grid resampling and basic color rendering |
| `png` | Custom PNG encoder (RGBA and indexed) |
| `contour` | Marching squares for isolines |
| `barbs` | Wind barb rendering |

## Rendering Approaches

### 1. Standard RGBA Rendering

Traditional approach outputting 4 bytes per pixel:

```rust
use renderer::style::{StyleConfig, apply_style_gradient};

let config = StyleConfig::from_file("config/styles/temperature.json")?;
let style = config.get_default_style().unwrap().1;

// Render to RGBA pixels (4 bytes/pixel)
let rgba = apply_style_gradient(&data, width, height, style);

// Encode to PNG
let png = renderer::png::create_png(&rgba, width, height)?;
```

### 2. Pre-computed Palette Rendering (Recommended)

**3-4x faster** approach using indexed PNG with pre-computed palettes:

```rust
use renderer::style::{StyleConfig, apply_style_gradient_indexed, PrecomputedPalette};
use renderer::png::create_png_from_precomputed;

let config = StyleConfig::from_file("config/styles/temperature.json")?;
let style = config.get_default_style().unwrap().1;

// Compute palette once at startup (cached)
let palette = style.compute_palette().expect("Failed to compute palette");

// Render to palette indices (1 byte/pixel) - FAST!
let indices = apply_style_gradient_indexed(&data, width, height, &palette, style);

// Encode to indexed PNG - FAST! ~40% smaller files
let png = create_png_from_precomputed(&indices, width, height, &palette)?;
```

**Benefits of pre-computed palettes**:
- **Memory**: 1 byte/pixel instead of 4
- **Speed**: No color interpolation or palette extraction at runtime
- **File size**: Indexed PNG is ~40% smaller than RGBA

### 3. Contour Lines

Isobars, isotherms using marching squares:

```rust
use renderer::contour::render_contours;
use renderer::style::ContourStyle;

let style = ContourStyle::from_file("config/styles/temperature.json")?;
let levels = style.generate_levels(data_min, data_max);
let rgba = render_contours(&data, width, height, &levels, &style)?;
```

### 4. Wind Barbs

Vector wind visualization:

```rust
use renderer::barbs::render_wind_barbs;

let rgba = render_wind_barbs(
    &u_component, &v_component,
    width, height,
    spacing: 32,  // Pixels between barbs
)?;
```

## PNG Encoding Options

| Function | Output | Use Case |
|----------|--------|----------|
| `create_png()` | RGBA PNG | General purpose, >256 colors |
| `create_png_auto()` | Auto-detect | Extracts palette if ≤256 colors |
| `create_png_from_precomputed()` | Indexed PNG | **Fastest** with pre-computed palette |
| `create_png_indexed()` | Indexed PNG | When you have palette + indices |

## Performance

### With Pre-computed Palette (Production)

| Operation | 256×256 | 512×512 | 1024×1024 |
|-----------|---------|---------|-----------|
| Indexed render | ~100 µs | ~300 µs | ~1.0 ms |
| PNG encode | **22 µs** | **63 µs** | **210 µs** |
| **Full pipeline** | **430 µs** | **1.46 ms** | ~5 ms |

### Without Pre-computed Palette (Legacy)

| Operation | 256×256 | 512×512 | 1024×1024 |
|-----------|---------|---------|-----------|
| RGBA render | ~200 µs | ~600 µs | ~2 ms |
| PNG encode | 87 µs | 672 µs | 2.97 ms |
| **Full pipeline** | 1.54 ms | 5.66 ms | ~15 ms |

### File Size Comparison

| Format | 256×256 | 512×512 | Savings |
|--------|---------|---------|---------|
| RGBA PNG | 6.4 KB | 18.4 KB | - |
| **Indexed PNG** | **4.0 KB** | **10.6 KB** | **~40%** |

## Key Types

### PrecomputedPalette

```rust
pub struct PrecomputedPalette {
    /// All unique colors (index 0 = transparent)
    pub colors: Vec<(u8, u8, u8, u8)>,
    /// LUT: quantized value → palette index
    pub value_to_index: Vec<u8>,
    /// Value range covered by palette
    pub min_value: f32,
    pub max_value: f32,
}
```

### StyleDefinition

```rust
pub struct StyleDefinition {
    pub name: String,
    pub style_type: String,  // "gradient", "contour", "wind_barbs"
    pub range: Option<ValueRange>,
    pub transform: Option<Transform>,
    pub stops: Vec<ColorStop>,
    pub interpolation: Option<String>,
    pub out_of_range: Option<String>,
}

impl StyleDefinition {
    /// Pre-compute palette for fast indexed rendering
    pub fn compute_palette(&self) -> Option<PrecomputedPalette>;
}
```

## Benchmarking

```bash
# Run all renderer benchmarks
cargo bench --package renderer

# Specific benchmark groups
cargo bench --package renderer -- precomputed_palette
cargo bench --package renderer -- png_encoding
cargo bench --package renderer -- full_pipeline

# Compare with baseline
cargo bench --package renderer -- --save-baseline before
# ... make changes ...
cargo bench --package renderer -- --baseline before
```

## See Also

- [Rendering Pipeline](../architecture/rendering-pipeline.md) - Full pipeline architecture
- [Styles Configuration](../configuration/styles.md) - JSON style definitions
- [Benchmarking](../development/benchmarking.md) - Performance testing
