# Web Dashboard

The Weather WMS web dashboard provides a real-time interactive interface for viewing weather data, monitoring system performance, and visualizing tile request patterns.

## Overview

**Location**: `web/`  
**URL**: http://localhost:8000  
**Technology**: HTML/CSS/JavaScript with Leaflet.js

## Features

### Interactive Map Viewer

The main map viewer uses Leaflet.js to display weather data layers:

- **Base Maps**: OpenStreetMap, dark theme options
- **Weather Layers**: All WMS/WMTS layers from the catalog
- **Protocol Toggle**: Switch between WMS and WMTS
- **Style Selection**: Choose from available styles per layer
- **Elevation Slider**: Select vertical levels (e.g., 500mb, 850mb)
- **Time Slider**: Animate through forecast hours or observation times
- **Click-to-Query**: GetFeatureInfo support for data values

### Info Bars (5 Rows)

Real-time system statistics displayed in the header:

#### 1. System Bar
- CPU cores and load averages (1m, 5m)
- RAM usage (used, total, percentage)
- Storage (total size, file count)
- Service uptime

#### 2. Tile Cache Bar
- **L1 (In-Memory)**: Hits, hit rate, tile count, size
- **L2 (Redis)**: Hits, hit rate, tile count, size

#### 3. Grid Cache Bar
- Total parses, hits, hit rate
- Per-source stats (GFS, GOES, HRRR, MRMS): cache rate %, parse time

#### 4. Requests Bar
- **WMS**: Total, 1-minute, 5-minute counts
- **WMTS**: Total, 1-minute, 5-minute counts
- **Render**: Total, 1m, 5m, average time, min/max

#### 5. Data Bar
- Total files, size, datasets, parameters
- Per-model status (GFS, HRRR, GOES, MRMS): active/inactive, file count, param count

### Tile Request Heatmap (Minimap)

A minimap panel on the right side displays geographic distribution of tile requests:

- **Global View**: Fixed world view showing request patterns
- **Viewport Indicator**: Orange rectangle showing main map bounds
- **Heatmap Visualization**: Color-coded squares indicating request density
  - Blue: Low (0-25% of max)
  - Green: Medium (25-50%)
  - Orange: High (50-75%)
  - Red: Hot (75-100%)
- **Statistics**: Total requests and hotspot count
- **Clear Button**: Reset heatmap data

The heatmap data comes from the server API (`/api/tile-heatmap`), showing ALL requests to the WMS API - not just from the web viewer. This is useful for monitoring load tests.

### OGC Compliance Badges

Header badges show OGC compliance status:
- **WMS 1.3.0**: Click to run validation
- **WMTS 1.0.0**: Click to run validation

### External Links

Quick access to related tools:
- Admin Dashboard
- Downloads Dashboard
- Load Tests
- Microbenchmarks
- Tile Visualizer
- Grafana
- Prometheus
- MinIO Console
- pgAdmin
- Kubernetes Dashboard

## API Endpoints Used

The dashboard polls these endpoints for real-time updates:

| Endpoint | Interval | Data |
|----------|----------|------|
| `/api/metrics` | 2s | Cache stats, request counts, render times |
| `/api/container/stats` | 5s | CPU, memory, load averages |
| `/api/admin/ingestion/status` | 10s | Model status, dataset counts |
| `/api/storage/stats` | 30s | MinIO file counts, sizes |
| `/api/tile-heatmap` | 2s | Tile request geographic distribution |
| `/api/validation/status` | 5min | OGC compliance status |

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
2. Verify data is ingested: Check "Data" info bar
3. Open browser console for errors

### Slow Performance

1. Check cache hit rates in info bars
2. Reduce animation speed
3. Zoom in to reduce tile count

### Heatmap Not Updating

1. Verify API is accessible: `curl http://localhost:8080/api/tile-heatmap`
2. Check browser console for fetch errors
3. Clear and retry

## Files

```
web/
├── index.html      # Main dashboard page
├── app.js          # Application logic
├── style.css       # Styling
├── admin.html      # Admin dashboard
├── admin.js        # Admin logic
├── downloads.html  # Downloads dashboard
├── downloads.js    # Downloads logic
├── benchmarks.html # Benchmark viewer
└── server.py       # Simple HTTP server
```

## Next Steps

- [REST API](../api-reference/rest-api.md) - API endpoints
- [Monitoring](../deployment/monitoring.md) - Grafana dashboards
- [WMS Endpoints](../api-reference/wms.md) - WMS parameters
