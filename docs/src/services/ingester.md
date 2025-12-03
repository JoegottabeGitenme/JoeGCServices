# Ingester Service

The Ingester service parses weather data files (GRIB2 and NetCDF), extracts parameters, shreds grid data into manageable chunks, and stores them in object storage with metadata registration in PostgreSQL.

## Overview

**Location**: `services/ingester/`  
**Language**: Rust  
**Port**: None (library called by WMS API)  
**Scaling**: Vertical (CPU/Memory intensive)

## Responsibilities

1. **File Parsing**: Reads GRIB2 and NetCDF-4 files
2. **Parameter Extraction**: Extracts weather parameters from files
3. **Data Shredding**: Splits large grids into ~1MB chunks
4. **Storage**: Uploads shreds to MinIO/S3
5. **Cataloging**: Registers metadata in PostgreSQL
6. **Cache Invalidation**: Notifies WMS API instances of new data

## Supported Formats

### GRIB2 (GRIB Edition 2)

**Used by**: GFS, HRRR, MRMS

**Features**:
- Binary grid format
- Multiple messages per file
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
    Input[Data File<br/>GRIB2 or NetCDF]
    
    subgraph "Parsing"
        Parse[File Parser]
        Extract[Extract Messages]
        Decode[Decode Grids]
    end
    
    subgraph "Processing"
        Shard[Grid Shredding]
        Compress[Optional Compression]
    end
    
    subgraph "Storage"
        Upload[Upload to MinIO]
        Catalog[Register in PostgreSQL]
        Invalidate[Invalidate Caches]
    end
    
    Input --> Parse
    Parse --> Extract
    Extract --> Decode
    Decode --> Shard
    Shard --> Compress
    Compress --> Upload
    Upload --> Catalog
    Catalog --> Invalidate
```

## Ingestion Flow

### 1. File Detection

Ingestion can be triggered in three ways:

**a) Automatic (Download Complete)**:
```rust
// Downloader triggers after successful download
POST http://wms-api:8080/admin/ingest
{
  "path": "/data/incoming/gfs.t00z.pgrb2.0p25.f000",
  "model": "gfs"
}
```

**b) Manual (CLI)**:
```bash
# Ingest specific file
docker-compose exec ingester ingester \
  --test-file /data/incoming/gfs.t00z.pgrb2.0p25.f000 \
  --test-model gfs \
  --forecast-hour 0
```

**c) Scheduled (Cron)**:
```bash
# Cron job to ingest all files in directory
0 * * * * /app/scripts/ingest_test_data.sh
```

---

### 2. Parse File

**GRIB2 Parsing**:
```rust
let bytes = tokio::fs::read(&path).await?;
let grib = Grib2Parser::new(&bytes)?;

for message in grib.messages() {
    let section0 = message.section0()?;  // Indicator
    let section1 = message.section1()?;  // Identification
    let section3 = message.section3()?;  // Grid definition
    let section4 = message.section4()?;  // Product definition
    let section5 = message.section5()?;  // Data representation
    let section6 = message.section6()?;  // Bitmap
    let section7 = message.section7()?;  // Data
    
    // Extract parameter info
    let param = ParameterInfo {
        discipline: section4.discipline,
        category: section4.category,
        number: section4.number,
        level_type: section4.level_type,
        level_value: section4.level_value,
    };
    
    // Decode grid values
    let grid_data = decode_data_section(section5, section7)?;
}
```

**NetCDF Parsing**:
```rust
let file = netcdf::open(&path)?;

let cmi_variable = file.variable("CMI")?;  // Cloud and Moisture Imagery
let x_coord = file.variable("x")?;
let y_coord = file.variable("y")?;

let data: Array2<f32> = cmi_variable.get()?;

// Extract geostationary projection
let proj_var = file.variable("goes_imager_projection")?;
let sat_height = proj_var.attribute("perspective_point_height")?;
let lon_origin = proj_var.attribute("longitude_of_projection_origin")?;
```

---

### 3. Extract Parameters

For GRIB2, each message contains one parameter. For NetCDF, each variable is a parameter.

**Parameter Identification**:
```rust
// GRIB2: Use discipline/category/number lookup
let parameter = match (discipline, category, number) {
    (0, 0, 0) => "TMP",      // Temperature
    (0, 2, 2) => "UGRD",     // U-component wind
    (0, 2, 3) => "VGRD",     // V-component wind
    (0, 1, 1) => "RH",       // Relative humidity
    _ => return Err(UnknownParameter),
};

