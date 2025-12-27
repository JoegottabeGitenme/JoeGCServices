# Rendering Pipeline

This document describes the complete rendering pipeline for Weather WMS, from tile request to PNG output. It covers
coordinate transformations, data loading from Zarr storage, and color styling.

## Overview

The rendering pipeline transforms gridded weather data into styled PNG tiles suitable for web map display. The key
challenges are:

1. **Coordinate system transformations** - Converting between tile coordinates, WGS84, and Web Mercator
2. **Grid coordinate conventions** - Handling 0-360 longitude (GFS) vs -180/180 longitude
3. **Efficient data loading** - Loading only necessary chunks from Zarr storage
4. **Consistent color mapping** - Using absolute value ranges for seamless tile boundaries

## Pipeline Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           WMTS/WMS Request                                  │
│                                                                             │
│  GET /tiles/gfs_TMP/default/WebMercatorQuad/4/3/5.png                      │
│       ↓                                                                     │
│  ┌──────────────────┐                                                       │
│  │ 1. Parse Request │  Extract: layer, style, z/x/y, dimensions            │
│  └────────┬─────────┘                                                       │
│           ↓                                                                 │
│  ┌────────────────────────┐                                                 │
│  │ 2. Tile → Lat/Lon BBox │  Convert XYZ tile to WGS84 bounding box        │
│  └────────┬───────────────┘                                                 │
│           ↓                                                                 │
│  ┌──────────────────────────┐                                               │
│  │ 3. Query Catalog         │  Find dataset in PostgreSQL                  │
│  │    → zarr_metadata       │  Get Zarr location and grid metadata         │
│  └────────┬─────────────────┘                                               │
│           ↓                                                                 │
│  ┌────────────────────────────────────────────────────────────────┐        │
│  │ 4. Load Grid Data from Zarr                                    │        │
│  │                                                                │        │
│  │  a) Normalize bbox to grid coordinate system (0-360 if GFS)   │        │
│  │  b) Calculate which chunks intersect the bbox                 │        │
│  │  c) Read chunks via byte-range requests                       │        │
│  │  d) Assemble into contiguous grid region                      │        │
│  │  e) Return data + actual_bbox of loaded region                │        │
│  └────────┬───────────────────────────────────────────────────────┘        │
│           ↓                                                                 │
│  ┌──────────────────────────────────────────────────────────────┐          │
│  │ 5. Resample to Output Tile                                   │          │
│  │                                                              │          │
│  │  For each output pixel:                                      │          │
│  │    a) Calculate pixel center in Web Mercator                 │          │
│  │    b) Convert to latitude/longitude                          │          │
│  │    c) Normalize longitude to grid convention                 │          │
│  │    d) Sample grid value (bilinear interpolation)             │          │
│  └────────┬─────────────────────────────────────────────────────┘          │
│           ↓                                                                 │
│  ┌────────────────────────────────────────────────────────────┐            │
│  │ 6. Apply Color Style                                       │            │
│  │                                                            │            │
│  │  Load style config (e.g., temperature.json)               │            │
│  │  For each pixel value:                                    │            │
│  │    a) Find surrounding color stops                        │            │
│  │    b) Interpolate RGB color                               │            │
│  │    c) Write to RGBA buffer                                │            │
│  └────────┬───────────────────────────────────────────────────┘            │
│           ↓                                                                 │
│  ┌──────────────────┐                                                       │
│  │ 7. Encode PNG    │  Compress and return to client                       │
│  └──────────────────┘                                                       │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Coordinate Systems

### 1. Tile Coordinates (XYZ/TMS)

Web map tiles use XYZ coordinates where:

- `z` = zoom level (0 = whole world in one tile)
- `x` = column (0 at 180°W, increases eastward)
- `y` = row (0 at ~85°N in XYZ/WMTS convention)

```rust
// Convert tile coordinates to lat/lon bounding box
pub fn tile_to_latlon_bounds(coord: &TileCoord) -> BoundingBox {
    let n = 2u32.pow(coord.z) as f64;
    
    // Longitude is linear
    let lon_min = coord.x as f64 / n * 360.0 - 180.0;
    let lon_max = (coord.x + 1) as f64 / n * 360.0 - 180.0;
    
    // Latitude uses Web Mercator inverse projection
    let lat_max = (PI * (1.0 - 2.0 * coord.y as f64 / n)).sinh().atan().to_degrees();
    let lat_min = (PI * (1.0 - 2.0 * (coord.y + 1) as f64 / n)).sinh().atan().to_degrees();
    
    BoundingBox::new(lon_min, lat_min, lon_max, lat_max)
}
```

