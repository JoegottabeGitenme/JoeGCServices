# Configuration Overview

This page provides a quick reference for the most common configuration options. For detailed configuration of specific components, see the [Configuration](../configuration/README.md) section.

## Environment Variables

Weather WMS is configured primarily through environment variables defined in the `.env` file. Copy `.env.example` to `.env` and customize as needed.

### Database Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgresql://weatherwms:weatherwms@postgres:5432/weatherwms` | PostgreSQL connection string |
| `DATABASE_POOL_SIZE` | `50` | Connection pool size |

### Cache Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `REDIS_URL` | `redis://redis:6379` | Redis connection string |
| `ENABLE_L1_CACHE` | `true` | Enable in-memory tile cache (L1) |
| `TILE_CACHE_SIZE` | `10000` | Max entries in L1 cache (~300MB) |
| `TILE_CACHE_TTL_SECS` | `300` | L1 cache entry TTL (5 minutes) |
| `REDIS_TILE_TTL_SECS` | `3600` | L2 cache entry TTL (1 hour) |

**Cache Strategy:**
- **L1 (In-Memory)**: Ultra-fast, per-instance, limited size
- **L2 (Redis)**: Fast, shared across instances, larger capacity

### Object Storage Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `S3_ENDPOINT` | `http://minio:9000` | MinIO/S3 endpoint URL |
| `S3_BUCKET` | `weather-data` | Storage bucket name |
| `S3_ACCESS_KEY` | `minioadmin` | Access key ID |
| `S3_SECRET_KEY` | `minioadmin` | Secret access key |
| `S3_REGION` | `us-east-1` | AWS region (for S3 compatibility) |
| `S3_ALLOW_HTTP` | `true` | Allow HTTP (disable for production) |

**Storage Layout:**
```
weather-data/
├── gfs/
│   ├── TMP_2m/
│   │   ├── 2024010100_f000_shard_0000.bin
│   │   └── ...
│   └── ...
├── hrrr/
├── goes18/
└── mrms/
```

### Performance Tuning

| Variable | Default | Description |
|----------|---------|-------------|
| `TOKIO_WORKER_THREADS` | `<CPU cores>` | Async runtime worker threads |
| `RUST_LOG` | `info` | Logging level (debug, info, warn, error) |
| `ENABLE_CHUNK_CACHE` | `true` | Enable Zarr chunk caching |
| `CHUNK_CACHE_SIZE_MB` | `1024` | Chunk cache size in MB (~1GB) |

### Tile Prefetching

Predictively fetch surrounding tiles to improve perceived performance:

| Variable | Default | Description |
|----------|---------|-------------|
| `ENABLE_PREFETCH` | `true` | Enable tile prefetching |
| `PREFETCH_RINGS` | `2` | Number of surrounding rings (1=8 tiles, 2=24 tiles) |
| `PREFETCH_MIN_ZOOM` | `3` | Minimum zoom level for prefetch |
| `PREFETCH_MAX_ZOOM` | `12` | Maximum zoom level for prefetch |

### Cache Warming

Pre-render tiles at startup for faster initial requests:

| Variable | Default | Description |
|----------|---------|-------------|
| `ENABLE_CACHE_WARMING` | `true` | Enable cache warming at startup |
| `CACHE_WARMING_MAX_ZOOM` | `4` | Maximum zoom level to warm |
| `CACHE_WARMING_HOURS` | `0` | Forecast hours to warm (comma-separated) |
| `CACHE_WARMING_LAYERS` | `gfs_TMP:temperature` | Layers to warm (semicolon-separated layer:style pairs) |
| `CACHE_WARMING_CONCURRENCY` | `10` | Parallel warming tasks |

**Example:**
```bash
CACHE_WARMING_HOURS=0,3,6
CACHE_WARMING_LAYERS=gfs_TMP_2m:temperature;hrrr_TMP_2m:temperature;goes18_CMI_C13:goes_ir
```

