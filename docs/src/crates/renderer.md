# renderer

Weather data visualization engine that converts numeric grids to styled PNG/JPEG images.

## Overview

**Location**: `crates/renderer/`  
**Dependencies**: `image`, `imageproc`, `rusttype`  
**LOC**: ~3,000

## Rendering Types

### 1. Gradient / Color Ramp

Maps values to colors via linear interpolation:

```rust
use renderer::{Renderer, Style, ColorStop};

let style = Style::gradient(vec![
    ColorStop { value: 233.15, color: [0, 0, 255, 255] },    // -40°C = Blue
    ColorStop { value: 273.15, color: [0, 255, 0, 255] },    //   0°C = Green
    ColorStop { value: 313.15, color: [255, 0, 0, 255] },    // +40°C = Red
]);

let renderer = Renderer::new(style);
let image = renderer.render(&grid_data, 256, 256)?;
```

### 2. Contour Lines

Isobars, isotherms using marching squares:

```rust
let style = Style::contours(
    10.0,  // Interval (e.g., every 10 K)
    2,     // Line width (pixels)
    [0, 0, 0, 255],  // Color (black)
);
```

### 3. Wind Barbs

Vector wind visualization:

```rust
let style = Style::wind_barbs(
    WindBarbStyle {
        spacing: 32,  // Pixels between barbs
        scale: 1.0,
        color: [0, 0, 0, 255],
    }
);
```

### 4. Numeric Labels

Point value annotations:

```rust
let style = Style::numeric(
    NumericStyle {
        precision: 1,  // Decimal places
        font_size: 12.0,
        color: [0, 0, 0, 255],
    }
);
```

## Typical Performance

| Rendering Type | 256×256 | 512×512 | 1024×1024 |
|----------------|---------|---------|-----------|
| Gradient | 5 ms | 15 ms | 60 ms |
| Contours | 50 ms | 150 ms | 600 ms |
| Wind barbs | 20 ms | 60 ms | 250 ms |
| Numeric labels | 30 ms | 100 ms | 400 ms |

## Example Styles

Temperature with rainbow gradient:

```rust
let temp_style = Style::gradient(vec![
    ColorStop { value: 233.15, color: [128, 0, 255, 255] },   // Purple
    ColorStop { value: 253.15, color: [0, 0, 255, 255] },     // Blue
    ColorStop { value: 273.15, color: [0, 255, 255, 255] },   // Cyan
    ColorStop { value: 283.15, color: [0, 255, 0, 255] },     // Green
    ColorStop { value: 293.15, color: [255, 255, 0, 255] },   // Yellow
    ColorStop { value: 303.15, color: [255, 128, 0, 255] },   // Orange
    ColorStop { value: 313.15, color: [255, 0, 0, 255] },     // Red
]);
```

## See Also

- [Styles Configuration](../configuration/styles.md) - JSON style definitions
- [WMS API](../services/wms-api.md) - Uses renderer for tiles
