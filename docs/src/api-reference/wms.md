# WMS Endpoints

OGC Web Map Service (WMS) 1.1.1 and 1.3.0 implementation.

## GetCapabilities

Returns XML document describing available layers, styles, and capabilities.

```http
GET /wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0
```

**Response**: XML (Content-Type: `application/vnd.ogc.wms_xml`)

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

## See Also

- [WMS API Service](../services/wms-api.md) - Implementation details
- [Examples](./examples.md) - Integration examples
