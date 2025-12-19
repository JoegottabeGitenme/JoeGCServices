# Ingester Service

The Ingester service parses weather data files (GRIB2 and NetCDF), extracts parameters, and writes them to Zarr V3 format with multi-resolution pyramids for efficient tile rendering.

## Overview

**Location**: `services/wms-api/src/admin.rs` (integrated into WMS API)  
**Language**: Rust  
**Port**: None (HTTP endpoint on WMS API: `POST /admin/ingest`)  
**Scaling**: Vertical (CPU/Memory intensive)

## Responsibilities

1. **File Parsing**: Reads GRIB2 and NetCDF-4 files
2. **Parameter Extraction**: Extracts weather parameters from files based on configuration
3. **Zarr Conversion**: Writes grid data as Zarr V3 arrays with sharding
4. **Pyramid Generation**: Creates multi-resolution pyramids for fast rendering
5. **Cataloging**: Registers metadata in PostgreSQL
6. **Cache Invalidation**: Triggers cache warming for new data

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

## Architecture

```mermaid
graph TB
    Input["Data File
    GRIB2 or NetCDF"]
    
    subgraph Parsing
        Parse["File Parser"]
        Extract["Extract Messages"]
        Decode["Decode Grids"]
    end
    
    subgraph Processing
        Zarr["Write Zarr Array"]
        Pyramid["Generate Pyramids"]
    end
    
    subgraph StorageLayer["Storage"]
        Upload["Upload to MinIO"]
        Catalog["Register in PostgreSQL"]
        Warm["Trigger Cache Warming"]
    end
    
    Input --> Parse
    Parse --> Extract
    Extract --> Decode
    Decode --> Zarr
    Zarr --> Pyramid
    Pyramid --> Upload
    Upload --> Catalog
    Catalog --> Warm
```

## Ingestion Flow

### 1. File Detection

Ingestion is triggered when the Downloader service completes a download:

```rust
// Downloader triggers after successful download
POST http://wms-api:8080/admin/ingest
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

Only configured parameters are ingested. The filter is defined in `admin.rs`:

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
    
    // Moisture
    ("RH", vec![(103, Some(2)), (100, None)]),       // 2m + pressure levels
    ("PWAT", vec![(200, None)]),                     // Precipitable water
    
    // Geopotential
    ("HGT", vec![(100, None)]),                      // Pressure levels
    
    // Precipitation
    ("APCP", vec![(1, None)]),                       // Total precipitation
    
    // Convective
    ("CAPE", vec![(1, None), (180, None)]),          // Surface CAPE
    ("CIN", vec![(1, None), (180, None)]),           // Surface CIN
    
    // Clouds
    ("TCDC", vec![(200, None), (10, None)]),         // Total cloud cover
    ("LCDC", vec![(212, None)]),                     // Low cloud
    ("MCDC", vec![(222, None)]),                     // Middle cloud
    ("HCDC", vec![(232, None)]),                     // High cloud
    
    // Visibility
    ("VIS", vec![(1, None)]),                        // Surface visibility
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

This ensures:
- Proper rendering (NaN pixels are transparent)
- Correct pyramid generation (NaN propagates correctly through downsampling)
- Standard missing data representation

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

// Copy to MinIO
copy_dir_to_minio(&local_path, &minio_path, &storage).await?;
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

**Catalog Schema**:
```sql
CREATE TABLE grid_catalog (
    id SERIAL PRIMARY KEY,
    model VARCHAR(50) NOT NULL,
    parameter VARCHAR(50) NOT NULL,
    level VARCHAR(100) NOT NULL,
    reference_time TIMESTAMPTZ NOT NULL,
    forecast_hour INTEGER NOT NULL,
    storage_path VARCHAR(500) NOT NULL,
    bbox JSONB NOT NULL,
    grid_shape JSONB NOT NULL,
    zarr_metadata JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    
    UNIQUE(model, parameter, level, reference_time, forecast_hour)
);
```

---

### 6. Cache Warming

After ingestion, the grid warmer pre-caches popular tiles:

```rust
// Notify warmer of new data
if let Some(warmer) = &state.grid_warmer {
    warmer.warm_grid(
        &model,
        &param,
        &level,
        reference_time,
        forecast_hour,
    ).await;
}
```

## Configuration

### Environment Variables

```bash
# Database
DATABASE_URL=postgresql://weatherwms:password@postgres:5432/weatherwms

# Object Storage (MinIO)
MINIO_ENDPOINT=http://minio:9000
MINIO_ACCESS_KEY=minioadmin
MINIO_SECRET_KEY=minioadmin
MINIO_BUCKET=weather-data

# Zarr settings
ZARR_CHUNK_SIZE=512
ZARR_PYRAMID_LEVELS=2

