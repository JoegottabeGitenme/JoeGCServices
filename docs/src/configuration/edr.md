# EDR Configuration

Configuration reference for the OGC API - Environmental Data Retrieval (EDR) service.

## Overview

EDR configuration files are located in `config/edr/` and define:
- Collections exposed by the API
- Parameters available per collection
- Response limits
- Named locations for human-readable queries

## Configuration Files

```
config/edr/
├── hrrr.yaml        # HRRR model collections
├── gfs.yaml         # GFS model collections (if configured)
└── locations.yaml   # Named locations (airports, cities)
```

## Model Configuration

Each model has its own YAML file defining collections.

### Basic Structure

```yaml
# config/edr/hrrr.yaml
model: hrrr

collections:
  - id: hrrr-isobaric
    title: "HRRR - Isobaric Levels"
    description: "Upper-air parameters on pressure levels"
    level_filter:
      level_type: isobaric
      level_code: 100
    parameters:
      - name: TMP
        levels: [850, 700, 500, 300, 250]
      - name: UGRD
        levels: [850, 700, 500]
      - name: VGRD
        levels: [850, 700, 500]
      - name: HGT
        levels: [850, 700, 500, 300, 250]
    run_mode: instances

settings:
  output_formats:
    - application/vnd.cov+json
    - application/geo+json
  default_crs: "CRS:84"
  supported_crs:
    - "CRS:84"
    - "EPSG:4326"

limits:
  max_parameters_per_request: 10
  max_time_steps: 48
  max_vertical_levels: 20
  max_response_size_mb: 50
  max_area_sq_degrees: 100
  max_radius_km: 500
  max_trajectory_points: 100
  max_corridor_length_km: 2000
```

### Collection Definition

| Field | Required | Description |
|-------|----------|-------------|
| `id` | Yes | Unique collection identifier |
| `title` | Yes | Human-readable title |
| `description` | No | Detailed description |
| `level_filter` | Yes | Filter for vertical level types |
| `parameters` | Yes | List of exposed parameters |
| `run_mode` | No | `instances` or `latest` (default: `latest`) |

### Level Filter

The `level_filter` determines which vertical levels are included in the collection:

| Field | Description |
|-------|-------------|
| `level_type` | Type of vertical level |
| `level_code` | GRIB2 level type code (optional) |

#### Supported Level Types

| Type | Level Code | Description |
|------|------------|-------------|
| `surface` | 1 | Ground/water surface |
| `mean_sea_level` | 101 | Mean sea level |
| `isobaric` | 100 | Pressure levels (mb/hPa) |
| `height_above_ground` | 103 | Height above ground (m) |
| `entire_atmosphere` | 10 | Entire atmosphere column |
| `cloud_layer` | 212/222/232 | Low/middle/high cloud layers |

### Parameter Definition

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Parameter short name (e.g., TMP, UGRD) |
| `levels` | No | Available vertical levels for this parameter |

Levels can be specified as:
- Numeric values: `[850, 700, 500]` (for isobaric, height_above_ground)
- Named values: `["surface"]`, `["2 m above ground"]`

### Run Mode

| Mode | Description |
|------|-------------|
| `latest` | Returns data from the most recent model run |
| `instances` | Exposes each model run as a separate instance |

When `run_mode: instances`, clients can:
1. List instances: `GET /collections/{id}/instances`
2. Query specific runs: `GET /collections/{id}/instances/{runTime}/position`

### Settings

| Field | Default | Description |
|-------|---------|-------------|
| `output_formats` | `[application/vnd.cov+json]` | Supported response formats |
| `default_crs` | `CRS:84` | Default coordinate reference system |
| `supported_crs` | `[CRS:84]` | All supported CRS values |

### Response Limits

| Field | Default | Description |
|-------|---------|-------------|
| `max_parameters_per_request` | 10 | Maximum parameters in single query |
| `max_time_steps` | 48 | Maximum temporal values |
| `max_vertical_levels` | 20 | Maximum z levels |
| `max_response_size_mb` | 50 | Maximum response payload size |
| `max_area_sq_degrees` | 100 | Maximum area for area/cube queries |
| `max_radius_km` | 500 | Maximum radius for radius queries |
| `max_trajectory_points` | 100 | Maximum waypoints in trajectory |
| `max_corridor_length_km` | 2000 | Maximum corridor centerline length |

Exceeding any limit returns HTTP 413 (Payload Too Large).

## Locations Configuration

Named locations allow queries using human-readable identifiers instead of coordinates.

### File Structure

