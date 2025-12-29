# wms-common

Shared types, utilities, and error definitions used across all services.

## Overview

**Location**: `crates/wms-common/`  
**Dependencies**: `serde`, `chrono`  
**LOC**: ~600

## Key Types

### BoundingBox

Geographic extent:

```rust
pub struct BoundingBox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
    pub crs: CRS,
}
```

### CRS

Coordinate reference system:

```rust
pub enum CRS {
    EPSG4326,   // Geographic (lat/lon)
    EPSG3857,   // Web Mercator
    EPSG3413,   // Polar Stereographic North
    EPSG3031,   // Polar Stereographic South
}
```

### Layer

Map layer metadata:

```rust
pub struct Layer {
    pub name: String,
    pub title: String,
    pub abstract_text: Option<String>,
    pub keywords: Vec<String>,
    pub crs_list: Vec<CRS>,
    pub bbox: BoundingBox,
    pub styles: Vec<Style>,
    pub dimensions: Vec<Dimension>,
}
```

### Style

Visualization style:

```rust
pub struct Style {
    pub name: String,
    pub title: String,
    pub legend_url: Option<String>,
}
```

### TileCoord

XYZ tile coordinates:

```rust
pub struct TileCoord {
    pub z: u32,  // Zoom level
    pub x: u32,  // Column
    pub y: u32,  // Row
}
```

### TileBufferConfig

Configuration for rendering tiles with a pixel buffer margin. Used to prevent edge clipping for features like wind barbs.

```rust
pub struct TileBufferConfig {
    pub buffer_pixels: u32,  // Buffer on each side (default: 120)
    pub tile_size: u32,      // Base tile size (default: 256)
}

impl TileBufferConfig {
    pub fn from_env() -> Self;              // Read from TILE_RENDER_BUFFER_PIXELS
    pub fn render_width(&self) -> u32;      // tile_size + 2*buffer
    pub fn expanded_bbox(&self, bbox: &BoundingBox) -> BoundingBox;
    pub fn crop_to_tile(&self, pixels: &[u8]) -> Vec<u8>;
}
```

**Performance**: 2.4x faster than the old 3x3 tile expansion approach.

## Utilities

```rust
// Tile math
pub fn latlon_to_tile(lat: f64, lon: f64, zoom: u32) -> TileCoord;
pub fn tile_to_bbox(z: u32, x: u32, y: u32) -> BoundingBox;

// Time parsing
pub fn parse_iso8601(s: &str) -> Result<DateTime<Utc>>;
```

## See Also

- All services use these types
- [WMS Protocol](./wms-protocol.md) - Uses for OGC compliance
