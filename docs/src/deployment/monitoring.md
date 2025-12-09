# Monitoring

Weather WMS provides comprehensive monitoring through Prometheus metrics and Grafana dashboards.

## Overview

**Stack**:
- **Prometheus**: Metrics collection and storage
- **Grafana**: Visualization and alerting
- **Structured Logs**: JSON logs to stdout

## Quick Start (Docker Compose)

Monitoring is included by default:

```bash
./scripts/start.sh

# Access Grafana
open http://localhost:3001
# Login: admin/admin

# Access Prometheus
open http://localhost:9090
```

## Metrics Endpoints

All services expose Prometheus metrics:

```bash
# WMS API
curl http://localhost:8080/metrics

# Downloader
curl http://localhost:8081/metrics
```

## Key Metrics

### Request Metrics

```
# Total requests
wms_requests_total{endpoint="GetMap",status="200"} 150234

# Request duration (histogram)
wms_request_duration_seconds_bucket{endpoint="GetMap",le="0.1"} 120000
wms_request_duration_seconds_sum{endpoint="GetMap"} 15234.5
wms_request_duration_seconds_count{endpoint="GetMap"} 150234
```

### Cache Metrics

```
# Cache hits by tier
wms_cache_hits_total{tier="l1"} 125000
wms_cache_hits_total{tier="l2"} 20000

# Cache misses
wms_cache_misses_total 5000

# Cache hit rate (calculated)
rate(wms_cache_hits_total[5m]) / (rate(wms_cache_hits_total[5m]) + rate(wms_cache_misses_total[5m]))
```

### System Metrics

```
# Active connections
wms_active_connections 42

# Memory usage
process_resident_memory_bytes 524288000

# CPU usage
process_cpu_seconds_total 1234.5
```

## Grafana Dashboards

### Pre-built Dashboards

Grafana dashboards are included in `deploy/grafana/provisioning/dashboards/`:

**Main Performance Dashboard** (`wms-performance.json`):
- Request Rate (WMS, WMTS, Renders)
- Request Latency (p50, p95, p99)
- Cache Hit Rates (L1, L2, Total)
- Error Rate
- Memory Usage
- CPU Usage
- Per-source parsing stats

**Per-Model Pipeline Dashboards**:
- `gfs-pipeline.json` - GFS model metrics
- `hrrr-pipeline.json` - HRRR model metrics  
- `goes-pipeline.json` - GOES satellite metrics
- `mrms-pipeline.json` - MRMS radar metrics

Each pipeline dashboard shows:
- Parse times and cache hit rates
- Request distribution by layer
- Grid cache efficiency
- Render performance

### Import Dashboard

```bash
# Via Grafana UI
1. Login to Grafana
2. Click "+" → "Import"
3. Upload deploy/grafana/provisioning/dashboards/*.json
4. Select Prometheus data source
5. Click "Import"
```

### Auto-Provisioning

Dashboards are auto-provisioned on startup via:
```yaml
# deploy/grafana/provisioning/dashboards/default.yaml
apiVersion: 1
providers:
- name: default
  folder: ''
  type: file
  options:
    path: /var/lib/grafana/dashboards
```

## Alerting

### Prometheus Alert Rules

```yaml
# prometheus-alerts.yml
groups:
- name: weather-wms
  rules:
  - alert: HighErrorRate
    expr: |
      rate(wms_requests_total{status=~"5.."}[5m])
      / rate(wms_requests_total[5m]) > 0.05
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "High error rate detected"
      description: "Error rate is {{ $value | humanizePercentage }} over last 5 minutes"
  
  - alert: LowCacheHitRate
    expr: |
      rate(wms_cache_hits_total[5m])
      / (rate(wms_cache_hits_total[5m]) + rate(wms_cache_misses_total[5m])) < 0.7
    for: 10m
    labels:
      severity: warning
    annotations:
      summary: "Cache hit rate below 70%"
      description: "Cache hit rate is {{ $value | humanizePercentage }}"
  
  - alert: ServiceDown
    expr: up{job="wms-api"} == 0
    for: 1m
    labels:
      severity: critical
    annotations:
      summary: "WMS API service is down"
      description: "{{ $labels.instance }} is not responding"
```

### Grafana Alerts

Configure in Grafana:
1. **Dashboards** → Select panel → **Edit**
2. **Alert** tab → **Create Alert**
3. Set conditions and thresholds
4. Configure notification channels (email, Slack, PagerDuty)

## Logging

### Structured JSON Logs

All services emit structured JSON logs:

```json
{
  "timestamp": "2024-12-03T18:30:45.123Z",
  "level": "INFO",
  "target": "wms_api::handlers",
  "message": "Handled GetMap request",
  "layer": "gfs_TMP_2m",
  "bbox": "-180,-90,180,90",
  "cache_hit": true,
  "duration_ms": 2.5
}
```

### Log Aggregation

#### Docker Compose (Loki)

```yaml
# docker-compose.yml
services:
  loki:
    image: grafana/loki:latest
    ports:
      - "3100:3100"
  
  promtail:
    image: grafana/promtail:latest
    volumes:
      - /var/lib/docker/containers:/var/lib/docker/containers:ro
      - ./promtail-config.yml:/etc/promtail/config.yml
```

#### Kubernetes (EFK Stack)

Use Elasticsearch + Fluentd + Kibana:

```bash
helm install elasticsearch elastic/elasticsearch -n logging
helm install fluentd fluent/fluentd -n logging
helm install kibana elastic/kibana -n logging
```

## Custom Dashboards

### PromQL Examples

**Average request latency**:
```promql
rate(wms_request_duration_seconds_sum[5m])
/ rate(wms_request_duration_seconds_count[5m])
```

**Cache hit rate**:
```promql
sum(rate(wms_cache_hits_total[5m]))
/ (sum(rate(wms_cache_hits_total[5m])) + sum(rate(wms_cache_misses_total[5m])))
```

**Requests per endpoint**:
```promql
sum by (endpoint) (rate(wms_requests_total[5m]))
```

**95th percentile latency**:
```promql
histogram_quantile(0.95,
  rate(wms_request_duration_seconds_bucket[5m])
)
```

## Health Checks

All services provide health endpoints:

```bash
# WMS API
curl http://localhost:8080/health
# {"status":"ok"}

curl http://localhost:8080/ready
# {"ready":true,"database":true,"redis":true,"storage":true}
```

## Performance Monitoring

### Key Performance Indicators (KPIs)

| Metric | Target | Critical |
|--------|--------|----------|
| Request latency (p99) | <100ms | >500ms |
| Cache hit rate | >85% | <70% |
| Error rate | <1% | >5% |
| Availability | >99.9% | <99% |

### Capacity Planning

Monitor these metrics for scaling decisions:

- CPU utilization (scale at >70%)
- Memory utilization (scale at >80%)
- Request rate (scale proactively)
- Cache miss rate (increase cache size)

## Troubleshooting

### High Latency

```promql
# Identify slow endpoints
topk(5, avg by (endpoint) (
  rate(wms_request_duration_seconds_sum[5m])
  / rate(wms_request_duration_seconds_count[5m])
))
```

### Memory Leaks

```promql
# Memory growth over time
rate(process_resident_memory_bytes[1h])
```

### Cache Performance

```promql
# Cache hit rate by tier
rate(wms_cache_hits_total[5m])
/ (rate(wms_cache_hits_total[5m]) + rate(wms_cache_misses_total[5m]))
```

## Next Steps

- [Architecture: Caching](../architecture/caching.md) - Cache optimization
- [WMS API Service](../services/wms-api.md) - Service metrics
- [Deployment](./README.md) - Deployment options
