# Services Overview

Weather WMS is built as a microservices architecture with four main services, each with a specific responsibility. All services are written in Rust for performance and reliability.

## Service Architecture

```mermaid
graph TB
    subgraph External
        NOAA["NOAA Data Sources"]
        Client["Map Clients"]
    end
    
    subgraph Services["Weather WMS Services"]
        DL["Downloader
        Port 8081"]
        ING["Ingester
        No Port"]
        API["WMS API
        Port 8080"]
        WORKER["Renderer Worker
        No Port"]
    end
    
    subgraph Infrastructure
        PG[("PostgreSQL")]
        MINIO[("MinIO")]
        REDIS[("Redis")]
    end
    
    NOAA -->|HTTP| DL
    DL -->|Trigger| API
    API -->|Call| ING
    ING --> PG
    ING --> MINIO
    
    Client -->|WMS/WMTS| API
    API --> PG
    API --> MINIO
    API --> REDIS
    
    WORKER --> REDIS
    WORKER --> PG
    WORKER --> MINIO
```

## Service Summary

| Service | Purpose | Port | Language | Scaling |
|---------|---------|------|----------|---------|
| [WMS API](./wms-api.md) | HTTP server for OGC WMS/WMTS requests | 8080 | Rust | Horizontal |
| [Downloader](./downloader.md) | Fetches weather data from NOAA sources | 8081 | Rust | Horizontal |
| [Ingester](./ingester.md) | Parses and stores weather data | - | Rust | Vertical |
| [Renderer Worker](./renderer-worker.md) | Background tile rendering | - | Rust | Horizontal |

## Service Responsibilities

### WMS API
- Implements OGC WMS 1.1.1/1.3.0 and WMTS 1.0.0 specifications
- Serves map tiles and metadata to clients
- Manages L1 (in-memory) cache
- Coordinates with L2 (Redis) cache
- Provides REST API for admin functions
- Exposes Prometheus metrics

**Key Operations**: GetCapabilities, GetMap, GetTile, GetFeatureInfo

### Downloader
- Downloads GRIB2 and NetCDF files from NOAA
- Implements resumable downloads (HTTP Range requests)
- Retries with exponential backoff
- Triggers ingestion after successful download
- Provides HTTP API for status and control
- Maintains download state across restarts

**Data Sources**: GFS, HRRR, MRMS, GOES (from NOMADS and AWS)

### Ingester
- Parses GRIB2 files (GFS, HRRR, MRMS)
- Parses NetCDF-4 files (GOES satellite)
- Extracts weather parameters
- Shreds large grids into ~1MB chunks
- Uploads shreds to MinIO/S3
- Registers metadata in PostgreSQL catalog
- Invalidates caches on new data

**Supported Formats**: GRIB2, NetCDF-4

### Renderer Worker
- Consumes render jobs from Redis queue
- Generates PNG tiles from grid data
- Stores tiles in L2 (Redis) cache
- Supports cache warming strategies
- Runs in background (no direct client interaction)
- Scales horizontally for throughput

**Use Cases**: Cache warming, prefetching, scheduled rendering

## Communication Patterns

### Synchronous (HTTP)
- Client → WMS API: Map tile requests
- Downloader → WMS API: Ingestion triggers
- WMS API → REST clients: Admin operations

### Asynchronous (Redis Pub/Sub)
- Ingester → WMS API: Cache invalidation messages
- WMS API → Renderer Workers: Render job queue

### Database
- All services → PostgreSQL: Metadata catalog queries
- All services → MinIO: Grid data storage/retrieval
- WMS API/Workers → Redis: Cache storage

## Deployment Configurations

### Development (Docker Compose)
```yaml
# docker-compose.yml
services:
  wms-api:
    replicas: 1
    resources:
      limits:
        memory: 2G
        
  downloader:
    replicas: 1
    resources:
      limits:
        memory: 1G
        
  ingester:
    replicas: 1
    resources:
      limits:
        memory: 4G
        
  renderer-worker:
    replicas: 2
    resources:
      limits:
        memory: 2G
```

### Production (Kubernetes)
```yaml
# Horizontal Pod Autoscaling
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: wms-api
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: wms-api
  minReplicas: 3
  maxReplicas: 10
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
```

## Common Configuration

All services share common configuration via environment variables:

```bash
# Database
DATABASE_URL=postgresql://weatherwms:password@postgres:5432/weatherwms
DATABASE_POOL_SIZE=50

# Redis
REDIS_URL=redis://redis:6379

# Object Storage
S3_ENDPOINT=http://minio:9000
S3_BUCKET=weather-data
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin

# Logging
RUST_LOG=info
RUST_BACKTRACE=1
```

## Service Health Checks

Each service provides health check endpoints:

```bash
# WMS API
curl http://localhost:8080/health
# {"status":"ok"}

# Downloader
curl http://localhost:8081/health
# {"status":"ok"}

# Ingester (via logs)
docker-compose logs ingester | grep "healthy"

# Renderer Worker (via logs)
docker-compose logs renderer-worker | grep "Processing render job"
```

## Monitoring

All services expose metrics for Prometheus:

- **WMS API**: `http://localhost:8080/metrics`
- **Downloader**: `http://localhost:8081/metrics`
- **Ingester**: Logs to stdout (JSON format)
- **Renderer Worker**: Logs to stdout (JSON format)

View in Grafana: http://localhost:3001

## Inter-Service Communication

### Ingestion Flow
```
Downloader → (HTTP POST) → WMS API /admin/ingest
WMS API → (Direct call) → Ingester library
Ingester → (Redis PUBLISH) → WMS API instances
```

### Rendering Flow
```
WMS API → (Redis LPUSH) → Render queue
Renderer Worker → (Redis BRPOPLPUSH) → Claim job
Renderer Worker → (Redis SET) → Store tile
```

## Resource Requirements

### CPU (per instance)
- WMS API: 2-4 cores
- Downloader: 1-2 cores
- Ingester: 4-8 cores (CPU-intensive parsing)
- Renderer Worker: 2-4 cores

### Memory (per instance)
- WMS API: 2-4 GB (L1 cache)
- Downloader: 512 MB - 1 GB
- Ingester: 4-8 GB (large file parsing)
- Renderer Worker: 1-2 GB

### Disk I/O
- Ingester: High (reads large files, writes shards)
- Downloader: High (writes large files)
- WMS API: Medium (reads shards from MinIO)
- Renderer Worker: Medium (reads shards, writes tiles)

## Next Steps

Dive into each service for detailed documentation:

- [WMS API Service](./wms-api.md) - HTTP server and request handling
- [Downloader Service](./downloader.md) - Data fetching and resumable downloads
- [Ingester Service](./ingester.md) - Data parsing and storage
- [Renderer Worker Service](./renderer-worker.md) - Background tile generation
