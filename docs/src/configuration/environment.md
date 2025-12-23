# Environment Variables

Complete reference of all environment variables used by Weather WMS services.

## Core Services

### Database
```bash
DATABASE_URL=postgresql://weatherwms:weatherwms@postgres:5432/weatherwms
DATABASE_POOL_SIZE=50               # Connection pool size (default: 20)
```

### Redis
```bash
REDIS_URL=redis://redis:6379
REDIS_TILE_TTL_SECS=3600           # L2 cache TTL (seconds)
```

### Object Storage (MinIO/S3)
```bash
S3_ENDPOINT=http://minio:9000
S3_BUCKET=weather-data
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_REGION=us-east-1
S3_ALLOW_HTTP=true                 # Disable for production
```

### Configuration
```bash
CONFIG_DIR=/app/config             # Path to config directory (contains models/)
                                   # Used by ingester for GRIB2 parameter tables
```

## Performance Tuning

### Runtime
```bash
TOKIO_WORKER_THREADS=8             # Async runtime threads (default: CPU cores)
RUST_LOG=info                      # Logging: trace, debug, info, warn, error
RUST_BACKTRACE=1                   # Enable backtraces
```

### Tile Rendering
```bash
# Buffer pixels for wind barbs and numbers rendering
# Prevents edge clipping artifacts at tile boundaries
TILE_RENDER_BUFFER_PIXELS=120      # Default: 120px (good for 108px barbs)
                                   # Renders 496x496 and crops to 256x256
                                   # 2.4x faster than old 3x3 tile expansion
```

### Caching
```bash
# L1 (In-Memory) Tile Cache
ENABLE_L1_CACHE=true
TILE_CACHE_SIZE=10000              # Max tiles (~300 MB)
TILE_CACHE_TTL_SECS=300            # TTL: 5 minutes

# Zarr Chunk Cache (decompressed grid data chunks)
ENABLE_CHUNK_CACHE=true
CHUNK_CACHE_SIZE_MB=1024           # ~1 GB for decompressed chunks

# Prefetching
ENABLE_PREFETCH=true
PREFETCH_RINGS=2                   # Surrounding tile rings (1=8, 2=24)
PREFETCH_MIN_ZOOM=3
PREFETCH_MAX_ZOOM=12

# Tile Cache Warming (at startup)
ENABLE_CACHE_WARMING=true
CACHE_WARMING_MAX_ZOOM=4           # Warm zooms 0-4 (341 tiles)
CACHE_WARMING_HOURS=0,3,6          # Forecast hours to warm
CACHE_WARMING_LAYERS=gfs_TMP_2m:temperature;goes18_CMI_C13:goes_ir
CACHE_WARMING_CONCURRENCY=10
```

## Monitoring

```bash
GF_SECURITY_ADMIN_PASSWORD=admin       # Grafana password
GF_USERS_ALLOW_SIGN_UP=false
PROMETHEUS_RETENTION_DAYS=15
```

## Development

```bash
# Uncomment for debugging
# RUST_LOG=debug
# RUST_BACKTRACE=full
# ENABLE_CACHE_WARMING=false       # Faster startup
```

See `.env.example` for complete annotated configuration.
