# Weather WMS - Data Ingestion Guide

## Overview

The Weather WMS ingestion system downloads weather data from public sources (AWS S3), parses GRIB2 and NetCDF files, extracts individual parameters, and stores them in object storage with metadata in PostgreSQL.

**Key Features:**
- **YAML-based configuration** - All models, parameters, and schedules defined in `config/` directory
- **Parameter shredding** - Extracts individual parameters from multi-parameter files
- **Multiple data sources** - GFS, HRRR, GOES-16/18, MRMS
- **Admin dashboard** - Web UI for monitoring and configuration at http://localhost:8000/admin.html

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        DATA SOURCES (AWS S3)                     │
│   GFS (GRIB2)   HRRR (GRIB2)   GOES (NetCDF)   MRMS (GRIB2)    │
└────────────────────────┬────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                    INGESTER SERVICE (Rust)                       │
│  ┌──────────┐  ┌──────────┐  ┌───────────────────────────┐     │
│  │ Download │→ │  Parse   │→ │  Shred (Extract Params)   │     │
│  │  Files   │  │GRIB2/NC  │  │                           │     │
│  └──────────┘  └──────────┘  └───────────────────────────┘     │
└────────────────────────┬────────────────────────────────────────┘
                         │
         ┌───────────────┼───────────────┐
         ▼               ▼               ▼
  ┌───────────┐   ┌──────────┐   ┌──────────┐
  │PostgreSQL │   │  MinIO   │   │  Redis   │
  │ (Catalog) │   │(Storage) │   │ (Cache)  │
  └───────────┘   └──────────┘   └──────────┘
```

---

## Configuration Files

### Directory Structure

```
config/
├── models/                   # Model-specific configurations
│   ├── gfs.yaml             # GFS model (global forecast)
│   ├── hrrr.yaml            # HRRR model (CONUS high-res)
│   ├── goes16.yaml          # GOES-16 satellite (eastern)
│   ├── goes18.yaml          # GOES-18 satellite (western)
│   └── mrms.yaml            # MRMS radar composite
├── parameters/               # Parameter definition tables
│   ├── grib2_wmo.yaml       # WMO standard parameters (109 params)
│   ├── grib2_ncep.yaml      # NCEP local parameters (73 params)
│   └── grib2_mrms.yaml      # MRMS local parameters (68 params)
├── styles/                   # Rendering styles (JSON)
│   ├── temperature.json
│   ├── wind.json
│   ├── precipitation.json
│   └── reflectivity.json
└── ingestion.yaml           # Global ingestion settings
```

### Model Configuration Format

Each model has a YAML file defining its data source, parameters to extract, and schedule.

**Example: `config/models/gfs.yaml`** (abbreviated)

```yaml
model:
  id: gfs
  name: "GFS - Global Forecast System"
  type: forecast
  description: "NCEP Global Forecast System, 0.25° resolution"

source:
  type: aws_s3_grib2
  bucket: noaa-gfs-bdp-pds
  prefix_template: "gfs.{date}/{cycle:02}/atmos"
  file_pattern: "gfs.t{cycle:02}z.pgrb2.{resolution}.f{forecast:03}"
  region: us-east-1

grid:
  projection: geographic
  resolution: "0.25deg"
  bbox:
    min_lon: 0.0
    min_lat: -90.0
    max_lon: 360.0
    max_lat: 90.0

schedule:
  cycles: [0, 6, 12, 18]
  forecast_hours:
    start: 0
    end: 120
    step: 3
  poll_interval_secs: 3600

parameters:
  - name: TMP
    description: "Temperature"
    levels:
      - type: height_above_ground
        value: 2
        display: "2 m above ground"
    style: temperature
    units: K

  - name: UGRD
    description: "U-Component of Wind"
    levels:
      - type: height_above_ground
        value: 10
        display: "10 m above ground"
    style: wind
    units: m/s

  - name: VGRD
    description: "V-Component of Wind"
    levels:
      - type: height_above_ground
        value: 10
        display: "10 m above ground"
    style: wind
    units: m/s

  - name: PRMSL
    description: "Pressure Reduced to MSL"
    levels:
      - type: mean_sea_level
        display: "mean sea level"
    style: atmospheric
    units: Pa
