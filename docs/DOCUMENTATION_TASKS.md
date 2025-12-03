# Documentation Tasks for Weather-WMS

This document contains detailed instructions for creating documentation. Each task is self-contained and can be completed independently.

---

## Setup Tasks

### Task 0.1: Initialize mdBook Structure

Create the mdBook configuration and directory structure.

**Create file: `docs/book.toml`**
```toml
[book]
title = "Weather WMS Documentation"
authors = ["Weather WMS Team"]
language = "en"
multilingual = false
src = "src"

[build]
build-dir = "book"

[output.html]
default-theme = "light"
preferred-dark-theme = "navy"
git-repository-url = "https://github.com/JoegottabeGitenme/JoeGCServices"
edit-url-template = "https://github.com/JoegottabeGitenme/JoeGCServices/edit/main/docs/{path}"

[output.html.search]
enable = true
limit-results = 30
use-hierarchical-headings = true
```

**Create directory structure:**
```
docs/src/
├── getting-started/
├── architecture/
├── services/
├── crates/
├── api-reference/
├── data-sources/
├── configuration/
├── deployment/
├── development/
└── reference/
```

---

### Task 0.2: Create SUMMARY.md (Navigation)

**Create file: `docs/src/SUMMARY.md`**

This file defines the sidebar navigation. Copy exactly:

```markdown
# Summary

[Introduction](./introduction.md)

# Getting Started

- [Overview](./getting-started/README.md)
  - [Prerequisites](./getting-started/prerequisites.md)
  - [Installation](./getting-started/installation.md)
  - [Quick Start](./getting-started/quickstart.md)
  - [Configuration](./getting-started/configuration.md)

# Architecture

- [Overview](./architecture/README.md)
  - [System Design](./architecture/system-design.md)
  - [Data Flow](./architecture/data-flow.md)
  - [Caching Strategy](./architecture/caching.md)

# Services

- [Overview](./services/README.md)
  - [WMS API](./services/wms-api.md)
  - [Ingester](./services/ingester.md)
  - [Downloader](./services/downloader.md)
  - [Renderer Worker](./services/renderer-worker.md)

# Crates (Libraries)

- [Overview](./crates/README.md)
  - [grib2-parser](./crates/grib2-parser.md)
  - [netcdf-parser](./crates/netcdf-parser.md)
  - [projection](./crates/projection.md)
  - [renderer](./crates/renderer.md)
  - [storage](./crates/storage.md)
  - [wms-common](./crates/wms-common.md)
  - [wms-protocol](./crates/wms-protocol.md)

# API Reference

- [Overview](./api-reference/README.md)
  - [WMS Endpoints](./api-reference/wms.md)
  - [WMTS Endpoints](./api-reference/wmts.md)
  - [REST API](./api-reference/rest-api.md)
  - [Examples](./api-reference/examples.md)

# Data Sources

- [Overview](./data-sources/README.md)
  - [GFS (Global Forecast System)](./data-sources/gfs.md)
  - [HRRR (High-Resolution Rapid Refresh)](./data-sources/hrrr.md)
  - [MRMS (Radar)](./data-sources/mrms.md)
  - [GOES (Satellite)](./data-sources/goes.md)

# Configuration

- [Overview](./configuration/README.md)
  - [Model Configuration](./configuration/models.md)
  - [Style Configuration](./configuration/styles.md)
  - [Parameter Tables](./configuration/parameters.md)
  - [Environment Variables](./configuration/environment.md)

# Deployment

- [Overview](./deployment/README.md)
  - [Docker Compose](./deployment/docker-compose.md)
  - [Kubernetes](./deployment/kubernetes.md)
  - [Helm Chart](./deployment/helm.md)
  - [Monitoring](./deployment/monitoring.md)

# Development

- [Overview](./development/README.md)
  - [Building](./development/building.md)
  - [Testing](./development/testing.md)
  - [Benchmarking](./development/benchmarking.md)
  - [Contributing](./development/contributing.md)

# Reference

- [Overview](./reference/README.md)
  - [Scripts](./reference/scripts.md)
  - [Troubleshooting](./reference/troubleshooting.md)
  - [Glossary](./reference/glossary.md)
```

---

## Phase 1: Introduction & Getting Started

### Task 1.1: Write Introduction

**Create file: `docs/src/introduction.md`**