### 2. WGS84 Geographic Coordinates

Standard lat/lon coordinates:

- Longitude: -180 to 180 (or 0 to 360 for some datasets)
- Latitude: -90 to 90

### 3. Grid Coordinate Conventions

Different weather models use different longitude conventions:

| Model | Longitude Range             | Example                |
|-------|-----------------------------|------------------------|
| GFS   | 0 to 360                    | New York at 286°E      |
| HRRR  | -180 to 180                 | New York at -74°W      |
| GOES  | Native satellite projection | Fixed-grid coordinates |

## Handling 0-360 Longitude (GFS)

GFS data uses 0-360 longitude convention, while web maps use -180/180. This requires careful coordinate normalization.

### The `grid_uses_360` Flag

When performing partial Zarr reads, the returned data may have a bounding box that doesn't span >180° (e.g., a small
regional _subset). This makes it impossible to infer the longitude convention from the data bounds alone. To solve this,
the rendering pipeline propagates an explicit `grid_uses_360` flag from the grid metadata through to the resampling
functions.

```rust
// In GridData struct
pub struct GridData {
    pub values: Vec<f32>,
    pub width: usize,
    pub height: usize,
    pub bbox: Option<[f32; 4]>,
    pub grid_uses_360: bool,  // Explicit flag from grid metadata
}
```

### Normalizing Request Bbox to Grid Space

```rust
impl BoundingBox {
    /// Check if grid uses 0-360 longitude convention
    pub fn uses_0_360_longitude(&self) -> bool {
        self.min_lon >= 0.0 && self.max_lon > 180.0
    }
    
    /// Normalize request bbox to grid's coordinate system
    pub fn normalize_to_grid(&self, grid_bbox: &BoundingBox) -> Self {
        if grid_bbox.uses_0_360_longitude() && self.min_lon < 0.0 {
            // Convert -180/180 to 0/360
            Self {
                min_lon: if self.min_lon < 0.0 { 
                    self.min_lon + 360.0 
                } else { 
                    self.min_lon 
                },
                max_lon: if self.max_lon < 0.0 { 
                    self.max_lon + 360.0 
                } else { 
                    self.max_lon 
                },
                min_lat: self.min_lat,
                max_lat: self.max_lat,
            }
        } else {
            *self
        }
    }
}
```

### Dateline Crossing Detection

Requests spanning negative to positive longitude on a 0-360 grid require special handling:

```rust
/// Check if request crosses dateline on 0-360 grid
/// Example: Request [-100, 30, 50, 40] would normalize to [260, 30, 50, 40]
/// which is invalid (min_lon > max_lon)
pub fn crosses_dateline_on_360_grid(&self, grid_bbox: &BoundingBox) -> bool {
    if !grid_bbox.uses_0_360_longitude() {
        return false;
    }
    // Crosses if min_lon < 0 AND max_lon >= 0
    self.min_lon < 0.0 && self.max_lon >= 0.0
}
```

**When dateline crossing is detected**, the Zarr processor loads the **full grid** rather than attempting a partial read
with invalid bounds.

## Zarr Partial Reads

The Zarr grid processor efficiently loads only the chunks needed for a tile request.

### Chunk Calculation

```rust
fn chunks_for_bbox(&self, bbox: &BoundingBox) -> Vec<(usize, usize)> {
    // Normalize bbox to grid coordinates (handles 0-360)
    let norm_bbox = bbox.normalize_to_grid(&self.metadata.bbox);
    
    // Convert bbox to grid cell indices
    let min_col = ((norm_bbox.min_lon - grid_bbox.min_lon) / lon_per_cell).floor();
    let max_col = ((norm_bbox.max_lon - grid_bbox.min_lon) / lon_per_cell).ceil();
    let min_row = ((grid_bbox.max_lat - norm_bbox.max_lat) / lat_per_cell).floor();
    let max_row = ((grid_bbox.max_lat - norm_bbox.min_lat) / lat_per_cell).ceil();
    
    // Convert to chunk indices
    let min_chunk_x = min_col / chunk_width;
    let max_chunk_x = (max_col + chunk_width - 1) / chunk_width;
    // ... similar for y
    
    // Return list of (chunk_x, chunk_y) pairs
}
```

### Returning Actual Bounding Box

**Critical**: When loading a partial region, the processor returns the **actual bounding box** of the loaded data, not
the requested bbox:

