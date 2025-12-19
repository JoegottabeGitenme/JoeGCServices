# Troubleshooting

Common issues and solutions for Weather WMS.

## Service Issues

### Services Won't Start

**Symptom**: `docker-compose up` fails or services crash

**Possible Causes**:
1. Port conflicts
2. Insufficient resources
3. Missing environment variables
4. Database initialization failure

**Solutions**:

```bash
# Check for port conflicts
sudo lsof -i :8080  # WMS API port
sudo lsof -i :5432  # PostgreSQL port

# Check Docker resources
docker stats

# View service logs
docker-compose logs wms-api
docker-compose logs postgres

# Rebuild and restart
docker-compose down
docker-compose build --no-cache
docker-compose up -d
```

---

### Service Keeps Restarting

**Symptom**: Service shows "Restarting" in `docker-compose ps`

**Solutions**:

```bash
# View crash logs
docker-compose logs --tail=100 wms-api

# Common issues:
# - Database connection failed
# - Redis connection failed
# - MinIO connection failed

# Test connections manually
docker-compose exec wms-api sh -c 'echo $DATABASE_URL'
docker-compose exec postgres psql -U weatherwms -d weatherwms -c "SELECT 1"
```

---

## Data Issues

### No Data Showing

**Symptom**: WMS returns empty/blank tiles

**Diagnostic Steps**:

```bash
# 1. Check if data was downloaded
ls -lh /data/downloads/

# 2. Check if ingestion completed
docker-compose logs ingester | grep "Ingestion complete"

# 3. Query database
docker-compose exec postgres psql -U weatherwms -d weatherwms \
  -c "SELECT model, parameter, COUNT(*) FROM grid_catalog GROUP BY model, parameter;"

# 4. Check MinIO storage
curl http://localhost:8080/api/storage/stats

# 5. Test specific layer
curl "http://localhost:8080/api/parameters/gfs"
```

**Solutions**:

```bash
# Re-download data
./scripts/download_gfs.sh

# Re-ingest
./scripts/ingest_test_data.sh

# Check WMS GetCapabilities
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | grep -A 5 "gfs_TMP"
```

---

### Ingestion Failures

**Symptom**: "Ingestion failed" in logs

**Common Errors**:

1. **"Failed to parse GRIB2"**
   ```bash
   # Validate GRIB2 file
   wgrib2 /data/downloads/gfs.grib2
   
   # Re-download if corrupted
   rm /data/downloads/gfs.grib2
   ./scripts/download_gfs.sh
   ```

2. **"S3 upload failed"**
   ```bash
   # Check MinIO is running
   docker-compose ps minio
   
   # Test MinIO connection
   curl http://localhost:9000/minio/health/live
   
   # Check MinIO logs
   docker-compose logs minio
   ```

3. **"Database error"**
   ```bash
   # Check PostgreSQL
   docker-compose exec postgres psql -U weatherwms -c "\dt"
   
   # Reset database (⚠️ destructive!)
   docker-compose down -v
   docker-compose up -d
   ```

---

## Performance Issues

### Slow Tile Rendering

**Symptom**: Tiles take >5 seconds to load

**Diagnostic**:

```bash
# Check cache hit rate
curl http://localhost:8080/metrics | grep cache_hits

# Check active connections
curl http://localhost:8080/metrics | grep active_connections

# Check MinIO performance
docker stats minio
```

**Solutions**:

1. **Low cache hit rate (<70%)**
   ```bash
   # Increase cache size
   # Edit .env:
   TILE_CACHE_SIZE=50000
   REDIS_TILE_TTL_SECS=7200
   
   # Restart
   docker-compose restart wms-api
   ```

2. **High memory usage**
   ```bash
   # Reduce cache size
   TILE_CACHE_SIZE=5000
   
   # Add more instances
   docker-compose up -d --scale wms-api=3
   ```

3. **Slow storage**
   ```bash
   # Check MinIO performance
   docker-compose exec minio sh -c "mc admin info local"
   
   # Use SSD for MinIO data
   # Edit docker-compose.yml volumes
   ```

---

### High CPU Usage

**Symptom**: CPU usage >90% sustained

**Solutions**:

```bash
# Profile CPU usage
./scripts/profile_flamegraph.sh wms-api

# Increase worker threads
TOKIO_WORKER_THREADS=16

# Scale horizontally
```

---

### Memory Leaks

**Symptom**: Memory usage continuously grows