```yaml
# config/edr/locations.yaml
locations:
  # Airports (ICAO codes)
  - id: KJFK
    name: "John F. Kennedy International Airport"
    description: "New York, NY"
    coords: [-73.7781, 40.6413]
    properties:
      type: airport
      country: US

  # Cities
  - id: NYC
    name: "New York City"
    description: "Manhattan, New York"
    coords: [-74.0060, 40.7128]
    properties:
      type: city
      country: US
```

### Location Fields

| Field | Required | Description |
|-------|----------|-------------|
| `id` | Yes | Unique identifier (case-insensitive lookup) |
| `name` | Yes | Human-readable name |
| `description` | No | Additional description |
| `coords` | Yes | `[longitude, latitude]` in CRS:84/WGS84 |
| `properties` | No | Custom metadata key-value pairs |

### Location ID Conventions

Recommended ID formats:
- **Airports**: ICAO codes (`KJFK`, `EGLL`, `RJTT`)
- **Weather stations**: WMO IDs or network codes
- **Cities**: Short codes (`NYC`, `CHI`, `LAX`)
- **Custom points**: Descriptive identifiers

### Querying Locations

```bash
# List all locations
curl http://localhost:8083/edr/collections/hrrr-surface/locations

# Query by ID (case-insensitive)
curl http://localhost:8083/edr/collections/hrrr-surface/locations/KJFK
curl http://localhost:8083/edr/collections/hrrr-surface/locations/kjfk  # Also works
```

## Example Configurations

### Surface Collection

```yaml
- id: hrrr-surface
  title: "HRRR - Surface"
  description: "Surface and near-surface parameters"
  level_filter:
    level_type: surface
    level_code: 1
  parameters:
    - name: TMP
      levels: ["surface"]
    - name: PRES
      levels: ["surface"]
    - name: CAPE
      levels: ["surface"]
    - name: CIN
      levels: ["surface"]
```

### Height Above Ground Collection

```yaml
- id: hrrr-height-agl
  title: "HRRR - Height Above Ground"
  description: "Parameters at specific heights above ground"
  level_filter:
    level_type: height_above_ground
    level_code: 103
  parameters:
    - name: TMP
      levels: [2]  # 2m temperature
    - name: DPT
      levels: [2]  # 2m dewpoint
    - name: UGRD
      levels: [10, 80]  # 10m and 80m wind
    - name: VGRD
      levels: [10, 80]
```

### Cloud Layer Collection

```yaml
- id: hrrr-cloud-layers
  title: "HRRR - Cloud Layers"
  description: "Low, middle, and high cloud cover"
  level_filter:
    level_type: cloud_layer
  parameters:
    - name: LCDC
      levels: ["low cloud layer"]
    - name: MCDC
      levels: ["middle cloud layer"]
    - name: HCDC
      levels: ["high cloud layer"]
```

### GFS Global Collection

```yaml
# config/edr/gfs.yaml
model: gfs

collections:
  - id: gfs-isobaric
    title: "GFS - Isobaric Levels"
    description: "Global upper-air parameters"
    level_filter:
      level_type: isobaric
      level_code: 100
    parameters:
      - name: TMP
        levels: [1000, 925, 850, 700, 500, 300, 250, 200]
      - name: HGT
        levels: [1000, 925, 850, 700, 500, 300, 250, 200]
      - name: UGRD
        levels: [850, 700, 500, 300, 250, 200]
      - name: VGRD
        levels: [850, 700, 500, 300, 250, 200]
    run_mode: instances

settings:
  output_formats:
    - application/vnd.cov+json
    - application/geo+json

limits:
  max_parameters_per_request: 10
  max_area_sq_degrees: 200  # Larger for global model
```

## Hot Reload

Configuration can be reloaded without restarting the service:

```bash
curl -X POST http://localhost:8083/api/config/reload
```

This reloads all YAML files in `config/edr/`.

## Validation

### Verify Collections

```bash
# List all collections
curl http://localhost:8083/edr/collections | jq '.collections[].id'

# Check specific collection
curl http://localhost:8083/edr/collections/hrrr-surface | jq '.parameter_names'
```

### Verify Locations

```bash
# List all locations
curl http://localhost:8083/edr/collections/hrrr-surface/locations | jq '.features[].id'

# Count locations
curl http://localhost:8083/edr/collections/hrrr-surface/locations | jq '.features | length'
```

### Coverage Validation

Use the web-based coverage validation tool to verify data availability:

```
http://localhost:8000/edr-coverage.html
```

This tests whether configured parameters actually exist in the database.

## See Also

- [EDR Endpoints](../api-reference/edr.md) - API documentation
- [EDR API Service](../services/edr-api.md) - Service details
- [Parameters](./parameters.md) - Parameter configuration
- [Models](./models.md) - Model configuration
