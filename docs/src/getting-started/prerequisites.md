# Prerequisites

Before installing Weather WMS, ensure your system meets the following requirements.

## Required Software

### For Docker Compose Deployment

- **Docker** 20.10+ and **Docker Compose** v2.0+
  - Installation guides: [Docker Desktop](https://docs.docker.com/get-docker/)
  - Verify: `docker --version && docker-compose --version`

### For Building from Source

- **Rust** 1.75 or later
  - Install via [rustup](https://rustup.rs/): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
  - Verify: `rustc --version`

- **Cargo** (included with Rust)
  - Verify: `cargo --version`

### For Kubernetes Deployment

- **kubectl** 1.27+
  - Installation: [kubectl docs](https://kubernetes.io/docs/tasks/tools/)
  - Verify: `kubectl version --client`

- **Helm** 3.12+
  - Installation: [Helm docs](https://helm.sh/docs/intro/install/)
  - Verify: `helm version`

## Optional Software

### Data Inspection Tools

- **wgrib2** - For inspecting GRIB2 files
  ```bash
  # Ubuntu/Debian
  sudo apt-get install wgrib2
  
  # macOS
  brew install wgrib2
  ```

- **ncdump** - For inspecting NetCDF files
  ```bash
  # Ubuntu/Debian
  sudo apt-get install netcdf-bin
  
  # macOS
  brew install netcdf
  ```

## Hardware Requirements

### Minimum Configuration

Suitable for testing and light development:

| Resource | Minimum |
|----------|---------|
| CPU | 2 cores |
| RAM | 4 GB |
| Storage | 20 GB |
| Network | 10 Mbps |

### Recommended Configuration

For production deployments or active development:

| Resource | Recommended |
|----------|-------------|
| CPU | 8+ cores |
| RAM | 16 GB |
| Storage | 100+ GB SSD |
| Network | 100+ Mbps |

### Storage Considerations

Weather data storage requirements vary by usage:

- **Minimal** (1-2 models, recent data only): ~10-20 GB
- **Moderate** (multiple models, 24h history): ~50-100 GB
- **Full** (all models, 7d history): ~500 GB - 1 TB

Storage is primarily used for:
- MinIO object storage (grid data)
- PostgreSQL database (metadata catalog)
- Redis cache (tile cache)

## Network Requirements

### Outbound Connectivity

Weather WMS requires internet access to download data from NOAA sources:

- **NOMADS** (GFS, HRRR): https://nomads.ncep.noaa.gov
- **MRMS** (Radar): https://mrms.ncep.noaa.gov
- **GOES** (Satellite): https://noaa-goes18.s3.amazonaws.com

### Inbound Ports

For client access (can be configured):

| Service | Default Port | Purpose |
|---------|-------------|----------|
| WMS API | 8080 | WMS/WMTS requests |
| Web Dashboard | 8000 | Admin interface |
| Grafana | 3001 | Monitoring |
| Prometheus | 9090 | Metrics |

## Operating System

Weather WMS is platform-independent and works on:

- **Linux** (Ubuntu 20.04+, Debian 11+, RHEL 8+, etc.)
- **macOS** (11.0+)
- **Windows** (with WSL2 or Docker Desktop)

Docker containers run on `linux/amd64` architecture.

## Next Steps

Once your system meets these prerequisites, proceed to:
- [Installation Guide](./installation.md) - Set up Weather WMS
- [Quick Start](./quickstart.md) - See weather data in 5 minutes