**Content to include:**
- Project name: Weather WMS
- One paragraph description: A Kubernetes-native OGC WMS/WMTS server for weather data visualization
- Key features (bullet list):
  - OGC WMS 1.1.1/1.3.0 and WMTS 1.0.0 compliant
  - Real-time weather data from NOAA (GFS, HRRR, MRMS, GOES)
  - Written in Rust for high performance
  - Two-tier caching (L1 in-memory, L2 Redis)
  - Kubernetes-native with Helm chart
  - Multiple visualization styles (gradients, contours, wind barbs)
- Architecture diagram using Mermaid (see architecture section for diagram)
- Links to Getting Started and API Reference sections

---

### Task 1.2: Write Prerequisites

**Create file: `docs/src/getting-started/README.md`**

Brief intro to the getting started section.

**Create file: `docs/src/getting-started/prerequisites.md`**

**Content to include:**
- Required software:
  - Docker and Docker Compose (for local development)
  - Rust 1.75+ (for building from source)
  - kubectl and Helm 3 (for Kubernetes deployment)
- Optional software:
  - wgrib2 (for inspecting GRIB2 files)
  - ncdump (for inspecting NetCDF files)
- Hardware recommendations:
  - Minimum: 4GB RAM, 2 CPU cores
  - Recommended: 16GB RAM, 8 CPU cores
  - Storage: 50GB+ for weather data cache

---

### Task 1.3: Write Installation Guide

**Create file: `docs/src/getting-started/installation.md`**

**Content to include:**

1. Clone the repository:
```bash
git clone https://github.com/JoegottabeGitenme/JoeGCServices.git
cd JoeGCServices
```

2. Copy environment file:
```bash
cp .env.example .env
```

3. Start services with Docker Compose:
```bash
./scripts/start.sh
```

4. Verify services are running:
```bash
docker-compose ps
curl http://localhost:8080/health
```

5. Access the web dashboard: http://localhost:8000

---

### Task 1.4: Write Quick Start Guide

**Create file: `docs/src/getting-started/quickstart.md`**

**Content to include:**

Step-by-step guide to see weather data in 5 minutes:

1. Start services (reference installation.md)

2. Download sample data:
```bash
./scripts/download_goes.sh
```

3. Ingest the data:
```bash
./scripts/ingest_test_data.sh
```

4. View in browser:
   - Open http://localhost:8000
   - Select "GOES-18 IR" layer
   - Pan to view satellite imagery

5. Direct WMS request:
```bash
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=goes18_CMI_C13&STYLES=default&CRS=EPSG:3857&BBOX=-13149614,3503549,-10018754,6634109&WIDTH=256&HEIGHT=256&FORMAT=image/png" -o tile.png
```

---

### Task 1.5: Write Configuration Overview

**Create file: `docs/src/getting-started/configuration.md`**

**Content to include:**

Reference to detailed configuration section, plus quick overview of key environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgresql://postgres:postgres@postgres:5432/weatherwms` | PostgreSQL connection |
| `REDIS_URL` | `redis://redis:6379` | Redis connection |
| `S3_ENDPOINT` | `http://minio:9000` | Object storage endpoint |
| `S3_BUCKET` | `weather-data` | Bucket name |
| `L1_CACHE_SIZE` | `10000` | In-memory cache entries |
| `L2_CACHE_TTL` | `3600` | Redis cache TTL (seconds) |

---

## Phase 2: Architecture

### Task 2.1: Write Architecture Overview

**Create file: `docs/src/architecture/README.md`**

Brief intro linking to subsections.

---

### Task 2.2: Write System Design

**Create file: `docs/src/architecture/system-design.md`**

**Content to include:**

1. High-level architecture diagram (Mermaid):
```mermaid
graph TB
    subgraph "External"
        NOAA[NOAA Data Sources]
        Client[Map Clients]
    end
    
    subgraph "Ingestion Pipeline"
        DL[Downloader]
        ING[Ingester]
    end
    
    subgraph "Storage Layer"
        PG[(PostgreSQL<br/>Catalog)]
        MINIO[(MinIO/S3<br/>Grid Data)]
        REDIS[(Redis<br/>Tile Cache)]
    end
    
    subgraph "Serving Layer"
        API[WMS API]
        L1[L1 Memory Cache]
        WORKER[Renderer Workers]
    end
    
    NOAA --> DL
    DL --> ING
    ING --> PG
    ING --> MINIO
    
    Client --> API
    API --> L1
    API --> REDIS
    API --> MINIO
    API --> WORKER
    WORKER --> REDIS
```

