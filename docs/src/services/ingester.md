# Ingester Service

The Ingester service is a standalone HTTP service that parses weather data files (GRIB2 and NetCDF), extracts parameters, and writes them to Zarr V3 format with multi-resolution pyramids for efficient tile rendering.

## Overview

**Location**: `services/ingester/`  
**Language**: Rust  
**Port**: 8082 (HTTP API)  
**Scaling**: Vertical (CPU/Memory intensive)

## Responsibilities

1. **File Parsing**: Reads GRIB2 and NetCDF-4 files
2. **Parameter Extraction**: Extracts weather parameters from files based on configuration
3. **Zarr Conversion**: Writes grid data as Zarr V3 arrays with sharding
4. **Pyramid Generation**: Creates multi-resolution pyramids for fast rendering
5. **Cataloging**: Registers metadata in PostgreSQL
6. **Status Tracking**: Tracks active and completed ingestions

## Architecture

```mermaid
graph TB
    Downloader["Downloader Service"]
    HTTP["HTTP Server
    Port 8082"]
    
    subgraph Ingester["Ingester Service"]
        Server["HTTP Handler"]
        Tracker["Ingestion Tracker"]
        
        subgraph Processing
            Parse["File Parser"]
            Extract["Extract Messages"]
            Decode["Decode Grids"]
        end
        
        subgraph Writing
            Zarr["Write Zarr Array"]
            Pyramid["Generate Pyramids"]
        end
    end
    
    subgraph StorageLayer["Storage"]
        Upload["Upload to MinIO"]
        Catalog["Register in PostgreSQL"]
    end
    
    Downloader -->|POST /ingest| HTTP
    HTTP --> Server
    Server --> Tracker
    Server --> Parse
    Parse --> Extract
    Extract --> Decode
    Decode --> Zarr
    Zarr --> Pyramid
    Pyramid --> Upload
    Upload --> Catalog
```

## HTTP API

### POST /ingest - Trigger Ingestion

Processes a downloaded file and stores it as Zarr format.

```bash
POST http://ingester:8082/ingest
Content-Type: application/json

{
  "file_path": "/data/downloads/gfs_20241217_12z_f003.grib2",
  "model": "gfs",
  "source_url": "https://noaa-gfs-bdp-pds.s3.amazonaws.com/...",
  "forecast_hour": 3
}
```

**Response**:
```json
{
  "success": true,
  "message": "Ingested 47 datasets",
  "datasets_registered": 47,
  "model": "gfs",
  "reference_time": "2024-12-17T12:00:00Z",
  "parameters": ["TMP", "UGRD", "VGRD", "RH", "HGT", ...]
}
```

---

### GET /status - Ingestion Status

Returns currently active and recently completed ingestions.

```bash
GET http://ingester:8082/status
```

**Response**:
```json
{
  "active": [
    {
      "id": "abc123-...",
      "file_path": "/data/downloads/gfs_20241217_12z_f006.grib2",
      "model": "gfs",
      "started_at": "2024-12-17T19:54:00Z",
      "status": "writing_zarr",
      "parameters_found": 25,
      "datasets_registered": 20
    }
  ],
  "recent": [
    {
      "id": "def456-...",
      "file_path": "/data/downloads/mrms_MRMS_SeamlessHSR_00.00_20241217-143000.grib2.gz",
      "started_at": "2024-12-17T19:53:00Z",
      "completed_at": "2024-12-17T19:53:01Z",
      "duration_ms": 450,
      "success": true,
      "datasets_registered": 1,
      "parameters": ["REFL"],
      "error_message": null
    }
  ],
  "total_completed": 156
}
```

---

### GET /health - Health Check

```bash
GET http://ingester:8082/health
```

**Response**:
```json
{
  "status": "ok",
  "service": "ingester",
  "version": "0.1.0"
}
```

---

### GET /metrics - Prometheus Metrics

Returns metrics in Prometheus format.

```bash
GET http://ingester:8082/metrics
```

## Supported Formats

### GRIB2 (GRIB Edition 2)

**Used by**: GFS, HRRR, MRMS

**Features**:
- Binary grid format
- Multiple messages per file (one per parameter/level)
- Various compression schemes (JPEG2000, PNG, simple packing)
- Rich metadata (projection, levels, parameters)

**Parser**: Custom Rust implementation (`grib2-parser` crate)

