# Quick Reference Card

## Build & Test (Local Development)

```bash
cargo build              # Build all
cargo test               # Run all tests
cargo test --package wms-common  # Test specific crate
cargo test test_name -- --exact  # Run single test
cargo fmt                # Format code
cargo clippy -- -D warnings       # Lint
```

## Running Services Locally (Fast - No Kubernetes)

```bash
# Terminal 1: Start dependencies
docker-compose up

# Terminal 2: Run API server (automatically loads .env)
cargo run --bin wms-api

# Or with debug logging (overrides .env RUST_LOG):
RUST_LOG=debug cargo run --bin wms-api

# Test it
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities"
```

Note: `.env` file is automatically loaded with database/service credentials

## Kubernetes (Full Setup)

```bash
./scripts/start.sh              # Full setup
./scripts/start.sh --quick      # Skip building images
./scripts/start.sh --status     # Show status
./scripts/start.sh --stop       # Stop cluster
./scripts/start.sh --clean      # Delete & restart
```

## Kubernetes Monitoring (instead of dashboard)

```bash
# Status
kubectl get all -n weather-wms
kubectl get pods -n weather-wms -w   # Watch live

# Logs
kubectl logs -n weather-wms <pod-name> -f

# Details
kubectl describe pod -n weather-wms <pod-name>

# Access services
kubectl port-forward -n weather-wms svc/postgresql 5432:5432
kubectl port-forward -n weather-wms svc/redis-master 6379:6379
kubectl port-forward -n weather-wms svc/wms-api 8080:8080
```

## Service Credentials

```
PostgreSQL:
  User: weatherwms
  Pass: weatherwms
  DB: weatherwms
  Host: localhost:5432

Redis:
  Host: localhost:6379
  No auth

MinIO:
  User: minioadmin
  Pass: minioadmin
  Endpoint: localhost:9000
  Console: localhost:9001
```

## Debugging

```bash
# Check if services are running
docker ps  # local Docker
kubectl get pods -n weather-wms  # Kubernetes

# View logs
cargo test -- --nocapture  # Show test output
kubectl logs -n weather-wms <pod> --previous  # Crashed pod logs
RUST_LOG=debug cargo run --bin wms-api  # Debug logs

# Execute in pod
kubectl exec -it -n weather-wms <pod> -- bash

# Connection test
kubectl exec -it -n weather-wms <pod> -- curl http://localhost:8080
```

## Common Issues

| Issue | Solution |
|-------|----------|
| `ImagePullBackOff` | `docker pull <image> && minikube -p weather-wms image load <image>` |
| `Pending` pods | `kubectl describe pod` to see resource/image issues |
| `CrashLoopBackOff` | `kubectl logs <pod> --previous` to see crash reason |
| Dashboard timeout | Use `kubectl` commands instead (see MONITORING.md) |
| Cargo lock version error | `rustup update` |
| Tests fail locally | `cargo clean && cargo build && cargo test` |

## File Structure

```
crates/              # Shared libraries
  ├── wms-common/   # Core types & errors
  ├── wms-protocol/ # OGC WMS/WMTS spec
  ├── grib2-parser/ # Data format parsing
  ├── renderer/     # Image rendering
  └── storage/      # DB/cache/S3 clients

services/            # Deployable microservices
  ├── wms-api/      # HTTP server
  ├── ingester/     # Data import

deploy/helm/         # Kubernetes manifests
scripts/             # Automation scripts
```

## Documentation

- **DEVELOPMENT.md** - Full development guide
- **MONITORING.md** - 100+ kubectl commands
- **AGENTS.md** - Code style & build commands
- **QUICKREF.md** - This file!

## Links