// NetCDF: Use variable name
let parameter = match variable_name {
    "CMI" => format!("CMI_C{:02}", channel),  // e.g., CMI_C13
    "Rad" => "RAD",
    _ => return Err(UnknownParameter),
};
```

**Level Identification**:
```rust
let level = match level_type {
    1 => "surface",
    103 => format!("{}m", level_value),  // e.g., "2m", "10m"
    100 => format!("{}mb", level_value), // e.g., "500mb"
    _ => "unknown",
};
```

---

### 4. Shard Grid Data

Large grids are split into ~1MB chunks for efficient storage and retrieval.

**Shredding Algorithm**:
```rust
const TARGET_SHARD_SIZE: usize = 1_000_000;  // 1 MB

fn shard_grid(grid: &Array2<f32>) -> Vec<Shard> {
    let (ny, nx) = grid.dim();
    let bytes_per_cell = std::mem::size_of::<f32>();
    
    // Calculate cells per shard
    let cells_per_shard = TARGET_SHARD_SIZE / (bytes_per_cell * ny);
    let num_shards = (nx + cells_per_shard - 1) / cells_per_shard;
    
    let mut shards = Vec::new();
    
    for i in 0..num_shards {
        let x_start = i * cells_per_shard;
        let x_end = usize::min((i + 1) * cells_per_shard, nx);
        
        // Extract column slice
        let shard_data = grid.slice(s![.., x_start..x_end]).to_owned();
        
        // Calculate bounds for this shard
        let (west, south, east, north) = calculate_shard_bounds(
            &projection, x_start, x_end, 0, ny
        );
        
        shards.push(Shard {
            index: i,
            data: shard_data,
            bounds: BoundingBox { west, south, east, north },
        });
    }
    
    shards
}
```

**Example: GFS Global Grid**
- Grid size: 1440 × 721 (lon × lat)
- Total cells: 1,038,240
- Cell size: 4 bytes (f32)
- Total size: ~4 MB
- Shard size: ~1 MB
- **Result: 4 shards** (360 columns each)

---

### 5. Store Shards

Upload shards to MinIO in parallel:

```rust
async fn store_shards(
    storage: &ObjectStorage,
    shards: Vec<Shard>,
    base_path: &str,
) -> Result<()> {
    // Upload shards in parallel
    let handles: Vec<_> = shards.into_iter().enumerate().map(|(i, shard)| {
        let storage = storage.clone();
        let path = format!("{}_shard_{:04}.bin", base_path, i);
        
        tokio::spawn(async move {
            // Serialize shard to binary
            let mut buffer = Vec::new();
            write_shard_header(&mut buffer, &shard)?;
            buffer.extend_from_slice(bytemuck::cast_slice(shard.data.as_slice().unwrap()));
            
            // Upload to MinIO
            storage.put_object(&path, buffer).await?;
            
            Ok::<_, Error>(path)
        })
    }).collect();
    
    // Wait for all uploads
    for handle in handles {
        handle.await??;
    }
    
    Ok(())
}
```

**Storage Path Format**:
```
s3://weather-data/{model}/{parameter}/{YYYYMMDDHH}_f{HHH}_shard_{NNNN}.bin

Examples:
s3://weather-data/gfs/TMP_2m/2024120300_f000_shard_0000.bin
s3://weather-data/gfs/TMP_2m/2024120300_f000_shard_0001.bin
s3://weather-data/hrrr/REFL/2024120315_f001_shard_0000.bin
s3://weather-data/goes18/CMI_C13/2024120318_f000_shard_0000.bin
```

---

### 6. Register in Catalog

Insert metadata into PostgreSQL:

```rust
async fn register_in_catalog(
    catalog: &Catalog,
    metadata: &GridMetadata,
) -> Result<i64> {
    let grid_id = sqlx::query_scalar!(
        r#"
        INSERT INTO grid_catalog (
            model, parameter, level_type, level_value,
            forecast_time, valid_time, forecast_hour,
            projection, grid_shape, bbox,
            storage_path, shard_count, file_size,
            created_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7,
            $8, $9, $10, $11, $12, $13, NOW()
        )
        RETURNING id
        "#,
        metadata.model,
        metadata.parameter,
        metadata.level_type,
        metadata.level_value,
        metadata.forecast_time,
        metadata.valid_time,
        metadata.forecast_hour as i32,
        serde_json::to_value(&metadata.projection)?,
        serde_json::to_value(&metadata.grid_shape)?,
        serde_json::to_value(&metadata.bbox)?,
        metadata.storage_path,
        metadata.shard_count as i32,
        metadata.file_size as i64,
    )
    .fetch_one(&catalog.pool)
    .await?;
    
    Ok(grid_id)
}
```

---

### 7. Invalidate Caches

Notify WMS API instances via Redis Pub/Sub:

```rust
async fn invalidate_caches(
    redis: &RedisClient,
    model: &str,
    parameter: &str,
    forecast_time: DateTime<Utc>,
) -> Result<()> {
    let message = serde_json::json!({
        "model": model,
        "parameter": parameter,
        "forecast_time": forecast_time.to_rfc3339(),
    });
    
    redis.publish("cache:invalidate", message.to_string()).await?;
    
    info!(
        model = model,
        parameter = parameter,
        "Published cache invalidation message"
    );
    
    Ok(())
}
```

## Configuration

### Environment Variables

```bash
# Database
DATABASE_URL=postgresql://weatherwms:password@postgres:5432/weatherwms
DATABASE_POOL_SIZE=10

