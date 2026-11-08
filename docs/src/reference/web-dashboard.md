# Web Dashboard

The Weather WMS web dashboard provides a real-time interactive interface for viewing weather data, checking service health, and reaching the other tools. Time-series metrics (request rates, cache hit rates, render latency, resource usage, ingestion and download throughput) live in Grafana; the header links to the FolkWeather dashboards.

## Overview

**Location**: `web/`  
**URL**: http://localhost:8000  
**Technology**: HTML/CSS/JavaScript with Leaflet.js

## Features

### Interactive Map Viewer

The main map viewer uses Leaflet.js to display weather data layers:

- **Base Maps**: Light and Dark vector maps from [OpenFreeMap](https://openfreemap.org) (free, no API key; drawn with MapLibre GL through the `maplibre-gl-leaflet` plugin, so the browser needs WebGL), plus raster OpenStreetMap. The EPSG:4326 view draws a graticule instead
- **Weather Layers**: All WMS/WMTS layers from the catalog
- **Protocol Toggle**: Switch between WMS and WMTS
- **Style Selection**: Choose from available styles per layer
- **Elevation Slider**: Select vertical levels (e.g., 500mb, 850mb)
- **Time Slider**: Animate through forecast hours or observation times
- **Click-to-Query**: GetFeatureInfo support for data values

### Data Panel (Left Sidebar)

A collapsible left sidebar showing data storage details:

#### PostgreSQL Section
- **Dataset Count**: Total ingested datasets (unique model/run/parameter/forecast hour combinations)
- **Expandable Tree**: Browse by model > run > parameters

#### MinIO Storage Section
- **Object Count** and **Size**: Total files and bytes stored in MinIO buckets
- **Expandable Tree**: Browse by bucket > prefix > files
- Loaded when the section is first opened, then refreshed every minute while it stays open. Listing the whole bucket can take over a minute.

#### Sync Status
- Shows whether database and storage are in sync (checked on load, then every 5 minutes, because it also lists the whole bucket)
- Preview and trigger sync operations

The panel is scrollable when expanded trees exceed the viewport height. Hover over count/size badges for detailed explanations.

### Footer

- **Dual Clock**: Real-time display of local time and UTC time (updates every second)
- **Service Attribution**: Weather WMS branding

### OGC Compliance Test Pages

Dedicated pages for comprehensive OGC protocol compliance testing:

#### WMS Compliance (`wms-compliance.html`)

Full WMS 1.3.0 compliance test suite:
- **GetCapabilities tests**: Version negotiation, XML structure, metadata validation
- **GetMap tests**: BBOX handling, CRS axis order, FORMAT/STYLES validation, exception handling
- **GetFeatureInfo tests**: Coordinate handling, response format validation
- **Per-layer testing**: Validates every layer in the capabilities document
- **External server testing**: Can test any WMS server by entering its URL
- **Visual results**: Shows pass/fail with detailed error messages and spec references

#### WMTS Compliance (`wmts-compliance.html`)

Full WMTS 1.0.0 compliance test suite:
- **GetCapabilities tests**: Contents structure, TileMatrixSet definitions
- **GetTile tests**: Both KVP and RESTful encodings
- **TileMatrix validation**: Scale denominators, tile dimensions
- **Per-layer testing**: Validates every layer and tile matrix set
- **Visual tile preview**: Shows actual rendered tiles

#### EDR Compliance (`edr-compliance.html`)

Full OGC API - EDR compliance test suite with 157+ tests:

- **Core conformance**: Landing page, links, conformance declaration
- **Collections**: Structure, metadata, extent validation
- **Query types**: Position, area, radius, trajectory, corridor, cube, locations
- **Parameter handling**: z levels, datetime, CRS, format negotiation
- **Content negotiation**: Accept headers, f parameter, CoverageJSON/GeoJSON validation
- **Error handling**: 400/404 responses, OGC exception format
- **Interactive testing**: Map-based coordinate selection, query builder
- **External server support**: Test any EDR server by entering its URL

Test categories include:
| Category | Tests | Description |
|----------|-------|-------------|
| Core | 3 | Landing page structure and links |
| Collections | 3 | Collection list and metadata |
| Position | 10 | Point queries with various formats |
| Area | 13 | Polygon queries |
| Radius | 17 | Circular area queries |
| Trajectory | 17 | Path queries (LINESTRING variants) |
| Corridor | 30+ | Buffered path queries |
| Cube | 16 | 3D bounding box queries |
| Locations | 11 | Named location queries |
| Content-Type | 7 | Format negotiation |
| Error Handling | 4 | Exception responses |

#### EDR Coverage Validation (`edr-coverage.html`)

Validates that advertised EDR data actually exists and can be retrieved:

- **Purpose**: Ensures data integrity between configuration and database
- **Test modes**:
  - **Quick**: One sample per parameter
  - **Full**: Multiple times per parameter
  - **Thorough**: All times and levels
- **Query type selection**: Position, area, radius, trajectory, corridor, cube, locations
- **Output format testing**: CoverageJSON, GeoJSON, or both formats
- **Concurrent execution**: Up to 20 parallel requests
- **Catalog comparison**: Compares advertised vs actual database contents
- **Results export**: JSON export for debugging

Result categories:
- **Pass** (green): Data retrieved successfully
- **Fail** (red): Request error or invalid response
- **Warn** (yellow): Success but empty/null data

### Service Health Badges (Main Dashboard)

The header shows an Online/Offline badge for each service, checked every 30 seconds:
- **WMS**, **WMTS**: GetCapabilities request
- **EDR**: landing page request
- **Downloader**: `/downloader/health` (click to open the Downloads dashboard)
- **Ingester**: reachability as seen by wms-api, through `/api/admin/ingestion/active` (click to open the Admin dashboard)

### OGC Compliance Badges (Main Dashboard)

Header badges show quick OGC compliance status:
- **WMS 1.3.0**: Click to open WMS compliance test page
- **WMTS 1.0.0**: Click to open WMTS compliance test page
- **EDR 1.1**: Click to open EDR compliance test page
- **EDR Coverage**: Click to open EDR coverage validation page

### External Links

Quick access to related tools:
- Admin Dashboard
- Downloads Dashboard
- Load Tests
- Microbenchmarks
- Tile Visualizer
- EDR Compliance
- EDR Coverage
- Grafana (FolkWeather dashboard list, `tag=folkweather`)
- System (Grafana: nodes, pods, CPU, memory, disk)
- Data (Grafana: downloads, ingestion, storage; hidden in local dev, where the docker-compose Grafana has no such dashboard)
- Prometheus (local dev only)
- MinIO Console (local dev only)

## API Endpoints Used

The dashboard polls these endpoints:

| Endpoint | Interval | Data |
|----------|----------|------|
| `/wms?REQUEST=GetCapabilities`, `/wmts?REQUEST=GetCapabilities`, `/edr`, `/downloader/health`, `/api/admin/ingestion/active` | 30s | Service status badges |
| `/api/validation/status` | 5min | OGC compliance status |
| `/api/admin/database/details` | 15s | PostgreSQL data tree |
| `/api/admin/storage/tree` | 1min, only while the MinIO section is open | MinIO storage tree |
| `/api/admin/sync/status` | 5min | Database/storage sync status |

Polling pauses while the browser tab is hidden, and a poll is skipped while the previous request of the same kind is still pending.

Operational metrics (cache hit rates, request counts, render times, CPU/memory, download and ingestion progress) are not shown on this page; use the Grafana dashboards linked in the header.

## Configuration

### Environment Variables

The dashboard automatically adapts to the environment:

- **Docker Compose (port 8000)**: Uses `localhost:8080` for API calls
- **Kubernetes (other ports)**: Uses relative URLs

### Customization

Edit `web/style.css` for styling changes:
- Color scheme
- Font sizes
- Layout adjustments

## Time Controls

### Forecast Layers (GFS, HRRR)

- **Time Slider**: Select forecast hour (e.g., +0h to +384h)
- **Animation**: Play through forecast hours
- **Speed Control**: 0.5x, 1x, 2x, 3x playback speed
- **Run Selection**: Choose model run time

### Observation Layers (GOES, MRMS)

- **Time Slider**: Select observation timestamp
- **Animation**: Loop through recent observations
- **Auto-update**: Latest data displayed by default

## Animation Controls

- **Play/Pause**: Toggle animation
- **Speed**: Adjust playback speed
- **Zoom Lock**: Prevents pan/zoom during animation
- **Preloading**: Tiles preloaded for smooth animation

## Troubleshooting

### No Layers Showing

1. Check WMS service is running: `curl http://localhost:8080/health`
2. Verify data is ingested: expand the PostgreSQL section in the data panel, or open the Data Pipeline dashboard in Grafana
3. Open browser console for errors

### Slow Performance

1. Check cache hit rates and render latency in the WMS/WMTS Grafana dashboard
2. Reduce animation speed
3. Zoom in to reduce tile count

### Data Panel Not Loading

1. Check admin API is accessible: `curl http://localhost:8080/api/admin/database/details`
2. Verify PostgreSQL connection in WMS API logs
3. Check MinIO is running: `curl http://localhost:9000/minio/health/live`

## Files

```
web/
├── index.html           # Main dashboard page
├── app.js               # Application logic
├── style.css            # Styling
├── admin.html           # Admin dashboard
├── admin.js             # Admin logic
├── downloads.html       # Downloads dashboard
├── downloads.js         # Downloads logic
├── benchmarks.html      # Benchmark viewer
├── tile-visualizer.html # Tile request visualizer
├── wms-compliance.html  # WMS 1.3.0 compliance tests
├── wms-compliance.js    # WMS test implementation
├── wms-compliance.css   # WMS test styling
├── wmts-compliance.html # WMTS 1.0.0 compliance tests
├── wmts-compliance.js   # WMTS test implementation
├── wmts-compliance.css  # WMTS test styling
├── edr-compliance.html  # EDR 1.1 compliance tests
├── edr-compliance.js    # EDR test implementation
├── edr-compliance.css   # EDR test styling
├── edr-coverage.html    # EDR coverage validation
├── edr-coverage.js      # Coverage validation logic
├── edr-coverage.css     # Coverage validation styling
└── server.py            # Simple HTTP server
```

## Next Steps

- [REST API](../api-reference/rest-api.md) - API endpoints
- [Monitoring](../deployment/monitoring.md) - Grafana dashboards
- [WMS Endpoints](../api-reference/wms.md) - WMS parameters