---

### NetCDF-4 (Network Common Data Form)

**Used by**: GOES-16, GOES-18 satellite data

**Features**:
- HDF5-based format
- Multiple variables per file
- Geostationary projection
- CF-compliant metadata

**Parser**: Custom Rust implementation (`netcdf-parser` crate)

## Ingestion Flow

### 1. File Detection

Ingestion is triggered when the Downloader service completes a download:

```rust
// Downloader triggers after successful download
POST http://ingester:8082/ingest
{
  "file_path": "/data/downloads/gfs_20241217_12z_f003.grib2",
  "model": "gfs",
  "source_url": "https://noaa-gfs-bdp-pds.s3.amazonaws.com/..."
}
```

---

### 2. Parse File

**GRIB2 Parsing**:
```rust
let bytes = Bytes::from(fs::read(&path)?);
let mut reader = grib2_parser::Grib2Reader::new(bytes);

while let Some(message) = reader.next_message().ok().flatten() {
    // Extract parameter info
    let param = &message.product_definition.parameter_short_name;
    let level = &message.product_definition.level_description;
    let level_type = message.product_definition.level_type;
    
    // Decode grid values
    let grid_data = message.unpack_data()?;
    let width = message.grid_definition.num_points_longitude;
    let height = message.grid_definition.num_points_latitude;
}
```

**NetCDF Parsing**:
```rust
use netcdf_parser::GoesParser;

let parser = GoesParser::open(&path)?;
let data = parser.read_data()?;
let projection = parser.get_projection()?;
```

---

### 3. Parameter Filtering

Only configured parameters are ingested. The filter is defined in `crates/ingestion/src/config.rs`:

```rust
// Pressure levels to ingest
let pressure_levels: HashSet<u32> = [
    1000, 975, 950, 925, 900, 850, 700, 500, 
    300, 250, 200, 100, 70, 50, 30, 20, 10
].into_iter().collect();

// Target parameters with level types
let target_params = vec![
    // Pressure
    ("PRMSL", vec![(101, None)]),                    // Mean sea level
    
    // Temperature
    ("TMP", vec![(103, Some(2)), (100, None)]),      // 2m + pressure levels
    ("DPT", vec![(103, Some(2))]),                   // Dew point at 2m
    
    // Wind
    ("UGRD", vec![(103, Some(10)), (100, None)]),    // 10m + pressure levels
    ("VGRD", vec![(103, Some(10)), (100, None)]),    // 10m + pressure levels
    ("GUST", vec![(1, None)]),                       // Surface gust
    
    // ... more parameters
];
```

---

### 3b. Sentinel Value Conversion

Data sources like MRMS use sentinel values (e.g., -999) for missing/invalid data. Before writing to Zarr, these are converted to NaN:

```rust
// In grid processing
let grid_data: Vec<f32> = raw_data.iter().map(|&v| {
    if v <= -90.0 {
        f32::NAN  // Convert sentinel to NaN
    } else {
        v
    }
}).collect();
```

**Common sentinel values by source**:
| Source | Sentinel | Meaning |
|--------|----------|---------|
| MRMS | -999 | Missing/no coverage |
| MRMS | -99 | Below minimum threshold |
| GFS | 9.999e20 | GRIB2 bitmap miss |

---

### 4. Write to Zarr

Grid data is written to Zarr V3 format with multi-resolution pyramids:

```rust
use grid_processor::{ZarrWriter, GridProcessorConfig, PyramidConfig, DownsampleMethod};

// Configure writer with pyramid support
let config = GridProcessorConfig {
    zarr_chunk_size: 512,
    pyramid: Some(PyramidConfig {
        levels: 2,
        method: DownsampleMethod::Average,
        min_dimension: 256,
    }),
    ..Default::default()
};

let writer = ZarrWriter::new(config);

// Storage path: grids/{model}/{run_date}/{param}_{level}_f{fhr:03}.zarr
let zarr_path = format!(
    "grids/{}/{}/{}_{}_f{:03}.zarr",
    model, run_date, param.to_lowercase(), level_sanitized, forecast_hour
);

// Write array with pyramids
let result = writer.write_with_pyramids(
    filesystem_store,
    &grid_data,
    width, height,
    &bbox,
    model, parameter, level, units,
    reference_time, forecast_hour,
)?;

// Upload to MinIO
upload_zarr_directory(&local_path, &minio_path, &storage).await?;
```