```

**Key Sections:**

- **`model`**: Metadata (id, name, type, description)
- **`source`**: Where to download data (S3 bucket, path templates)
- **`grid`**: Spatial info (projection, resolution, bounding box)
- **`schedule`**: When to poll for new data (cycles, forecast hours, poll interval)
- **`parameters`**: Which parameters to extract and at what levels

### Parameter Tables

Parameter tables map GRIB2 codes to human-readable names.

**Example: `config/parameters/grib2_wmo.yaml`** (abbreviated)

```yaml
# WMO GRIB2 Code Table 4.2
# Discipline 0 = Meteorological Products

discipline_0:
  category_0:  # Temperature
    - number: 0
      name: TMP
      description: Temperature
      units: K

  category_2:  # Momentum
    - number: 2
      name: UGRD
      description: U-Component of Wind
      units: m/s
    - number: 3
      name: VGRD
      description: V-Component of Wind
      units: m/s

level_types:
  1:
    name: surface
    description: Ground or Water Surface
  100:
    name: isobaric
    description: Isobaric Surface
    units: Pa
  103:
    name: height_above_ground
    description: Specified Height Level Above Ground
    units: m
```

### Global Ingestion Settings

**`config/ingestion.yaml`**:

```yaml
ingestion:
  enabled_models:
    - gfs
    - hrrr
    - goes16
    - mrms

  parallel_downloads: 4
  max_retries: 3
  retry_delay_secs: 30

  storage:
    type: s3
    endpoint: "${S3_ENDPOINT}"
    bucket: "${S3_BUCKET}"
    access_key: "${S3_ACCESS_KEY}"
    secret_key: "${S3_SECRET_KEY}"

  paths:
    raw: "raw/{model}/{date}/{cycle:02}/{filename}"
    shredded: "shredded/{model}/{run_time}/{param}_{level}/f{forecast:03}.grib2"

  database:
    url: "${DATABASE_URL}"
```

**Environment variables** (from `.env`):
- `S3_ENDPOINT` - MinIO/S3 endpoint
- `S3_BUCKET` - Storage bucket name
- `S3_ACCESS_KEY` - Access key
- `S3_SECRET_KEY` - Secret key
- `DATABASE_URL` - PostgreSQL connection string

---

## Data Models

### Supported Models

| Model | Source | Format | Update Frequency | Coverage |
|-------|--------|--------|-----------------|----------|
| **GFS** | NOAA/NCEP | GRIB2 | Every 6 hours | Global |
| **HRRR** | NOAA/NCEP | GRIB2 | Hourly | CONUS |
| **GOES-16** | NOAA/NESDIS | NetCDF | Every 5-15 min | Eastern Americas |
| **GOES-18** | NOAA/NESDIS | NetCDF | Every 5-15 min | Western Americas |
| **MRMS** | NOAA/NSSL | GRIB2 | Every 2 minutes | CONUS |

### GFS (Global Forecast System)

```
Source:    s3://noaa-gfs-bdp-pds/gfs.{date}/{cycle}/atmos/
Format:    GRIB2 (lat/lon grid, 0.25° resolution)
Coverage:  Global (0°-360° lon, -90°-90° lat)
Cycles:    00, 06, 12, 18 UTC
Forecasts: 0-120 hours (3-hour intervals)
Parameters:
  - TMP (2m above ground)
  - UGRD/VGRD (10m above ground)
  - PRMSL (mean sea level)
Storage:   shredded/gfs/{run_date}/{param}_{level}/f{fhr}.grib2
```

### HRRR (High-Resolution Rapid Refresh)

```
Source:    s3://noaa-hrrr-bdp-pds/hrrr.{date}/conus/
Format:    GRIB2 (Lambert Conformal, 3km resolution)
Coverage:  CONUS (-122.72° to -60.92° lon, 21.14° to 47.84° lat)
Cycles:    00-23 UTC (hourly)
Forecasts: 0-18 hours
Parameters:
  - TMP (2m above ground)
  - UGRD/VGRD (10m above ground)
  - REFC (composite reflectivity)
