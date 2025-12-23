# Development Guide

## Quick Start

### Prerequisites

- Rust 1.75+ (or `rustup update`)
- Docker & Docker Compose (for local services)
- kubectl & minikube (for Kubernetes deployment)
- Helm (for Kubernetes package management)

### Build & Test

```bash
# Build all crates
cargo build

# Run all tests
cargo test

# Run tests for a specific crate
cargo test --package wms-common

# Run a single test
cargo test test_parse_iso8601 -- --exact

# Run with output
cargo test -- --nocapture

# Check without building
cargo check

# Format code
cargo fmt

# Lint
cargo clippy -- -D warnings
```

### Local Development (without Kubernetes)

For rapid development iteration without the overhead of Kubernetes:

**Note:** The `.env` file is automatically loaded with all local configuration.

```bash
# Terminal 1: Start PostgreSQL, Redis, MinIO
docker-compose up

# Terminal 2: Run WMS API server (automatically loads .env)
cargo run --bin wms-api

# Test it
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities"
```

The `.env` file contains:
- `DATABASE_URL` - PostgreSQL connection (weatherwms/weatherwms)
- `REDIS_URL` - Redis connection
- `S3_*` - MinIO credentials (minioadmin/minioadmin)
- `RUST_LOG` - Logging level (set to `debug` for verbose output)

### Kubernetes Deployment

For full testing with Kubernetes (slower startup, more realistic):

```bash
# Start the complete stack with minikube
./scripts/start.sh

# View status
./scripts/start.sh --status

# Stop cluster
./scripts/start.sh --stop

# Clean up and restart
./scripts/start.sh --clean
```

## Common Development Tasks

### Running a Specific Service Locally

```bash
# WMS API
cargo run --bin wms-api -- --listen 0.0.0.0:8080

# Ingester
cargo run --bin ingester

# Renderer Worker
```

### Debugging

Set environment variables for logging:

```bash
RUST_LOG=debug cargo run --bin wms-api
RUST_LOG=weather_wms=trace cargo test
```

### Making Code Changes

1. Make your changes
2. Run `cargo fmt` to format
3. Run `cargo clippy` to lint
4. Run `cargo test` to test
5. For Kubernetes: run `./scripts/start.sh --rebuild` to rebuild and redeploy

### Benchmarks

Performance benchmarks run automatically on PRs and pushes to main when these crates change:
- `crates/renderer/`
- `crates/grib2-parser/`
- `crates/projection/`
- `crates/wms-common/`

**Running benchmarks locally:**

```bash
# Run all renderer benchmarks
cargo bench --package renderer

# Run a specific benchmark
cargo bench --package renderer -- render_tile

# Compare against baseline
cargo bench --package renderer -- --save-baseline current
```

**Benchmark history:**

All benchmark results are stored permanently and can be viewed at:
- **GitHub Pages**: `https://<owner>.github.io/<repo>/dev/bench/`

The CI automatically:
- Runs benchmarks on relevant code changes
- Compares against previous results
- Posts comparison comments on PRs
- Alerts if performance regresses >10%

**Local comparison scripts:**

```bash
# Save a baseline
./scripts/save_benchmark_baseline.sh my-baseline "Description"

# Compare baselines
./scripts/compare_benchmark_baselines.sh baseline1 baseline2
./scripts/compare_benchmark_baselines.sh my-baseline --current
```

### Database

PostgreSQL is configured via environment variables:

```bash
# Connection
DATABASE_URL=postgresql://weatherwms:weatherwms@localhost:5432/weatherwms

# Connect to running instance
psql -h localhost -U weatherwms -d weatherwms
```

### Object Storage

MinIO (S3-compatible) configuration:

```bash
# Credentials
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_ENDPOINT=http://localhost:9000
S3_BUCKET=weather-data

# Access web console
# http://localhost:9001
```

### Redis Cache

```bash
# Connection
REDIS_URL=redis://localhost:6379

# Connect
redis-cli -h localhost
```

## Architecture

