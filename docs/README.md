# Weather WMS/WMTS

A Docker-native OGC WMS, WMTS, and EDR service for meteorological data, written in Rust.

## Overview

This project implements complete OGC Web Map Service (WMS), Web Map Tile Service (WMTS), and Environmental Data Retrieval (EDR) APIs for weather data visualization, designed to run with Docker Compose with the following components:

- **Downloader**: Polls NOAA data sources (AWS Open Data, NOMADS) on a schedule and downloads GRIB2/NetCDF files
- **Ingester**: Processes downloaded files into Zarr format with multi-resolution pyramids, stores in object storage
- **WMS/WMTS API**: HTTP server implementing OGC WMS 1.1.1/1.3.0 and WMTS 1.0.0 specifications, with inline rendering of gradients, contours, and wind barbs
- **EDR API**: HTTP server implementing OGC API - Environmental Data Retrieval for point, area, trajectory, and other query types

## Architecture

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                              Docker Compose Stack                             │
│                                                                              │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────────────────────┐    │
│  │   Nginx     │────▶│   WMS API   │────▶│          Redis              │    │
│  │  (reverse   │     │  (renders   │     │      (tile cache)           │    │
│  │   proxy)    │     │   inline)   │     └─────────────────────────────┘    │
│  └─────────────┘     └──────┬──────┘                                         │
│                             │                                                │
│                      ┌──────┴──────┐                                         │
│                      │             │                                         │
│                      ▼             ▼                                         │
│  ┌─────────────────────────┐  ┌─────────────────────────────────────────┐   │
│  │  PostgreSQL (Catalog)   │  │       Object Storage (MinIO/S3)         │   │
│  └─────────────────────────┘  │            (Zarr data)                  │   │
│              ▲                └─────────────────────────────────────────┘   │
│              │                             ▲                                 │
│              │                             │                                 │
│  ┌───────────┴─┐     ┌─────────────┐       │                                │
│  │  Downloader │────▶│   Ingester  │───────┘                                │
│  │  (scheduled)│     │ (GRIB2/NC   │                                        │
│  └──────┬──────┘     │  to Zarr)   │                                        │
│         │            └─────────────┘                                         │
│         ▼                                                                    │
│  ┌─────────────┐                                                             │
│  │    NOAA     │                                                             │
│  │   Sources   │                                                             │
│  └─────────────┘                                                             │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Quick Start

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/)
- [Docker Compose](https://docs.docker.com/compose/install/)
- [Rust](https://rustup.rs/) (for local development)

### Local Development

```bash
# Start with docker-compose (default, fastest)
./scripts/start.sh

# Force rebuild of Docker images
./scripts/start.sh --rebuild

# Clear Redis tile cache (after rendering changes)
./scripts/start.sh --clear-cache

# Check status
./scripts/start.sh --status

# Stop containers
./scripts/start.sh --stop

# Clean everything and start fresh
./scripts/start.sh --clean
```

### Access Services

After running the start script with docker-compose:

```bash
# Dashboard
open http://localhost:8000

# WMS API
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities"

# EDR API
curl "http://localhost:8083/edr/collections"

# MinIO Console (minioadmin/minioadmin)
open http://localhost:9001
```

## Project Structure

```
weather-wms/
├── crates/                     # Shared library crates
│   ├── wms-common/            # Common types, errors, utilities
│   ├── wms-protocol/          # OGC WMS/WMTS protocol handling
│   ├── edr-protocol/          # OGC EDR protocol handling
│   ├── grib2-parser/          # GRIB2 format parser (GFS, HRRR, MRMS)
│   ├── netcdf-parser/         # NetCDF parser (GOES satellite)
│   ├── grid-processor/        # Zarr V3 data access with chunk caching
│   ├── ingestion/             # File ingestion logic, Zarr pyramid generation
│   ├── projection/            # CRS transformations (Geographic, Mercator, Lambert, etc.)
│   ├── renderer/              # Image rendering (gradients, contours, wind barbs)
│   ├── storage/               # S3, PostgreSQL, Redis clients
│   └── test-utils/            # Test utilities and fixtures
├── services/                   # Deployable services
│   ├── downloader/            # Scheduled data downloading from NOAA
│   ├── ingester/              # GRIB2/NetCDF to Zarr processing
│   ├── wms-api/               # WMS/WMTS HTTP API server with inline rendering
│   └── edr-api/               # EDR HTTP API server
├── config/                     # Configuration files
│   ├── models/                # Model definitions (GFS, HRRR, GOES, MRMS)
│   ├── layers/                # WMS/WMTS layer definitions
│   ├── edr/                   # EDR collection definitions
│   ├── styles/                # Rendering style definitions (JSON)
│   └── ingestion.yaml         # Global ingestion settings
├── deploy/                     # Deployment configurations
│   └── production/            # Production Docker Compose files
└── scripts/                    # Development scripts
    └── start.sh               # Local dev startup script
```

## Supported Data Sources

- **GFS** (Global Forecast System) - 0.25° global grid, 384-hour forecasts
- **HRRR** (High-Resolution Rapid Refresh) - 3km CONUS grid, hourly updates
- **GOES-16/18** (Geostationary Satellites) - Multi-band imagery (visible, IR, water vapor)
- **MRMS** (Multi-Radar Multi-Sensor) - 1km precipitation and reflectivity

Additional models can be added via configuration in `config/models/`.

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

## EDR Capabilities

### OGC API - Environmental Data Retrieval
- **Version**: EDR 1.0
- **Query Types**: Position, Area, Radius, Trajectory, Corridor, Cube, Locations
- **Formats**: CoverageJSON, GeoJSON
- **Features**: Named locations, instances (model runs), data availability filtering

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

The `config/styles/` directory contains ready-to-use styles for various meteorological parameters:

| Category | Styles |
|----------|--------|
| Temperature | `temperature.json` - gradients for Celsius, Fahrenheit, anomaly |
| Precipitation | `precipitation.json`, `precip_rate.json`, `reflectivity.json` |
| Wind | `wind.json`, `wind_barbs.json` - speed gradients and barb overlays |
| Atmospheric | `atmospheric.json`, `mslp.json`, `humidity.json`, `cloud.json`, `visibility.json` |
| Convective | `cape.json`, `cin.json`, `helicity.json`, `lightning.json` |
| Satellite | `goes_ir.json`, `goes_visible.json` - GOES imagery colormaps |
| Upper Air | `geopotential.json` - height contours |

See `config/styles/README.md` for the full schema and customization guide.

## Configuration

### Directory Structure

```
config/
├── ingestion.yaml         # Global settings (database, Redis, download behavior)
├── models/                # Model definitions
│   ├── gfs.yaml          # GFS: source URLs, schedule, GRIB2 parameter mappings
│   ├── hrrr.yaml         # HRRR model
│   ├── goes16.yaml       # GOES-16 satellite bands
│   ├── goes18.yaml       # GOES-18 satellite bands
│   └── mrms.yaml         # MRMS radar products
├── layers/                # WMS/WMTS layer definitions
│   ├── gfs.yaml          # GFS layers (parameter -> style mappings, levels)
│   ├── hrrr.yaml         # HRRR layers
│   └── ...               # Layer configs per model
├── edr/                   # EDR collection definitions
│   ├── hrrr.yaml         # HRRR EDR collections
│   ├── gfs.yaml          # GFS EDR collections
│   └── locations.yaml    # Named locations for EDR
└── styles/                # Rendering style definitions (JSON)
    ├── temperature.json  # Color gradients, contour settings
    └── ...               # One file per style category
```

### Environment Variables

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
