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

## See Also

- [WMS API Service](../services/wms-api.md) - Implementation
- [Monitoring](../deployment/monitoring.md) - Metrics setup
