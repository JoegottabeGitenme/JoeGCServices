# Numbers Style Implementation Plan

## Overview

Add a new "numbers" style to all WMS/WMTS layers that displays color-coded numeric values directly on the data grid points. This is useful for debugging and verifying data values.

## Key Features

1. **Zoom-aware density**: At low zoom (zoomed out), show sparse samples to avoid overcrowding; at high zoom, show values at actual grid points
2. **Color-coded values**: Use the layer's color gradient to color the numbers for visual consistency
3. **Clean formatting**: Display values truncated to 2 decimal places only when needed (e.g., "273.15" but "25" not "25.00")
4. **Readable labels**: White background behind numbers ensures readability on any map background
5. **Universal availability**: Available on ALL WMS/WMTS layers without additional configuration

## Architecture

### Data Flow

```
Request: LAYERS=gfs_TMP&STYLES=numbers
    │
    ▼
handlers.rs: detect style="numbers"
    │
    ▼
rendering.rs: render_numbers_tile()
    │
    ├── Load GRIB/NetCDF data (same as other styles)
    ├── Get grid dimensions and data bounds
    ├── Calculate pixel positions for each grid point
    ├── Determine which points to render based on zoom/density
    │
    ▼
For each visible grid point:
    ├── Get raw data value
    ├── Format value to string (smart decimal handling)
    ├── Determine color from gradient/style
    └── Draw number with white background
    │
    ▼
numbers.rs: render_numbers_to_canvas()
    │
    ▼
PNG output
```

## Implementation Details

### Step 1: Create Numbers Renderer Module

**New file: `crates/renderer/src/numbers.rs`**

```rust
/// Configuration for numbers rendering
pub struct NumbersConfig {
    /// Font size in pixels
    pub font_size: f32,
    /// Minimum spacing between numbers (pixels)
    pub min_spacing: f32,
    /// Background color [R, G, B, A]
    pub background_color: [u8; 4],
    /// Default text color (used if no gradient)
    pub default_text_color: [u8; 4],
    /// Unit conversion offset for display (e.g., -273.15 for K to C)
    pub unit_offset: f32,
    /// Optional color stops for value-based coloring
    pub color_stops: Option<Vec<(f32, [u8; 4])>>,
}

impl Default for NumbersConfig {
    fn default() -> Self {
        Self {
            font_size: 10.0,
            min_spacing: 45.0,  // Minimum pixels between number centers
            background_color: [255, 255, 255, 230],
            default_text_color: [0, 0, 0, 255],
            unit_offset: 0.0,
            color_stops: None,
        }
    }
}

/// Render numeric values at grid points
pub fn render_numbers(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    config: &NumbersConfig,
) -> Vec<u8>
```

Key functions:
- `render_numbers()` - Main entry point
- `calculate_grid_sampling()` - Determine which grid points to render based on output size
- `format_value()` - Smart number formatting (2 decimals only if needed)
- `get_color_for_value()` - Interpolate color from gradient stops
- `draw_number_label()` - Reuse character drawing from contour.rs

### Step 2: Update Renderer Library

**Modify: `crates/renderer/src/lib.rs`**

```rust
pub mod barbs;
pub mod contour;
pub mod gradient;
pub mod numbers;  // ADD THIS
pub mod png;
pub mod style;
```

### Step 3: Add Rendering Function

**Modify: `services/wms-api/src/rendering.rs`**

Add new function:

```rust
/// Render grid data as numeric values at grid points
pub async fn render_numbers_tile(
    chunk_cache: &ChunkCache,
    catalog: &Catalog,
    model: &str,
    parameter: &str,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    forecast_hour: Option<u32>,
    level: Option<&str>,
    use_mercator: bool,
) -> Result<Vec<u8>, String>
```

This function will:
1. Load grid data (reuse existing `load_grid_data()`)
2. Get data dimensions and bounds
3. Calculate grid point positions in output pixel space
4. Determine appropriate sampling based on density
5. Call `renderer::numbers::render_numbers()`
6. Encode as PNG

### Step 4: Update WMS/WMTS Handlers

**Modify: `services/wms-api/src/handlers.rs`**

#### 4a. Add "numbers" to Capabilities

Update the styles generation to append "numbers" style to ALL layers:

```rust
// For each parameter's styles, append:
// <Style><Name>numbers</Name><Title>Debug Values</Title></Style>
```

This should be added after the existing style logic, so every layer gets it.

#### 4b. Handle "numbers" Style in GetMap/GetTile

In `render_wms_layer()` and WMTS tile handler, add:

```rust
} else if style == "numbers" {
    // Render numeric values at grid points
    crate::rendering::render_numbers_tile(
        &state.chunk_cache,
        &state.catalog,
        model,
        &parameter,
        width,
        height,
        parsed_bbox.unwrap_or([-180.0, -90.0, 180.0, 90.0]),
        forecast_hour,
        level.as_deref(),
        use_mercator,
    )
    .await
}
```

## Zoom-Aware Sampling Strategy

The key challenge is avoiding overcrowding at low zoom levels while showing actual grid point values at high zoom.

### Approach: Output-Density Based Sampling

Calculate how many grid points would fit in the output image, then sample accordingly:

```rust
fn calculate_grid_sampling(
    data_width: usize,
    data_height: usize, 
    output_width: usize,
    output_height: usize,
    min_spacing: f32,
) -> (usize, usize) {
    // How many numbers can fit horizontally/vertically?
    let max_labels_x = (output_width as f32 / min_spacing).floor() as usize;
    let max_labels_y = (output_height as f32 / min_spacing).floor() as usize;
    
    // Sample rate: show every Nth grid point
    let sample_x = (data_width / max_labels_x.max(1)).max(1);
    let sample_y = (data_height / max_labels_y.max(1)).max(1);
    
    (sample_x, sample_y)
}
```

### Expected Behavior by Zoom Level

| Zoom | Typical Tile Coverage | Grid Points Visible | Sample Rate | Numbers Shown |
|------|----------------------|---------------------|-------------|---------------|
| 0-2  | Whole world/continent | 1000s | Every 20-50 | ~25-50 |
| 3-5  | Large region | 100s | Every 5-15 | ~50-100 |
| 6-8  | State/province | 50-200 | Every 2-5 | ~100-200 |
| 9-11 | City/county | 10-50 | Every 1-2 | All that fit |
| 12+  | Neighborhood | 1-10 | 1 (all) | All points |

## Number Formatting

```rust
fn format_value(value: f32, unit_offset: f32) -> String {
    let display_value = value + unit_offset;
    
    // Check if value is effectively an integer
    if (display_value.round() - display_value).abs() < 0.005 {
        format!("{:.0}", display_value.round())
    } else if (display_value * 10.0).round() / 10.0 == display_value {
        // One decimal place is sufficient
        format!("{:.1}", display_value)
    } else {
        // Use two decimal places
        format!("{:.2}", display_value)
    }
}
```

Examples:
- `273.0` → `"273"`
- `273.5` → `"273.5"`
- `273.15` → `"273.15"`
- `273.156` → `"273.16"` (rounded)

## Color Mapping

Numbers should be colored based on the data value using the same color scale as the gradient style. This provides visual consistency and helps identify value ranges.

### Default Color Stops by Parameter Type

```rust
fn get_default_color_stops(parameter: &str) -> Vec<(f32, [u8; 4])> {
    if parameter.contains("TMP") || parameter.contains("TEMP") {
        // Temperature: blue (cold) to red (hot)
        vec![
            (233.15, [0, 0, 255, 255]),     // -40C: blue
            (273.15, [255, 255, 255, 255]), // 0C: white
            (313.15, [255, 0, 0, 255]),     // 40C: red
        ]
    } else if parameter.contains("WIND") || parameter.contains("GRD") {
        // Wind: green (calm) to red (strong)
        vec![
            (0.0, [0, 128, 0, 255]),   // 0 m/s: green
            (25.0, [255, 255, 0, 255]), // 25 m/s: yellow
            (50.0, [255, 0, 0, 255]),   // 50 m/s: red
        ]
    } else {
        // Default: black text
        vec![]
    }
}
```

## File Changes Summary

| File | Change Type | Description |
|------|-------------|-------------|
| `crates/renderer/src/numbers.rs` | NEW | Numbers rendering module |
| `crates/renderer/src/lib.rs` | MODIFY | Add `pub mod numbers;` |
| `crates/renderer/Cargo.toml` | MODIFY | No changes needed (uses existing deps) |
| `services/wms-api/src/rendering.rs` | MODIFY | Add `render_numbers_tile()` function |
| `services/wms-api/src/handlers.rs` | MODIFY | Add "numbers" style to capabilities and handlers |

## Testing Plan

### Unit Tests

1. `format_value()` - Test various value formats
2. `calculate_grid_sampling()` - Test sampling at different zoom levels
3. `get_color_for_value()` - Test color interpolation

### Integration Tests

1. Request `gfs_TMP` with `STYLES=numbers` at different zoom levels
2. Verify number count decreases as zoom decreases
3. Verify colors match expected gradient
4. Verify values are readable (white background visible)

### Manual Testing

```bash
# Low zoom - should show sparse numbers
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=gfs_TMP&STYLES=numbers&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=512&HEIGHT=256&FORMAT=image/png" -o numbers_world.png

# High zoom - should show dense numbers at grid points
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=gfs_TMP&STYLES=numbers&CRS=EPSG:4326&BBOX=35,-100,45,-90&WIDTH=512&HEIGHT=512&FORMAT=image/png" -o numbers_zoomed.png
```

## Future Enhancements

1. **Unit display**: Optionally show unit suffix (e.g., "25°C" or "10 m/s")
2. **Configurable precision**: Allow override of decimal places
3. **Grid lines**: Option to draw grid lines connecting the points
4. **Hover values**: In web viewer, show value on hover instead of rendering all
5. **Vector output**: SVG output for print-quality debugging

## Timeline Estimate

| Task | Estimated Time |
|------|---------------|
| Create numbers.rs module | 1-2 hours |
| Add render_numbers_tile() | 30 min |
| Update handlers/capabilities | 30 min |
| Testing and refinement | 1 hour |
| **Total** | **3-4 hours** |

## Dependencies

- Reuses character drawing code from `contour.rs`
- Reuses tiny-skia for rendering
- No new external dependencies required
