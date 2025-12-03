# WMTS Tile Alignment Analysis

## Executive Summary

Investigation into WMTS tile boundary misalignment in the weather-wms project revealed **multiple issues** in the current implementation. This document details the findings, root causes, and recommended fixes.

---

## Issues Identified

### 1. X/Y Coordinate Swap in REST Handler (CRITICAL)

**Location**: `services/wms-api/src/handlers.rs:208-228`

**Problem**: Leaflet sends tiles in `{z}/{x}/{y}` format, but our REST handler parses them as `{z}/{row}/{col}` where row=y, col=x. However, it then calls `wmts_get_tile(z, tile_col, tile_row)` which effectively swaps X and Y.

**Current Code**:
```rust
pub async fn wmts_rest_handler(...) {
    let parts: Vec<&str> = path.split('/').collect();
    // parts = [layer, style, tms, z, row, col.png]
    let tile_row: u32 = parts[4].parse().unwrap_or(0);  // This is Leaflet's X!
    let tile_col: u32 = parts[5].parse().unwrap_or(0);  // This is Leaflet's Y!
    let z: u32 = parts[3].parse().unwrap_or(0);
    wmts_get_tile(state, layer, style, z, tile_col, tile_row).await  // SWAPPED!
}
```

**Analysis**:
- Leaflet URL: `/gfs_PRMSL/default/WebMercatorQuad/3/2/5.png`
- Leaflet means: z=3, x=2, y=5
- Our parsing: z=3, tile_row=2, tile_col=5
- We call: wmts_get_tile(z=3, x=tile_col=5, y=tile_row=2)
- Result: x and y are swapped!

**Impact**: Tiles are rendered for wrong geographic regions, causing boundary mismatches.

**Fix**:
```rust
// Leaflet sends {z}/{x}/{y}.png
let tile_x: u32 = parts[4].parse().unwrap_or(0);  // x
let tile_y: u32 = parts[5].parse().unwrap_or(0);  // y
wmts_get_tile(state, layer, style, z, tile_x, tile_y).await  // Correct order
```

---

### 2. Grid Registration Mismatch

**Location**: `services/wms-api/src/rendering.rs:260-319`

**Problem**: GFS GRIB2 data uses **grid-point registration** (values at grid intersection points), but our resampling treats pixels as area-centered.

**GRIB2 Grid Definition** (from wgrib2):
```
lat-lon grid:(1440 x 721) 
lat 90.000000 to -90.000000 by 0.250000
lon 0.000000 to 359.750000 by 0.250000
```

This means:
- First point is at (90°N, 0°E) - the actual coordinate, not a pixel center
- Grid points are at exact 0.25° intervals
- Data values represent conditions AT those points, not averages over areas

**Current Code Issue**:
```rust
// We calculate pixel centers in the output:
let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
let y_ratio = (out_y as f32 + 0.5) / output_height as f32;

// But we don't account for whether source is point or area registered
let grid_x = norm_lon / lon_step;  // Assumes area registration
```

**Impact**: Half-pixel offset can cause visible seams at tile boundaries.

**Recommendation**: For point-registered data like GRIB2:
```rust
// Grid point at index i is at: lon = i * lon_step (not (i + 0.5) * lon_step)
// No half-pixel offset needed when reading source coordinates
let grid_x = norm_lon / lon_step;  // Point at index i is at lon = i * lon_step
let grid_y = (90.0 - lat) / lat_step;  // Point at index j is at lat = 90 - j * lat_step
```

---

### 3. Longitude Normalization Edge Case

**Location**: `services/wms-api/src/rendering.rs:287-288`

**Problem**: When normalizing longitude from -180..180 to 0..360, we don't handle the 180/-180 boundary correctly.

**Current Code**:
```rust
let norm_lon = if lon < 0.0 { lon + 360.0 } else { lon };
```

**Issues**:
1. lon = 180.0 stays as 180.0 (ok)
2. lon = -180.0 becomes 180.0 (ok)
3. lon = 179.9 stays as 179.9 (ok)
4. lon = -0.001 becomes 359.999 (correct, but may cause precision issues)

**Edge Case**: Tiles that span the 0° meridian may have inconsistent normalization.

---

### 4. Bilinear Interpolation at Grid Boundaries

**Location**: `services/wms-api/src/rendering.rs:294-312`

**Problem**: At the edges of the GRIB grid (x=0, x=1439, y=0, y=720), the bilinear interpolation can't access neighboring pixels properly.

**Current Code**:
```rust
let x2 = (x1 + 1).min(global_width - 1);  // Clamps, doesn't wrap
let y2 = (y1 + 1).min(global_height - 1);
```

**Issues**:
1. **Longitude wrap-around**: x=1439 should wrap to x=0 (360° = 0°)
2. **Latitude clamp**: y clamp at poles is correct (no wrap-around for latitude)

**Impact**: Tiles near 0°/360° longitude boundary may have interpolation artifacts.

**Fix**:
```rust
// Handle longitude wrap-around
let x2 = if x1 + 1 >= global_width { 0 } else { x1 + 1 };
// Latitude clamps at poles (no wrap)
let y2 = (y1 + 1).min(global_height - 1);
```

---

### 5. Floating Point Precision in Tile Bounds

**Location**: `crates/wms-common/src/tile.rs:237-253` and `services/wms-api/src/handlers.rs:283-288`

**Problem**: Tile bounds are calculated in f64 but then cast to f32 for rendering.

