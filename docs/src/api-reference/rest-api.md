# REST API

Custom RESTful API for administration and metadata queries.

## Health & Status

### Health Check
```http
GET /health
```

Response: `{"status":"ok"}`

### Readiness Check
```http
GET /ready
```

Response: `{"ready":true}`

## Metadata

### List Parameters
```http
GET /api/parameters/{model}
```

Example: `GET /api/parameters/gfs`

Response:
```json
{
  "model": "gfs",
  "parameters": [
    {"name": "TMP_2m", "description": "Temperature at 2m", "units": "K"},
    {"name": "UGRD_10m", "description": "U-wind at 10m", "units": "m/s"}
  ]
}
```

### Get Forecast Times
```http
GET /api/forecast-times/{model}/{parameter}
```

Example: `GET /api/forecast-times/gfs/TMP_2m`

Response:
```json
{
  "times": [
    {"forecast_time": "2024-12-03T00:00:00Z", "forecast_hours": [0,3,6,9,...,384]},
    {"forecast_time": "2024-12-02T18:00:00Z", "forecast_hours": [0,3,6,9,...,240]}
  ]
}
```

## Cache Management

### Clear Cache
```http
POST /api/cache/clear
```

Clears L1 (in-memory) cache.

### List Cached Tiles
```http
GET /api/cache/list?limit=100
```

## Storage

### Storage Statistics
```http
GET /api/storage/stats
```

Returns MinIO usage statistics.

## Admin

### Trigger Ingestion
```http
POST /admin/ingest
Content-Type: application/json

{
  "path": "/data/incoming/gfs.t00z.pgrb2.0p25.f000",
  "model": "gfs"
}
```

### Get Configuration
```http
GET /api/config
```

Returns current service configuration.

## Metrics

### Prometheus Metrics
```http
GET /metrics
```

Returns Prometheus-format metrics for monitoring.

### Application Metrics (JSON)
```http
GET /api/metrics
```

Returns JSON-formatted metrics for the web dashboard including:
- Request counts and rates (WMS/WMTS)
- Cache statistics (L1/L2 hits, misses, hit rates)
- Render statistics (count, avg/min/max times)
- Per-data-source parsing stats (GFS, HRRR, GOES, MRMS)
- Pipeline timing breakdown

Response:
```json
{
  "uptime_secs": 3600,
  "wms_requests": 15234,
  "wmts_requests": 89234,
  "wms_rate_1m": 12.5,
  "wms_count_1m": 750,
  "cache_hits": 95000,
  "cache_misses": 5000,
  "cache_hit_rate": 95.0,
  "render_avg_ms": 45.2,
  "render_count_1m": 125,
  "data_source_stats": {
    "gfs": {"cache_hit_rate": 92.5, "avg_parse_ms": 12.3},
    "goes": {"cache_hit_rate": 88.0, "avg_parse_ms": 8.5}
  }
}
```

### Container Stats
```http
GET /api/container/stats
```

Returns container/pod resource statistics:
- CPU count and load averages (1m, 5m, 15m)
- Memory usage (used, total, percentage)
- Process RSS and VMS

## Tile Request Heatmap

Geographic visualization of tile request distribution. Useful for monitoring load test patterns and identifying hot spots.

### Get Heatmap Data
```http
GET /api/tile-heatmap
```

Returns aggregated tile request locations (rounded to 0.1° for efficiency).

Response:
```json
{
  "cells": [
    {"lat": 38.5, "lng": -95.5, "count": 42},
    {"lat": 39.0, "lng": -94.0, "count": 15}
  ],
  "total_requests": 57
}
```

### Clear Heatmap
```http
POST /api/tile-heatmap/clear
```

Clears all heatmap data. Useful when starting a new load test.

Response:
```json
{"status": "cleared"}
```

## See Also

- [WMS API Service](../services/wms-api.md) - Implementation
- [Monitoring](../deployment/monitoring.md) - Metrics setup
