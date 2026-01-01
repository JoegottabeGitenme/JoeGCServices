# EDR Endpoints

OGC API - Environmental Data Retrieval (EDR) v1.1 implementation for accessing weather data.

## Overview

The EDR API provides RESTful access to weather model data through standardized query patterns. Unlike WMS which returns rendered images, EDR returns raw data values in structured formats like CoverageJSON.

## Base URL

```
http://localhost:8083/edr
```

## Conformance Classes

This implementation supports the following conformance classes:

| Conformance Class | URI |
|------------------|-----|
| Core | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/core` |
| Collections | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/collections` |
| Position | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/position` |
| Area | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/area` |
| Radius | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/radius` |
| Trajectory | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/trajectory` |
| Corridor | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/corridor` |
| Cube | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/cube` |
| Instances | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/instances` |
| CoverageJSON | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/covjson` |

## Landing Page

Returns the API landing page with links to available resources.

```http
GET /edr
```

**Response**:
```json
{
  "title": "Weather WMS EDR API",
  "description": "OGC API - Environmental Data Retrieval for weather model data",
  "links": [
    {"href": "/edr", "rel": "self", "type": "application/json"},
    {"href": "/edr/conformance", "rel": "conformance", "type": "application/json"},
    {"href": "/edr/collections", "rel": "data", "type": "application/json"}
  ]
}
```

## Conformance

Returns the conformance classes supported by this API.

```http
GET /edr/conformance
```

**Response**:
```json
{
  "conformsTo": [
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/core",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/collections",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/position",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/area",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/radius",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/trajectory",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/corridor",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/cube",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/instances",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/covjson"
  ]
}
```

## Collections

### List Collections

Returns all available collections.

```http
GET /edr/collections
```

**Response**:
```json
{
  "links": [...],
  "collections": [
    {
      "id": "hrrr-isobaric",
      "title": "HRRR - Isobaric Levels",
      "description": "Upper-air parameters on pressure levels",
      "links": [...],
      "extent": {
        "spatial": {"bbox": [[-125, 24, -66, 50]]},
        "temporal": {"interval": [["2024-12-29T00:00:00Z", null]]}
      },
      "data_queries": {
        "position": {"link": {"href": "/edr/collections/hrrr-isobaric/position", "rel": "data"}},
        "area": {"link": {"href": "/edr/collections/hrrr-isobaric/area", "rel": "data"}},
        "radius": {"link": {"href": "/edr/collections/hrrr-isobaric/radius", "rel": "data"}},
        "trajectory": {"link": {"href": "/edr/collections/hrrr-isobaric/trajectory", "rel": "data"}},
        "corridor": {"link": {"href": "/edr/collections/hrrr-isobaric/corridor", "rel": "data"}},
        "cube": {"link": {"href": "/edr/collections/hrrr-isobaric/cube", "rel": "data"}}
      },
      "crs": ["CRS:84", "EPSG:4326"],
      "output_formats": ["application/vnd.cov+json"]
    }
  ]
}
```

### Get Collection

Returns metadata for a specific collection.

```http
GET /edr/collections/{collectionId}
```

**Parameters**:

| Parameter | Required | Description |
|-----------|----------|-------------|
| collectionId | Yes | Collection identifier |

## Instances

Instances represent specific versions of a collection, typically model runs.

### List Instances

```http
GET /edr/collections/{collectionId}/instances
```

**Response**:
```json
{
  "links": [...],
  "instances": [
    {
      "id": "2024-12-29T12:00:00Z",
      "title": "HRRR run at 2024-12-29 12Z",
      "links": [...],
      "extent": {
        "temporal": {"interval": [["2024-12-29T12:00:00Z", "2024-12-31T00:00:00Z"]]}
      }
    }
  ]
}
```

### Get Instance

```http
GET /edr/collections/{collectionId}/instances/{instanceId}
```

## Query Types

The EDR API supports six query types for extracting data from collections.

### Common Query Parameters

These parameters are supported by all query types:

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| parameter-name | No | Parameter(s) to retrieve | `TMP,UGRD,VGRD` |
| datetime | No | Time instant or interval | `2024-12-29T12:00:00Z` |
| z | No | Vertical level(s) | `850` or `850,700,500` or `1000/500` or `R5/1000/100` |
| crs | No | Coordinate reference system | `CRS:84` |
| f | No | Output format | `CoverageJSON` |

### Z Parameter Formats

The `z` parameter supports multiple formats:

| Format | Example | Description |
|--------|---------|-------------|
| Single value | `z=850` | Single vertical level |
| List | `z=850,700,500` | Multiple specific levels |
| Range | `z=1000/500` | All levels between min/max |
| Recurring | `z=R5/1000/100` | 5 levels starting at 1000, decrementing by 100 |

