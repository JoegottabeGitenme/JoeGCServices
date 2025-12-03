# Rendering Pipeline Quick Reference

## File Locations

### WMS-API Service (Main rendering path - WORKING)
```
services/wms-api/
├── main.rs (100 lines) - HTTP server with Axum
├── handlers.rs (736 lines) - Request handlers & rendering
│   ├── wms_handler() - WMS GetCapabilities/GetMap router
│   ├── wms_get_map() - GetMap handler (line 105)
│   └── render_weather_data() - ACTUAL RENDERING (line 291) ✓ WORKING
├── state.rs (47 lines) - App state initialization
└── Dockerfile
```

### Renderer Crate (Rendering library - PARTIAL)
```
crates/renderer/
├── lib.rs (15 lines) - Library root (TODO stub)
├── gradient.rs (305 lines) ✓ FULLY IMPLEMENTED
│   ├── temperature_color()
│   ├── wind_speed_color()
│   ├── pressure_color()
│   ├── humidity_color()
│   ├── render_grid() - Generic renderer
│   ├── render_temperature()
│   ├── render_wind_speed()
│   ├── render_pressure()
│   └── render_humidity()
├── png.rs (78 lines) ✓ FULLY IMPLEMENTED
│   ├── create_png() - PNG encoder
│   ├── write_chunk() - PNG chunk writer
│   ├── deflate_idat() - Zlib compression
│   └── crc32_checksum() - CRC calculation
├── contour.rs (2 lines) ✗ EMPTY STUB
└── barbs.rs (2 lines) ✗ EMPTY STUB
```

### Renderer-Worker Service (Non-functional)
```
services/renderer-worker/
├── main.rs (215 lines) ✗ MOSTLY PLACEHOLDER
│   ├── main() - Service startup & job loop (line 31)
│   ├── render_tile() - BROKEN (line 127) TODO
│   │   └── Returns test gradient instead of rendering
│   ├── encode_test_png() - Duplicate PNG code (line 158)
│   └── write_png_chunk() - Duplicate code
└── Dockerfile
```

---

## Key Code Locations

### RENDERING PIPELINE (render_weather_data)
**File**: services/wms-api/src/handlers.rs:291-464

| Step | Lines | Function | Status |
|------|-------|----------|--------|
| Parse layer | 299-305 | Split on '_' | ✓ Works |
| Query catalog | 314-330 | find_by_forecast_hour/get_latest | ✓ Works |
| Load GRIB2 | 333-337 | storage.get() | ✓ Works |
| Parse GRIB2 | 340-352 | Grib2Reader::next_message() | ✓ Works |
| Unpack data | 357-361 | msg.unpack_data() | ✓ Works |
| Validate | 365-379 | Size/dimension checks | ✓ Works |
| Min/max | 382-388 | Data range calculation | ✓ Works |
| Select renderer | 396-452 | Parameter-based routing | ✓ Works |
| Render | 407/416/428/436 | renderer:: functions | ✓ Works |
| PNG encode | 460-461 | renderer::png::create_png() | ✓ Works |

### ISSUES LOCATIONS

| Issue | File | Lines | Type |
|-------|------|-------|------|
| No grid resampling | handlers.rs | 392 | TODO comment |
| Ignores width/height | handlers.rs | 393-394 | Bug |
| Simple parameter match | handlers.rs | 348 | Bug |
| Only first message | handlers.rs | 348-352 | Bug |
| Silent error fallback | handlers.rs | 124-140 | Design issue |
| Fragile substring match | handlers.rs | 397-429 | Bug |
| Test pattern output | renderer-worker/main.rs | 127-155 | Non-functional |
| TODO in lib | crates/renderer/src/lib.rs | 14 | Documentation |
| Empty contour stub | crates/renderer/src/contour.rs | 1-2 | Not implemented |
| Empty barbs stub | crates/renderer/src/barbs.rs | 1-2 | Not implemented |
| Duplicate PNG code | renderer-worker/main.rs | 158-211 | Duplication |

---

## Data Structures

### Input: RenderJob (storage::queue)
```rust
pub struct RenderJob {
    pub id: Uuid,
    pub layer: String,              // "gfs_TMP"
    pub style: String,              // "default"
    pub crs: CrsCode,               // "EPSG:4326"
    pub bbox: BoundingBox,          // [minx, miny, maxx, maxy]
    pub width: u32,                 // Requested pixel width
    pub height: u32,                // Requested pixel height
    pub time: Option<String>,       // Forecast hour or ISO timestamp
    pub format: String,             // "image/png"
}
```

### Processing: Grib2Message (grib2-parser)
```rust
pub struct Grib2Message {
    pub offset: usize,              // File offset
    pub indicator: Indicator,       // Section 0
    pub identification: Identification, // Section 1
    pub grid_definition: GridDefinition, // Section 3
    pub product_definition: ProductDefinition, // Section 4
    pub data_representation: DataRepresentation, // Section 5
    pub bitmap: Option<Bitmap>,     // Section 6 (optional)
    pub data_section: DataSection,  // Section 7
    pub raw_data: Bytes,            // Raw message bytes
}

// Key methods:
pub fn parameter(&self) -> &str          // "TMP", "WIND", etc.
pub fn grid_dims(&self) -> (u32, u32)   // (latitude_points, longitude_points)
pub fn unpack_data(&self) -> Result<Vec<f32>>  // Grid values
```

### Output: RGBA Pixels
```rust
Vec<u8>  // 4 bytes per pixel: R, G, B, A
         // Row-major order
         // Size: width * height * 4 bytes
```

### PNG Binary Format
```
PNG Signature (8 bytes)
├─ IHDR chunk (header)
├─ IDAT chunk(s) (zlib-compressed image data)
└─ IEND chunk (end marker)
```