# Logging
RUST_LOG=info,wms_api::admin=debug
```

### Model Configuration

Model configurations in `config/models/*.yaml` define which parameters to ingest:

```yaml
# config/models/gfs.yaml
model:
  id: gfs
  name: "GFS - Global Forecast System"
  enabled: true

parameters:
  - name: TMP
    description: "Temperature"
    levels:
      - type: height_above_ground
        value: 2
        display: "2 m above ground"
      - type: isobaric
        values: [1000, 925, 850, 700, 500, 300, 250, 200, 100, 70, 50, 30, 20, 10]
    style: temperature
    units: K
    
  - name: UGRD
    description: "U-Component of Wind"
    levels:
      - type: height_above_ground
        value: 10
        display: "10 m above ground"
      - type: isobaric
        values: [1000, 925, 850, 700, 500, 300, 250, 200, 100]
    style: wind
    units: m/s
```

## Storage Path Format

Path format differs by data type:

### Forecast Models (GFS, HRRR)
```
grids/{model}/{date}/{HH}/{param}_f{fhr:03}.zarr
```

**Examples**:
```
grids/gfs/2024-12-17/12/tmp_f003.zarr
grids/gfs/2024-12-17/12/ugrd_f003.zarr
grids/hrrr/2024-12-17/18/refl_f001.zarr
```

### Observation Models (MRMS, GOES)
```
grids/{model}/{date}/{HH}/{param}_{MM}.zarr
```

The `{MM}` component stores the minute of observation, allowing minute-level temporal resolution:

**Examples**:
```
grids/mrms/2024-12-17/12/REFL_05.zarr    # 12:05 UTC observation
grids/mrms/2024-12-17/12/REFL_07.zarr    # 12:07 UTC observation
grids/goes18/2024-12-17/18/CMI_C13_30.zarr  # 18:30 UTC scan
```

This enables MRMS to store 2-minute update frequency data without overwriting previous observations within the same hour.

## Performance

### Throughput

| Format | File Size | Grid Size | Parameters | Time |
|--------|-----------|-----------|------------|------|
| GRIB2 (GFS) | 550 MB | 1440×721 | ~50 | ~30s |
| GRIB2 (HRRR) | 250 MB | 1799×1059 | ~20 | ~15s |
| NetCDF (GOES) | 50 MB | 5424×5424 | 1 | ~8s |

### Resource Usage

- **CPU**: High during parsing and pyramid generation (multi-threaded)
- **Memory**: 2-4 GB typical (grid held in memory during processing)
- **Disk I/O**: High writes to local temp, then MinIO uploads
- **Network**: High uploads to MinIO

### Storage Efficiency

Zarr with Blosc LZ4 compression:
- GFS TMP grid (1440×721 f32): ~1.8 MB (from 4.1 MB raw)
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
  wms-api:
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
- Not in `target_params` list
- Level type mismatch
- Pressure level not in `pressure_levels` set

**Solution**:
1. Check `admin.rs` `target_params` configuration
2. Verify level type codes match GRIB2 table 4.5
3. Add pressure level if needed

## Monitoring

### Logs

Structured JSON logs to stdout:

```json
{
  "timestamp": "2024-12-17T19:53:36Z",
  "level": "INFO",
  "target": "wms_api::admin",
  "message": "Admin: Ingesting file",
  "file_path": "/data/downloads/gfs_20241217_12z_f003.grib2",
  "model": "gfs"
}
```

```json
{
  "timestamp": "2024-12-17T19:53:45Z",
  "level": "INFO",
  "message": "Ingestion complete",
  "model": "gfs",
  "datasets_registered": 47,
  "duration_ms": 8823
}
```

### Metrics

Track ingestion via database queries:

```sql
-- Recent ingestions
SELECT model, parameter, level, reference_time, created_at
FROM grid_catalog
ORDER BY created_at DESC
LIMIT 20;

-- Ingestion counts by model
SELECT model, COUNT(*) as count, MAX(created_at) as last_ingestion
FROM grid_catalog
GROUP BY model;

-- Storage usage by model
SELECT model, 
       COUNT(*) as datasets,
       COUNT(DISTINCT parameter) as parameters
FROM grid_catalog
GROUP BY model;
```

### Admin Dashboard

The web dashboard at `http://localhost:8080/admin.html` shows:
- Active ingestions in progress
- Recent ingestion history
- Parameter counts per model
- Storage tree visualization

## API Endpoints

### Trigger Ingestion

```bash
POST /admin/ingest
Content-Type: application/json

{
  "file_path": "/data/downloads/gfs_20241217_12z_f003.grib2",
  "model": "gfs",
  "source_url": "https://...",
  "forecast_hour": 3
}

Response:
{
  "success": true,
  "message": "Ingested 47 datasets",
  "datasets_registered": 47,
  "model": "gfs",
  "reference_time": "2024-12-17T12:00:00Z",
  "parameters": ["TMP", "UGRD", "VGRD", ...]
}
```

### Check Ingestion Status

```bash
GET /api/admin/ingestion/active

Response:
{
  "active_ingestions": [
    {
      "id": "abc123",
      "file": "gfs_20241217_12z_f006.grib2",
      "model": "gfs",
      "status": "shredding",
      "progress": 45,
      "started_at": "2024-12-17T19:54:00Z"
    }
  ]
}
```

## Next Steps

- [grid-processor](../crates/grid-processor.md) - Zarr reading/writing details
- [GRIB2 Parser](../crates/grib2-parser.md) - GRIB2 format details
- [NetCDF Parser](../crates/netcdf-parser.md) - NetCDF format details
- [Data Sources](../data-sources/README.md) - Supported weather data sources
- [GFS Configuration](../data-sources/gfs.md) - GFS-specific parameters and levels