**Diagnostic**:

```bash
# Monitor memory over time
watch -n 5 'docker stats --no-stream'

# Check for memory leaks
docker-compose exec wms-api cat /proc/$(pgrep wms-api)/status | grep Vm
```

**Solutions**:

```bash
# Restart services periodically (workaround)
# Add to crontab:
0 3 * * * docker-compose restart wms-api

# Report issue with:
# - Memory usage pattern
# - Duration to leak
# - Workload characteristics
```

---

## Cache Issues

### Cache Not Working

**Symptom**: Every request is slow (cache miss)

**Diagnostic**:

```bash
# Check L1 cache
curl http://localhost:8080/api/cache/list | jq '.count'

# Check L2 cache (Redis)
docker-compose exec redis redis-cli INFO keyspace

# Check configuration
curl http://localhost:8080/api/config | jq '.l1_cache_enabled'
```

**Solutions**:

```bash
# Verify cache is enabled
# .env:
ENABLE_L1_CACHE=true

# Clear and rebuild cache
curl -X POST http://localhost:8080/api/cache/clear
./scripts/reset_test_state.sh

# Test cache
curl "http://localhost:8080/tiles/gfs_TMP_2m/temperature/4/3/5.png"
# Second request should be faster
curl "http://localhost:8080/tiles/gfs_TMP_2m/temperature/4/3/5.png"
```

---

### Redis Connection Failed

**Symptom**: "Failed to connect to Redis"

**Solutions**:

```bash
# Check Redis is running
docker-compose ps redis

# Test connection
docker-compose exec redis redis-cli PING
# Should return: PONG

# Check URL
echo $REDIS_URL
# Should be: redis://redis:6379

# Restart Redis
docker-compose restart redis
```

---

## Network Issues

### Connection Timeouts

**Symptom**: Requests timeout after 30s

**Solutions**:

```bash
# Increase timeout
# In client code or nginx:
proxy_read_timeout 120s;

# Check network latency
docker-compose exec wms-api ping postgres

# Check database connections
docker-compose exec postgres psql -U weatherwms -c \
  "SELECT count(*) FROM pg_stat_activity;"
```

---

### Port Already in Use

**Symptom**: "bind: address already in use"

**Solutions**:

```bash
# Find process using port
sudo lsof -i :8080

# Kill process
kill -9 <PID>

# Or change port in docker-compose.override.yml
services:
  wms-api:
    ports:
      - "8888:8080"
```

---

## Database Issues

### Connection Pool Exhausted

**Symptom**: "connection pool timeout"

**Solutions**:

```bash
# Increase pool size
DATABASE_POOL_SIZE=100

# Check active connections
docker-compose exec postgres psql -U weatherwms -c \
  "SELECT count(*), state FROM pg_stat_activity GROUP BY state;"

# Kill idle connections
docker-compose exec postgres psql -U weatherwms -c \
  "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE state = 'idle';"
```

---

### Slow Queries

**Symptom**: Database queries take >1s

**Diagnostic**:

```bash
# Enable slow query log
# In PostgreSQL:
ALTER DATABASE weatherwms SET log_min_duration_statement = 1000;

# Check slow queries
docker-compose exec postgres psql -U weatherwms -c \
  "SELECT query, calls, total_time, mean_time FROM pg_stat_statements ORDER BY mean_time DESC LIMIT 10;"
```

**Solutions**:

```bash
# Add missing indexes
# Check execution plan
EXPLAIN ANALYZE SELECT * FROM grid_catalog WHERE model='gfs' AND parameter='TMP_2m';

# Vacuum database
docker-compose exec postgres vacuumdb -U weatherwms --analyze weatherwms
```

---

## Rendering Issues

### Tiles Render as Solid Colors

**Symptom**: Each tile appears as a single solid color instead of showing gradients.

**Cause**: Style name not being passed to the renderer, causing fallback to per-tile min/max normalization.

**Solution**: Ensure the WMTS/WMS handlers pass the style parameter:
```rust
// In handlers.rs - WMTS GetTile
crate::rendering::render_weather_data_with_lut(
    ...
    Some(style),  // Must pass style name, not None
    ...
)
```

See [Rendering Pipeline](../architecture/rendering-pipeline.md) for details.

---

### Tiles Show Wrong Geographic Data

**Symptom**: Tiles outside the prime meridian region (0° longitude) show data from incorrect locations.

