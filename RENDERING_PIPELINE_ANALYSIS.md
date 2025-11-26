# GRIB Data Rendering Pipeline Analysis - weather-wms

## Executive Summary

The GRIB rendering pipeline is partially implemented with significant incomplete sections. The data flows from storage → GRIB2 parsing → gradient rendering → PNG encoding, but several components are stubbed out or placeholder implementations. The pipeline works for basic temperature/wind visualization but lacks proper grid resampling, advanced rendering styles, and comprehensive error handling.

---

## 1. Data Flow Architecture

### Flow Diagram
```
WMS/WMTS HTTP Request (handlers.rs)
    ↓
Query Catalog for Dataset (AppState.catalog)
    ↓
Load GRIB2 File from Storage (ObjectStorage.get)
    ↓
Parse GRIB2 Messages (grib2-parser::Grib2Reader)
    ↓
Find Parameter Match in Messages
    ↓
Unpack Grid Data (Grib2Message::unpack_data)
    ↓
Select Renderer based on Parameter Type
    ↓
Gradient Rendering (renderer::gradient::render_*)
    ↓
PNG Encoding (renderer::png::create_png)
    ↓
Cache Result & Return to Client
```

### Entry Points
1. **WMS GetMap**: `/wms?SERVICE=WMS&REQUEST=GetMap&LAYERS=model_parameter&...`
2. **WMTS GetTile**: `/wmts?SERVICE=WMTS&REQUEST=GetTile&LAYER=...`
3. **XYZ Tile**: `/tiles/:layer/:style/:z/:x/:y`

---

## 2. Component Analysis

### 2.1 WMS-API Service (services/wms-api/src/)

**Location**: services/wms-api/src/

**Key Files**:
- `main.rs` - Axum HTTP server setup
- `handlers.rs` - HTTP request handlers (736 lines)
- `state.rs` - Application state & configuration

**Functionality**:
- HTTP server implementing OGC WMS 1.1.1/1.3.0 and WMTS 1.0.0 specs
- Parses WMS/WMTS query parameters
- Routes to rendering pipeline
- Returns PNG images or XML capabilities

**Key Handler**: `wms_get_map()` (handlers.rs:105-142)
```rust
async fn wms_get_map(state: Arc<AppState>, params: WmsParams) -> Response
```
- Calls `render_weather_data()` for actual rendering
- Falls back to placeholder PNG on error
- Returns image/png with appropriate headers

**Rendering Entry**: `render_weather_data()` (handlers.rs:291-464)

---

### 2.2 Renderer Crate (crates/renderer/src/)

**Status**: Partially implemented library

**Files and Implementation Status**:

| File | Status | Details |
|------|--------|---------|
| `lib.rs` | **Stub** | TODO comment only - "Implement rendering algorithms" |
| `gradient.rs` | **Complete** | 305 lines - Fully functional |
| `contour.rs` | **Empty stub** | 2 lines - Not implemented |
| `barbs.rs` | **Empty stub** | 2 lines - Not implemented |
| `png.rs` | **Complete** | 78 lines - PNG encoding functional |

**Implemented Features**:

#### gradient.rs (305 lines) - **Fully Functional**
Color scales for weather parameters:
- `temperature_color()` - Celsius scale: purple (-50°C) → red (50°C)
- `wind_speed_color()` - 0-20 m/s scale: gray → dark red
- `pressure_color()` - 950-1050 hPa: indigo → red
- `humidity_color()` - 0-100%: tan → dark blue

Generic rendering function:
```rust
pub fn render_grid<F>(
    data: &[f32],
    width: usize,
    height: usize,
    min_val: f32,
    max_val: f32,
    color_fn: F,
) -> Vec<u8>
```
- Takes 1D row-major grid data
- Normalizes values (0-1 range)
- Applies color function
- Returns RGBA pixels (4 bytes/pixel)

Parameter-specific wrappers:
- `render_temperature()` - K→C conversion
- `render_wind_speed()` - m/s scaling
- `render_pressure()` - Pa→hPa conversion
- `render_humidity()` - 0-100% scaling