2. Component descriptions:
   - **Downloader**: Fetches GRIB2/NetCDF from NOAA with resume support
   - **Ingester**: Parses files, extracts parameters, stores in MinIO, registers in PostgreSQL
   - **WMS API**: Serves OGC WMS/WMTS requests, manages caching
   - **Renderer Workers**: Background tile rendering for cache warming

3. Technology choices:
   - Rust for performance-critical services
   - PostgreSQL for metadata catalog
   - MinIO for S3-compatible object storage
   - Redis for distributed caching and job queues

---

### Task 2.3: Write Data Flow

**Create file: `docs/src/architecture/data-flow.md`**

**Content to include:**

1. **Ingestion Flow** (diagram + description):
```mermaid
sequenceDiagram
    participant NOAA
    participant Downloader
    participant Ingester
    participant MinIO
    participant PostgreSQL
    
    Downloader->>NOAA: HTTP GET (with Range support)
    NOAA-->>Downloader: GRIB2/NetCDF data
    Downloader->>Ingester: Trigger ingestion
    Ingester->>Ingester: Parse GRIB2/NetCDF
    Ingester->>MinIO: Store grid data
    Ingester->>PostgreSQL: Register in catalog
```

2. **Request Flow** (diagram + description):
```mermaid
sequenceDiagram
    participant Client
    participant API as WMS API
    participant L1 as L1 Cache
    participant Redis as L2 Cache
    participant MinIO
    
    Client->>API: GetMap request
    API->>L1: Check L1 cache
    alt L1 hit
        L1-->>API: Return tile
    else L1 miss
        API->>Redis: Check L2 cache
        alt L2 hit
            Redis-->>API: Return tile
            API->>L1: Store in L1
        else L2 miss
            API->>MinIO: Fetch grid data
            API->>API: Render tile
            API->>Redis: Store in L2
            API->>L1: Store in L1
        end
    end
    API-->>Client: PNG tile
```

---

### Task 2.4: Write Caching Strategy

**Create file: `docs/src/architecture/caching.md`**

**Content to include:**

1. **Two-Tier Cache Architecture**:
   - L1 (In-Memory): Sub-millisecond access, per-instance, LRU eviction
   - L2 (Redis): Millisecond access, shared across instances, TTL-based

2. **Cache Key Format**:
```
wms:{layer}:{style}:{crs}:{bbox}:{dimensions}:{time}:{format}
```

3. **Configuration options** (table of env vars)

4. **Cache warming**: Prefetch neighboring tiles on request

5. **Cache invalidation**: On new data ingestion

---

## Phase 3: Services

### Task 3.1: Write Services Overview

**Create file: `docs/src/services/README.md`**

Table of services with brief descriptions and ports.

---

### Task 3.2: Document WMS API Service

**Create file: `docs/src/services/wms-api.md`**

**Source to reference:** `services/wms-api/src/main.rs`, `services/wms-api/src/handlers.rs`

**Content to include:**
- Purpose: HTTP server implementing OGC WMS/WMTS
- Port: 8080
- Key endpoints table (see API reference for details)
- Configuration options (env vars)
- Health/readiness endpoints
- Metrics endpoint

---

### Task 3.3: Document Ingester Service

**Create file: `docs/src/services/ingester.md`**

**Source to reference:** `services/ingester/src/main.rs`

**Content to include:**
- Purpose: Parses weather data files and stores in catalog
- Supported formats: GRIB2, NetCDF
- Shredded storage explanation
- Configuration via YAML
- Ingestion commands/triggers

---

### Task 3.4: Document Downloader Service

**Create file: `docs/src/services/downloader.md`**

**Source to reference:** `services/downloader/src/main.rs`

**Content to include:**
- Purpose: Downloads weather data from NOAA
- Port: 8081
- Features: Resume support, retry, progress tracking
- API endpoints for status
- Configuration options

---

### Task 3.5: Document Renderer Worker

**Create file: `docs/src/services/renderer-worker.md`**

**Source to reference:** `services/renderer-worker/src/main.rs`

**Content to include:**
- Purpose: Background tile rendering
- Job queue (Redis)
- Scaling considerations
- Configuration options

---

## Phase 4: Crates

### Task 4.1: Write Crates Overview

**Create file: `docs/src/crates/README.md`**

Dependency diagram showing how crates relate.

---

### Task 4.2: Document grib2-parser

**Create file: `docs/src/crates/grib2-parser.md`**

**Source to reference:** `crates/grib2-parser/src/lib.rs`

**Content:**
- Purpose: Pure Rust GRIB2 parser
- GRIB2 format explanation (8 sections)
- Supported compression types
- Usage example
- Link to WMO GRIB2 spec

