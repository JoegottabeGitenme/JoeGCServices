# Monitoring Setup

## Overview

The WMS API now exposes detailed performance metrics via Prometheus, visualized in Grafana dashboards.

## Quick Access

- **Grafana Dashboard**: http://localhost:3000/d/wms-perf/wms-performance?orgId=1&from=now-5m&to=now
  - Username: `admin`
  - Password: `admin`
- **Prometheus**: http://localhost:9090
- **Prometheus Metrics**: http://localhost:8080/metrics
- **JSON Metrics API**: http://localhost:8080/api/metrics

## Available Metrics

### Request Metrics
- `wms_requests_total` - Total WMS requests
- `wmts_requests_total` - Total WMTS requests
- `cache_hits_total` - Cache hits
- `cache_misses_total` - Cache misses
- `renders_total` - Total render operations

### Pipeline Stage Metrics (Histograms)
All metrics include percentiles (p50, p90, p95, p99, max):

- `grib_load_duration_ms` - Time to load GRIB file from storage
- `grib_parse_duration_ms` - Time to parse and decompress GRIB2
- `resample_duration_ms` - Time for grid resampling
- `png_encode_duration_ms` - Time for PNG encoding
- `render_duration_ms` - Total render time

### Layer Type Metrics
- `render_duration_by_type_ms{layer_type="gradient"}` - Gradient layer renders
- `render_duration_by_type_ms{layer_type="wind_barbs"}` - Wind barb renders
- `render_duration_by_type_ms{layer_type="isolines"}` - Isoline renders

## Dashboard Panels

The Grafana dashboard includes:

1. **Requests per Second** - WMS and WMTS request rates
2. **Cache Hit Rate** - Percentage of requests served from cache
3. **Pipeline Stage Latencies** - p50 latency for each stage:
   - GRIB Load
   - GRIB Parse
   - Resample
   - PNG Encode
4. **Total Render Latency** - p50/p90/p99/max percentiles
5. **Render Count** - Total renders since startup
6. **Avg Render Time** - p50 latency
7. **Cache Hits** - Total cache hits
8. **Cache Misses** - Total cache misses

## Querying Prometheus Directly

Example queries:

```bash
# Current request rate (per second)
curl -s "http://localhost:9090/api/v1/query?query=rate(wms_requests_total[1m])"

# p99 render latency
curl -s "http://localhost:9090/api/v1/query?query=render_duration_ms{quantile=\"0.99\"}"

# Cache hit rate
curl -s "http://localhost:9090/api/v1/query?query=rate(cache_hits_total[1m])/(rate(cache_hits_total[1m])+rate(cache_misses_total[1m]))*100"

# Pipeline breakdown percentages
curl -s "http://localhost:9090/api/v1/query?query=grib_load_duration_ms_sum/render_duration_ms_sum*100"
```

## JSON Metrics API

For application dashboards, use the JSON endpoint:

```bash
curl -s http://localhost:8080/api/metrics | jq
```

Returns:
```json
{
  "metrics": {
    "grib_load_avg_ms": 1.44,
    "grib_parse_avg_ms": 0.99,
    "resample_avg_ms": 0.29,
    "png_encode_avg_ms": 0.58,
    "render_avg_ms": 3.13,
    "cache_hit_rate": 100.0,
    ...
  }
}
```

## Starting/Stopping

```bash
# Start monitoring stack
docker-compose up -d prometheus grafana

# Stop monitoring stack
docker-compose stop prometheus grafana

# View logs
docker-compose logs -f prometheus
docker-compose logs -f grafana
```

## Adding Custom Dashboards

1. Create JSON dashboard in `deploy/grafana/provisioning/dashboards/`
2. Restart Grafana: `docker-compose restart grafana`
3. Access at: http://localhost:3000

## Profiling Results

Current pipeline breakdown (from baseline tests):

| Stage       | Time   | % Total |
|-------------|--------|---------|
| GRIB Load   | 415ms  | 65%     | ← Primary bottleneck
| GRIB Parse  | 221ms  | 35%     | ← Secondary bottleneck
| Resample    | 1ms    | <1%     |
| PNG Encode  | 0.1ms  | <1%     |

**Optimization Target**: GRIB load/parse takes 99% of render time. Implementing GRIB message caching will significantly improve cache-miss performance.

## Alerting (Future)

Prometheus can be configured to send alerts via:
- Email
- Slack
- PagerDuty
- Webhook

Example alert rules can be added to `deploy/prometheus/alerts.yml`.
