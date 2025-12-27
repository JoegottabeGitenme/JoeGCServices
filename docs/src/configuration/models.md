# Model Configuration

Model configuration files define data sources, grid properties, schedules, and available parameters.

## File Location

`config/models/{model}.yaml`

## Configuration Schema

### `model` (required)

Basic model identification:

```yaml
model:
  id: gfs                              # Unique identifier (lowercase, alphanumeric)
  name: "GFS - Global Forecast System" # Display name
  description: "..."                   # Optional description
  enabled: true                        # Enable/disable this model
```

### `dimensions` (required)

Defines which WMS/WMTS dimensions are exposed:

```yaml
# For forecast models (GFS, HRRR)
dimensions:
  type: forecast      # Uses RUN + FORECAST dimensions
  run: true           # Model initialization time (ISO8601)
  forecast: true      # Hours ahead from run time
  elevation: true     # Vertical levels (pressure, height)

# For observation data (GOES, MRMS)
dimensions:
  type: observation   # Uses TIME dimension
  time: true          # Observation timestamp (ISO8601)
  elevation: false    # Usually no vertical levels
```

### `source` (required)

Data source configuration:

```yaml
source:
  type: aws_s3              # Source type
  bucket: noaa-gfs-bdp-pds  # S3 bucket name
  region: us-east-1         # AWS region
  prefix_template: "..."    # Path template with variables
  file_pattern: "..."       # Filename pattern
  compression: gzip         # Optional: gzip, none
```

**Source Types**:
- `aws_s3` - Standard AWS S3 bucket (GRIB2 data)
- `aws_s3_goes` - GOES satellite data (NetCDF)
- `aws_s3_grib2` - GRIB2 data with compression support

### `grid` (required)

Grid and projection information:

```yaml
grid:
  projection: geographic    # Projection type
  resolution: "0.25deg"     # Grid resolution
  bbox:
    min_lon: -180.0
    min_lat: -90.0
    max_lon: 180.0
    max_lat: 90.0
  dimensions:               # Grid size (optional, for validation)
    x: 1440
    y: 721
  lon_convention: 0_to_360  # Optional: 0_to_360 or -180_to_180
```

**Projection Types** and Grid Reading Behavior:

| Projection | `requires_full_grid` | Description |
|------------|---------------------|-------------|
| `geographic` / `latlon` | `false` (auto) | Lat/lon (EPSG:4326) - supports partial bbox reads |
| `geostationary` | `true` (auto) | Geostationary satellite - requires full grid |
| `lambert_conformal` | `true` (auto) | Lambert Conformal Conic - requires full grid |
| `mercator` | `true` (auto) | Mercator - requires full grid |

The `requires_full_grid` setting is **automatically inferred** from the projection type. Non-geographic projections have a non-linear mapping between grid indices and geographic coordinates, so partial bounding box reads would produce incorrect results.

You can override the inferred value if needed:

```yaml
grid:
  projection: geographic
  requires_full_grid: true   # Force full grid reads even for geographic
```

### `schedule` (required)

Data availability schedule:

```yaml
# For forecast models
schedule:
  cycles: [0, 6, 12, 18]    # UTC hours when model runs
  forecast_hours:
    start: 0
    end: 384
    step: 3
  poll_interval_secs: 3600
  delay_hours: 4            # Time after cycle before data available

# For observation data
schedule:
  type: observation
  poll_interval_secs: 120
  lookback_minutes: 30
```

### `retention` (recommended)

How long to keep ingested data:

```yaml
retention:
  hours: 24    # Keep data for 24 hours
```

### `parameters` (required)

List of available parameters with pyramid configuration:

```yaml
parameters:
  - name: TMP
    description: "Temperature"
    downsample: mean              # Pyramid downsampling method
    grib2:                        # GRIB2 identification codes
      discipline: 0               # 0=meteorological, 209=MRMS local
      category: 0                 # Parameter category within discipline
      number: 0                   # Parameter number within category
    levels:
      - type: height_above_ground
        level_code: 103           # GRIB2 level type code
        value: 2
        display: "2 m above ground"
      - type: isobaric
        level_code: 100           # GRIB2 level type code for pressure levels
        values: [1000, 850, 500, 250]
        display_template: "{value} mb"  # Template with {value} placeholder
    style: temperature
    units: K
    display_units: "C"
    conversion: K_to_C
    valid_range: [180, 340]       # Expected data range (optional)
```

#### GRIB2 Code Mapping

The `grib2:` section maps GRIB2 numeric codes to parameter names. The ingestion system uses these to identify parameters in GRIB2 files:

| Field | Description | Example |
|-------|-------------|---------|
| `discipline` | WMO discipline code | 0 (meteorological), 209 (MRMS) |
| `category` | Parameter category | 0 (temperature), 2 (momentum) |
| `number` | Parameter number | 0 (TMP), 2 (UGRD), 3 (VGRD) |