**Zarr Output Structure**:
```
grids/gfs/20241217_12z/tmp_2_m_above_ground_f003.zarr/
├── zarr.json                    # Root metadata
├── 0/                           # Full resolution (1440x721)
│   ├── zarr.json
│   └── c/0/0, c/0/1, ...       # Chunked data (512x512 chunks)
└── 1/                           # 2x downsampled (720x360)
    ├── zarr.json
    └── c/0/0, ...
```

---

### 5. Register in Catalog

Insert metadata into PostgreSQL:

```rust
let entry = CatalogEntry {
    model: model.to_string(),
    parameter: param.to_string(),
    level: level.to_string(),
    reference_time,
    forecast_hour: forecast_hour as i32,
    storage_path: zarr_path.clone(),
    bbox: serde_json::to_value(&bbox)?,
    grid_shape: serde_json::to_value(&[width, height])?,
    zarr_metadata: result.metadata.to_json(),
    created_at: Utc::now(),
};

catalog.insert(&entry).await?;
```

## Configuration

### Environment Variables

```bash
# Server
PORT=8082                              # HTTP server port

# Database
DATABASE_URL=postgresql://weatherwms:password@postgres:5432/weatherwms

# Object Storage (MinIO)
S3_ENDPOINT=http://minio:9000
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_BUCKET=weather-data
S3_REGION=us-east-1
S3_ALLOW_HTTP=true

# Logging
RUST_LOG=info,ingestion=debug
```

### Command-Line Arguments

```bash
ingester --help

USAGE:
    ingester [OPTIONS]

OPTIONS:
    -p, --port <PORT>           HTTP server port [default: 8082]
        --test-file <PATH>      Test with local file (bypasses HTTP server)
        --test-model <MODEL>    Model name for test file
        --forecast-hour <HOUR>  Forecast hour for test file
        --log-level <LEVEL>     Log level [default: info]
    -h, --help                  Print help information
```

### Test File Mode

For development and testing, you can ingest a file directly without the HTTP server:

```bash
# Ingest a local file directly
ingester --test-file /data/downloads/gfs_20241217_12z_f003.grib2 --test-model gfs
```

## Storage Path Format

Path format differs by data type:

### Forecast Models (GFS, HRRR)
```
grids/{model}/{date}_{HH}z/{param}_{level}_f{fhr:03}.zarr
```

**Examples**:
```
grids/gfs/20241217_12z/tmp_2_m_above_ground_f003.zarr
grids/hrrr/20241217_18z/refc_entire_atmosphere_f001.zarr
```

### Observation Models (MRMS, GOES)
```
grids/{model}/{date}_{HH}{MM}z/{param}_{level}_f000.zarr
```

The `{MM}` component stores the minute of observation, allowing minute-level temporal resolution:

**Examples**:
```
grids/mrms/20241217_1205z/refl_0_m_above_msl_f000.zarr    # 12:05 UTC observation
grids/mrms/20241217_1207z/refl_0_m_above_msl_f000.zarr    # 12:07 UTC observation
grids/goes18/20241217_1830z/cmi_c13_ir_f000.zarr          # 18:30 UTC scan
```

## Performance

### Throughput

| Format | File Size | Grid Size | Parameters | Time |
|--------|-----------|-----------|------------|------|
| GRIB2 (GFS) | 550 MB | 1440x721 | ~50 | ~30s |
| GRIB2 (HRRR) | 250 MB | 1799x1059 | ~20 | ~15s |
| GRIB2 (MRMS) | 3 MB | 3500x7000 | 1 | ~0.5s |
| NetCDF (GOES) | 50 MB | 5424x5424 | 1 | ~8s |

### Resource Usage

- **CPU**: High during parsing and pyramid generation (multi-threaded)
- **Memory**: 2-4 GB typical (grid held in memory during processing)
- **Disk I/O**: High writes to local temp, then MinIO uploads
- **Network**: High uploads to MinIO

### Storage Efficiency

Zarr with Blosc LZ4 compression:
- GFS TMP grid (1440x721 f32): ~1.8 MB (from 4.1 MB raw)
- With 2 pyramid levels: ~2.5 MB total
- Compression ratio: ~2:1 typical for weather data

## Troubleshooting

### Parse Errors

**Symptom**: "Failed to parse GRIB2 message"