#### png.rs (78 lines) - **Functional**
PNG encoding from RGBA data:
```rust
pub fn create_png(
    pixels: &[u8],
    width: usize,
    height: usize
) -> Result<Vec<u8>, String>
```
- Constructs valid PNG structure
- IHDR chunk with metadata
- IDAT chunk with zlib compression
- IEND terminator
- CRC32 checksums
- Uses `flate2` for deflate compression

---

### 2.3 Renderer-Worker Service (services/renderer-worker/src/)

**Status**: Mostly placeholder/incomplete

**Location**: services/renderer-worker/src/main.rs (215 lines)

**Architecture**:
- Async Rust service consuming Redis jobs
- Processes RenderJob from queue
- Returns PNG data or error

**Flow**:
1. Connect to Redis queue & object storage (lines 60-74)
2. Main loop claiming jobs (line 78)
3. Process job with `render_tile()` (line 82)
4. Store in cache & mark complete (lines 85-103)

**render_tile() Function** (lines 127-155):
```rust
async fn render_tile(storage: &ObjectStorage, job: &RenderJob) -> Result<Vec<u8>>
```
**MAJOR ISSUES**:
- **TODO comment** (line 128): "Implement actual rendering"
- Generates test gradient pattern instead of real rendering
- Creates dummy pixels with simple color gradients
- Does not load GRIB data from storage
- Does not apply coordinate transformation
- Does not match WMS request dimensions

**Current test pattern output**:
```rust
pixels[idx] = (x * 255 / width) as u8;     // Red gradient
pixels[idx + 1] = (y * 255 / height) as u8; // Green gradient
pixels[idx + 2] = 128;                      // Fixed blue
pixels[idx + 3] = 255;                      // Full alpha
```

**PNG Encoding** (lines 158-211):
- Has own `encode_test_png()` implementation
- Comments say "placeholder - real implementation would use the renderer crate"
- **TODO comment** (line 153): "Use proper PNG encoder from renderer crate"
- Duplicates PNG logic from renderer crate

**Known Issues**:
1. Renderer-worker is not integrated with actual rendering pipeline
2. Duplicate PNG encoding code (should use renderer::png module)
3. No GRIB2 parsing integration
4. No projection/coordinate transformation
5. Test pattern is not weather-realistic data

---

### 2.4 WMS-API Rendering Logic (handlers.rs:291-464)

**Status**: Partially implemented, uses renderer library correctly

This is the **main rendering implementation** that actually processes GRIB data.

**Function**: `render_weather_data()` (291-464)

**Steps**:

1. **Parse Layer Name** (299-305):
   ```rust
   let parts: Vec<&str> = layer.split('_').collect();
   let model = parts[0];
   let parameter = parts[1..].join("_");
   ```
   - Format: `"gfs_TMP"` → model="gfs", parameter="TMP"

2. **Query Catalog** (314-330):
   - If TIME parameter: Find dataset matching forecast hour
   - Otherwise: Get latest dataset
   - Returns `CatalogEntry` with storage path

3. **Load GRIB2 Data** (333-337):
   ```rust
   let grib_data = state.storage.get(&entry.storage_path).await?;
   ```
   - Fetches raw GRIB2 bytes from S3/MinIO

4. **Parse GRIB2** (340-352):
   ```rust
   let mut reader = grib2_parser::Grib2Reader::new(grib_data);
   while let Some(msg) = reader.next_message()? {
       if msg.parameter() == &parameter[..] {
           message = Some(msg);
           break;
       }
   }
   ```
   - Iterates messages until parameter matches
   - Note: **Only matches first message** with parameter

5. **Unpack Grid Data** (357-361):
   ```rust
   let grid_data = msg.unpack_data()?;
   let (grid_height, grid_width) = msg.grid_dims();
   ```
   - Calls GRIB2 message unpacking
   - Gets grid dimensions (2 u32 values)
   - Returns Vec<f32> with grid values

6. **Data Validation** (365-379):
   - Logs grid dimensions and sample values
   - Validates data size matches dimensions

7. **Find Min/Max for Scaling** (382-388):
   ```rust
   let (min_val, max_val) = grid_data.iter()
       .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &val| {
           (min.min(val), max.max(val))
       });
   ```