Storage:   shredded/hrrr/{run_date}/{param}_{level}/f{fhr}.grib2
```

### GOES-16/18 (Geostationary Satellites)

```
Source:    s3://noaa-goes16/ABI-L2-CMIPC/{year}/{doy}/{hour}/
Format:    NetCDF4 (Geostationary projection)
Coverage:  CONUS (GOES-16: -143° to -53° lon, 14.5° to 55.5° lat)
Update:    Every 5-15 minutes
Bands:     C02 (visible), C08 (water vapor), C13 (IR)
Storage:   raw/goes16/{run_date}/cmi_c{band}.nc
```

### MRMS (Multi-Radar Multi-Sensor)

```
Source:    s3://noaa-mrms-pds/CONUS/{product}/{timestamp}/
Format:    GRIB2 (lat/lon grid, ~1km resolution, LOCAL parameter tables)
Coverage:  CONUS (-130° to -60° lon, 20° to 55° lat)
Update:    Every 2 minutes
Products:  MergedReflectivityQComposite, PrecipRate
Storage:   shredded/mrms/{observation_time}/{param}/latest.grib2
```

---

## Ingestion Workflow

### 1. Download

The ingester polls S3 buckets on a schedule:
- **GFS/HRRR**: Poll every hour, check for new cycles
- **GOES**: Poll every 5 minutes, check for new scans
- **MRMS**: Poll every 2 minutes, check for new observations

Files are downloaded to `/tmp` and processed immediately.

### 2. Parse

Files are parsed using format-specific parsers:
- **GRIB2**: `crates/grib2-parser` - Reads sections, decodes grid, extracts metadata
- **NetCDF**: `crates/netcdf-parser` - Reads dimensions, variables, attributes

### 3. Shred (Extract Parameters)

For multi-parameter files (GRIB2), individual parameters are extracted:

**Example: GFS file contains 486 messages**
```
Input:  gfs.t12z.pgrb2.0p25.f006 (486 GRIB2 messages)
Output: 
  - shredded/gfs/20251130_12/TMP_2m/f006.grib2
  - shredded/gfs/20251130_12/UGRD_10m/f006.grib2
  - shredded/gfs/20251130_12/VGRD_10m/f006.grib2
  - shredded/gfs/20251130_12/PRMSL_msl/f006.grib2
```

Only parameters listed in the model config are extracted. Others are ignored.

### 4. Store

Shredded files are uploaded to MinIO/S3 with catalog entries in PostgreSQL:

**Catalog entry** (`datasets` table):
```sql
INSERT INTO datasets (
  model, parameter, level, reference_time, forecast_hour,
  file_size, storage_path, bbox, grid_shape
) VALUES (
  'gfs', 'TMP', '2 m above ground', '2025-11-30 12:00:00',
  6, 523142, 'shredded/gfs/20251130_12/TMP_2m/f006.grib2',
  ST_MakeEnvelope(0, -90, 360, 90, 4326), '1440x721'
);
```

---

## Admin Dashboard

### Overview

The admin dashboard provides a web UI for monitoring ingestion and editing configurations.

**URL**: http://localhost:8000/admin.html

### Features

#### 1. System Status

Displays:
- Service status (Online/Offline)
- CPU cores
- Worker threads
- System uptime

#### 2. Catalog Summary

Shows:
- Total datasets ingested
- Number of active models
- Total storage size
- Latest ingest timestamp

#### 3. Ingestion Log

Real-time log of recent ingestion activity:
```
12:45:32  gfs    TMP    2 m above ground    shredded/gfs/.../f006.grib2
12:45:30  mrms   REFL   surface             shredded/mrms/.../latest.grib2
```

#### 4. Model Configuration

List all models with:
- Model ID, name, type
- Source type (AWS S3, HTTP, etc.)
- Projection
- Parameter count

Click a model to view/edit its YAML configuration.

#### 5. Config Editor

**View Tab**: Display raw YAML
**Edit Tab**: Edit YAML with syntax validation
**Shred Preview Tab**: Preview what parameters will be extracted

**Actions**:
- **Validate**: Check YAML syntax and required fields
- **Save**: Write changes to `config/models/{id}.yaml`
- **Reset**: Discard changes

#### 6. Shred Preview

Shows what parameters will be extracted from a model's files:

```
GFS - Global Forecast System
Source: aws_s3_grib2