---

## Color Functions Quick Reference

### Temperature (Celsius)
```
-50°C  → RGB(25, 0, 76)    // Deep purple
-30°C  → RGB(0, 0, 255)    // Blue
  0°C  → RGB(0, 255, 255)  // Cyan
 10°C  → RGB(0, 255, 0)    // Green
 20°C  → RGB(255, 255, 0)  // Yellow
 30°C  → RGB(255, 165, 0)  // Orange
 40°C  → RGB(255, 0, 0)    // Red
 50°C  → RGB(139, 0, 0)    // Dark red
```

### Wind Speed (m/s)
```
  0 m/s → RGB(200, 200, 200)   // Light gray
  5 m/s → RGB(0, 200, 255)     // Light cyan
 10 m/s → RGB(255, 255, 0)     // Yellow
 15 m/s → RGB(255, 165, 0)     // Orange
 20 m/s → RGB(139, 0, 0)       // Dark red
```

### Pressure (hPa)
```
<970 hPa → RGB(75, 0, 130)      // Indigo (stormy)
 990 hPa → RGB(0, 0, 255)       // Blue
1010 hPa → RGB(0, 255, 0)       // Green
1030 hPa → RGB(255, 255, 0)     // Yellow
>1050 hPa → RGB(139, 0, 0)      // Dark red
```

### Humidity (%)
```
  0% → RGB(210, 180, 140)  // Tan (dry)
 25% → RGB(255, 255, 150)  // Light yellow
 50% → RGB(173, 255, 47)   // Yellow-green
 75% → RGB(100, 200, 255)  // Light blue
100% → RGB(25, 50, 200)    // Dark blue (saturated)
```

---

## Error Handling Patterns

### Current (Silent Fallback)
```rust
match render_weather_data(...).await {
    Ok(png_data) => {
        Response::builder()
            .header("content-type", "image/png")
            .body(png_data.into())
    }
    Err(e) => {
        info!(error = %e, "Rendering failed, using placeholder");
        // Returns placeholder image - client never knows!
        Response::builder()
            .header("content-type", "image/png")
            .body(placeholder.into())
    }
}
```

### WMS Exception Format
```xml
<?xml version="1.0"?>
<ServiceExceptionReport>
  <ServiceException code="LayerNotDefined">
    gfs_INVALID: No such parameter
  </ServiceException>
</ServiceExceptionReport>
```

---

## TODOs in Code

### CRITICAL
- `renderer-worker/src/main.rs:128` - "TODO: Implement actual rendering"
  - **Impact**: Worker doesn't actually render anything
  - **Status**: Non-functional placeholder
  - **Priority**: CRITICAL

### HIGH
- `handlers.rs:392` - "TODO: Implement proper resampling to match WMS request dimensions"
  - **Impact**: Ignores width/height parameters
  - **Status**: Always outputs grid resolution
  - **Priority**: HIGH

- `crates/renderer/src/lib.rs:14` - "TODO: Implement rendering algorithms"
  - **Impact**: Library documentation is outdated
  - **Status**: Gradient/PNG work, but contour/barbs don't
  - **Priority**: MEDIUM

### MEDIUM
- `renderer-worker/src/main.rs:153` - "TODO: Use proper PNG encoder from renderer crate"
  - **Impact**: Duplicated code
  - **Status**: Should remove local encoder
  - **Priority**: MEDIUM

---

## Environment Variables

```bash
# Database
DATABASE_URL=postgresql://postgres:postgres@postgres:5432/weatherwms

# Caching & Jobs
REDIS_URL=redis://redis:6379

# Object Storage
S3_ENDPOINT=http://minio:9000
S3_BUCKET=weather-data
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
```

---

## Testing Points

### Test Cases Needed
1. Render temperature layer
2. Render wind layer
3. Render pressure layer
4. Render humidity layer
5. Render unknown parameter (generic gradient)
6. Test grid resampling (various sizes)
7. Test error handling (missing file, corrupt GRIB)
8. Test PNG validity (can be opened in viewers)
9. Test color scales (verify pixel colors)
10. Test performance (large grids)

### Test Data
- `testdata/gfs_sample.grib2` - Sample GRIB2 file
- Should create test files with known structure

---

## API Endpoints

### WMS
```
/wms?SERVICE=WMS&REQUEST=GetCapabilities
/wms?SERVICE=WMS&REQUEST=GetMap&LAYERS=gfs_TMP&WIDTH=256&HEIGHT=256&BBOX=...&CRS=EPSG:4326
```

### WMTS (KVP)
```
/wmts?SERVICE=WMTS&REQUEST=GetCapabilities
/wmts?SERVICE=WMTS&REQUEST=GetTile&LAYER=gfs_TMP&STYLE=default&TILEMATRIX=10&TILEROW=512&TILECOL=512
```

### WMTS (REST)
```
/wmts/rest/gfs_TMP/default/0/10/512/512.png
```

### XYZ Tile
```
/tiles/gfs_TMP/default/10/512/512.png
```

### Metadata
```
/api/parameters/gfs
/api/forecast-times/gfs/TMP
/api/ingestion/events
/health
/ready
/metrics
```

---

## Deployment

### Docker Images
- `wms-api:latest` - HTTP server
- `renderer-worker:latest` - Tile renderer (currently broken)

### Kubernetes (Helm)
```
deploy/helm/weather-wms/
├── values.yaml
├── values-dev.yaml
└── templates/
    ├── api-deployment.yaml
    ├── renderer-deployment.yaml
    ├── configmap.yaml
    ├── ingress.yaml
    └── ...
```

### Docker Compose
```
docker-compose.yml - Local dev environment
```

