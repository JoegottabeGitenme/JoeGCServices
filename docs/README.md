# Weather WMS/WMTS

A Kubernetes-native OGC WMS and WMTS service for meteorological data, written in Rust.

## Overview

This project implements complete OGC Web Map Service (WMS) and Web Map Tile Service (WMTS) for weather data visualization, designed to run on Kubernetes with the following components:

- **Ingester**: Polls NOAA data sources (AWS Open Data, NOMADS) and ingests GRIB2/NetCDF files
- **Renderer Workers**: Process render jobs from a queue, generating tiles with gradients, contours, and wind barbs
- **WMS/WMTS API**: HTTP server implementing OGC WMS 1.1.1/1.3.0 and WMTS 1.0.0 specifications

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Kubernetes Cluster                             │
│                                                                             │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────────────────────┐   │
│  │   Ingress   │────▶│   WMS API   │────▶│      Redis Cluster          │   │
│  │             │     │   Service   │     │  (tile cache + job queue)   │   │
│  └─────────────┘     └──────┬──────┘     └─────────────────────────────┘   │
│                             │                        ▲                      │
│                             ▼                        │                      │
│                      ┌─────────────┐                 │                      │
│                      │  Renderer   │─────────────────┘                      │
│                      │   Workers   │                                        │
│                      └──────┬──────┘                                        │
│                             │                                               │
│                             ▼                                               │
│  ┌─────────────┐     ┌─────────────────────────────────────────────────┐   │
│  │   Ingester  │────▶│              Object Storage (MinIO/S3)          │   │
│  │             │     └─────────────────────────────────────────────────┘   │
│  └─────────────┘                                                            │
│         │            ┌─────────────────────────────────────────────────┐   │
│         ▼            │              PostgreSQL (Catalog)               │   │
│  ┌─────────────┐     └─────────────────────────────────────────────────┘   │
│  │    NOAA     │                                                            │
│  │   Sources   │                                                            │
│  └─────────────┘                                                            │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Quick Start

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/)
- [minikube](https://minikube.sigs.k8s.io/docs/start/)
- [kubectl](https://kubernetes.io/docs/tasks/tools/)
- [Helm](https://helm.sh/docs/intro/install/)
- [Rust](https://rustup.rs/) (for local development)

### Local Development with Minikube

```bash
# Start the complete stack
./scripts/start.sh

# Check status
./scripts/start.sh --status

# Rebuild and redeploy after code changes
./scripts/start.sh --rebuild

# Stop the cluster
./scripts/start.sh --stop

# Clean everything and start fresh
./scripts/start.sh --clean
```

### Access Services

After running the start script:

```bash
# WMS API
kubectl port-forward -n weather-wms svc/wms-weather-wms-api 8080:8080
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities"

# MinIO Console (minioadmin/minioadmin)
kubectl port-forward -n weather-wms svc/minio 9001:9001
open http://localhost:9001

# PostgreSQL
kubectl port-forward -n weather-wms svc/postgresql 5432:5432
psql -h localhost -U weatherwms -d weatherwms
```

## Project Structure

```
weather-wms/
├── crates/                     # Shared library crates
│   ├── wms-common/            # Common types, errors, utilities
│   ├── grib2-parser/          # GRIB2 format parser
│   ├── netcdf-parser/         # NetCDF-3 parser
│   ├── projection/            # CRS transformations
│   ├── renderer/              # Image rendering (gradients, contours)
│   ├── wms-protocol/          # OGC WMS protocol handling
│   └── storage/               # S3, PostgreSQL, Redis clients
├── services/                   # Deployable services
│   ├── ingester/              # Data ingestion service
│   └── wms-api/               # HTTP API server
├── deploy/                     # Deployment configurations
│   └── helm/                  # Helm charts
│       └── weather-wms/
└── scripts/                    # Development scripts
    └── start.sh               # Local dev startup script
```

## Supported Data Sources

- **GFS** (Global Forecast System) - 0.25° global grid
- **HRRR** (High-Resolution Rapid Refresh) - 3km CONUS grid
- More models can be added via configuration

## WMS/WMTS Capabilities

### WMS Support
- **Versions**: WMS 1.1.1, WMS 1.3.0
- **Operations**: GetCapabilities, GetMap, GetFeatureInfo
- **Formats**: PNG, JPEG
- **CRS**: EPSG:4326, EPSG:3857, and more

### WMTS Support
- **Version**: WMTS 1.0.0
- **Bindings**: KVP (query string) and RESTful
- **TileMatrixSets**: WebMercatorQuad (EPSG:3857), WorldCRS84Quad (EPSG:4326)
- **Dimensions**: TIME support for temporal data

## Style Configuration

Rendering styles are defined via JSON configuration files. This allows easy customization of color gradients, contours, and other visualizations without code changes.

### Example: Temperature Gradient

```json
{
  "version": "1.0",
  "styles": {
    "temperature_celsius": {
      "name": "Temperature (Celsius)",
      "type": "gradient",
      "units": "°C",
      "transform": {
        "type": "kelvin_to_celsius"
      },
      "stops": [
        { "value": -40, "color": "#9400D3", "label": "-40°C" },
        { "value": -20, "color": "#0000FF", "label": "-20°C" },
        { "value": 0, "color": "#00FFFF", "label": "0°C" },
        { "value": 20, "color": "#FFFF00", "label": "20°C" },
        { "value": 40, "color": "#FF0000", "label": "40°C" }
      ],
      "interpolation": "linear",
      "out_of_range": "clamp"
    }
  }
}
```

### Supported Style Types

| Type | Description | Use Case |
|------|-------------|----------|
| `gradient` | Continuous color interpolation | Temperature, wind speed, humidity |
| `classified` | Discrete color classes | Precipitation type, warnings |
| `contour` | Isolines with optional labels | Pressure, geopotential height |
| `filled_contour` | Color-filled ranges | Radar reflectivity |
| `wind_barbs` | Traditional meteorological barbs | Wind visualization |
| `wind_arrows` | Vector arrows colored by speed | Wind visualization |

### Value Transforms

Transform raw data values before rendering:

- `kelvin_to_celsius` - Temperature conversion
- `kelvin_to_fahrenheit` - Temperature conversion
- `mps_to_knots` - Wind speed conversion
- `pa_to_hpa` - Pressure conversion
- `linear` - Custom scale and offset: `value * scale + offset`

### Pre-built Style Configurations

The `config/styles/` directory contains ready-to-use styles:

- `temperature.json` - Temperature gradients (Celsius, Fahrenheit, anomaly)
- `precipitation.json` - Rain rate, radar reflectivity, accumulated precip
- `wind.json` - Wind speed, barbs, arrows, gusts
- `atmospheric.json` - Pressure, humidity, cloud cover, CAPE

## Configuration

See `deploy/helm/weather-wms/values.yaml` for all configuration options.

Key environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgresql://...` |
| `REDIS_URL` | Redis connection string | `redis://redis:6379` |
| `S3_ENDPOINT` | MinIO/S3 endpoint | `http://minio:9000` |
| `S3_BUCKET` | Bucket for weather data | `weather-data` |
| `LOG_LEVEL` | Logging verbosity | `info` |

## Development

### Building Locally

```bash
# Build all crates
cargo build

# Run tests
cargo test

# Build specific service
cargo build --package wms-api
```

### Running Tests

```bash
# Unit tests
cargo test

# With logging
RUST_LOG=debug cargo test -- --nocapture
```

## OGC CITE Testing

Use the [OGC CITE TeamEngine](https://cite.opengeospatial.org/teamengine/) to validate WMS compliance:

1. Deploy the service
2. Run the WMS 1.1.1 or WMS 1.3.0 test suite against your endpoint

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR.