**Cause**: When Zarr performs partial reads, the returned data bounding box differs from the full grid bbox. If the renderer uses the wrong bbox, pixel-to-grid coordinate mapping fails.

**Solution**: Use the actual bounding box returned from Zarr partial reads:
```rust
// Use actual bbox from Zarr if available
let data_bounds = grid_result.bbox.unwrap_or_else(|| [
    entry.bbox.min_x,
    entry.bbox.min_y,
    entry.bbox.max_x,
    entry.bbox.max_y,
]);
```

See [Rendering Pipeline - Zarr Partial Reads](../architecture/rendering-pipeline.md#zarr-partial-reads).

---

### Tiles Near Dateline Look Wrong

**Symptom**: Tiles crossing the antimeridian (180° longitude) or with requests spanning negative to positive longitude appear distorted.

**Cause**: For 0-360 longitude grids (like GFS), requests spanning from negative to positive longitude (e.g., [-100°, 50°]) normalize to inverted bounds (e.g., [260°, 50°]).

**Solution**: The Zarr processor detects this case and loads the full grid instead of attempting an invalid partial read:
```rust
// Detect and handle dateline crossing
if bbox.crosses_dateline_on_360_grid(&grid_bbox) {
    // Load full grid instead of partial
}
```

---

### Vertical Line at Prime Meridian (0° Longitude)

**Symptom**: A visible vertical seam or line artifact appears at 0° longitude on GFS temperature (or other global 0-360° grid) tiles.

**Cause**: GFS uses 0-360° longitude with 0.25° resolution, creating a gap between the last grid column (359.75°) and 360°. When tiles near the prime meridian request negative longitudes (e.g., -0.1°), these normalize to the gap region (e.g., 359.9°). Without special handling, pixels in this gap fail the bounds check and render as transparent/NaN.

**Solution**: The resampling functions now detect the "wrap gap" and handle it specially:
1. Pixels in the gap (between 359.75° and 360°) skip the normal bounds check
2. Grid coordinates are calculated to position past the last column
3. Bilinear interpolation wraps from column 1439 back to column 0

This fix requires the `grid_uses_360` flag to be propagated from grid metadata through the rendering pipeline (cannot be inferred from partial read bounds).

See [Rendering Pipeline - Prime Meridian Wrap Gap](../architecture/rendering-pipeline.md#handling-the-prime-meridian-wrap-gap) for implementation details.

---

### Temperature Colors Don't Match Expected Values

**Symptom**: Temperature appears with wrong colors (e.g., hot areas showing blue).

**Possible Causes**:
1. Style file missing the requested style key
2. Unit mismatch (Kelvin vs Celsius)
3. Style range doesn't cover data values

**Solution**:
```bash
# Check style configuration
cat config/styles/temperature.json | jq '.styles | keys'

# Verify data units
# GFS temperature is in Kelvin (e.g., 293.15 K = 20°C)

# Check style covers data range
cat config/styles/temperature.json | jq '.styles.default.stops'
```

---

## Common Error Messages

### "Layer not found"

**Cause**: Layer doesn't exist or wasn't ingested

**Solution**:
```bash
# List available layers
curl "http://localhost:8080/api/parameters/gfs"

# Ingest data if missing
./scripts/download_gfs.sh
./scripts/ingest_test_data.sh
```

---

### "Invalid CRS"

**Cause**: Unsupported coordinate system

**Solution**:
Use supported CRS:
- `EPSG:4326` (Geographic)
- `EPSG:3857` (Web Mercator)
- `CRS:84` (WMS 1.1.1 equivalent of EPSG:4326)

---

### "GRIB2 parse error"

**Cause**: Corrupted or unsupported GRIB2 file

**Solution**:
```bash
# Validate with wgrib2
wgrib2 file.grib2

# Re-download
rm file.grib2
./scripts/download_gfs.sh
```

---

## Getting Help

If issues persist:

1. **Check logs**: `docker-compose logs -f`
2. **Search issues**: [GitHub Issues](https://github.com/JoegottabeGitenme/JoeGCServices/issues)
3. **Report bug**: Include:
   - Error message
   - Steps to reproduce
   - `docker-compose ps` output
   - Relevant log excerpts
   - Environment (OS, Docker version)

## Next Steps

- [Scripts Reference](./scripts.md) - Utility scripts
- [Monitoring](../deployment/monitoring.md) - Set up monitoring
- [Architecture](../architecture/README.md) - Understand system design