**Causes**:
- Corrupted download
- Unsupported compression (PNG with predictor, etc.)
- Invalid format

**Solution**:
```bash
# Validate GRIB2 file
wgrib2 /data/downloads/gfs_20241217_12z_f000.grib2 | head

# Re-download file
docker compose restart downloader
```

---

### Out of Memory

**Symptom**: Container killed (OOM)

**Causes**:
- Very large NetCDF files (GOES full-disk)
- Many parameters processed simultaneously

**Solution**:
```yaml
# docker-compose.yml
services:
  ingester:
    mem_limit: 8g
    mem_reservation: 4g
```

---

### Slow Ingestion

**Symptom**: Takes >1 minute per file

**Causes**:
- Slow MinIO uploads
- Large pyramid generation
- Too many pressure levels

**Solution**:
```bash
# Check MinIO performance
docker compose logs minio

# Reduce pyramid levels
ZARR_PYRAMID_LEVELS=1

# Use faster storage for MinIO
# Mount SSD volume in docker-compose.yml
```

---

### Missing Parameters

**Symptom**: Parameter in GRIB2 not appearing in catalog

**Causes**:
- Not in `target_params` list in `crates/ingestion/src/config.rs`
- Level type mismatch
- Pressure level not in `pressure_levels` set

**Solution**:
1. Check `crates/ingestion/src/config.rs` for `should_ingest_parameter()` configuration
2. Verify level type codes match GRIB2 table 4.5
3. Add pressure level if needed

## Monitoring

### Logs

Structured JSON logs to stdout:

```json
{
  "timestamp": "2024-12-17T19:53:36Z",
  "level": "INFO",
  "target": "ingester::server",
  "message": "Received ingest request",
  "id": "abc123-...",
  "file_path": "/data/downloads/gfs_20241217_12z_f003.grib2"
}
```

```json
{
  "timestamp": "2024-12-17T19:53:45Z",
  "level": "INFO",
  "target": "ingestion::grib2",
  "message": "GRIB2 ingestion complete",
  "model": "gfs",
  "datasets": 47,
  "parameters": "[\"TMP\", \"UGRD\", \"VGRD\", ...]"
}
```

### Metrics

Track ingestion via the `/status` endpoint or database queries:

```sql
-- Recent ingestions
SELECT model, parameter, level, reference_time, created_at
FROM datasets
ORDER BY created_at DESC
LIMIT 20;

-- Ingestion counts by model
SELECT model, COUNT(*) as count, MAX(created_at) as last_ingestion
FROM datasets
GROUP BY model;

-- Storage usage by model
SELECT model, 
       COUNT(*) as datasets,
       COUNT(DISTINCT parameter) as parameters
FROM datasets
GROUP BY model;
```

### Admin Dashboard

The web dashboard at `http://localhost:8000/admin.html` shows:
- Active ingestions in progress (via wms-api proxy to ingester)
- Recent ingestion history
- Parameter counts per model
- Storage tree visualization

## Code Structure

```
services/ingester/src/
├── main.rs              # Entry point, HTTP server or test-file mode
└── server.rs            # HTTP endpoints (/ingest, /status, /health)

crates/ingestion/src/
├── lib.rs               # Public API exports
├── ingester.rs          # Core Ingester struct
├── grib2.rs             # GRIB2 file ingestion
├── netcdf.rs            # NetCDF file ingestion  
├── metadata.rs          # File type detection, model/param extraction
├── config.rs            # Parameter filtering rules
├── upload.rs            # Upload Zarr to MinIO
└── error.rs             # Error types
```

## Dependencies

Key Rust crates used:

- **axum**: Web framework
- **tokio**: Async runtime
- **grib2-parser**: GRIB2 parsing
- **netcdf-parser**: NetCDF parsing
- **grid-processor**: Zarr writing with pyramids
- **storage**: MinIO/PostgreSQL access
- **tracing**: Structured logging

## Next Steps

- [grid-processor](../crates/grid-processor.md) - Zarr reading/writing details
- [GRIB2 Parser](../crates/grib2-parser.md) - GRIB2 format details
- [NetCDF Parser](../crates/netcdf-parser.md) - NetCDF format details
- [Data Sources](../data-sources/README.md) - Supported weather data sources
- [GFS Configuration](../data-sources/gfs.md) - GFS-specific parameters and levels