**Current Code**:
```rust
// tile.rs - calculates in f64
let lon_min = coord.x as f64 / n * 360.0 - 180.0;

// handlers.rs - casts to f32
let bbox_array = [
    latlon_bbox.min_x as f32,  // Loses precision!
    latlon_bbox.min_y as f32,
    ...
];
```

**Impact**: At high zoom levels, f32 precision (~7 decimal digits) may not be sufficient for exact tile boundary alignment.

**Recommendation**: Keep f64 throughout the pipeline or ensure consistent rounding.

---

## Web Mercator Tile Coordinate Conventions

### Quick Reference

| System | URL Format | Y Origin | Notes |
|--------|------------|----------|-------|
| **Leaflet/OSM (XYZ)** | `/{z}/{x}/{y}.png` | Top-left (NW) | Most common |
| **TMS** | `/{z}/{x}/{y}.png` | Bottom-left (SW) | Y flipped |
| **WMTS KVP** | `TILEMATRIX={z}&TILEROW={y}&TILECOL={x}` | Top-left | Standard |
| **WMTS REST** | `/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}` | Top-left | Row=Y, Col=X |

### Leaflet Default Behavior

Leaflet's `L.tileLayer()` uses **XYZ convention**:
- `{z}` = zoom level
- `{x}` = column (longitude direction)
- `{y}` = row (latitude direction, 0 at top)

Our URL template: `.../{z}/{x}/{y}.png`

When Leaflet requests tile (z=3, x=2, y=5):
- URL becomes: `.../3/2/5.png`
- parts[3]=3, parts[4]=2, parts[5]=5.png

---

## Recommended Fixes

### Fix 1: Correct X/Y Parsing (CRITICAL)

```rust
// services/wms-api/src/handlers.rs
pub async fn wmts_rest_handler(...) {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    // URL: {layer}/{style}/{TileMatrixSet}/{z}/{x}/{y}.png
    // Leaflet sends {z}/{x}/{y} in XYZ convention
    let layer = parts[0];
    let style = parts[1];
    let z: u32 = parts[3].parse().unwrap_or(0);
    let x: u32 = parts[4].parse().unwrap_or(0);  // Column (longitude)
    let y_with_ext = parts[5];
    let (y_str, _) = y_with_ext.rsplit_once('.').unwrap_or((y_with_ext, "png"));
    let y: u32 = y_str.parse().unwrap_or(0);  // Row (latitude)
    
    wmts_get_tile(state, layer, style, z, x, y).await
}
```

### Fix 2: Longitude Wrap-Around in Interpolation

```rust
// services/wms-api/src/rendering.rs
fn resample_from_geographic(...) {
    // ...
    // Handle longitude wrap-around for global grids
    let x2 = (x1 + 1) % global_width;  // Wrap at 360°
    let y2 = (y1 + 1).min(global_height - 1);  // Clamp at poles
    // ...
}
```

### Fix 3: Use f64 Throughout Pipeline

```rust
// services/wms-api/src/rendering.rs
pub async fn render_weather_data(
    // ...
    bbox: Option<[f64; 4]>,  // Change from f32 to f64
) -> Result<Vec<u8>, String> {
```

### Fix 4: Add Buffer Pixels for Resampling

For seamless tile boundaries, consider fetching 1 extra row/column of source data around each tile boundary to ensure interpolation has full neighbor access.

---

## Validation Tests

After fixes, validate with these tests:

### Test 1: Adjacent Tile Boundary Check
```bash
# Fetch two adjacent tiles
curl ".../3/3/3.png" -o tile_3_3_3.png
curl ".../3/4/3.png" -o tile_4_3_3.png

# Compare right edge of tile_3_3_3 with left edge of tile_4_3_3
# They should have matching colors
```

### Test 2: Known Coordinate Test
```bash
# Tile (z=0, x=0, y=0) should cover entire world
# Should show global pressure pattern
curl ".../0/0/0.png" -o world.png
```

### Test 3: Zoom Level Consistency
```bash
# Parent tile should be visual average of 4 children
# z=2 tile (1,1) should match z=3 tiles (2,2), (3,2), (2,3), (3,3)
```

---

## GRIB2 Grid Specifics

### GFS 0.25° Grid Properties

```
Grid Template: 0 (Lat/Lon)
Dimensions: 1440 x 721
Lat range: 90.0°N to 90.0°S (by 0.25°)
Lon range: 0.0°E to 359.75°E (by 0.25°)
Registration: Grid-point (values at exact coordinates)
Scan mode: WE:SN (west to east, south to north row order in file)
```

### Coordinate Calculations

For grid index (i, j):
```
lon = i * 0.25          // i = 0..1439, lon = 0..359.75
lat = 90.0 - j * 0.25   // j = 0..720, lat = 90..-90
```

### Array Layout

GRIB2 data is stored row-major with row 0 at the top (90°N):
```
index = j * 1440 + i
// where j = row (0 = north), i = column (0 = prime meridian)
```

---

## Summary of Changes Required

1. **CRITICAL**: Fix X/Y swap in `wmts_rest_handler` - this is likely the main cause
2. **HIGH**: Add longitude wrap-around in bilinear interpolation
3. **MEDIUM**: Consider upgrading to f64 precision for bbox
4. **LOW**: Add validation logging to verify coordinate transformations

---

## References

- OGC WMTS 1.0.0 Specification
- Web Mercator (EPSG:3857) Definition
- GFS GRIB2 Product Definition
- Leaflet TileLayer Documentation
- GDAL Grid Tutorial