---

### Task 4.3: Document netcdf-parser

**Create file: `docs/src/crates/netcdf-parser.md`**

**Source to reference:** `crates/netcdf-parser/src/lib.rs`

**Content:**
- Purpose: NetCDF-4 parser for GOES data
- GOES-R ABI data format
- Geostationary projection handling
- Usage example

---

### Task 4.4: Document projection

**Create file: `docs/src/crates/projection.md`**

**Source to reference:** `crates/projection/src/`

**Content:**
- Purpose: Coordinate transformations
- Supported projections: Geographic, Mercator, Lambert, Polar, Geostationary
- EPSG codes supported
- Usage examples

---

### Task 4.5: Document renderer

**Create file: `docs/src/crates/renderer.md`**

**Source to reference:** `crates/renderer/src/`

**Content:**
- Purpose: Weather data visualization
- Rendering types:
  - Gradient/color ramp
  - Contour lines (marching squares)
  - Wind barbs (SVG-based)
  - Numeric values
- Style configuration format
- Usage examples

---

### Task 4.6: Document storage

**Create file: `docs/src/crates/storage.md`**

**Source to reference:** `crates/storage/src/`

**Content:**
- Purpose: Storage abstractions
- Components:
  - ObjectStorage (MinIO/S3)
  - Catalog (PostgreSQL)
  - TileCache (Redis)
  - TileMemoryCache (in-memory)
  - GribCache (in-memory)
- Configuration options

---

### Task 4.7: Document wms-common

**Create file: `docs/src/crates/wms-common.md`**

**Source to reference:** `crates/wms-common/src/`

**Content:**
- Purpose: Shared types and utilities
- Key types: BoundingBox, CRS, Layer, Style, TileCoord
- Error handling

---

### Task 4.8: Document wms-protocol

**Create file: `docs/src/crates/wms-protocol.md`**

**Source to reference:** `crates/wms-protocol/src/`

**Content:**
- Purpose: OGC protocol implementation
- WMS versions: 1.1.1, 1.3.0
- WMTS version: 1.0.0
- Operations: GetCapabilities, GetMap, GetTile, GetFeatureInfo
- XML generation

---

## Phase 5: API Reference

### Task 5.1: Write API Overview

**Create file: `docs/src/api-reference/README.md`**

Overview of all API endpoints with quick reference table.

---

### Task 5.2: Document WMS Endpoints

**Create file: `docs/src/api-reference/wms.md`**

**Content:**

1. **GetCapabilities**
```
GET /wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0
```
Response: XML document describing layers, styles, CRS

2. **GetMap**
```
GET /wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap
    &LAYERS=gfs_TMP_2m
    &STYLES=default
    &CRS=EPSG:3857
    &BBOX=-20037508,-20037508,20037508,20037508
    &WIDTH=256
    &HEIGHT=256
    &FORMAT=image/png
```
Parameters table with descriptions

3. **GetFeatureInfo**
```
GET /wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo
    &LAYERS=gfs_TMP_2m
    &QUERY_LAYERS=gfs_TMP_2m
    &CRS=EPSG:4326
    &BBOX=-180,-90,180,90
    &WIDTH=256
    &HEIGHT=256
    &I=128
    &J=128
    &INFO_FORMAT=application/json
```

---

### Task 5.3: Document WMTS Endpoints

**Create file: `docs/src/api-reference/wmts.md`**

**Content:**

1. **KVP Binding**
```
GET /wmts?SERVICE=WMTS&REQUEST=GetCapabilities
GET /wmts?SERVICE=WMTS&REQUEST=GetTile&LAYER=...&TILEMATRIXSET=...&TILEMATRIX=...&TILEROW=...&TILECOL=...
```

2. **RESTful Binding**
```
GET /wmts/rest/{layer}/{style}/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}.png
```

3. **Simple XYZ Endpoint** (non-standard but convenient)
```
GET /tiles/{layer}/{style}/{z}/{x}/{y}.png
```

---

### Task 5.4: Document REST API

**Create file: `docs/src/api-reference/rest-api.md`**

**Content:**

Table of all `/api/*` endpoints:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check |
| `/ready` | GET | Readiness check |
| `/metrics` | GET | Prometheus metrics |
| `/api/forecast-times/:model/:parameter` | GET | Available forecast hours |
| `/api/parameters/:model` | GET | Available parameters |
| `/api/cache/list` | GET | List cached tiles |
| `/api/cache/clear` | POST | Clear in-memory caches |
| `/api/storage/stats` | GET | MinIO statistics |
| `/api/config` | GET | Current configuration |