---

## Position Query

Retrieves data at a specific geographic point.

```http
GET /edr/collections/{collectionId}/position?coords=POINT(-97.5 35.2)
GET /edr/collections/{collectionId}/instances/{instanceId}/position?coords=POINT(-97.5 35.2)
```

### Parameters

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| coords | Yes | WKT POINT or MULTIPOINT | `POINT(-97.5 35.2)` |

### Response (CoverageJSON)

```json
{
  "type": "Coverage",
  "domain": {
    "type": "Domain",
    "domainType": "Point",
    "axes": {
      "x": {"values": [-97.5]},
      "y": {"values": [35.2]},
      "t": {"values": ["2024-12-29T12:00:00Z"]},
      "z": {"values": [850]}
    }
  },
  "parameters": {
    "TMP": {
      "type": "Parameter",
      "observedProperty": {"label": {"en": "Temperature"}},
      "unit": {"symbol": "K"}
    }
  },
  "ranges": {
    "TMP": {
      "type": "NdArray",
      "dataType": "float",
      "values": [288.5]
    }
  }
}
```

---

## Area Query

Retrieves data within a polygon area.

```http
GET /edr/collections/{collectionId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))
GET /edr/collections/{collectionId}/instances/{instanceId}/area?coords=...
```

### Parameters

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| coords | Yes | WKT POLYGON or MULTIPOLYGON | `POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))` |

### Response

Returns a Coverage with `domainType: "Grid"` containing data for all grid points within the polygon.

---

## Radius Query

Retrieves data within a circular radius of a point.

```http
GET /edr/collections/{collectionId}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km
GET /edr/collections/{collectionId}/instances/{instanceId}/radius?coords=...
```

### Parameters

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| coords | Yes | WKT POINT or MULTIPOINT | `POINT(-97.5 35.2)` |
| within | Yes | Search radius value | `50` |
| within-units | Yes | Radius units | `km`, `mi`, or `m` |

### Response

Returns a Coverage with `domainType: "Grid"` containing data for all grid points within the specified radius.

---

## Trajectory Query

Retrieves data along a path defined by a linestring.

```http
GET /edr/collections/{collectionId}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)
GET /edr/collections/{collectionId}/instances/{instanceId}/trajectory?coords=...
```

### Parameters

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| coords | Yes | WKT LINESTRING variant | See formats below |

### Coordinate Formats

| Format | Example | Description |
|--------|---------|-------------|
| LINESTRING | `LINESTRING(-100 40,-99 40.5,-98 41)` | 2D path |
| LINESTRINGZ | `LINESTRINGZ(-100 40 850,-99 40.5 700,-98 41 500)` | Path with vertical levels |
| LINESTRINGM | `LINESTRINGM(-100 40 1735574400,-99 40.5 1735578000,-98 41 1735581600)` | Path with timestamps (Unix epoch) |
| LINESTRINGZM | `LINESTRINGZM(-100 40 850 1735574400,-99 40.5 700 1735578000,-98 41 500 1735581600)` | Path with both Z and M |
| MULTILINESTRING | `MULTILINESTRING((-100 40,-99 40.5),(-98 41,-97 41.5))` | Multiple path segments |

### Response

Returns a Coverage with `domainType: "Trajectory"` containing data interpolated along the path.

---

## Corridor Query

Retrieves data within a corridor (buffered path) with specified width and height.

```http
GET /edr/collections/{collectionId}/corridor?coords=LINESTRING(-100 40,-99 40.5,-98 41)&corridor-width=10&width-units=km&corridor-height=1000&height-units=m
GET /edr/collections/{collectionId}/instances/{instanceId}/corridor?coords=...
```

### Parameters

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| coords | Yes | WKT LINESTRING variant | `LINESTRING(-100 40,-99 40.5,-98 41)` |
| corridor-width | Yes | Total corridor width | `10` |
| width-units | Yes | Width units | `km`, `mi`, or `m` |
| corridor-height | Yes | Total corridor height | `1000` |
| height-units | Yes | Height units | `m`, `hPa`, or `Pa` |
| resolution-x | No | Cross-track resolution | `3` (returns left, center, right) |

### Response

Returns a CoverageCollection with multiple Coverage objects representing cross-sections of the corridor.

---

## Cube Query

Retrieves a 3D data cube defined by bounding box and vertical levels.

```http
GET /edr/collections/{collectionId}/cube?bbox=-98,35,-97,36&z=850
GET /edr/collections/{collectionId}/instances/{instanceId}/cube?bbox=...&z=...
```

### Parameters

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| bbox | Yes | Bounding box (west,south,east,north) | `-98,35,-97,36` |
| z | Yes | Vertical level(s) | `850` or `850,700,500` |
| resolution-x | No | Grid points along x-axis | `10` |
| resolution-y | No | Grid points along y-axis | `10` |