8. **Select Renderer** (396-452):
   - **Temperature** (397-408): K→Celsius, use `render_temperature()`
   - **Wind** (409-416): Use `render_wind_speed()`
   - **Pressure** (417-428): Pa→hPa, use `render_pressure()`
   - **Humidity** (429-436): Use `render_humidity()`
   - **Generic** (437-451): Custom gradient with HSV→RGB

9. **PNG Encoding** (460-461):
   ```rust
   let png = renderer::png::create_png(&rgba_data, grid_width, grid_height)?;
   ```

**ISSUES**:

1. **No Grid Resampling** (TODO at line 392):
   ```rust
   // TODO: Implement proper resampling to match WMS request dimensions
   ```
   - WMS requests can specify any width/height
   - Current implementation ignores width/height params
   - Renders at full grid resolution instead
   - Should use bilinear or nearest-neighbor interpolation

2. **Limited GRIB2 Message Matching** (348):
   - Only matches first message with parameter
   - GRIB2 files may have multiple messages per parameter (different levels)
   - Should filter by level/pressure/height as well

3. **Parameter Name Matching** (348, 397-429):
   - Uses substring matching: `contains("TMP")`, `contains("WIND")`
   - Fragile - could match unexpected parameters
   - No standard parameter code mapping (CF conventions, WMO codes)

4. **Generic Gradient HSV→RGB** (445-450):
   - HSV color space conversion is complex and approximate
   - Hard-coded hue range (blue→red)

---

## 3. Data Structures

### RenderJob (storage::queue)
```rust
struct RenderJob {
    id: Uuid,
    layer: String,
    style: String,
    crs: CrsCode,
    bbox: BoundingBox,
    width: u32,
    height: u32,
    time: Option<String>,
    format: String,
}
```

### Grib2Message (grib2-parser)
```rust
pub struct Grib2Message {
    pub offset: usize,
    pub indicator: Indicator,
    pub identification: Identification,
    pub grid_definition: GridDefinition,
    pub product_definition: ProductDefinition,
    pub data_representation: DataRepresentation,
    pub bitmap: Option<Bitmap>,
    pub data_section: DataSection,
    pub raw_data: Bytes,
}
```

Key methods:
- `parameter() -> &str` - Parameter short name (e.g., "TMP")
- `grid_dims() -> (u32, u32)` - (latitude points, longitude points)
- `unpack_data() -> Grib2Result<Vec<f32>>` - Grid values

---

## 4. Logging & Error Handling

### Logging (Using `tracing` crate)

**WMS-API**:
- `info!()` logs in handlers (lines 121, 310, 365-389, 403-434, 454-456)
- Includes layer, style, dimensions, data ranges
- **Good**: Informative structured logging
- **Gap**: No detailed error context in catch blocks

**Renderer-Worker**:
- `info!()` for startup and job processing
- `warn!()` for cache failures (line 97)
- `error!()` for job failures (lines 108-109)
- **Issue**: No logging inside render_tile placeholder

### Error Handling

**Pattern 1: String-based errors**
```rust
match render_weather_data(...).await {
    Ok(png_data) => { /* return */ }
    Err(e) => {
        info!(error = %e, "Rendering failed, using placeholder");
        // Fall back to placeholder
    }
}
```
- `Err(String)` return type
- Silent fallback to placeholder image
- **Problem**: Users don't know if they're getting real or placeholder data

**Pattern 2: WMS Exceptions**
```rust
fn wms_exception(code: &str, msg: &str, status: StatusCode) -> Response
```
- Constructs XML exception report
- Returns OGC-compliant error responses
- Used for missing parameters, invalid CRS, etc.

**Missing Error Cases**:
- No validation of GRIB2 grid dimensions against WMS request
- No timeout handling for storage operations
- No retry logic for transient failures
- No circuit breaker for failing dependencies

---

## 5. Dependencies & Configuration

### Environment Variables (AppState::new)