---

### Task 5.5: Write API Examples

**Create file: `docs/src/api-reference/examples.md`**

**Content:**

Practical examples for common use cases:

1. **Leaflet integration** (JavaScript code)
2. **OpenLayers integration** (JavaScript code)
3. **Python requests** (Python code)
4. **curl examples** for each endpoint

---

## Phase 6: Data Sources

### Task 6.1: Write Data Sources Overview

**Create file: `docs/src/data-sources/README.md`**

Comparison table of all data sources.

---

### Task 6.2-6.5: Document Each Data Source

**Create files:**
- `docs/src/data-sources/gfs.md`
- `docs/src/data-sources/hrrr.md`
- `docs/src/data-sources/mrms.md`
- `docs/src/data-sources/goes.md`

**For each, include:**
- Official name and source
- Resolution and coverage
- Update frequency
- Available parameters
- Layer naming convention
- Example requests

---

## Phase 7: Configuration

### Task 7.1-7.4: Document Configuration

**Create files:**
- `docs/src/configuration/README.md`
- `docs/src/configuration/models.md` (reference `config/models/*.yaml`)
- `docs/src/configuration/styles.md` (reference `config/styles/*.json`)
- `docs/src/configuration/parameters.md` (reference `config/parameters/*.yaml`)
- `docs/src/configuration/environment.md` (all env vars)

---

## Phase 8: Deployment

### Task 8.1-8.4: Document Deployment

**Create files:**
- `docs/src/deployment/README.md`
- `docs/src/deployment/docker-compose.md`
- `docs/src/deployment/kubernetes.md`
- `docs/src/deployment/helm.md`
- `docs/src/deployment/monitoring.md`

---

## Phase 9: Development

### Task 9.1-9.4: Document Development

**Create files:**
- `docs/src/development/README.md`
- `docs/src/development/building.md`
- `docs/src/development/testing.md`
- `docs/src/development/benchmarking.md`
- `docs/src/development/contributing.md`

---

## Phase 10: Reference

### Task 10.1: Document Scripts

**Create file: `docs/src/reference/scripts.md`**

Table of all scripts in `scripts/` with descriptions.

---

### Task 10.2: Write Troubleshooting Guide

**Create file: `docs/src/reference/troubleshooting.md`**

Common issues and solutions:
- Service won't start
- No data showing
- Slow performance
- Cache issues

---

### Task 10.3: Write Glossary

**Create file: `docs/src/reference/glossary.md`**

Definitions for:
- GRIB2
- NetCDF
- WMS
- WMTS
- OGC
- CRS
- EPSG
- GFS, HRRR, MRMS, GOES
- etc.

---

## GitHub Actions Workflow

### Task 11.1: Create CI/CD for Docs

**Create file: `.github/workflows/docs.yml`**

```yaml
name: Deploy Documentation

on:
  push:
    branches: [main]
    paths: ['docs/**']
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: "pages"
  cancel-in-progress: false

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install mdBook
        run: |
          mkdir -p $HOME/.local/bin
          curl -sSL https://github.com/rust-lang/mdBook/releases/download/v0.4.36/mdbook-v0.4.36-x86_64-unknown-linux-gnu.tar.gz | tar -xz -C $HOME/.local/bin
          echo "$HOME/.local/bin" >> $GITHUB_PATH
      
      - name: Build documentation
        run: cd docs && mdbook build
      
      - name: Upload artifact
        uses: actions/upload-pages-artifact@v3
        with:
          path: ./docs/book

  deploy:
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    runs-on: ubuntu-latest
    needs: build
    steps:
      - name: Deploy to GitHub Pages
        id: deployment
        uses: actions/deploy-pages@v4
```

---

## Completion Checklist

- [ ] Task 0.1: Initialize mdBook structure
- [ ] Task 0.2: Create SUMMARY.md
- [ ] Task 1.1-1.5: Getting Started section
- [ ] Task 2.1-2.4: Architecture section
- [ ] Task 3.1-3.5: Services section
- [ ] Task 4.1-4.8: Crates section
- [ ] Task 5.1-5.5: API Reference section
- [ ] Task 6.1-6.5: Data Sources section
- [ ] Task 7.1-7.4: Configuration section
- [ ] Task 8.1-8.4: Deployment section
- [ ] Task 9.1-9.4: Development section
- [ ] Task 10.1-10.3: Reference section
- [ ] Task 11.1: GitHub Actions workflow
- [ ] Final review and link checking
