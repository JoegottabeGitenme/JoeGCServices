# WMS CRS Conversion Fix

## Problem

WMS GetMap requests were showing artifacts (vertical lines, strange patterns) because Leaflet's `L.tileLayer.wms()` sends bbox coordinates in **EPSG:3857 (Web Mercator)** by default, but our code was treating all bbox values as **EPSG:4326 (WGS84)** lat/lon degrees.

### Example of the Issue

Leaflet WMS request:
```
BBOX=0,10018754.171394628,10018754.171394622,20037508.34278071
CRS=EPSG:3857
```

Our code was treating these values as degrees:
- 0° to 10,018,754° longitude (INVALID!)
- 10,018,754° to 20,037,508° latitude (INVALID!)

This caused the rendering function to sample completely wrong areas of the GRIB grid.

## Solution

Added CRS-aware bbox parsing with automatic conversion from Web Mercator to WGS84:

```rust
fn mercator_to_wgs84(x: f64, y: f64) -> (f64, f64) {
    let lon = (x / 20037508.34) * 180.0;
    let lat = (y / 20037508.34) * 180.0;
    let lat = 180.0 / std::f64::consts::PI * 
        (2.0 * (lat * std::f64::consts::PI / 180.0).exp().atan() - std::f64::consts::PI / 2.0);
    (lon, lat)
}
```

### Coordinate Systems

| CRS | Type | Units | Example BBOX |
|-----|------|-------|--------------|
| EPSG:4326 | Geographic | Degrees | `-180,-90,180,90` |
| EPSG:3857 | Web Mercator | Meters | `-20037508,-20037508,20037508,20037508` |

### Conversion Formula

**Mercator X to Longitude:**
```
lon = (x / 20037508.34) * 180.0
```

**Mercator Y to Latitude:**
```
lat_rad = (y / 20037508.34) * 180.0
lat = 2 * atan(exp(lat_rad * π / 180)) - π/2
lat_degrees = lat * 180 / π
```

## Testing

### EPSG:3857 (Web Mercator) - Full World
```bash
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=gfs_PRMSL&BBOX=-20037508,-20037508,20037508,20037508&WIDTH=512&HEIGHT=512&FORMAT=image/png&CRS=EPSG:3857" -o world_mercator.png
```

Result: 512x512, 337KB - shows global pressure map

### EPSG:4326 (WGS84) - Full World
```bash
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=gfs_PRMSL&BBOX=-180,-90,180,90&WIDTH=512&HEIGHT=256&FORMAT=image/png&CRS=EPSG:4326" -o world_wgs84.png
```

Result: 512x256, 165KB - shows global pressure map

## Changes Made

**File**: `services/wms-api/src/handlers.rs`

1. Added `mercator_to_wgs84()` conversion function
2. Updated `render_weather_data()` to accept `crs` parameter
3. Added CRS detection and automatic conversion in bbox parsing:
   - If CRS contains "3857" → convert from Mercator to WGS84
   - Otherwise → assume WGS84 (no conversion)

## Impact

- **WMS with EPSG:3857** (Leaflet default): Now works correctly
- **WMS with EPSG:4326**: Still works as before
- **WMTS**: Unaffected (always uses lat/lon internally)

## Web Dashboard

The web viewer's WMS mode now displays correctly without artifacts. Both protocols work:
- **WMTS** (default): Faster, better caching
- **WMS**: Flexible bbox, both CRS supported

