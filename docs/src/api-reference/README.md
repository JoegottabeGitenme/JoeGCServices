# API Reference

Complete API documentation for all Weather WMS endpoints. The API implements standard OGC protocols (WMS, WMTS) plus a RESTful admin API.

## API Overview

| Protocol | Base URL | Standard | Purpose |
|----------|----------|----------|---------|
| [WMS](./wms.md) | `/wms` | OGC WMS 1.1.1/1.3.0 | Arbitrary bbox rendering |
| [WMTS](./wmts.md) | `/wmts` | OGC WMTS 1.0.0 | Tiled map access |
| [REST API](./rest-api.md) | `/api/*` | Custom | Admin and metadata |

## Quick Reference

### Get a Map Tile (WMS)

```bash
curl "http://localhost:8080/wms?\
SERVICE=WMS&\
VERSION=1.3.0&\
REQUEST=GetMap&\
LAYERS=gfs_TMP_2m&\
STYLES=temperature&\
CRS=EPSG:3857&\
BBOX=-20037508,-20037508,20037508,20037508&\
WIDTH=256&\
HEIGHT=256&\
FORMAT=image/png" -o tile.png
```

### Get a Tile (WMTS XYZ)

```bash
curl "http://localhost:8080/tiles/gfs_TMP_2m/temperature/4/3/5.png" -o tile.png
```

### List Available Layers

```bash
curl "http://localhost:8080/api/parameters/gfs"
```

## Common Parameters

### Layers

Layer names follow the pattern: `{model}_{parameter}_{level}`

Examples:
- `gfs_TMP_2m` - GFS temperature at 2 meters
- `hrrr_UGRD_10m` - HRRR U-wind component at 10 meters
- `goes18_CMI_C13` - GOES-18 channel 13 (IR)
- `mrms_REFL` - MRMS radar reflectivity

### Styles

- `default` - Automatic style selection
- `temperature` - Temperature color ramp
- `wind` - Wind visualization
- `precipitation` - Precipitation colors
- `reflectivity` - Radar reflectivity colors
- `goes_ir` - GOES infrared enhancement

### Coordinate Systems (CRS)

- `EPSG:4326` - Geographic (lat/lon), WGS84
- `EPSG:3857` - Web Mercator (most common for web maps)
- `CRS:84` - Geographic (WMS 1.1.1 equivalent of EPSG:4326)

### Time Parameter

```
TIME=2024-12-03T00:00:00Z     # Specific time (ISO 8601)
TIME=latest                    # Most recent data (default)
```

### Image Formats

- `image/png` - PNG (default, supports transparency)
- `image/jpeg` - JPEG (smaller, no transparency)

## Client Libraries

### JavaScript (Leaflet)

```javascript
L.tileLayer.wms('http://localhost:8080/wms', {
    layers: 'gfs_TMP_2m',
    styles: 'temperature',
    format: 'image/png',
    transparent: true,
    version: '1.3.0',
}).addTo(map);
```

### JavaScript (OpenLayers)

```javascript
new ol.layer.Tile({
    source: new ol.source.TileWMS({
        url: 'http://localhost:8080/wms',
        params: {
            'LAYERS': 'gfs_TMP_2m',
            'STYLES': 'temperature',
            'FORMAT': 'image/png',
        },
    }),
});
```

### Python

```python
import requests

response = requests.get('http://localhost:8080/wms', params={
    'SERVICE': 'WMS',
    'VERSION': '1.3.0',
    'REQUEST': 'GetMap',
    'LAYERS': 'gfs_TMP_2m',
    'STYLES': 'temperature',
    'CRS': 'EPSG:3857',
    'BBOX': '-20037508,-20037508,20037508,20037508',
    'WIDTH': '256',
    'HEIGHT': '256',
    'FORMAT': 'image/png',
})

with open('tile.png', 'wb') as f:
    f.write(response.content)
```

### QGIS

1. **Layer** → **Add Layer** → **Add WMS/WMTS Layer**
2. **New** connection:
   - Name: `Weather WMS`
   - URL: `http://localhost:8080/wms`
3. **Connect** and select layers

## Error Responses

### WMS Errors (XML)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<ServiceExceptionReport version="1.3.0">
  <ServiceException code="InvalidParameterValue">
    Layer 'invalid_layer' does not exist
  </ServiceException>
</ServiceExceptionReport>
```

### REST API Errors (JSON)

```json
{
  "error": "LayerNotFound",
  "message": "Layer 'invalid_layer' does not exist",
  "status": 404
}
```

## Rate Limiting

Currently no rate limiting. For production deployments, consider adding rate limiting at the reverse proxy level (nginx, Caddy).

## CORS

CORS is enabled by default for all origins. To restrict:

```rust
// services/wms-api/src/main.rs
let cors = CorsLayer::new()
    .allow_origin("https://yourdomain.com".parse::<HeaderValue>()?)
    .allow_methods([Method::GET, Method::POST]);
```

## Next Steps

- [WMS Endpoints](./wms.md) - Detailed WMS operations
- [WMTS Endpoints](./wmts.md) - Detailed WMTS operations
- [REST API](./rest-api.md) - Admin and metadata endpoints
- [Examples](./examples.md) - Integration examples
