# Reference

Quick reference materials, utilities, and troubleshooting guides.

## Contents

### [Scripts](./scripts.md)
Utility scripts for data management, testing, and operations.

**Quick Reference**:
- `./scripts/start.sh` - Start all services
- `./scripts/download_gfs.sh` - Download GFS data
- `./scripts/ingest_test_data.sh` - Trigger ingestion
- `./scripts/reset_test_state.sh` - Reset for testing

### [Troubleshooting](./troubleshooting.md)
Common issues and solutions.

**Common Problems**:
- Service won't start
- No data showing
- Slow performance
- Cache issues

### [Glossary](./glossary.md)
Definitions of technical terms and acronyms.

**Key Terms**:
- GRIB2, NetCDF, WMS, WMTS
- GFS, HRRR, MRMS, GOES
- CRS, EPSG, OGC

## Quick Command Reference

### Service Management

```bash
# Start services
./scripts/start.sh

# Stop services
docker-compose down

# View logs
docker-compose logs -f wms-api

# Restart service
docker-compose restart wms-api

# Check status
docker-compose ps
```

### Data Management

```bash
# Download data
./scripts/download_gfs.sh
./scripts/download_goes.sh

# Trigger ingestion
./scripts/ingest_test_data.sh

# Reset state
./scripts/reset_test_state.sh
```

### Testing

```bash
# Run tests
cargo test --workspace

# Load testing
./scripts/run_load_test.sh

# WMS validation
./scripts/validate-wms.sh
```

### Monitoring

```bash
# View metrics
curl http://localhost:8080/metrics

# Check health
curl http://localhost:8080/health

# Cache stats
curl http://localhost:8080/api/cache/list

# Storage stats
curl http://localhost:8080/api/storage/stats
```

## Environment Quick Reference

### Development
```bash
RUST_LOG=debug
RUST_BACKTRACE=1
ENABLE_CACHE_WARMING=false
```

### Production
```bash
RUST_LOG=info
ENABLE_L1_CACHE=true
TILE_CACHE_SIZE=50000
DATABASE_POOL_SIZE=100
```

## Port Reference

| Service | Port | Purpose |
|---------|------|---------|
| WMS API | 8080 | HTTP API |
| Downloader | 8081 | Status API |
| Web Dashboard | 8000 | Admin UI |
| PostgreSQL | 5432 | Database |
| Redis | 6379 | Cache |
| MinIO | 9000 | Storage API |
| MinIO Console | 9001 | Web UI |
| Prometheus | 9090 | Metrics |
| Grafana | 3001 | Dashboards |

## File Locations

| Type | Location |
|------|----------|
| Configuration | `config/` |
| Scripts | `scripts/` |
| Downloaded data | `/data/downloads/` |
| Logs | Docker logs (stdout) |
| Database | `postgres_data` volume |
| Object storage | `minio_data` volume |

## URL Quick Reference

### Local Development

- WMS API: http://localhost:8080
- GetCapabilities: http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0
- Sample Tile: http://localhost:8080/tiles/gfs_TMP_2m/temperature/4/3/5.png
- Web Dashboard: http://localhost:8000
- Grafana: http://localhost:3001 (admin/admin)
- Prometheus: http://localhost:9090
- MinIO Console: http://localhost:9001 (minioadmin/minioadmin)

## Next Steps

- [Scripts](./scripts.md) - Detailed script documentation
- [Troubleshooting](./troubleshooting.md) - Problem solving
- [Glossary](./glossary.md) - Term definitions
