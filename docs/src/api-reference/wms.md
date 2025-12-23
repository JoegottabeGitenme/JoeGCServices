# WMS Endpoints

OGC Web Map Service (WMS) 1.1.1 and 1.3.0 implementation with full OGC compliance.

## GetCapabilities

Returns XML document describing available layers, styles, and capabilities.

```http
GET /wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0
```

**Response**: XML (Content-Type: `application/vnd.ogc.wms_xml`)

### Capabilities Caching

GetCapabilities responses are cached in-memory to improve response times. The cache is automatically invalidated when:
- New data is ingested (layer list changes)
- Configuration is reloaded

This ensures clients always receive up-to-date capabilities while avoiding redundant XML generation.

## GetMap

Renders a map image for specified parameters.

```http
GET /wms?
  SERVICE=WMS&
  VERSION=1.3.0&
  REQUEST=GetMap&
  LAYERS=gfs_TMP_2m&
  STYLES=temperature&
  CRS=EPSG:3857&
  BBOX=-20037508,-20037508,20037508,20037508&
  WIDTH=256&
  HEIGHT=256&
  FORMAT=image/png&
  TIME=2024-12-03T00:00:00Z
```

### Parameters

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| SERVICE | Yes | Must be "WMS" | `WMS` |
| VERSION | Yes | WMS version | `1.3.0` or `1.1.1` |
| REQUEST | Yes | Must be "GetMap" | `GetMap` |
| LAYERS | Yes | Comma-separated layer list | `gfs_TMP_2m` |
| STYLES | Yes | Comma-separated style list (or empty) | `temperature` or `` |
| CRS/SRS | Yes | Coordinate system | `EPSG:3857` |
| BBOX | Yes | Bounding box (west,south,east,north) | `-180,-90,180,90` |
| WIDTH | Yes | Image width (pixels) | `256` |
| HEIGHT | Yes | Image height (pixels) | `256` |
| FORMAT | Yes | Image format | `image/png` |
| TIME | No | Forecast time (ISO 8601) | `2024-12-03T00:00:00Z` |
| TRANSPARENT | No | Background transparency | `TRUE` |
| BGCOLOR | No | Background color (hex) | `0xFFFFFF` |

**Response**: PNG or JPEG image

## GetFeatureInfo

Queries data value at a specific pixel location.

```http
GET /wms?
  SERVICE=WMS&
  VERSION=1.3.0&
  REQUEST=GetFeatureInfo&
  QUERY_LAYERS=gfs_TMP_2m&
  LAYERS=gfs_TMP_2m&
  CRS=EPSG:4326&
  BBOX=-180,-90,180,90&
  WIDTH=256&
  HEIGHT=256&
  I=128&
  J=128&
  INFO_FORMAT=application/json
```

### Additional Parameters

| Parameter | Description |
|-----------|-------------|
| QUERY_LAYERS | Layers to query |
| I | X pixel coordinate |
| J | Y pixel coordinate |
| INFO_FORMAT | Response format: `application/json` or `text/plain` |

**Response** (JSON):

```json
{
  "layer": "gfs_TMP_2m",
  "value": 288.15,
  "units": "K",
  "location": {
    "lat": 40.0,
    "lon": -100.0
  },
  "time": "2024-12-03T00:00:00Z"
}
```

## Version Differences

### WMS 1.1.1 vs 1.3.0

| Aspect | WMS 1.1.1 | WMS 1.3.0 |
|--------|-----------|-----------|
| CRS parameter | `SRS` | `CRS` |
| BBOX axis order (EPSG:4326) | lon,lat | lat,lon |
| Exception format | `application/vnd.ogc.se_xml` | `XML` |

## Supported Formats

### Image Formats

| Format | MIME Type | Description |
|--------|-----------|-------------|
| PNG | `image/png` | Lossless with transparency (recommended) |
| JPEG | `image/jpeg` | Lossy compression, no transparency |

### Exception Formats

| Format | MIME Type | Description |
|--------|-----------|-------------|
| XML | `XML` | WMS 1.3.0 standard exception format |
| XML (1.1.1) | `application/vnd.ogc.se_xml` | WMS 1.1.1 exception format |
| Blank | `BLANK` | Returns blank/transparent image on error |
| JSON | `application/json` | JSON-formatted error details |

## OGC Compliance

The WMS implementation follows OGC WMS 1.3.0 specification strictly, including:

- **Proper exception handling**: Returns OGC ServiceException XML for invalid requests
- **Parameter validation**: Validates all required parameters with appropriate error codes
- **BBOX axis order**: Respects CRS-dependent axis ordering (lat/lon vs lon/lat)
- **Version negotiation**: Supports both 1.1.1 and 1.3.0 with proper version negotiation
- **Default style**: The literal string `default` can be used to request the default style

### Compliance Testing

A comprehensive WMS compliance test suite is available at:

```
http://localhost:8000/wms-compliance.html
```

This web-based test runner validates:
- GetCapabilities response structure and XML validity
- GetMap parameter handling and error responses
- GetFeatureInfo coordinate handling
- Exception format compliance
- All layers in the capabilities document

The test page can also be pointed at external WMS servers for comparison testing.

## See Also

- [WMS API Service](../services/wms-api.md) - Implementation details
- [Examples](./examples.md) - Integration examples
- [WMTS Endpoints](./wmts.md) - Alternative tile-based API
