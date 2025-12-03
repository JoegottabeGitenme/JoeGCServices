# wms-protocol

OGC Web Map Service (WMS) and Web Map Tile Service (WMTS) protocol implementation.

## Overview

**Location**: `crates/wms-protocol/`  
**Dependencies**: `quick-xml`, `serde`  
**LOC**: ~1,000

## Supported Standards

- **WMS 1.1.1**: OGC 01-068r3
- **WMS 1.3.0**: ISO 19128
- **WMTS 1.0.0**: OGC 07-057r7

## WMS Operations

### GetCapabilities

Generate XML capabilities document:

```rust
use wms_protocol::wms::GetCapabilities;

let caps = GetCapabilities {
    service_title: "Weather WMS",
    service_abstract: "Real-time weather data visualization",
    layers: vec![...],
    crs_list: vec![CRS::EPSG4326, CRS::EPSG3857],
    formats: vec!["image/png", "image/jpeg"],
};

let xml = caps.to_xml()?;
```

### GetMap

Parse and validate GetMap requests:

```rust
use wms_protocol::wms::GetMapRequest;

let request = GetMapRequest::from_query_string(query)?;

// Validated parameters
println!("Layer: {}", request.layers);
println!("BBox: {:?}", request.bbox);
println!("Size: {}×{}", request.width, request.height);
```

## WMTS Operations

### GetCapabilities

```rust
use wms_protocol::wmts::GetCapabilities;

let caps = GetCapabilities {
    layers: vec![...],
    tile_matrix_sets: vec![
        TileMatrixSet::web_mercator_quad(),
    ],
};

let xml = caps.to_xml()?;
```

### GetTile

Parse WMTS tile requests:

```rust
use wms_protocol::wmts::GetTileRequest;

// KVP binding
let request = GetTileRequest::from_query_string(query)?;

// RESTful binding
let request = GetTileRequest::from_path_segments(&["gfs_TMP_2m", "default", "WebMercatorQuad", "4", "3", "5"])?;
```

## TileMatrixSet

Standard tile grid definitions:

```rust
pub struct TileMatrixSet {
    pub identifier: String,
    pub crs: CRS,
    pub matrices: Vec<TileMatrix>,
}

// Web Mercator Quad (most common)
let wmq = TileMatrixSet::web_mercator_quad();
```

## See Also

- [WMS API Service](../services/wms-api.md) - Implements these protocols
- [API Reference](../api-reference/wms.md) - Endpoint documentation
