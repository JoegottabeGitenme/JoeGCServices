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

## EDR API Examples

The EDR API provides raw data access (as opposed to rendered images). All examples use the base URL `http://localhost:8083/edr`.

### Position Query (Point Data)

Get temperature at a specific location:

```bash
curl "http://localhost:8083/edr/collections/hrrr-surface/position?\
coords=POINT(-97.5 35.2)&\
parameter-name=TMP"
```

### Area Query (Polygon Data)

Get all data within a polygon:

```bash
curl "http://localhost:8083/edr/collections/hrrr-surface/area?\
coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))&\
parameter-name=TMP,UGRD,VGRD"
```

### Radius Query (Circular Area)

Get data within 50km of a point:

```bash
curl "http://localhost:8083/edr/collections/hrrr-surface/radius?\
coords=POINT(-97.5 35.2)&\
within=50&\
within-units=km&\
parameter-name=TMP"
```

### Trajectory Query (Path Data)

Get data along a flight path:

```bash
curl "http://localhost:8083/edr/collections/hrrr-isobaric/trajectory?\
coords=LINESTRING(-100 40,-99 40.5,-98 41)&\
z=850&\
parameter-name=TMP,UGRD,VGRD"
```

With altitude (LINESTRINGZ):

```bash
curl "http://localhost:8083/edr/collections/hrrr-isobaric/trajectory?\
coords=LINESTRINGZ(-100 40 850,-99 40.5 700,-98 41 500)&\
parameter-name=TMP"
```

### Corridor Query (Buffered Path)

Get data within a 10km wide, 1000m tall corridor:

```bash
curl "http://localhost:8083/edr/collections/hrrr-isobaric/corridor?\
coords=LINESTRING(-100 40,-99 40.5,-98 41)&\
corridor-width=10&\
width-units=km&\
corridor-height=1000&\
height-units=m&\
parameter-name=TMP"
```

### Cube Query (3D Volume)

Get a 3D data cube:

```bash
curl "http://localhost:8083/edr/collections/hrrr-isobaric/cube?\
bbox=-98,35,-97,36&\
z=850,700,500&\
parameter-name=TMP,HGT"
```

### Locations Query (Named Points)

List all available named locations:

```bash
curl "http://localhost:8083/edr/collections/hrrr-surface/locations"
```

Get weather at a specific airport:

```bash
curl "http://localhost:8083/edr/collections/hrrr-surface/locations/KJFK?\
parameter-name=TMP,UGRD,VGRD"
```

Get weather at multiple named locations:

```bash
# JFK Airport
curl "http://localhost:8083/edr/collections/hrrr-surface/locations/KJFK"

# Chicago O'Hare
curl "http://localhost:8083/edr/collections/hrrr-surface/locations/KORD"

# New York City (case-insensitive)
curl "http://localhost:8083/edr/collections/hrrr-surface/locations/nyc"
```

### GeoJSON Output Format

Get data in GeoJSON format using the `f` parameter:

```bash
curl "http://localhost:8083/edr/collections/hrrr-surface/position?\
coords=POINT(-97.5 35.2)&\
parameter-name=TMP&\
f=geojson"
```

Or using the Accept header:

```bash
curl -H "Accept: application/geo+json" \
  "http://localhost:8083/edr/collections/hrrr-surface/position?\
coords=POINT(-97.5 35.2)&\
parameter-name=TMP"
```

GeoJSON output for locations:

```bash
curl "http://localhost:8083/edr/collections/hrrr-surface/locations/KJFK?\
parameter-name=TMP&\
f=geojson"
```

### Python EDR Client

