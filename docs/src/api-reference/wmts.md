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

### WebMercatorQuad

Most common tile matrix set for web maps (EPSG:3857).

| Zoom | Tiles | Resolution | Scale |
|------|-------|------------|-------|
| 0 | 1×1 | 156,543 m/px | 1:559,082,264 |
| 4 | 16×16 | 9,783 m/px | 1:34,942,642 |
| 8 | 256×256 | 611 m/px | 1:2,183,915 |
| 12 | 4096×4096 | 38 m/px | 1:136,495 |

## GetCapabilities

```http
GET /wmts?SERVICE=WMTS&REQUEST=GetCapabilities
```

Returns XML with available layers and tile matrix sets.

## See Also

- [Examples](./examples.md) - Client integration
- [WMS Endpoints](./wms.md) - Alternative API