```rust
// In assemble_region():
let actual_bbox = BoundingBox::new(
    grid_bbox.min_lon + min_col as f64 * res_x,
    grid_bbox.max_lat - max_row as f64 * res_y,
    grid_bbox.min_lon + max_col as f64 * res_x,
    grid_bbox.max_lat - min_row as f64 * res_y,
);

Ok(GridRegion {
    data: output,
    width: out_width,
    height: out_height,
    bbox: actual_bbox,  // Actual bounds of returned data
    resolution: (res_x, res_y),
})
```

This `actual_bbox` is then used by the resampler to correctly map output pixels to grid positions.

## Resampling for Web Mercator

The resampler converts grid data to output tile pixels, accounting for Web Mercator's non-linear latitude spacing.

### Key Algorithm

```rust
fn resample_for_mercator(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],  // Tile bounds in WGS84
    data_bounds: [f32; 4],  // Actual grid data bounds
    grid_uses_360: bool,    // Explicit flag from grid metadata
) -> Vec<f32> {
    // Use explicit flag - cannot infer from partial read bounds
    let data_uses_360 = grid_uses_360;
    
    let mut output = vec![f32::NAN; output_width * output_height];
    
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // 1. Calculate output pixel center position
            let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
            let y_ratio = (out_y as f32 + 0.5) / output_height as f32;
            
            // 2. Longitude is linear
            let lon = out_min_lon + x_ratio * (out_max_lon - out_min_lon);
            
            // 3. Latitude requires Mercator → WGS84 conversion
            let merc_y = max_merc_y - y_ratio * (max_merc_y - min_merc_y);
            let lat = mercator_y_to_lat(merc_y);
            
            // 4. Normalize longitude for 0-360 grids
            let norm_lon = if data_uses_360 && lon < 0.0 {
                lon + 360.0
            } else {
                lon
            };
            
            // 5. Handle wrap gap for global grids (see below)
            let is_global_grid = data_uses_360 && (data_max_lon - data_min_lon) > 359.0;
            let in_wrap_gap = is_global_grid && norm_lon > data_max_lon && norm_lon < 360.0;
            
            // 6. Check if within data bounds (with wrap gap exception)
            if !in_wrap_gap {
                if norm_lon < data_min_lon || norm_lon > data_max_lon {
                    continue;  // Leave as NaN (transparent)
                }
            }
            
            // 7. Convert to grid coordinates (with wrap gap handling)
            let grid_x = if in_wrap_gap {
                // Map wrap gap position to interpolate between last column and first
                let gap_start = data_max_lon;
                let gap_size = 360.0 - data_max_lon + data_min_lon;
                let pos_in_gap = norm_lon - gap_start;
                (data_width as f32 - 1.0) + (pos_in_gap / gap_size) as f32
            } else {
                (norm_lon - data_min_lon) / data_lon_range * data_width
            };
            let grid_y = (data_max_lat - lat) / data_lat_range * data_height;
            
            // 8. Bilinear interpolation (with wrapping for x)
            output[out_y * output_width + out_x] = bilinear_sample_wrap(data, grid_x, grid_y);
        }
    }
    
    output
}
```

## Handling the Prime Meridian Wrap Gap

GFS and other global grids using 0-360° longitude have a gap between the last grid column and 360°. For example, GFS
0.25° resolution:

- Column 0 = 0°
- Column 1439 = 359.75°
- **Gap**: 359.75° to 360° (no data column at exactly 360°)

When rendering tiles near the prime meridian (0° longitude), negative request longitudes (e.g., -0.1°) normalize to
values in this gap (359.9°). Without special handling, these pixels would be skipped as "out of bounds," causing a
visible vertical line artifact at 0° longitude.

### Wrap Gap Solution

The resampling functions detect when a normalized longitude falls in the wrap gap and handle it specially:

```rust
// Detect wrap gap
let is_global_grid = data_uses_360 && (data_max_lon - data_min_lon) > 359.0;
let in_wrap_gap = is_global_grid && norm_lon > data_max_lon && norm_lon < 360.0;

// For wrap gap pixels:
// 1. Skip the normal bounds check (only check latitude)
// 2. Calculate grid_x to position past the last column
// 3. Bilinear interpolation wraps from column 1439 to column 0
```

This allows seamless interpolation across the 360°/0° boundary, eliminating the line artifact.

### Bilinear Interpolation with Wrapping

When `grid_x` exceeds `data_width - 1`, the interpolation wraps the `x2` coordinate back to column 0:

```rust
let x2 = if is_global_grid && x1 == data_width - 1 {
    0  // Wrap to first column
} else {
    (x1 + 1).min(data_width - 1)
};
```

## Color Styling

Styles define how data values map to colors. Style configurations are stored in `config/styles/*.json`.