```python
import requests

EDR_BASE = "http://localhost:8083/edr"

# Get available collections
collections = requests.get(f"{EDR_BASE}/collections").json()
print("Available collections:")
for col in collections['collections']:
    print(f"  - {col['id']}: {col['title']}")

# Position query (CoverageJSON)
response = requests.get(
    f"{EDR_BASE}/collections/hrrr-surface/position",
    params={
        "coords": "POINT(-97.5 35.2)",
        "parameter-name": "TMP"
    }
)

if response.ok:
    covjson = response.json()
    temp = covjson['ranges']['TMP']['values'][0]
    print(f"Temperature: {temp} K ({temp - 273.15:.1f} C)")

# Position query (GeoJSON)
response = requests.get(
    f"{EDR_BASE}/collections/hrrr-surface/position",
    params={
        "coords": "POINT(-97.5 35.2)",
        "parameter-name": "TMP",
        "f": "geojson"
    }
)

if response.ok:
    geojson = response.json()
    feature = geojson['features'][0]
    temp = feature['properties']['TMP']['value']
    print(f"Temperature: {temp} K")

# Query named locations
locations = requests.get(f"{EDR_BASE}/collections/hrrr-surface/locations").json()
print(f"\nAvailable locations: {len(locations['features'])}")
for feature in locations['features'][:5]:
    loc_id = feature['id']
    name = feature['properties']['name']
    coords = feature['geometry']['coordinates']
    print(f"  {loc_id}: {name} at ({coords[0]}, {coords[1]})")

# Get weather at JFK Airport
response = requests.get(
    f"{EDR_BASE}/collections/hrrr-surface/locations/KJFK",
    params={"parameter-name": "TMP,UGRD,VGRD"}
)

if response.ok:
    covjson = response.json()
    temp = covjson['ranges']['TMP']['values'][0]
    print(f"\nJFK Temperature: {temp - 273.15:.1f}C")
```

### JavaScript EDR Client

```javascript
const EDR_BASE = 'http://localhost:8083/edr';

// Get temperature at a point (CoverageJSON)
async function getTemperature(lon, lat) {
    const url = `${EDR_BASE}/collections/hrrr-surface/position?` +
        `coords=POINT(${lon} ${lat})&parameter-name=TMP`;
    
    const response = await fetch(url);
    const covjson = await response.json();
    
    const tempK = covjson.ranges.TMP.values[0];
    const tempC = tempK - 273.15;
    
    console.log(`Temperature at (${lon}, ${lat}): ${tempC.toFixed(1)}C`);
    return tempC;
}

// Get temperature as GeoJSON
async function getTemperatureGeoJson(lon, lat) {
    const url = `${EDR_BASE}/collections/hrrr-surface/position?` +
        `coords=POINT(${lon} ${lat})&parameter-name=TMP&f=geojson`;
    
    const response = await fetch(url);
    const geojson = await response.json();
    
    const feature = geojson.features[0];
    const tempK = feature.properties.TMP.value;
    const tempC = tempK - 273.15;
    
    console.log(`Temperature: ${tempC.toFixed(1)}C`);
    return geojson;
}

// List available named locations
async function getLocations() {
    const url = `${EDR_BASE}/collections/hrrr-surface/locations`;
    const response = await fetch(url);
    const geojson = await response.json();
    
    console.log(`Found ${geojson.features.length} locations:`);
    geojson.features.forEach(f => {
        console.log(`  ${f.id}: ${f.properties.name}`);
    });
    return geojson;
}

// Get weather at a named location
async function getLocationWeather(locationId) {
    const url = `${EDR_BASE}/collections/hrrr-surface/locations/${locationId}?` +
        `parameter-name=TMP,UGRD,VGRD`;
    
    const response = await fetch(url);
    const covjson = await response.json();
    
    const tempK = covjson.ranges.TMP.values[0];
    const tempC = tempK - 273.15;
    
    console.log(`${locationId} Temperature: ${tempC.toFixed(1)}C`);
    return covjson;
}

// Examples
getTemperature(-97.5, 35.5);
getLocations();
getLocationWeather('KJFK');
```

## See Also

- [WMS Endpoints](./wms.md) - WMS API details
- [WMTS Endpoints](./wmts.md) - WMTS API details
- [EDR Endpoints](./edr.md) - EDR API details
- [Quick Start](../getting-started/quickstart.md) - Getting started guide
