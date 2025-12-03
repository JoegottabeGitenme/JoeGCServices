# Quick Start

Get Weather WMS up and running with real weather data in 5 minutes. This guide assumes you've completed the [Installation](./installation.md).

## Step 1: Start the Services

If not already running:

```bash
cd JoeGCServices
./scripts/start.sh
```

Wait for all services to be healthy (check with `docker-compose ps`).

## Step 2: Download Sample Data

Let's start with GOES-18 satellite data for immediate visual results:

```bash
./scripts/download_goes.sh
```

This script:
- Downloads recent GOES-18 infrared (C13) and visible (C02) imagery
- Automatically triggers ingestion via the WMS API
- Takes 2-3 minutes depending on your connection (~500 MB)

**What's being downloaded?**
- GOES-18 is a geostationary weather satellite
- Channel 13 (IR) shows cloud-top temperatures
- Channel 2 (Visible) shows reflected sunlight
- Data covers the Western Hemisphere

## Step 3: Monitor Ingestion

Watch the ingestion progress:

```bash
docker-compose logs -f ingester
```

Look for messages like:
```
[INFO] Ingesting: /data/goes18/OR_ABI-L2-CMIPF-M6C13_G18_...
[INFO] Stored 226 shreds in MinIO
[INFO] Registered in catalog: goes18_CMI_C13
[INFO] Ingestion complete
```

Press `Ctrl+C` to stop following logs.

## Step 4: View in the Web Dashboard

Open your browser to:

```
http://localhost:8000
```

### Using the Map Interface

1. **Select a Layer**: Click the layers control (top-right corner)
2. **Choose "GOES-18 IR (C13)"**
3. **Pan and zoom** to explore satellite imagery

The map shows:
- Cloud patterns in the infrared spectrum
- Colder areas (higher clouds) appear brighter
- Warmer areas (surface/low clouds) appear darker

## Step 5: Test WMS Directly

### Get Available Layers

```bash
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | grep -A 5 "goes18_CMI_C13"
```

### Request a Specific Tile

Get a 256x256 tile over the continental US:

```bash
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=goes18_CMI_C13&STYLES=default&CRS=EPSG:3857&BBOX=-13149614,3503549,-10018754,6634109&WIDTH=256&HEIGHT=256&FORMAT=image/png" -o goes_tile.png
```

Open `goes_tile.png` to view the satellite imagery.

### Get Point Data

Query the temperature value at a specific location:

```bash
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&QUERY_LAYERS=goes18_CMI_C13&LAYERS=goes18_CMI_C13&CRS=EPSG:4326&BBOX=-125,25,-65,50&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json"
```

Returns JSON with the brightness temperature at the queried point.

## Step 6: Download More Data (Optional)

### GFS Global Forecast

Download global weather forecast data:

```bash
./scripts/download_gfs.sh
```

This provides:
- Temperature at 2m (TMP_2m)
- Wind U/V components (UGRD, VGRD)
- Relative humidity (RH)
- Mean sea level pressure (PRMSL)
- And more...

Takes ~5-10 minutes (~2 GB).

### HRRR High-Resolution Forecast

Download high-resolution CONUS forecast:

```bash
./scripts/download_hrrr.sh
```

Provides same parameters as GFS but with 3km resolution over North America.

Takes ~3-5 minutes (~1 GB).

### MRMS Radar Data

Download real-time radar composites:

```bash
./scripts/download_mrms.sh
```

Provides:
- Reflectivity (REFL)
- Precipitation rate (PRECIP_RATE)

Takes ~2 minutes (~500 MB).

## Next Steps

### Explore Layers

After downloading multiple data sources, explore different layers:

```bash
# List all available layers
curl "http://localhost:8080/api/parameters/gfs"
curl "http://localhost:8080/api/parameters/hrrr"
curl "http://localhost:8080/api/parameters/goes18"
```

### Try Different Styles

Weather WMS supports multiple visualization styles:

```bash
# Temperature with custom colormap
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=gfs_TMP_2m&STYLES=temperature&CRS=EPSG:3857&BBOX=-20037508,-20037508,20037508,20037508&WIDTH=512&HEIGHT=512&FORMAT=image/png" -o temperature.png

# Wind with wind barbs
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=gfs_UGRD_10m&STYLES=wind&CRS=EPSG:3857&BBOX=-20037508,-20037508,20037508,20037508&WIDTH=512&HEIGHT=512&FORMAT=image/png" -o wind.png
```

See [Style Configuration](../configuration/styles.md) for available styles.

### Use with Leaflet

Integrate Weather WMS into a web application:

```html
<!DOCTYPE html>
<html>
<head>
  <link rel="stylesheet" href="https://unpkg.com/leaflet@1.9.4/dist/leaflet.css" />
  <script src="https://unpkg.com/leaflet@1.9.4/dist/leaflet.js"></script>
  <style>
    #map { height: 600px; }
  </style>
</head>
<body>
  <div id="map"></div>
  <script>
    // Create map
    const map = L.map('map').setView([40, -100], 4);
    
    // Add base layer
    L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png').addTo(map);
    
    // Add weather layer
    L.tileLayer.wms('http://localhost:8080/wms', {
      layers: 'gfs_TMP_2m',
      styles: 'temperature',
      format: 'image/png',
      transparent: true,
      opacity: 0.7
    }).addTo(map);
  </script>
</body>
</html>
```

## Common Commands

```bash
# Check service status
docker-compose ps

# View logs
docker-compose logs -f wms-api
docker-compose logs -f ingester

# Restart a service
docker-compose restart wms-api

# Clear caches
curl -X POST http://localhost:8080/api/cache/clear

# Check database content
docker-compose exec postgres psql -U weatherwms -d weatherwms -c "SELECT model, parameter, COUNT(*) FROM grid_catalog GROUP BY model, parameter;"

# Check MinIO storage
curl http://localhost:8080/api/storage/stats
```

## Learn More

- [API Reference](../api-reference/README.md) - Complete WMS/WMTS endpoint documentation
- [Configuration](../configuration/README.md) - Customize layers, styles, and behavior
- [Data Sources](../data-sources/README.md) - Learn about GFS, HRRR, MRMS, GOES
- [Architecture](../architecture/README.md) - Understand how Weather WMS works