### Style Configuration Format

```json
{
  "version": "1.0",
  "styles": {
    "default": {
      "name": "Temperature",
      "type": "gradient",
      "units": "K",
      "range": { "min": 233.15, "max": 313.15 },
      "stops": [
        { "value": 233.15, "color": "#9400D3", "label": "-40°C" },
        { "value": 253.15, "color": "#0000FF", "label": "-20°C" },
        { "value": 273.15, "color": "#00FFFF", "label": "0°C" },
        { "value": 293.15, "color": "#FFFF00", "label": "20°C" },
        { "value": 313.15, "color": "#FF0000", "label": "40°C" }
      ],
      "interpolation": "linear"
    }
  }
}
```

### Style Application

**Critical**: Styles use **absolute value ranges**, not per-tile min/max. This ensures consistent colors across tile
boundaries.

```rust
pub fn apply_style_gradient(
    data: &[f32],
    width: usize,
    height: usize,
    style: &StyleDefinition,
) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height * 4];
    
    // Use absolute color stops from style config
    let stops = &style.stops;  // e.g., [233K, 253K, 273K, 293K, 313K]
    
    for (idx, &value) in data.iter().enumerate() {
        // Handle NaN/missing as transparent
        if value.is_nan() {
            continue;
        }
        
        // Find surrounding color stops
        let (low_idx, high_idx) = find_surrounding_stops(&stops, value);
        
        // Interpolate color
        let t = (value - stops[low_idx].value) / 
                (stops[high_idx].value - stops[low_idx].value);
        let color = interpolate_color(&stops[low_idx], &stops[high_idx], t);
        
        // Write RGBA
        pixels[idx * 4..idx * 4 + 4].copy_from_slice(&color);
    }
    
    pixels
}
```

## Common Issues and Solutions

### Issue: Style Loading Failure at Startup

**Symptom**: Service fails to start with "unable to load style" error.

**Cause**: The layer configuration references a style file that doesn't exist or is invalid.

**Solution**: Ensure all style files referenced in `config/layers/*.yaml` exist in `config/styles/`:

```yaml
# config/layers/gfs.yaml
layers:
  - parameter: TMP
    style_file: temperature.json  # Must exist at config/styles/temperature.json
```

Style loading failures are **fatal errors** - there is no fallback. This ensures consistent rendering across all tiles.

### Issue: Tiles Showing Wrong Geographic Data

**Symptom**: Tiles outside the prime meridian region show data from wrong locations.

**Cause**: Using full grid bbox instead of actual loaded data bbox for partial Zarr reads.

**Solution**: Use the `actual_bbox` returned from Zarr, not `entry.bbox`:

```rust
// Use actual bbox from Zarr partial read if available
let data_bounds = grid_result.bbox.unwrap_or_else(|| [
    entry.bbox.min_x,
    entry.bbox.min_y,
    entry.bbox.max_x,
    entry.bbox.max_y,
]);
```

### Issue: Tiles Along Prime Meridian Look Different

**Symptom**: Tiles crossing longitude 0 render correctly, but others don't.

**Cause**: Dateline-crossing tiles load full grid (correct), but non-crossing tiles do partial reads with bbox mismatch.

**Solution**: This is fixed by properly propagating `actual_bbox` from Zarr reads.

## Performance Optimizations

The rendering pipeline includes several optimizations that provide **3-4x faster tile generation** and **40% smaller
file sizes**.

### 1. Parallel Chunk Fetching

When a tile request spans multiple Zarr chunks, they are fetched in parallel:

```rust
// Before: Sequential (200ms for 4 chunks @ 50ms each)
for (cx, cy) in &chunks {
    chunk_data.push(self.read_chunk(*cx, *cy).await?);
}

// After: Parallel (50ms for 4 chunks @ 50ms each)
let chunk_futures: Vec<_> = chunks
    .iter()
    .map(|(cx, cy)| self.read_chunk(*cx, *cy))
    .collect();
let chunk_data = futures::future::join_all(chunk_futures).await;
```

**Impact**: 4x faster chunk loading for multi-chunk requests.

### 2. Parallel Pixel Rendering (rayon)

Color mapping and resampling use rayon for parallel row processing:

```rust
// Process rows in parallel across all CPU cores
pixels
    .par_chunks_mut(width * 4)
    .enumerate()
    .for_each(|(y, row)| {
        for x in 0..width {
            // Color mapping for each pixel
        }
    });
```

**Impact**: Near-linear scaling with CPU cores for rendering operations.

### 3. Pre-computed Palette for Indexed PNG

The fastest rendering path uses pre-computed color palettes:

```rust
// At style load time (once per style):
let palette = style.compute_palette().unwrap();

// At render time (per request):
let indices = apply_style_gradient_indexed(&data, w, h, &palette, &style);
let png = create_png_from_precomputed(&indices, w, h, &palette)?;
```

**Benefits**:

- **1 byte per pixel** instead of 4 (RGBA) during rendering
- **No palette extraction** at PNG encoding time
- **~40% smaller** PNG files (indexed vs RGBA)
- **3-4x faster** full pipeline

#### Performance Comparison

| Pipeline             | 256×256 | 512×512 | Improvement       |
|----------------------|---------|---------|-------------------|
| RGBA (baseline)      | 1.54 ms | 5.66 ms | -                 |
| Pre-computed palette | 430 µs  | 1.46 ms | **3.6-4x faster** |

| PNG Encoding     | 256×256   | 512×512   | 1024×1024  |
|------------------|-----------|-----------|------------|
| RGBA direct      | 87 µs     | 672 µs    | 2.97 ms    |
| Auto extract     | 349 µs    | 803 µs    | 3.56 ms    |
| **Pre-computed** | **22 µs** | **63 µs** | **210 µs** |

### 4. Tile Buffer for Edge Features

Wind barbs, numbers, and other features that can extend beyond their anchor point use a **pixel buffer** approach to
prevent clipping at tile boundaries.

#### The Problem

Features rendered near tile edges can be clipped:

```
┌─────────────┬─────────────┐
│      ↗      │             │
│         ↗   │←clipped     │  Wind barb cut off at tile edge
│    ↗        │             │
└─────────────┴─────────────┘
```

#### Old Approach: 3x3 Tile Expansion (Slow)

Previously, we rendered a 3x3 grid of tiles (768×768) and cropped to the center tile:

- **589,824 pixels** rendered
- **9x the data** fetched
- **~17ms** per tile

#### New Approach: Pixel Buffer (Fast)

Now we render with a configurable pixel buffer (default 120px):

```
         ┌─────────────────────────┐
         │      120px buffer       │
         │   ┌─────────────────┐   │
         │   │                 │   │
         │   │   256×256 tile  │   │
         │   │                 │   │
         │   └─────────────────┘   │
         │                         │
         └─────────────────────────┘
              496×496 render
```

- **246,016 pixels** rendered (2.4x less than 3x3)
- **~6.9ms** per tile (**2.4x faster**)
- No edge clipping artifacts

#### Configuration

```bash
# Default: 120px buffer (sufficient for 108px wind barbs)
TILE_RENDER_BUFFER_PIXELS=120
```

#### Implementation

```rust
use wms_common::tile::TileBufferConfig;

// Get buffer config (reads TILE_RENDER_BUFFER_PIXELS env var)
let buffer_config = TileBufferConfig::from_env();

// Expand bbox by buffer amount
let expanded_bbox = buffer_config.expanded_bbox(&tile_bbox);

// Render at expanded size
let render_width = buffer_config.render_width();   // 496
let render_height = buffer_config.render_height(); // 496

// ... render features ...

// Crop to center tile
let final_pixels = buffer_config.crop_to_tile(&expanded_pixels);
```

#### Performance Comparison

| Approach         | Render Size | Time       | Speedup         |
|------------------|-------------|------------|-----------------|
| 3x3 expansion    | 768×768     | 16.8 ms    | baseline        |
| **120px buffer** | **496×496** | **6.9 ms** | **2.4x faster** |

### Chunk Size Selection

Zarr chunk size affects read efficiency:

- **Too small**: Many HTTP requests for partial reads
- **Too large**: Over-fetching data for small tiles

Recommended: 512x512 chunks for 0.25° GFS data (~1MB per chunk).

### Cache Layers

1. **L1 (Memory)**: Recent tiles, <1ms
2. **L2 (Redis)**: Cross-instance sharing, 2-5ms
3. **Chunk Cache**: Decompressed Zarr chunks, avoids re-reads
4. **Palette Cache**: Pre-computed palettes per style (in-memory)

### Prefetching

When rendering a tile, prefetch neighboring tiles in the likely pan direction:

```rust
spawn_tile_prefetch(state, layer, style, coord, rings: 1);
```

## Related Documentation

- [Data Flow](./data-flow.md) - Overall system data flow
- [Caching](./caching.md) - Cache layer details
- [Styles Configuration](../configuration/styles.md) - Style file format
- [WMS API](../api-reference/wms.md) - WMS request parameters
- [WMTS API](../api-reference/wmts.md) - WMTS request parameters
