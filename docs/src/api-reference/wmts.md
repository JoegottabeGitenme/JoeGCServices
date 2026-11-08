# WMTS Endpoints

OGC Web Map Tile Service (WMTS) 1.0.0 implementation with three access patterns.

## 1. KVP Binding (GetTile)

```http
GET /wmts?
  SERVICE=WMTS&
  REQUEST=GetTile&
  VERSION=1.0.0&
  LAYER=gfs_TMP_2m&
  STYLE=temperature&
  TILEMATRIXSET=WebMercatorQuad&
  TILEMATRIX=4&
  TILEROW=5&
  TILECOL=3&
  FORMAT=image/png
```

## 2. RESTful Binding

```http
GET /wmts/rest/{layer}/{style}/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}.{format}
```

**Example**:
```http
GET /wmts/rest/gfs_TMP_2m/temperature/WebMercatorQuad/4/5/3.png
```

## 3. XYZ Tiles (Non-standard)

Simplified XYZ endpoint compatible with Leaflet/OpenLayers:

```http
GET /tiles/{layer}/{style}/{z}/{x}/{y}.{format}
```

**Example**:
```http
GET /tiles/gfs_TMP_2m/temperature/4/3/5.png
```

Note: XYZ uses different row numbering (Y increases southward).

## Tile Matrix Sets

Two OGC-standard tile matrix sets are supported:

### WebMercatorQuad (EPSG:3857)

Most common tile matrix set for web maps. Compatible with Google Maps, OpenStreetMap, and Mapbox.

| Zoom | Tiles | Resolution | Scale |
|------|-------|------------|-------|
| 0 | 1×1 | 156,543 m/px | 1:559,082,264 |
| 4 | 16×16 | 9,783 m/px | 1:34,942,642 |
| 8 | 256×256 | 611 m/px | 1:2,183,915 |
| 12 | 4096×4096 | 38 m/px | 1:136,495 |

### WorldCRS84Quad (CRS:84)

Geographic coordinate system with 2:1 aspect ratio at zoom 0.

| Zoom | Tiles | Resolution | Scale |
|------|-------|------------|-------|
| 0 | 2×1 | ~156 km/px | 1:559,082,264 |
| 4 | 32×16 | ~9.8 km/px | 1:34,942,642 |
| 8 | 512×256 | ~611 m/px | 1:2,183,915 |
| 12 | 8192×4096 | ~38 m/px | 1:136,495 |

Use WorldCRS84Quad when working with geographic coordinates or when Web Mercator distortion is unacceptable for your use case.

## GetCapabilities

```http
GET /wmts?SERVICE=WMTS&REQUEST=GetCapabilities
```

Returns XML with available layers and tile matrix sets.

## Supported Formats

| Format | MIME Type | Extension |
|--------|-----------|-----------|
| PNG | `image/png` | `.png` |
| JPEG | `image/jpeg` | `.jpg` |

## Caching

Tile responses carry `Cache-Control: max-age` and an `X-Cache` header
(`L1-HIT`, `L2-HIT` or `MISS`).

| Request | `max-age` |
|---------|-----------|
| Names its time (`TIME=` on radar/satellite layers) | 300 s from the in-memory cache, 3600 s otherwise |
| Names no time (or `TIME=default`) | 60 s |
| Model layer with no `RUN`, or a `RUN` that is no longer the newest run | 60 s |
| Model layer with a `RUN` that is the newest run | 300 s from the in-memory cache, 3600 s otherwise |

A tile without a time is the newest radar scan or satellite image, and a model
tile always comes from the newest model run (the `RUN` dimension is accepted but
does not select a run), so these change as new data arrives and are kept for
only a minute by browsers and CDNs. A `RUN` is treated as naming the tile only
while it is the run being rendered; once a newer run lands the same URL returns
that run and drops to 60 s. The server's own tile cache keys such a tile by the
dataset it was rendered from, so a new scan or run is never answered with an
older tile. Wind barb layers are keyed by the wind components they are drawn
from.

## OGC Compliance

The WMTS implementation follows OGC WMTS 1.0.0 specification, including:

- **KVP Encoding**: Standard query parameter requests
- **RESTful Encoding**: URL-path based tile requests
- **GetCapabilities**: Full capabilities document with Contents, ServiceIdentification, and ServiceProvider
- **GetTile**: Proper parameter validation and error responses
- **TileMatrixSet definitions**: Accurate scale denominators and tile dimensions

### Compliance Testing

A comprehensive WMTS compliance test suite is available at:

```
http://localhost:8000/wmts-compliance.html
```

This web-based test runner validates:
- GetCapabilities response structure and XML validity
- GetTile parameter handling for both KVP and RESTful encodings
- TileMatrixSet definitions and scale denominators
- All layers in the capabilities document

The test page can also be pointed at external WMTS servers for comparison testing.

## See Also

- [Examples](./examples.md) - Client integration
- [WMS Endpoints](./wms.md) - Alternative API