| Variable | Default | Purpose |
|----------|---------|---------|
| `DATABASE_URL` | `postgresql://postgres:postgres@postgres:5432/weatherwms` | Catalog database |
| `REDIS_URL` | `redis://redis:6379` | Cache & job queue |
| `S3_ENDPOINT` | `http://minio:9000` | Object storage |
| `S3_BUCKET` | `weather-data` | Data bucket name |
| `S3_ACCESS_KEY` | `minioadmin` | Storage credentials |
| `S3_SECRET_KEY` | `minioadmin` | Storage credentials |

### Workspace Dependencies (Cargo.toml)

**Critical for Rendering**:
- `tokio` - Async runtime (workspace version)
- `bytes` - Efficient byte buffers
- `flate2` - PNG zlib compression
- `crc32fast` - PNG checksums
- `serde`/`serde_json` - Serialization
- `tracing` - Structured logging
- `thiserror` - Error types
- `anyhow` - Error handling

**Crate Cross-Dependencies**:
```
wms-api
  ├─ wms-common (error types, grid specs)
  ├─ wms-protocol (WMS/WMTS specs)
  ├─ storage (catalog, cache, queues)
  ├─ grib2-parser (GRIB parsing)
  ├─ renderer (gradient, PNG)
  └─ projection (coordinate transforms - NOT USED!)

renderer
  ├─ wms-common
  ├─ projection
  └─ flate2, crc32fast (PNG)

renderer-worker
  ├─ wms-common
  ├─ storage
  ├─ renderer
  ├─ projection (imported but NOT USED!)
  ├─ grib2-parser (imported but NOT USED!)
  └─ tokio
```

**Issue**: `projection` crate is imported but never used in rendering pipeline!

---

## 6. Incomplete Features & TODOs

### Critical TODOs

| Location | Code | Priority | Impact |
|----------|------|----------|--------|
| `crates/renderer/src/lib.rs:14` | "TODO: Implement rendering algorithms" | HIGH | Library main documentation |
| `services/renderer-worker/src/main.rs:128-132` | "TODO: Implement actual rendering" | CRITICAL | Worker is non-functional |
| `services/renderer-worker/src/main.rs:153` | "TODO: Use proper PNG encoder from renderer crate" | MEDIUM | Code duplication |
| `services/wms-api/src/handlers.rs:392` | "TODO: Implement proper resampling to match WMS request dimensions" | HIGH | Output size mismatch |

### Unimplemented Rendering Styles

**Stub Files**:
- `crates/renderer/src/contour.rs` - Marching squares algorithm (2 lines, empty)
- `crates/renderer/src/barbs.rs` - Wind barbs/arrows (2 lines, empty)

**What's Missing**:
- Contour lines (isoline rendering)
- Wind vector fields (barbs/arrows)
- Composite rendering (overlay multiple parameters)
- Custom style definitions (from JSON config)

---

## 7. Known Issues & Limitations

### Critical Issues

1. **Renderer-Worker Non-Functional**
   - Returns test patterns, not real GRIB data
   - Can't be used for production tile generation
   - Path: `services/renderer-worker/src/main.rs:127-155`

2. **No Grid Resampling**
   - WMS allows arbitrary output dimensions
   - Current code ignores width/height parameters
   - Always outputs original grid resolution
   - Path: `services/wms-api/src/handlers.rs:392`

3. **Single Message per Parameter**
   - GRIB2 files often have multiple messages per parameter (different levels)
   - Current code takes first match only
   - Should support level/height selection
   - Path: `services/wms-api/src/handlers.rs:343-352`

### Medium Issues

4. **Fragile Parameter Matching**
   - Uses substring matching (`contains("TMP")`)
   - Could match unintended parameters
   - No standard mappings (WMO codes, CF conventions)

5. **Silent Fallback on Errors**
   - Rendering failures return placeholder images
   - Client can't distinguish real vs placeholder data
   - No error indicators or warnings
   - Path: `services/wms-api/src/handlers.rs:124-140`

6. **Duplicate PNG Encoding**
   - renderer-worker has its own PNG encoder (lines 158-211)
   - Duplicates code from `renderer::png::create_png()`
   - Should consolidate to single implementation

7. **Projection Unused**
   - `projection` crate imported but never used
   - Coordinate transformations not implemented
   - All data rendered in grid space (not map projections)
   - Path: All three services import but don't use