### Response

Returns a CoverageCollection with one Coverage per z-level, each containing a grid of values.

```json
{
  "type": "CoverageCollection",
  "domainType": "Grid",
  "parameters": {...},
  "coverages": [
    {
      "type": "Coverage",
      "domain": {
        "axes": {
          "x": {"start": -98, "stop": -97, "num": 10},
          "y": {"start": 36, "stop": 35, "num": 10},
          "z": {"values": [850]},
          "t": {"values": ["2024-12-29T12:00:00Z"]}
        }
      },
      "ranges": {...}
    }
  ]
}
```

---

## Error Responses

Errors follow the OGC exception format:

```json
{
  "type": "http://www.opengis.net/def/exceptions/ogcapi-edr-1/1.0/not-found",
  "title": "Not Found",
  "status": 404,
  "detail": "Collection not found: invalid-collection"
}
```

### Error Codes

| Status | Type | Description |
|--------|------|-------------|
| 400 | invalid-parameter-value | Invalid or missing required parameter |
| 404 | not-found | Resource not found |
| 413 | response-too-large | Requested data exceeds limits |
| 500 | server-error | Internal server error |

## Collections Structure

Collections are grouped by level type for weather models:

| Collection ID | Level Type | Description |
|--------------|------------|-------------|
| hrrr-isobaric | Pressure levels | 850, 700, 500 hPa, etc. |
| hrrr-surface | Ground surface | Surface pressure, CAPE, etc. |
| hrrr-height-agl | Height AGL | 2m temperature, 10m wind |
| hrrr-atmosphere | Column values | PWAT, TCDC |
| hrrr-cloud-layers | Cloud layers | Low/Mid/High cloud cover |

## Response Limits

The API enforces response size limits to prevent excessive resource usage:

| Limit | Default | Description |
|-------|---------|-------------|
| Max parameters/request | 10 | Maximum parameters per query |
| Max time steps | 48 | Maximum temporal values |
| Max vertical levels | 20 | Maximum z levels |
| Max area | 100 sq deg | Maximum bbox/polygon area |
| Max response size | 50 MB | Maximum response payload |

Exceeding limits returns a `413 Payload Too Large` error.

## Compliance Testing

A web-based compliance test suite is available at:

```
http://localhost:8000/edr-compliance.html
```

This validates all conformance classes with 150+ tests covering:
- Core API structure (landing page, conformance, collections)
- All six query types (position, area, radius, trajectory, corridor, cube)
- Parameter handling (z, datetime, crs, f, parameter-name)
- Error responses and edge cases
- CoverageJSON structure validation

## Coverage Validation

In addition to compliance testing, a coverage validation tool verifies that all advertised parameters can actually be retrieved:

```
http://localhost:8000/edr-coverage.html
```

### Purpose

The coverage validation tool addresses a critical gap: an EDR API can be fully OGC-compliant but still advertise data that doesn't exist in the database. This happens when:

- Configuration lists parameters that haven't been ingested yet
- Data expired or was deleted but config wasn't updated
- Ingestion filters excluded certain levels or parameters

### Features

- **Three test modes**:
  - **Quick**: Tests one parameter per collection
  - **Full**: Tests all parameters at one level each
  - **Thorough**: Tests all parameters at all advertised levels

- **Concurrent testing**: Runs up to 20 requests in parallel for fast validation

- **Catalog comparison**: Compares advertised collections/parameters against actual database contents (requires `catalog-check` endpoint)

- **Detailed logging**: Full request log with JSON export for debugging

- **External server support**: Can test any EDR server (gracefully skips catalog comparison if `catalog-check` endpoint unavailable)

### Results

The tool displays:
- **Pass** (green): Parameter retrieved successfully with data
- **Warn** (yellow): Request succeeded but returned no data (empty coverage)
- **Fail** (red): Request failed with error

### Catalog Check Endpoint

The coverage tool uses an optional diagnostic endpoint:

```http
GET /edr/catalog-check
```

This returns what's actually in the database (not what's configured), enabling comparison between advertised and available data:

```json
{
  "status": "ok",
  "database_contents": {
    "hrrr": {
      "parameters": ["TMP", "UGRD", "VGRD", "HGT"],
      "level_codes": [100, 103, 200],
      "level_values": [85000, 70000, 50000, 10]
    }
  }
}
```

## See Also

- [EDR API Service](../services/edr-api.md) - Service implementation details
- [API Examples](./examples.md) - EDR usage examples
- [WMS Endpoints](./wms.md) - Map tile rendering API
- [edr-protocol Crate](../crates/edr-protocol.md) - Protocol type definitions