# Object Storage
S3_ENDPOINT=http://minio:9000
S3_BUCKET=weather-data
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin

# Redis (for cache invalidation)
REDIS_URL=redis://redis:6379

# Logging
RUST_LOG=info
```

### Configuration Files

Model configurations in `config/models/*.yaml`:

```yaml
# config/models/gfs.yaml
name: gfs
description: Global Forecast System
format: grib2
projection: latlon
resolution: 0.25  # degrees
update_frequency: 6h
forecast_hours: [0, 3, 6, 9, ..., 384]
parameters:
  - TMP_2m
  - UGRD_10m
  - VGRD_10m
  - RH_2m
  - PRMSL
```

### Command-Line Arguments

```bash
ingester --help

USAGE:
    ingester [OPTIONS]

OPTIONS:
    -c, --config <PATH>           Configuration file [default: /etc/ingester/config.yaml]
        --once                    Run once and exit
    -m, --model <MODEL>           Specific model to ingest
        --test-file <PATH>        Test with local file
        --test-model <MODEL>      Model name for test file
        --forecast-hour <HOUR>    Forecast hour for test file
        --log-level <LEVEL>       Log level [default: info]
    -h, --help                    Print help
```

## Performance

### Throughput

| Format | File Size | Grid Size | Shards | Time |
|--------|-----------|-----------|--------|------|
| GRIB2 (GFS) | 100 MB | 1440×721 | 4 | ~10s |
| GRIB2 (HRRR) | 250 MB | 1799×1059 | 8 | ~25s |
| NetCDF (GOES) | 50 MB | 5424×5424 | 120 | ~15s |

### Resource Usage

- **CPU**: High during parsing and shredding (multi-threaded)
- **Memory**: 4-8 GB (large grids held in memory)
- **Disk I/O**: High reads (input files), high writes (shards to MinIO)
- **Network**: High uploads to MinIO

### Optimization

```bash
# Increase memory for large files
docker-compose up -d --scale ingester=1 --no-recreate
docker-compose exec ingester sh -c 'export RUST_MIN_STACK=8388608'

# Use SSD for temp storage
TEMP_DIR=/mnt/ssd/temp
```

## Troubleshooting

### Parse Errors

**Symptom**: "Failed to parse GRIB2 message"

**Causes**:
- Corrupted download
- Unsupported compression
- Invalid format

**Solution**:
```bash
# Validate GRIB2 file
wgrib2 /data/incoming/gfs.t00z.pgrb2.0p25.f000

# Re-download file
./scripts/download_gfs.sh
```

---

### Out of Memory

**Symptom**: Container killed (OOM)

**Causes**:
- Very large NetCDF files
- Insufficient container memory

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
- Network congestion
- Large shard count

**Solution**:
```bash
# Check MinIO performance
docker-compose logs minio

# Increase shard size (fewer shards)
TARGET_SHARD_SIZE=5_000_000  # 5 MB

# Use faster storage for MinIO
# Mount SSD volume in docker-compose.yml
```

## Monitoring

### Logs

Structured JSON logs to stdout:

```json
{
  "timestamp": "2024-12-03T18:30:45Z",
  "level": "INFO",
  "target": "ingester::ingest",
  "message": "Ingesting file",
  "path": "/data/incoming/gfs.t00z.pgrb2.0p25.f000",
  "model": "gfs"
}
```

```json
{
  "timestamp": "2024-12-03T18:30:55Z",
  "level": "INFO",
  "target": "ingester::ingest",
  "message": "Ingestion complete",
  "model": "gfs",
  "parameter": "TMP_2m",
  "shards": 4,
  "duration_ms": 9823
}
```

### Metrics

Track ingestion via database queries:

```sql
-- Recent ingestions
SELECT model, parameter, forecast_time, created_at
FROM grid_catalog
ORDER BY created_at DESC
LIMIT 10;

-- Ingestion counts by model
SELECT model, COUNT(*) as count, MAX(created_at) as last_ingestion
FROM grid_catalog
GROUP BY model;
```

## Next Steps

- [GRIB2 Parser](../crates/grib2-parser.md) - GRIB2 format details
- [NetCDF Parser](../crates/netcdf-parser.md) - NetCDF format details
- [Data Sources](../data-sources/README.md) - Supported weather data sources