- **crates/**: Shared libraries
  - `wms-common/`: Common types and utilities
  - `wms-protocol/`: OGC WMS/WMTS protocol handling
  - `grib2-parser/`: GRIB2 format parsing
  - `netcdf-parser/`: NetCDF format parsing
  - `projection/`: CRS transformations
  - `renderer/`: Image rendering (gradients, contours, wind barbs)
  - `storage/`: Database, cache, and object storage clients

- **services/**: Deployable microservices
  - `wms-api/`: HTTP API server
  - `ingester/`: Data ingestion from NOAA sources

- **config/**: Configuration files (YAML-based)
  - `models/`: Model-specific configs (GFS, HRRR, GOES, MRMS)
  - `parameters/`: GRIB2 parameter tables (WMO, NCEP, MRMS)
  - `styles/`: Rendering styles (JSON)
  - `ingestion.yaml`: Global ingestion settings

## Data Ingestion Workflow

The Weather WMS system ingests weather data from public sources (AWS S3), parses GRIB2/NetCDF files, and extracts individual parameters for rendering.

### Quick Start: Admin Dashboard

**View ingestion status and configuration:**
```bash
# Start services
docker-compose up

# Open admin dashboard
open http://localhost:8000/admin.html
```

The dashboard provides:
- Real-time ingestion status
- Catalog summary (datasets, storage size)
- Recent ingestion log
- Model configuration viewer/editor
- Parameter extraction preview

### Ingestion Configuration

All ingestion is configured via YAML files in `config/`:

**Example: Add a new parameter to GFS**
```bash
# 1. Edit config file
vim config/models/gfs.yaml

# 2. Add parameter under parameters: section
- name: RH
  description: "Relative Humidity"
  levels:
    - type: height_above_ground
      value: 2
      display: "2 m above ground"
  style: atmospheric
  units: "%"

# 3. Restart ingester
docker-compose restart ingester
```

**Configuration Files:**
- `config/models/gfs.yaml` - GFS model (global forecast)
- `config/models/hrrr.yaml` - HRRR model (CONUS high-res)
- `config/models/goes16.yaml` - GOES-16 satellite
- `config/models/goes18.yaml` - GOES-18 satellite
- `config/models/mrms.yaml` - MRMS radar composite
- `config/parameters/grib2_wmo.yaml` - WMO standard parameter tables (109 params)
- `config/parameters/grib2_ncep.yaml` - NCEP local tables (73 params)
- `config/parameters/grib2_mrms.yaml` - MRMS local tables (68 params)
- `config/ingestion.yaml` - Global settings (storage, database, etc.)

### Ingestion Pipeline

```
Download → Parse → Shred → Store
```

1. **Download**: Fetch files from AWS S3 (noaa-gfs-bdp-pds, noaa-hrrr-bdp-pds, etc.)
2. **Parse**: Read GRIB2/NetCDF metadata, grid info, and data
3. **Shred**: Extract individual parameters (e.g., TMP at 2m, UGRD at 10m)
4. **Store**: Upload to MinIO/S3, catalog in PostgreSQL

**Example: GFS file processing**
```
Input:  gfs.t12z.pgrb2.0p25.f006 (486 GRIB2 messages)
Output: 
  - shredded/gfs/20251130_12/TMP_2m/f006.grib2
  - shredded/gfs/20251130_12/UGRD_10m/f006.grib2
  - shredded/gfs/20251130_12/VGRD_10m/f006.grib2
  - shredded/gfs/20251130_12/PRMSL_msl/f006.grib2
```

### Running the Ingester

```bash
# Run in Docker (recommended)
docker-compose up ingester

# Run locally (for development)
cargo run --bin ingester

# View ingester logs
docker-compose logs -f ingester

# Check catalog for ingested data
psql -h localhost -U weatherwms -d weatherwms -c "SELECT model, parameter, COUNT(*) FROM datasets GROUP BY model, parameter;"
```

### Admin API Endpoints

The WMS API exposes admin endpoints for monitoring and configuration:

```bash
# Get ingestion status
curl http://localhost:8080/api/admin/ingestion/status

# Get recent ingestion log
curl http://localhost:8080/api/admin/ingestion/log?limit=50

# Preview parameter extraction for a model
curl http://localhost:8080/api/admin/preview-shred?model=gfs

# List all model configs
curl http://localhost:8080/api/admin/config/models

# Get specific model config (raw YAML)
curl http://localhost:8080/api/admin/config/models/gfs

# Update model config
curl -X PUT http://localhost:8080/api/admin/config/models/gfs \
  -H "Content-Type: application/json" \
  -d '{"yaml": "model:\n  id: gfs\n..."}'
```

### Supported Data Models

| Model | Source | Format | Update Freq | Coverage |
|-------|--------|--------|-------------|----------|
| **GFS** | NOAA/NCEP | GRIB2 | Every 6 hours | Global |
| **HRRR** | NOAA/NCEP | GRIB2 | Hourly | CONUS |
| **GOES-16** | NOAA/NESDIS | NetCDF | Every 5-15 min | Eastern Americas |
| **GOES-18** | NOAA/NESDIS | NetCDF | Every 5-15 min | Western Americas |
| **MRMS** | NOAA/NSSL | GRIB2 | Every 2 min | CONUS |

### Manual Data Ingestion

Download and ingest test data:

```bash
# Download sample GFS file
./scripts/download_gfs.sh

# Download sample HRRR file
./scripts/download_hrrr.sh

# Download sample GOES-16 data
./scripts/download_goes.sh 16

# Download sample MRMS data
./scripts/download_mrms.sh

# Ingest downloaded files (run ingester)
cargo run --bin ingester
```

**See also:**
- [INGESTION.md](INGESTION.md) - Comprehensive ingestion guide
- [INGESTION_CONSOLIDATION_PLAN.md](INGESTION_CONSOLIDATION_PLAN.md) - Technical design doc
- [config/models/](config/models/) - Model configuration examples

## Troubleshooting

### Cargo Lock Version Mismatch

If you see:
```
lock file version `4` was found, but this version of Cargo does not understand this lock file
```

Update Rust:
```bash
rustup update
```

### Docker Network Issues

If building Docker images fails with DNS errors, try:

```bash
# Pre-pull base images
docker pull rust:latest
docker pull debian:bookworm-slim

# Or use a different registry mirror
# Edit /etc/docker/daemon.json and add registry mirrors
```

### Minikube Issues

```bash
# Restart minikube
minikube stop -p weather-wms
minikube delete -p weather-wms
minikube start -p weather-wms

# Check cluster health
kubectl cluster-info
kubectl get nodes

# View logs
kubectl logs -n weather-wms <pod-name>
```

### Dashboard Not Accessible

The Kubernetes dashboard addon may fail to start due to network issues. Use kubectl instead:

```bash
# List all resources
kubectl get all -n weather-wms

# Watch pods in real-time
kubectl get pods -n weather-wms -w

# Get detailed info about a pod
kubectl describe pod -n weather-wms <pod-name>

# View pod logs
kubectl logs -n weather-wms <pod-name>
kubectl logs -n weather-wms <pod-name> -f  # follow logs

# Get pod events
kubectl get events -n weather-wms
```

## Contributing

Before submitting PRs:

1. Run `cargo fmt` - code formatting
2. Run `cargo clippy -- -D warnings` - linting
3. Run `cargo test` - unit tests
4. Add tests for new functionality
5. Update documentation as needed

See AGENTS.md for detailed code style guidelines.
