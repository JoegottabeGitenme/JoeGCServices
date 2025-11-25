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
cargo run --bin renderer-worker
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
  - `renderer-worker/`: Tile rendering worker

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