Parameters to Extract:
  TMP (Temperature)
    - 2 m above ground → shredded/gfs/{run}/TMP_2m/f{fhr}.grib2
  UGRD (U-Component of Wind)
    - 10 m above ground → shredded/gfs/{run}/UGRD_10m/f{fhr}.grib2
  VGRD (V-Component of Wind)
    - 10 m above ground → shredded/gfs/{run}/VGRD_10m/f{fhr}.grib2
  PRMSL (Pressure Reduced to MSL)
    - mean sea level → shredded/gfs/{run}/PRMSL_msl/f{fhr}.grib2

Total Extractions: 4
```

---

## API Reference

### Admin Endpoints

All admin endpoints are prefixed with `/api/admin/`.

#### `GET /api/admin/ingestion/status`

Get overall ingestion status.

**Response**:
```json
{
  "models": [
    {
      "id": "gfs",
      "name": "GFS Model",
      "status": "active",
      "enabled": true,
      "last_ingest": "2025-11-30 12:45:32 UTC",
      "total_files": 42,
      "parameters": ["TMP", "UGRD", "VGRD", "PRMSL"]
    }
  ],
  "catalog_summary": {
    "total_datasets": 100,
    "total_parameters": 6,
    "total_size_bytes": 31507165,
    "models": [
      {
        "model": "gfs",
        "parameter_count": 4,
        "dataset_count": 40
      }
    ]
  },
  "system_info": {
    "cache_enabled": true,
    "rendering_workers": 12,
    "uptime_seconds": 3600,
    "cpu_cores": 12,
    "worker_threads": 12
  }
}
```

#### `GET /api/admin/ingestion/log`

Get recent ingestion activity.

**Query Parameters**:
- `limit` (optional, default=50, max=500): Number of entries
- `model` (optional): Filter by model ID

**Response**:
```json
{
  "entries": [
    {
      "timestamp": "2025-11-30 12:45:32 UTC",
      "model": "gfs",
      "parameter": "TMP",
      "level": "2 m above ground",
      "reference_time": "2025-11-30 12:00 UTC",
      "forecast_hour": 6,
      "file_size": 523142,
      "storage_path": "shredded/gfs/20251130_12/TMP_2m/f006.grib2"
    }
  ],
  "total_count": 1
}
```

#### `GET /api/admin/preview-shred?model={id}`

Preview what parameters will be extracted for a model.

**Query Parameters**:
- `model` (required): Model ID (e.g., "gfs", "hrrr")

**Response**:
```json
{
  "model_id": "gfs",
  "model_name": "GFS - Global Forecast System",
  "source_type": "aws_s3_grib2",
  "parameters_to_extract": [
    {
      "name": "TMP",
      "description": "Temperature",
      "levels": [
        {
          "level_type": "height_above_ground",
          "value": "2",
          "display": "2 m above ground",
          "storage_path_template": "shredded/gfs/{run}/TMP_2m/f{fhr}.grib2"
        }
      ],
      "style": "temperature",
      "units": "K"
    }
  ],
  "total_extractions": 4
}
```

#### `GET /api/admin/config/models`

List all model configurations.

**Response**:
```json
{
  "models": [
    {
      "id": "gfs",
      "name": "GFS - Global Forecast System",
      "model_type": "forecast",
      "source_type": "aws_s3_grib2",
      "projection": "geographic",
      "parameter_count": 4
    }
  ]
}
```

#### `GET /api/admin/config/models/:id`

Get raw YAML configuration for a model.

**Response**:
```json
{
  "id": "gfs",
  "yaml": "model:\n  id: gfs\n  name: \"GFS - Global Forecast System\"\n..."
}
```

#### `PUT /api/admin/config/models/:id`

Update model configuration.

**Request Body**:
```json
{
  "yaml": "model:\n  id: gfs\n  name: \"GFS - Global Forecast System\"\n..."
}
```

**Response**:
```json
{
  "success": true,
  "message": "Configuration for 'gfs' saved successfully",
  "validation_errors": []
}
```

---

## Common Tasks

### Add a New Parameter to a Model

1. Open admin dashboard: http://localhost:8000/admin.html
2. Click on the model (e.g., "GFS")
3. Switch to "Edit" tab
4. Add parameter to the `parameters:` list:
   ```yaml
   - name: RH
     description: "Relative Humidity"
     levels:
       - type: height_above_ground
         value: 2
         display: "2 m above ground"
     style: atmospheric
     units: "%"
   ```
5. Click **Validate** to check syntax
6. Click **Save** to write changes
7. Switch to "Shred Preview" tab to verify
8. Restart ingester: `docker-compose restart ingester`

### Change Forecast Hours

Edit the model YAML:

```yaml
schedule:
  cycles: [0, 6, 12, 18]
  forecast_hours:
    start: 0
    end: 240    # Changed from 120 to 240 hours
    step: 3