### Configuration Issues

8. **Hardcoded Color Scales**
   - Temperature range: -50°C to 50°C (fixed)
   - Wind speed: 0-20 m/s (fixed)
   - Pressure: 950-1050 hPa (fixed)
   - No configuration file support

9. **Missing Style Configuration**
   - Capabilities XML mentions styles but doesn't load them
   - All parameters use gradient rendering
   - `config/styles/*.json` files exist but not loaded
   - No support for user-defined styles

10. **Limited GRIB2 Support**
    - Only supports simple packing (method 0)
    - Other packing methods return "not yet supported" error
    - JPEG2000, complex packing not supported

---

## 8. Error Handling Capabilities

### What Works
✅ WMS exception XML generation  
✅ OGC-compliant error codes  
✅ Structured logging with tracing  
✅ Result types for fallible operations  

### What's Missing
❌ Timeout handling (storage, GRIB parsing)  
❌ Retry logic (transient failures)  
❌ Circuit breakers (cascading failures)  
❌ Error metrics/monitoring  
❌ Detailed error context (stack traces, data samples)  
❌ Client-facing error details (not just "rendering failed")  

### Error Propagation
```rust
render_weather_data()
  └─> String error
      └─> Silently falls back to placeholder
          └─> Client never knows
```

---

## 9. Performance Characteristics

### Bottlenecks

1. **GRIB2 Parsing**
   - Full file parsed on every request
   - Linear search for parameter match
   - Should cache parsed metadata

2. **No Output Caching**
   - Tile cache exists but never populated
   - Every request re-renders from scratch
   - Should generate cache keys and store tiles

3. **No Grid Resampling**
   - Large grids (GFS: 1440×721) rendered at full size
   - Always does RGBA pixel generation
   - PNG compression is slow for large images

4. **Memory Usage**
   - `Vec<f32>` for grid data: 4 bytes × grid size
   - GFS example: ~4.1 MB per message
   - RGBA output: 4 bytes × grid size = ~4.1 MB

### Optimization Opportunities

- **In-memory GRIB cache**: Store parsed messages
- **Tile caching**: Use RenderJob queue for pre-generation
- **Lazy resampling**: Only when needed for output dimensions
- **Streaming PNG**: Don't buffer entire image
- **GPU rendering**: For large grids

---

## 10. Testing & Validation

### What Exists
- Test files in `crates/grib2-parser/tests/`
- GRIB2 parser has unit tests

### What's Missing
- Integration tests for rendering pipeline
- Test GRIB2 files with known results
- Rendering validation (pixel correctness)
- Performance benchmarks
- Error case coverage

---

## Summary Table

| Component | Status | Completeness | Issues |
|-----------|--------|--------------|--------|
| WMS-API Handler | Working | 85% | No grid resampling, silent errors |
| GRIB2 Parsing | Working | 95% | Simple packing only |
| Gradient Rendering | Working | 100% | Fixed color scales |
| PNG Encoding | Working | 100% | Code duplication |
| Renderer-Worker | Non-functional | 10% | Test patterns only |
| Contour Rendering | Unimplemented | 0% | Empty stub |
| Barb Rendering | Unimplemented | 0% | Empty stub |
| Projection Transforms | Available | Unused | 0% integration |
| Style Configuration | Not loaded | 0% | Config files exist |
| Error Handling | Partial | 60% | Silent failures, no recovery |
| Caching | Partial | 20% | Cache infrastructure exists, not used |

---

## Recommendations

### High Priority (Blocking production)
1. Implement actual rendering in renderer-worker service
2. Add grid resampling for WMS output dimensions
3. Implement proper error reporting (don't silently fall back)
4. Add GRIB2 message filtering (by level, height)

### Medium Priority (Feature completeness)
5. Implement contour line rendering
6. Implement wind barb/arrow rendering
7. Load and apply style configurations
8. Consolidate PNG encoding (remove duplication)
9. Add projection coordinate transformations

### Low Priority (Nice to have)
10. Implement other GRIB2 packing methods
11. Add caching/tile pre-generation
12. Performance optimization (GPU, streaming)
13. Comprehensive error monitoring
