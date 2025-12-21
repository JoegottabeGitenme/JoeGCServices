# Configuration Overview

Weather WMS is configured through environment variables and YAML/JSON configuration files.

## Configuration Methods

### 1. Environment Variables

Primary configuration via `.env` file:

```bash
# Copy example
cp .env.example .env

# Edit with your values
nano .env
```

See [Environment Variables](./environment.md) for complete reference.

### 2. Model Configuration

YAML files in `config/models/`:

```yaml
# config/models/gfs.yaml
name: gfs
description: Global Forecast System
resolution: 0.25
format: grib2
```

See [Model Configuration](./models.md)

### 3. Style Configuration

JSON files in `config/styles/`:

```json
{
  "name": "temperature",
  "type": "gradient",
  "colormap": [...]
}
```

See [Style Configuration](./styles.md)

### 4. Parameter Tables

YAML files in `config/parameters/`:

```yaml
# config/parameters/grib2_ncep.yaml
parameters:
  - discipline: 0
    category: 0
    number: 0
    abbrev: TMP
```

See [Parameter Configuration](./parameters.md)

## Quick Configuration

### Minimal Setup (Default)

```bash
# .env with defaults works out of the box
./scripts/start.sh
```

### Custom PostgreSQL

```bash
# .env
DATABASE_URL=postgresql://user:pass@custom-host:5432/weatherwms
```

### Custom MinIO/S3

```bash
# .env
S3_ENDPOINT=https://s3.amazonaws.com
S3_BUCKET=my-weather-data
S3_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE
S3_SECRET_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
S3_ALLOW_HTTP=false
```

### High-Performance Tuning

```bash
# .env
DATABASE_POOL_SIZE=100
TOKIO_WORKER_THREADS=16
TILE_CACHE_SIZE=50000
CHUNK_CACHE_SIZE_MB=2048
ENABLE_PREFETCH=true
PREFETCH_RINGS=2
```

## Configuration Hierarchy

1. Environment variables (highest priority)
2. `.env` file
3. YAML configuration files
4. Built-in defaults (lowest priority)

## Validation

Check configuration:

```bash
# View Docker Compose configuration
docker-compose config

# Test database connection
docker-compose exec wms-api psql $DATABASE_URL -c "SELECT 1;"

# View runtime config via API
curl http://localhost:8080/api/config
```

## Next Steps

- [Environment Variables](./environment.md) - Complete reference
- [Model Configuration](./models.md) - Add custom data sources
- [Style Configuration](./styles.md) - Customize visualizations
- [Parameter Tables](./parameters.md) - GRIB2 parameter mappings