- [Kubernetes Docs](https://kubernetes.io/docs/)
- [Minikube Docs](https://minikube.sigs.k8s.io/)
- [kubectl Cheatsheet](https://kubernetes.io/docs/reference/kubectl/cheatsheet/)
- [Rust Book](https://doc.rust-lang.org/book/)
- [OGC WMS Spec](https://www.ogc.org/standards/wms)

## Tips

1. **Fast iteration**: Use `docker-compose up` + `cargo run` instead of full Kubernetes
2. **Debugging**: `kubectl logs -f` is your friend
3. **Testing**: Run `cargo test` before committing
4. **Formatting**: Always run `cargo fmt` before git commit
5. **Port forwarding**: Keep `kubectl port-forward` running in background terminal

## Temporal Testing Commands

### Download MRMS Data (Already Done)
```bash
# Download last 2 hours of radar data
MRMS_HOURS=2 ./scripts/download_mrms.sh

# Download last 24 hours
MRMS_HOURS=24 ./scripts/download_mrms.sh
```

### Ingest MRMS Data
```bash
# Ingest all downloaded MRMS files
for grib_file in ./data/mrms/MergedReflectivityQC_00.50/*.grib2; do
  cargo run --package ingester -- --test-file "$grib_file"
done

# Verify ingestion
psql -h localhost -U postgres -d weather_data -c \
  "SELECT reference_time, COUNT(*) FROM grib_files 
   WHERE dataset = 'mrms' GROUP BY reference_time 
   ORDER BY reference_time DESC LIMIT 10;"
```

### Run Temporal Load Tests
```bash
# Single tile temporal test (easiest to analyze)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_single_tile_temporal.yaml

# Random temporal access (unpredictable pattern)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_temporal_random.yaml

# Full temporal stress test (5 min, 30 concurrent)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/mrms_temporal_stress.yaml
```

### Monitor Cache Performance
```bash
# Check GRIB cache metrics
curl http://localhost:8080/metrics | grep -E "grib_cache"

# Monitor memory usage
docker stats wms-api --no-stream

# Watch cache hits/misses in real-time
watch -n 2 'curl -s http://localhost:8080/metrics | grep -E "cache_(hit|miss)"'
```

## GOES-19 Satellite Temporal Testing

### Download GOES Data
```bash
# Download last 3 hours of GOES-19 data (Band 08, 13 recommended - smaller files)
GOES_HOURS=3 MAX_FILES=30 ./scripts/download_goes_temporal.sh

# Download specific bands only (edit BANDS array in script)
# Band 02: Visible (28MB/file, day only)
# Band 08: Water Vapor (2.8MB/file, 24/7)
# Band 13: Clean IR (2.8MB/file, 24/7)
```

### Extract Timestamps
```bash
# Extract timestamps from downloaded GOES files
./scripts/extract_goes_timestamps.sh ./data/goes/band08 > /tmp/goes_times.txt

# Count time steps
wc -l /tmp/goes_times.txt
```

### Create Temporal Test Scenarios
```bash
# Use extracted timestamps to create YAML scenario
# Copy template from GOES_TEMPORAL_SETUP.md
# Insert timestamps into time_selection.times array
```

### Ingest GOES Data
```bash
# Ingest NetCDF files
for nc_file in ./data/goes/band08/*.nc; do
  cargo run --package ingester -- --test-file "$nc_file"
done
```

### Run GOES Temporal Tests
```bash
# After creating scenarios with actual timestamps:
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_single_tile_temporal.yaml

cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_temporal_stress.yaml
```

## GOES Temporal Test Scenarios

### Run GOES Temporal Tests
```bash
# After ingesting GOES data (5 NetCDF files):

# 1. Single tile (easiest to analyze - pure temporal test)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_single_tile_temporal.yaml

# 2. Random temporal access (unpredictable pattern)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_random_temporal.yaml

# 3. Full temporal + spatial stress (15 workers, 2 min)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_temporal_5min.yaml
```

### GOES vs MRMS Comparison
```
MRMS: 59 files × 400 KB = 24 MB (2 hours, 2-min intervals)
GOES: 5 files × 2.8 MB = 14 MB (20 min, 5-min intervals)

MRMS: Better for long animations, smaller files
GOES: Better for testing larger file cache, satellite data
```

## HRRR Temporal Test Scenarios

### Download HRRR Data
```bash
# Download 3 cycles × 3 forecast hours (9 files, ~1.3 GB)
MAX_CYCLES=3 FORECAST_HOURS="0 1 2" ./scripts/download_hrrr_temporal.sh

# Download more for extended testing
MAX_CYCLES=5 FORECAST_HOURS="0 1 2 3 6" ./scripts/download_hrrr_temporal.sh
```

### Extract HRRR Timestamps
```bash
# Shows reference time, forecast hour, and valid time
./scripts/extract_hrrr_timestamps.sh ./data/hrrr-temporal
```

### Ingest HRRR Data
```bash
for f in ./data/hrrr-temporal/*/*/*Z/*.grib2; do
  cargo run --package ingester -- --test-file "$f"
done
```

### Run HRRR Temporal Tests
```bash
# 1. Single tile (easiest - 3 files, 405 MB cache)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/hrrr_single_tile_temporal.yaml

# 2. Forecast animation (forecast hour progression)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/hrrr_forecast_animation.yaml

# 3. Multi-cycle (model comparison)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/hrrr_multi_cycle.yaml

# 4. Comprehensive (9 files, 1.2 GB cache stress test!)
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/hrrr_comprehensive_temporal.yaml
```

### Cache Comparison
```
MRMS: 59 files × 400 KB = 24 MB    (radar, 2-min updates)
GOES:  5 files × 2.8 MB = 14 MB    (satellite, 5-min scans)
HRRR:  9 files × 135 MB = 1.2 GB   (model, hourly forecasts)

HRRR is the ultimate cache stress test!
```