```

### Add Pressure Levels

For pressure-level parameters:

```yaml
- name: TMP
  description: "Temperature"
  levels:
    - type: isobaric
      values: [1000, 925, 850, 700, 500, 300, 250, 200]
      display_template: "{value} mb"
  style: temperature
  units: K
```

This will extract TMP at 8 pressure levels.

### Monitor Ingestion

1. **Check logs**: `docker-compose logs -f ingester`
2. **View admin dashboard**: http://localhost:8000/admin.html
3. **Query catalog**:
   ```sql
   SELECT model, parameter, COUNT(*) as count
   FROM datasets
   GROUP BY model, parameter
   ORDER BY model, parameter;
   ```

---

## Troubleshooting

### No data being ingested

1. **Check ingester logs**:
   ```bash
   docker-compose logs -f ingester
   ```

2. **Verify config files exist**:
   ```bash
   ls -la config/models/
   ls -la config/parameters/
   ```

3. **Check S3 connectivity**:
   ```bash
   aws s3 ls s3://noaa-gfs-bdp-pds/ --no-sign-request
   ```

### Parameter not being extracted

1. **Check model config** includes the parameter in `parameters:` list
2. **Check parameter tables** have the GRIB2 codes defined
3. **Use shred preview** to see if parameter is found in source files
4. **Check GRIB2 discipline/category/number** matches parameter tables

### Admin dashboard not loading

1. **Check WMS API is running**:
   ```bash
   docker-compose ps wms-api
   curl http://localhost:8080/api/admin/ingestion/status
   ```

2. **Check web server**:
   ```bash
   docker-compose ps web
   curl http://localhost:8000/admin.html
   ```

### Configuration changes not applied

1. **Restart ingester** after editing YAML files:
   ```bash
   docker-compose restart ingester
   ```

2. **Check for validation errors** in ingester logs

---

## File Locations

| Type | Path | Description |
|------|------|-------------|
| Model configs | `config/models/*.yaml` | Model-specific settings |
| Parameter tables | `config/parameters/*.yaml` | GRIB2 parameter mappings |
| Global config | `config/ingestion.yaml` | Global ingestion settings |
| Rendering styles | `config/styles/*.json` | Rendering style definitions |
| Ingester source | `services/ingester/src/` | Ingestion service code |
| Config loader | `services/ingester/src/config_loader.rs` | YAML config parser (720 lines) |
| Admin API | `services/wms-api/src/admin.rs` | Admin dashboard API (900 lines) |
| Admin UI | `web/admin.html` | Admin dashboard HTML |
| Admin JS | `web/admin.js` | Admin dashboard JavaScript (550 lines) |

---

## Next Steps

- **Add unit tests** for config loader (`services/ingester/src/config_loader.rs`)
- **Add integration tests** for ingestion pipeline
- **Implement hot-reload** for config changes (optional)
- **Add more models** (e.g., NAM, RAP, etc.)
- **Implement retention policies** to auto-delete old data

---

## See Also

- [INGESTION_CONSOLIDATION_PLAN.md](INGESTION_CONSOLIDATION_PLAN.md) - Original consolidation plan
- [DEVELOPMENT.md](DEVELOPMENT.md) - Development workflow
- [AGENTS.md](AGENTS.md) - Build, test, and profiling commands
- [QUICKREF.md](QUICKREF.md) - Quick reference for common operations
