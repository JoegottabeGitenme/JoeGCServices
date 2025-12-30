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
    "http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/position"
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
        "position": {
          "link": {"href": "/edr/collections/hrrr-isobaric/position", "rel": "data"}
        }
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

## Position Query

Retrieves data at a specific geographic point.

### Query Collection

```http
GET /edr/collections/{collectionId}/position?coords=POINT(-97.5 35.2)
```

### Query Instance

```http
GET /edr/collections/{collectionId}/instances/{instanceId}/position?coords=POINT(-97.5 35.2)
```

### Parameters

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| coords | Yes | WKT POINT or lon,lat | `POINT(-97.5 35.2)` or `-97.5,35.2` |
| parameter-name | No | Parameter(s) to retrieve | `TMP,UGRD,VGRD` |
| datetime | No | Time instant or interval | `2024-12-29T12:00:00Z` |
| z | No | Vertical level(s) | `850` or `850,700,500` |
| crs | No | Coordinate reference system | `CRS:84` |
| f | No | Output format | `application/vnd.cov+json` |

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
    },
    "referencing": [
      {
        "coordinates": ["x", "y"],
        "system": {"type": "GeographicCRS", "id": "http://www.opengis.net/def/crs/EPSG/0/4326"}
      }
    ]
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
| 400 | invalid-parameter-value | Invalid request parameter |
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

| Limit | Default | Environment Variable |
|-------|---------|---------------------|
| Max parameters/request | 10 | (config file) |
| Max time steps | 48 | (config file) |
| Max vertical levels | 20 | (config file) |
| Max response size | 50 MB | (config file) |

Exceeding limits returns a `413 Payload Too Large` error.

## Compliance Testing

A web-based compliance test suite is available at:

```
http://localhost:8000/edr-compliance.html
```

This validates:
- Landing page structure
- Conformance declaration
- Collection metadata
- Position query functionality
- Error handling

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
- [WMS Endpoints](./wms.md) - Map tile rendering API
- [edr-protocol Crate](../crates/edr-protocol.md) - Protocol type definitions
