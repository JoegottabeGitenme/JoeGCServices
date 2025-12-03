# Docker Compose Deployment

Docker Compose provides a simple way to run Weather WMS locally for development and testing.

## Prerequisites

- Docker 20.10+
- Docker Compose v2.0+
- 8 GB RAM minimum
- 50 GB disk space

## Quick Start

```bash
# Clone repository
git clone https://github.com/JoegottabeGitenme/JoeGCServices.git
cd JoeGCServices

# Configure environment
cp .env.example .env

# Start services
./scripts/start.sh
```

## Services

The `docker-compose.yml` defines 8 services:

```yaml
services:
  postgres:      # PostgreSQL 15 (metadata catalog)
  redis:         # Redis 7 (L2 cache)
  minio:         # MinIO (object storage)
  wms-api:       # WMS/WMTS API server
  ingester:      # Data ingestion service
  downloader:    # Data download service
  renderer-worker: # Background tile rendering
  web:           # Web dashboard (Python)
```

## Common Commands

```bash
# Start all services
docker-compose up -d

# View logs
docker-compose logs -f wms-api

# Restart a service
docker-compose restart wms-api

# Stop all services
docker-compose down

# Stop and remove volumes (deletes data!)
docker-compose down -v

# Scale a service
docker-compose up -d --scale renderer-worker=3

# Check service status
docker-compose ps
```

## Configuration Override

Create `docker-compose.override.yml` for local customization:

```yaml
version: '3.8'

services:
  wms-api:
    ports:
      - "9090:8080"  # Use different port
    environment:
      RUST_LOG: debug  # Enable debug logging
      
  postgres:
    volumes:
      - /custom/path:/var/lib/postgresql/data
```

Docker Compose automatically merges with `docker-compose.yml`.

## Volumes

Data is persisted in Docker volumes:

- `postgres_data`: PostgreSQL database
- `minio_data`: MinIO object storage
- `redis_data`: Redis cache (optional persistence)

```bash
# List volumes
docker volume ls | grep weather-wms

# Inspect volume
docker volume inspect weather-wms_postgres_data

# Backup volume
docker run --rm -v weather-wms_postgres_data:/data -v $(pwd):/backup ubuntu tar czf /backup/postgres_backup.tar.gz /data
```

## Accessing Services

| Service | URL | Credentials |
|---------|-----|-------------|
| WMS API | http://localhost:8080 | - |
| Web Dashboard | http://localhost:8000 | - |
| Grafana | http://localhost:3001 | admin/admin |
| Prometheus | http://localhost:9090 | - |
| MinIO Console | http://localhost:9001 | minioadmin/minioadmin |

## Troubleshooting

### Port Conflicts

Change ports in `docker-compose.override.yml`:

```yaml
services:
  wms-api:
    ports:
      - "8888:8080"
```

### Out of Memory

Increase Docker memory limit:
- Docker Desktop: Settings → Resources → Memory
- Allocate at least 8 GB

### Slow Performance

Use bind mounts for faster I/O on macOS/Windows:

```yaml
services:
  minio:
    volumes:
      - type: bind
        source: ./data/minio
        target: /data
```

## Production Considerations

Docker Compose is **not recommended for production**. For production deployments:

- Use [Kubernetes](./kubernetes.md) or [Helm](./helm.md)
- Managed PostgreSQL (AWS RDS, Cloud SQL)
- Managed Redis (ElastiCache, MemoryStore)
- S3 instead of MinIO

## Next Steps

- [Quick Start Guide](../getting-started/quickstart.md) - Download data
- [Configuration](../configuration/README.md) - Customize settings
- [Monitoring](./monitoring.md) - Set up dashboards
