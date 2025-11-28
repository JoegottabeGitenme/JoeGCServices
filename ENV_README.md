# Environment Configuration Guide

This document explains how to use the `.env` file to configure the Weather WMS system.

## Quick Start

```bash
# 1. Copy the example file
cp .env.example .env

# 2. (Optional) Edit .env to customize settings
nano .env

# 3. Start the system
./scripts/start.sh
```

The system will automatically load settings from `.env` if it exists, or use defaults from `.env.example`.

## Configuration Categories

### Performance Optimization Flags

Toggle individual performance features on/off:

```env
ENABLE_L1_CACHE=true          # In-memory tile cache (0.1ms latency)
ENABLE_GRIB_CACHE=true        # GRIB data cache (reduces I/O)
ENABLE_PREFETCH=true          # Predictive tile prefetching
ENABLE_CACHE_WARMING=true     # Pre-render tiles at startup
```

### Cache Configuration

Fine-tune cache sizes and behavior:

```env
TILE_CACHE_SIZE=10000         # L1 cache: max tiles (~300MB)
TILE_CACHE_TTL_SECS=300       # L1 cache: TTL in seconds
GRIB_CACHE_SIZE=500           # GRIB cache: max files (~2.5GB)
PREFETCH_RINGS=2              # Prefetch: 1=8 tiles, 2=24 tiles
```

### Data Ingestion

Control which data sources are downloaded:

```env
INGEST_GFS=true               # Global Forecast System
INGEST_HRRR=true              # High-Res Rapid Refresh
INGEST_GOES=true              # Satellite imagery
INGEST_MRMS=true              # Radar data

HRRR_MAX_FILES=6              # Limit HRRR files for demos
GOES_MAX_FILES=2              # Limit GOES files
```

## Common Use Cases

### Maximum Performance (Demo Mode)

```env
ENABLE_L1_CACHE=true
ENABLE_GRIB_CACHE=true
ENABLE_PREFETCH=true
ENABLE_CACHE_WARMING=true
TILE_CACHE_SIZE=15000
PREFETCH_RINGS=2
```

### Benchmarking Individual Features

Test L1 cache impact:
```bash
# Baseline (no L1 cache)
echo "ENABLE_L1_CACHE=false" >> .env
docker-compose restart wms-api
./scripts/run_load_test.sh warm_cache --save

# With L1 cache
echo "ENABLE_L1_CACHE=true" >> .env
docker-compose restart wms-api
./scripts/run_load_test.sh warm_cache --save
```

### Minimal Resource Usage

```env
ENABLE_L1_CACHE=false
ENABLE_PREFETCH=false
ENABLE_CACHE_WARMING=false
TILE_CACHE_SIZE=1000
GRIB_CACHE_SIZE=100
```

### Development Mode (Fast Startup)

```env
ENABLE_CACHE_WARMING=false    # Skip warming for faster startup
CACHE_WARMING_MAX_ZOOM=2      # Or reduce zoom if enabled
INGEST_HRRR=false             # Skip large data downloads
INGEST_GOES=false
```

## Environment Variables Reference

### Database
- `POSTGRES_USER` - Database username (default: weatherwms)
- `POSTGRES_PASSWORD` - Database password (default: weatherwms)
- `DATABASE_POOL_SIZE` - Connection pool size (default: 50)

### Redis Cache
- `REDIS_URL` - Redis connection string
- `REDIS_TILE_TTL_SECS` - L2 cache TTL (default: 3600)

### Object Storage
- `S3_ENDPOINT` - MinIO/S3 endpoint
- `S3_BUCKET` - Bucket name (default: weather-data)
- `S3_ACCESS_KEY` - Access key
- `S3_SECRET_KEY` - Secret key

### Performance
- `TOKIO_WORKER_THREADS` - Async worker threads (default: 8)
- `RUST_LOG` - Log level: debug, info, warn, error

### Monitoring
- `GF_SECURITY_ADMIN_PASSWORD` - Grafana admin password
- `PROMETHEUS_RETENTION_DAYS` - Data retention (default: 15)

## Applying Changes

Most settings require a restart:

```bash
# For docker-compose changes
docker-compose restart wms-api

# For complete rebuild (if code changed)
docker-compose up -d --build wms-api

# For ingestion settings
./scripts/start.sh  # Will use new settings
```

## Troubleshooting

### Changes Not Taking Effect

1. Verify `.env` file exists in project root
2. Check syntax (no spaces around `=`)
3. Restart the service: `docker-compose restart wms-api`
4. View applied config: `docker-compose config`

### Service Won't Start

```bash
# Check configuration validity
docker-compose config --quiet

# View detailed errors
docker-compose logs wms-api

# Reset to defaults
rm .env && cp .env.example .env
docker-compose restart
```

### Check Current Settings

```bash
# View environment variables in container
docker-compose exec wms-api env | grep ENABLE_

# View docker-compose interpolation
docker-compose config | grep -A 20 "wms-api:"
```

## Security Notes

- The `.env` file is excluded from git (via `.gitignore`)
- Never commit `.env` files with production secrets
- Use different `.env` files for dev/staging/prod
- Consider using environment-specific files: `.env.production`, `.env.staging`

## Examples

See `.env.example` for a complete reference with inline documentation.