#### Level Configuration

| Field | Description | Example |
|-------|-------------|---------|
| `level_code` | GRIB2 level type code | 1, 100, 103 |
| `display` | Static description | "surface", "mean sea level" |
| `display_template` | Template with `{value}` | "{value} mb", "{value} m above ground" |

**Common Level Codes**:
| Code | Type | Example Display |
|------|------|-----------------|
| 1 | Surface | "surface" |
| 100 | Isobaric (pressure) | "500 mb", "850 mb" |
| 101 | Mean sea level | "mean sea level" |
| 103 | Height above ground | "2 m above ground", "10 m above ground" |
| 200 | Entire atmosphere | "entire atmosphere" |

**Downsample Methods**:

| Method | Use Case |
|--------|----------|
| `max` | Radar reflectivity, precipitation rate (preserve peaks) |
| `mean` | Temperature, wind, humidity (smooth gradients) |
| `nearest` | Categorical data (no interpolation) |

**Level Types**:
- `surface` - Ground level
- `height_above_ground` - Height in meters (e.g., 2m, 10m)
- `isobaric` - Pressure level (mb/hPa)
- `mean_sea_level` - MSL pressure
- `entire_atmosphere` - Column-integrated
- `height_above_msl` - Height above sea level (MRMS)
- `top_of_atmosphere` - TOA (satellite)

### `composites` (optional)

Derived layers from multiple parameters:

```yaml
composites:
  - name: WIND_BARBS
    description: "Wind barbs visualization"
    requires: [UGRD, VGRD]
    renderer: wind_barbs
    style: wind_barbs
```

## Example: Forecast Model (GFS)

```yaml
model:
  id: gfs
  name: "GFS - Global Forecast System"
  enabled: true

dimensions:
  type: forecast
  run: true
  forecast: true
  elevation: true

source:
  type: aws_s3
  bucket: noaa-gfs-bdp-pds
  region: us-east-1

grid:
  projection: geographic
  resolution: "0.25deg"
  bbox:
    min_lon: 0
    min_lat: -90
    max_lon: 360
    max_lat: 90
  lon_convention: 0_to_360

schedule:
  cycles: [0, 6, 12, 18]
  forecast_hours:
    start: 0
    end: 120
    step: 3
  poll_interval_secs: 3600
  delay_hours: 4

retention:
  hours: 48

parameters:
  - name: TMP
    description: "Temperature"
    grib2:
      discipline: 0
      category: 0
      number: 0
    downsample: mean
    levels:
      - type: height_above_ground
        level_code: 103
        value: 2
        display: "2 m above ground"
      - type: isobaric
        level_code: 100
        values: [1000, 850, 500, 300, 200]
        display_template: "{value} mb"
    style: temperature
    units: K
```

## Example: Observation Model (MRMS)

```yaml
model:
  id: mrms
  name: "MRMS - Multi-Radar Multi-Sensor"
  enabled: true

dimensions:
  type: observation
  time: true
  elevation: false

source:
  type: aws_s3_grib2
  bucket: noaa-mrms-pds
  region: us-east-1
  compression: gzip

grid:
  projection: latlon
  resolution: "0.01deg"
  bbox:
    min_lon: -130.0
    min_lat: 20.0
    max_lon: -60.0
    max_lat: 55.0

schedule:
  type: observation
  poll_interval_secs: 120
  lookback_minutes: 30

retention:
  hours: 1

parameters:
  - name: REFL
    description: "Seamless Hybrid Scan Reflectivity"
    grib2:
      discipline: 209    # MRMS local discipline
      category: 0
      number: 16
    downsample: max
    product: "SeamlessHSR_00.00"
    levels:
      - type: height_above_msl
        level_code: 102
        value: 0
        display: "surface"
    style: reflectivity
    units: "dBZ"
    valid_range: [-30, 80]
```

## Storage Path Format

Ingested data is stored as Zarr V3 pyramids:

**Forecast**: `grids/{model}/{date}/{HH}/{param}_f{fhr:03}.zarr`
```
grids/gfs/2024-12-17/12/TMP_f003.zarr
```

**Observation**: `grids/{model}/{date}/{HH}/{param}_{MM}.zarr`
```
grids/mrms/2024-12-17/12/REFL_05.zarr  (12:05 UTC observation)
```

## Validation

Run the validation script to check configurations:

```bash
# Validate all model configs
python config/models/validate.py

# Validate specific model
python config/models/validate.py mrms.yaml

# Verbose output
python config/models/validate.py -v
```

## See Also

- [config/models/README.md](../../../config/models/README.md) - Full schema reference
- [Data Sources](../data-sources/README.md) - Model-specific documentation
- [Ingester Service](../services/ingester.md) - How configuration drives ingestion
