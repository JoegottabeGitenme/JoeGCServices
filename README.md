# JoeGCServices

A high-performance, OGC-compliant weather data service written in Rust. Provides WMS/WMTS map tile rendering and EDR (Environmental Data Retrieval) API for accessing raw weather data from NOAA sources.

[![OGC WMS 1.3.0](https://img.shields.io/badge/OGC-WMS%201.3.0-blue)](https://www.ogc.org/standard/wms/)
[![OGC WMTS 1.0.0](https://img.shields.io/badge/OGC-WMTS%201.0.0-blue)](https://www.ogc.org/standard/wmts/)
[![OGC EDR 1.0](https://img.shields.io/badge/OGC-EDR%201.0-blue)](https://www.ogc.org/standard/ogcapi-edr/)
[![Rust](https://img.shields.io/badge/Rust-1.75+-orange)](https://www.rust-lang.org/)

## Overview

JoeGCServices automatically ingests weather data from NOAA sources and serves it through three OGC-compliant APIs:

| Service | Port | Protocol | Description |
|---------|------|----------|-------------|
| **WMS/WMTS API** | 8080 | OGC WMS 1.3.0 / WMTS 1.0.0 | Rendered map tiles (PNG, JPEG, WebP) |
| **EDR API** | 8083 | OGC API - EDR 1.0 | Raw data queries (CoverageJSON, GeoJSON) |
| **Web Dashboard** | 8000 | HTTP | Admin UI, map viewer, compliance testing |

### Key Features

- **OGC Compliant**: Full support for WMS 1.3.0, WMTS 1.0.0, and EDR 1.0 specifications
- **Real-Time Data**: Automatic ingestion from NOAA (GFS, HRRR, GOES, MRMS)
- **High Performance**: Written in Rust with two-tier caching (L1 in-memory, L2 Redis)
- **Flexible Rendering**: Gradients, contours, wind barbs, and custom colormaps
- **Cloud Native**: Docker Compose for development and production
- **Horizontally Scalable**: Stateless API services with shared storage

## Architecture

```
                                    ┌─────────────────────────────────────────────────────────────┐
                                    │                    JoeGCServices                            │
                                    │                                                             │
   ┌──────────────┐                 │   ┌─────────────┐         ┌─────────────┐                  │
   │  Map Client  │─── tiles ──────────▶│  WMS/WMTS   │────────▶│    Redis    │                  │
   │  (Leaflet,   │                 │   │    API      │         │ (tile cache)│                  │
   │  OpenLayers) │                 │   │   :8080     │         └─────────────┘                  │
   └──────────────┘                 │   └──────┬──────┘                                          │
                                    │          │                                                  │
   ┌──────────────┐                 │          │         ┌─────────────────────────────────────┐ │
   │  Data Client │─── queries ────────▶┌─────┴─────┐   │                                     │ │
   │  (Python,    │                 │   │  EDR API  │   │         MinIO / S3                  │ │
   │  curl, etc.) │                 │   │   :8083   │──▶│       (Zarr weather data)           │ │
   └──────────────┘                 │   └───────────┘   │                                     │ │
                                    │          │         └─────────────────────────────────────┘ │
   ┌──────────────┐                 │          │                          ▲                      │
   │    Admin     │─────────────────────▶┌─────┴─────┐                    │                      │
   │   Browser    │                 │   │ Dashboard │                    │                      │
   └──────────────┘                 │   │   :8000   │         ┌──────────┴──────────┐           │
                                    │   └───────────┘         │     Ingester        │           │
                                    │                         │  (GRIB2/NetCDF →    │           │
                                    │   ┌─────────────┐       │   Zarr pyramids)    │           │
                                    │   │ PostgreSQL  │       └──────────┬──────────┘           │
                                    │   │  (catalog)  │                  │                      │
                                    │   └─────────────┘       ┌──────────┴──────────┐           │
                                    │          ▲              │    Downloader       │           │
                                    │          └──────────────│  (scheduled NOAA    │           │
                                    │                         │   data fetching)    │           │
                                    │                         └──────────┬──────────┘           │
                                    └─────────────────────────────────────┼───────────────────────┘
                                                                         │
                                                                         ▼
                                                              ┌─────────────────────┐
                                                              │    NOAA Sources     │
                                                              │  (AWS Open Data,    │
                                                              │   NOMADS, etc.)     │
                                                              └─────────────────────┘
```

## Quick Start

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and Docker Compose
- 8 GB RAM minimum (16 GB recommended)
- 50 GB disk space

### Start the Services

```bash
# Clone the repository
git clone https://github.com/JoegottabeGitenme/JoeGCServices.git
cd JoeGCServices

# Copy environment file (optional - defaults work out of the box)
cp .env.example .env

# Start all services
./scripts/start.sh
```

The start script will:
1. Build Docker images (first run takes ~5-10 minutes)
2. Start PostgreSQL, Redis, MinIO, and all API services
3. Initialize the database and storage bucket
4. Begin downloading weather data automatically

### Access the Services

| Service | URL | Credentials |
|---------|-----|-------------|
| **Dashboard** | http://localhost:8000 | - |
| **WMS Capabilities** | http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities | - |
| **EDR Landing Page** | http://localhost:8083/edr | - |
| **MinIO Console** | http://localhost:9001 | minioadmin / minioadmin |
| **Grafana** | http://localhost:3000 | admin / admin |
| **pgAdmin** | http://localhost:5050 | admin@localhost.com / admin |

### Common Commands

```bash
# Start services (default)
./scripts/start.sh

# Development mode (faster builds, debug symbols)
./scripts/start.sh --dev

# Force rebuild images
./scripts/start.sh --rebuild

# Clear tile cache (after style changes)
./scripts/start.sh --clear-cache

# Check service status
./scripts/start.sh --status

# Stop all services
./scripts/start.sh --stop

# Clean everything (removes data)
./scripts/start.sh --clean
```

## Supported Weather Data

| Model | Type | Resolution | Coverage | Update Frequency |
|-------|------|------------|----------|------------------|
| **GFS** | Forecast | 0.25° | Global | Every 6 hours |
| **HRRR** | Forecast | 3 km | CONUS | Hourly |
| **GOES-16** | Satellite | 1-2 km | Eastern US | 5-15 minutes |
| **GOES-18** | Satellite | 1-2 km | Western US | 5-15 minutes |
| **MRMS** | Radar | 1 km | CONUS | 2 minutes |

Additional models can be added via configuration in `config/models/`.

## WMS/WMTS API

The WMS/WMTS API renders weather data as map tiles for use in web mapping applications.

### WMS Example

```bash
# Get capabilities
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities"

# Get a map tile (GFS temperature)
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap\
&LAYERS=gfs_TMP&STYLES=temperature&CRS=EPSG:4326\
&BBOX=-90,-180,90,180&WIDTH=512&HEIGHT=256&FORMAT=image/png" -o map.png
```

### WMTS RESTful Tiles

```bash
# Get a tile using RESTful URL pattern
curl "http://localhost:8080/wmts/rest/gfs_TMP/temperature/WebMercatorQuad/3/4/2.png" -o tile.png
```

### XYZ Tiles (Leaflet/OpenLayers)

```javascript
// Leaflet example
L.tileLayer('http://localhost:8080/tiles/gfs_TMP/temperature/{z}/{x}/{y}.png', {
  attribution: 'JoeGCServices'
}).addTo(map);
```

### Supported Features

- **WMS Versions**: 1.1.1, 1.3.0
- **WMTS Version**: 1.0.0
- **Operations**: GetCapabilities, GetMap, GetTile, GetFeatureInfo
- **Formats**: PNG, JPEG, WebP
- **CRS**: EPSG:4326, EPSG:3857, CRS:84
- **Dimensions**: TIME, RUN, FORECAST, ELEVATION

## EDR API

The EDR (Environmental Data Retrieval) API provides access to raw weather data values through standardized query patterns.

### Query Types

| Query | Description | Example Use Case |
|-------|-------------|------------------|
| **Position** | Point data at coordinates | Weather at a specific location |
| **Area** | Data within a polygon | Regional analysis |
| **Radius** | Data within a circle | Weather near an airport |
| **Trajectory** | Data along a path | Flight route weather |
| **Corridor** | Buffered path data | Highway weather corridor |
| **Cube** | 3D volume data | Vertical atmospheric profile |
| **Locations** | Named locations (airports, cities) | KJFK, KLAX, etc. |

### EDR Examples

```bash
# Get collections
curl "http://localhost:8083/edr/collections"

# Position query - temperature at a point
curl "http://localhost:8083/edr/collections/hrrr-surface/position?\
coords=POINT(-97.5 35.2)&parameter-name=TMP&datetime=2024-01-15T12:00:00Z"

# Area query - data within a polygon
curl "http://localhost:8083/edr/collections/gfs-surface/area?\
coords=POLYGON((-100 35,-95 35,-95 40,-100 40,-100 35))&parameter-name=TMP"

# Location query - weather at JFK airport
curl "http://localhost:8083/edr/collections/hrrr-surface/locations/KJFK?\
parameter-name=TMP,UGRD,VGRD"

# Trajectory query - data along a flight path
curl "http://localhost:8083/edr/collections/gfs-isobaric/trajectory?\
coords=LINESTRING(-122.4 37.8,-87.6 41.9,-74.0 40.7)&z=500&parameter-name=TMP,HGT"
```

### Response Formats

- **CoverageJSON** (default): Full metadata with units and axes
- **GeoJSON**: Standard GIS format for easy integration

```bash
# Request GeoJSON format
curl "http://localhost:8083/edr/collections/hrrr-surface/position?\
coords=POINT(-97.5 35.2)&f=geojson"
```

### EDR Conformance Classes

| Conformance Class | Status |
|------------------|--------|
| Core | Supported |
| Collections | Supported |
| Position | Supported |
| Area | Supported |
| Radius | Supported |
| Trajectory | Supported |
| Corridor | Supported |
| Cube | Supported |
| Locations | Supported |
| Instances | Supported |
| CoverageJSON | Supported |

## Configuration

### Directory Structure

```
config/
├── ingestion.yaml         # Global settings (database, storage, schedules)
├── models/                # Weather model definitions
│   ├── gfs.yaml          # GFS parameters, levels, download schedule
│   ├── hrrr.yaml         # HRRR configuration
│   ├── goes16.yaml       # GOES-16 satellite bands
│   ├── goes18.yaml       # GOES-18 satellite bands
│   └── mrms.yaml         # MRMS radar products
├── layers/                # WMS/WMTS layer definitions
│   ├── gfs.yaml          # GFS layers (parameter → style mappings)
│   ├── hrrr.yaml         # HRRR layers
│   └── ...
├── edr/                   # EDR collection definitions
│   ├── hrrr.yaml         # HRRR EDR collections
│   ├── gfs.yaml          # GFS EDR collections
│   └── locations.yaml    # Named locations (airports, cities)
└── styles/                # Rendering styles (JSON)
    ├── temperature.json  # Temperature colormaps
    ├── wind.json         # Wind speed/barbs
    ├── precipitation.json
    └── ...
```

### Environment Variables

Key environment variables (see `.env.example` for full list):

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgresql://weatherwms:weatherwms@postgres:5432/weatherwms` |
| `REDIS_URL` | Redis connection string | `redis://redis:6379` |
| `S3_ENDPOINT` | MinIO/S3 endpoint | `http://minio:9000` |
| `S3_BUCKET` | Data bucket name | `weather-data` |
| `RUST_LOG` | Log level | `info` |

### Style Configuration

Rendering styles are defined in JSON files. Example temperature gradient:

```json
{
  "version": "1.0",
  "styles": {
    "temperature_celsius": {
      "name": "Temperature (Celsius)",
      "type": "gradient",
      "transform": {"type": "kelvin_to_celsius"},
      "stops": [
        {"value": -40, "color": "#9400D3"},
        {"value": -20, "color": "#0000FF"},
        {"value": 0, "color": "#00FFFF"},
        {"value": 20, "color": "#FFFF00"},
        {"value": 40, "color": "#FF0000"}
      ]
    }
  }
}
```

## Project Structure

```
JoeGCServices/
├── services/                    # Deployable services
│   ├── wms-api/                # WMS/WMTS HTTP server
│   ├── edr-api/                # EDR HTTP server
│   ├── downloader/             # Scheduled data downloading
│   └── ingester/               # GRIB2/NetCDF → Zarr processing
├── crates/                      # Shared library crates
│   ├── wms-protocol/           # WMS/WMTS protocol handling
│   ├── edr-protocol/           # EDR types and CoverageJSON
│   ├── grib2-parser/           # GRIB2 format parser
│   ├── netcdf-parser/          # NetCDF parser (GOES)
│   ├── grid-processor/         # Zarr data access with caching
│   ├── renderer/               # Image rendering (gradients, contours, barbs)
│   ├── projection/             # CRS transformations
│   ├── storage/                # S3, PostgreSQL, Redis clients
│   └── ingestion/              # File processing logic
├── config/                      # Configuration files
├── deploy/                      # Deployment configurations
│   ├── production/             # Single-server production setup
│   └── grafana/                # Grafana dashboards
├── schemas/                     # JSON schemas for config validation
├── scripts/                     # Development and utility scripts
├── validation/                  # OGC compliance test suites
│   ├── ogc-compliance/         # Compliance test tools
│   └── load-test/              # Performance testing
├── web/                         # Web dashboard and static files
├── docs/                        # Documentation (mdBook source)
├── docker-compose.yml           # Local development setup
└── Cargo.toml                   # Rust workspace definition
```

## Deployment

### Docker Compose (Development/Small Production)

```bash
# Development
./scripts/start.sh

# With monitoring stack (Prometheus, Grafana, Loki)
docker-compose up -d
```

### Production (Single Server)

For single-server production with TLS via Cloudflare Tunnel:

```bash
cd deploy/production
cp .env.example .env
# Edit .env with your settings
./deploy.sh
```

Features:
- Nginx reverse proxy
- Cloudflare Tunnel for TLS (works behind CGNAT/Starlink)
- Auto-generated secure passwords
- Persistent storage volumes

## Monitoring

The stack includes full observability:

- **Prometheus** (`:9090`): Metrics collection
- **Grafana** (`:3000`): Dashboards and visualization
- **Loki**: Log aggregation
- **Promtail**: Log shipping

### Key Metrics

```
# WMS/WMTS
wms_requests_total{layer,style,operation}
wms_request_duration_seconds{layer}
tile_cache_hits_total / tile_cache_misses_total

# EDR
edr_requests_total{endpoint,collection}
edr_request_duration_seconds{endpoint}

# Grid Processor
grid_processor_chunk_cache_hits_total
grid_processor_chunk_cache_misses_total
```

## OGC Compliance Testing

The web dashboard includes built-in compliance testing:

- **WMS Compliance**: http://localhost:8000/wms-compliance.html
- **WMTS Compliance**: http://localhost:8000/wmts-compliance.html
- **EDR Compliance**: http://localhost:8000/edr-compliance.html
- **EDR Coverage**: http://localhost:8000/edr-coverage.html

For official OGC CITE testing, use the [OGC TeamEngine](https://cite.opengeospatial.org/teamengine/).

## Development

### Building Locally

```bash
# Build all crates
cargo build

# Build in release mode
cargo build --release

# Run tests
cargo test

# Run a specific service
cargo run --package wms-api
cargo run --package edr-api
```

### Running Tests

```bash
# Unit tests
cargo test

# With logging
RUST_LOG=debug cargo test -- --nocapture

# Integration tests (requires running services)
cargo test --package integration-tests
```

### Code Structure

The codebase follows a workspace structure with shared library crates:

- **Services** (`services/`): Standalone HTTP servers
- **Crates** (`crates/`): Reusable libraries
- **No circular dependencies**: Clear separation of concerns

## Screenshots

*Screenshots coming soon*

<!-- TODO: Add screenshots of:
- Web dashboard map viewer
- Grafana dashboards
- EDR compliance test results
- Sample rendered tiles
-->

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## Acknowledgments

- Weather data provided by [NOAA](https://www.noaa.gov/) via [AWS Open Data](https://registry.opendata.aws/)
- OGC standards: [WMS](https://www.ogc.org/standard/wms/), [WMTS](https://www.ogc.org/standard/wmts/), [EDR](https://www.ogc.org/standard/ogcapi-edr/)
- Built with [Rust](https://www.rust-lang.org/), [Axum](https://github.com/tokio-rs/axum), and [Tokio](https://tokio.rs/)