## Configuration Files

### Model Configuration

Models are defined in `config/models/*.yaml`:

```yaml
# config/models/gfs.yaml
name: gfs
description: Global Forecast System
source: NOAA NCEP
format: grib2
projection: latlon
resolution: 0.25  # degrees
update_frequency: 6h  # hours
forecast_hours: [0, 3, 6, ..., 384]
```

See [Model Configuration](../configuration/models.md) for details.

### Style Configuration

Visualization styles are defined in `config/styles/*.json`:

```json
{
  "name": "temperature",
  "type": "gradient",
  "parameter": "TMP",
  "colormap": [
    {"value": 233.15, "color": "#0000FF"},
    {"value": 273.15, "color": "#00FF00"},
    {"value": 313.15, "color": "#FF0000"}
  ],
  "opacity": 0.7
}
```

See [Style Configuration](../configuration/styles.md) for details.

### Parameter Tables

GRIB2 parameter mappings are defined in `config/parameters/*.yaml`:

```yaml
# config/parameters/grib2_ncep.yaml
parameters:
  - discipline: 0
    category: 0
    number: 0
    abbrev: TMP
    name: Temperature
    units: K
```

See [Parameter Configuration](../configuration/parameters.md) for details.

## Docker Compose Overrides

For local customization without modifying `docker-compose.yml`, create `docker-compose.override.yml`:

```yaml
version: '3.8'

services:
  wms-api:
    ports:
      - "9090:8080"  # Change port
    environment:
      RUST_LOG: debug  # Enable debug logging
      
  postgres:
    volumes:
      - /custom/path:/var/lib/postgresql/data  # Custom data directory
```

Docker Compose automatically merges `docker-compose.yml` and `docker-compose.override.yml`.

## Common Configuration Scenarios

### High-Performance Setup

Maximize throughput for production:

```bash
# .env
DATABASE_POOL_SIZE=100
TOKIO_WORKER_THREADS=16
TILE_CACHE_SIZE=50000
CHUNK_CACHE_SIZE_MB=2048
ENABLE_PREFETCH=true
PREFETCH_RINGS=2
ENABLE_CACHE_WARMING=true
CACHE_WARMING_MAX_ZOOM=6
```

### Memory-Constrained Environment

Reduce memory usage for smaller deployments:

```bash
# .env
DATABASE_POOL_SIZE=20
TILE_CACHE_SIZE=1000
CHUNK_CACHE_SIZE_MB=256
ENABLE_PREFETCH=false
ENABLE_CACHE_WARMING=false
```

### Development Mode

Fast iteration with detailed logging:

```bash
# .env
RUST_LOG=debug
RUST_BACKTRACE=full
ENABLE_CACHE_WARMING=false  # Faster startup
TILE_CACHE_TTL_SECS=10  # Quick cache expiration for testing
```

## Monitoring Configuration

### Grafana

| Variable | Default | Description |
|----------|---------|-------------|
| `GF_SECURITY_ADMIN_PASSWORD` | `admin` | Grafana admin password |
| `GF_USERS_ALLOW_SIGN_UP` | `false` | Disable public sign-up |

Access at: http://localhost:3001

### Prometheus

| Variable | Default | Description |
|----------|---------|-------------|
| `PROMETHEUS_RETENTION_DAYS` | `15` | Metrics retention period |

Access at: http://localhost:9090

## Configuration Validation

Verify your configuration:

```bash
# Check environment variables are loaded
docker-compose config

# Test database connection
docker-compose exec wms-api sh -c 'echo $DATABASE_URL'

# View current runtime config
curl http://localhost:8080/api/config
```

## Next Steps

- [Model Configuration](../configuration/models.md) - Define data sources
- [Style Configuration](../configuration/styles.md) - Customize visualizations
- [Environment Variables](../configuration/environment.md) - Complete reference
- [Deployment](../deployment/README.md) - Production deployment guides
