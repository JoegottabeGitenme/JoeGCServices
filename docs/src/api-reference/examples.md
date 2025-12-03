# API Examples

Practical examples for integrating Weather WMS into your applications.

## Leaflet (JavaScript)

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
        L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
            attribution: '© OpenStreetMap'
        }).addTo(map);
        
        // Add weather layer (WMS)
        L.tileLayer.wms('http://localhost:8080/wms', {
            layers: 'gfs_TMP_2m',
            styles: 'temperature',
            format: 'image/png',
            transparent: true,
            opacity: 0.7,
            attribution: 'Weather data: NOAA GFS'
        }).addTo(map);
    </script>
</body>
</html>
```

## OpenLayers (JavaScript)

```javascript
import Map from 'ol/Map';
import View from 'ol/View';
import TileLayer from 'ol/layer/Tile';
import {OSM, TileWMS} from 'ol/source';

const map = new Map({
    target: 'map',
    layers: [
        // Base layer
        new TileLayer({
            source: new OSM()
        }),
        // Weather layer
        new TileLayer({
            source: new TileWMS({
                url: 'http://localhost:8080/wms',
                params: {
                    'LAYERS': 'gfs_TMP_2m',
                    'STYLES': 'temperature',
                    'FORMAT': 'image/png',
                },
                serverType: 'geoserver',
            }),
            opacity: 0.7,
        }),
    ],
    view: new View({
        center: [-11000000, 4500000],
        zoom: 4,
    }),
});
```

## Python (requests)

```python
import requests
from datetime import datetime, timedelta

# Get available forecast times
response = requests.get('http://localhost:8080/api/forecast-times/gfs/TMP_2m')
times = response.json()['times']
latest_time = times[0]['forecast_time']

print(f"Latest forecast: {latest_time}")

# Download map tile
params = {
    'SERVICE': 'WMS',
    'VERSION': '1.3.0',
    'REQUEST': 'GetMap',
    'LAYERS': 'gfs_TMP_2m',
    'STYLES': 'temperature',
    'CRS': 'EPSG:3857',
    'BBOX': '-13149614,3503549,-10018754,6634109',
    'WIDTH': 512,
    'HEIGHT': 512,
    'FORMAT': 'image/png',
    'TIME': latest_time,
}

response = requests.get('http://localhost:8080/wms', params=params)

if response.ok:
    with open('temperature_map.png', 'wb') as f:
        f.write(response.content)
    print("Map saved to temperature_map.png")
```

## Python (OWSLib)

```python
from owslib.wms import WebMapService

# Connect to WMS
wms = WebMapService('http://localhost:8080/wms', version='1.3.0')

# List available layers
print("Available layers:")
for layer in wms.contents:
    print(f"  - {layer}: {wms[layer].title}")

# Get map
img = wms.getmap(
    layers=['gfs_TMP_2m'],
    styles=['temperature'],
    srs='EPSG:3857',
    bbox=(-20037508, -20037508, 20037508, 20037508),
    size=(512, 512),
    format='image/png',
    transparent=True
)

with open('map.png', 'wb') as f:
    f.write(img.read())
```

## Curl Examples

### Get Latest Temperature Map
```bash
curl "http://localhost:8080/wms?\
SERVICE=WMS&\
VERSION=1.3.0&\
REQUEST=GetMap&\
LAYERS=gfs_TMP_2m&\
STYLES=temperature&\
CRS=EPSG:3857&\
BBOX=-20037508,-20037508,20037508,20037508&\
WIDTH=512&\
HEIGHT=512&\
FORMAT=image/png" -o temp.png
```

### Query Point Value
```bash
curl "http://localhost:8080/wms?\
SERVICE=WMS&\
VERSION=1.3.0&\
REQUEST=GetFeatureInfo&\
QUERY_LAYERS=gfs_TMP_2m&\
LAYERS=gfs_TMP_2m&\
CRS=EPSG:4326&\
BBOX=-180,-90,180,90&\
WIDTH=360&\
HEIGHT=180&\
I=180&\
J=90&\
INFO_FORMAT=application/json"
```

### Get XYZ Tile
```bash
curl "http://localhost:8080/tiles/gfs_TMP_2m/temperature/4/3/5.png" -o tile.png
```

## QGIS

1. Open QGIS
2. **Layer** → **Add Layer** → **Add WMS/WMTS Layer**
3. Click **New** to create a connection:
   - **Name**: Weather WMS
   - **URL**: `http://localhost:8080/wms`
4. Click **Connect**
5. Select layers from the list
6. Click **Add** to add to map

## See Also

- [WMS Endpoints](./wms.md) - WMS API details
- [WMTS Endpoints](./wmts.md) - WMTS API details
- [Quick Start](../getting-started/quickstart.md) - Getting started guide
