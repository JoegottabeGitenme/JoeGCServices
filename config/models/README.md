# Model Configuration

This directory contains YAML configuration files for each weather data model/source.
Each file defines the data source, grid properties, schedule, and available parameters.

## Validation

Run the validation script to check all configuration files:

```bash
# Validate all YAML files
./validate.py

# Validate with verbose output
./validate.py -v

# Validate specific files
./validate.py gfs.yaml hrrr.yaml

# Quiet mode (errors only)
./validate.py -q
```

## Adding a New Model

1. Copy an existing configuration file as a template
2. Update all sections for your model
3. Run `./validate.py your_model.yaml` to check for errors
4. Test with the ingester and WMS API

## Configuration Schema

### `model` (required)

Basic model identification.

```yaml
model:
  id: gfs                              # Unique identifier (lowercase, alphanumeric)
  name: "GFS - Global Forecast System" # Display name
  description: "..."                   # Optional description
  enabled: true                        # Enable/disable this model
```

### `dimensions` (recommended)

Defines which WMS/WMTS dimensions are exposed for this model.

```yaml
# For forecast models (GFS, HRRR, etc.)
dimensions:
  type: forecast      # Uses RUN + FORECAST dimensions
  run: true           # RUN dimension - model initialization time (ISO8601)
  forecast: true      # FORECAST dimension - hours ahead from run time
  elevation: true     # ELEVATION dimension - vertical levels

# For observation data (GOES, MRMS, etc.)
dimensions:
  type: observation   # Uses TIME dimension
  time: true          # TIME dimension - observation timestamp (ISO8601)
  elevation: false    # Usually no vertical levels for imagery/radar
```

**Dimension Types:**
- `forecast` - Data with model run cycles and forecast hours (NWP models)
- `observation` - Real-time observational data (satellite, radar)

### `source` (required)

Data source configuration.

```yaml
source:
  type: aws_s3              # Source type (see below)
  bucket: noaa-gfs-bdp-pds  # S3 bucket name
  region: us-east-1         # AWS region
  prefix_template: "..."    # Path template with variables
  file_pattern: "..."       # Filename pattern
```

**Source Types:**
- `aws_s3` - Standard AWS S3 bucket (GRIB2 data)
- `aws_s3_goes` - GOES satellite data (NetCDF)
- `aws_s3_grib2` - GRIB2 data with compression support
- `local` - Local filesystem
- `http` - HTTP/HTTPS endpoint

### `grid` (required)

Grid and projection information.

```yaml
grid:
  projection: geographic    # Projection type
  resolution: "0.25deg"     # Grid resolution
  bbox:
    min_lon: -180.0
    min_lat: -90.0
    max_lon: 180.0
    max_lat: 90.0
  lon_convention: 0_to_360  # Optional: longitude convention
```

**Projection Types:**
- `geographic` / `latlon` - Lat/lon (EPSG:4326)
- `geostationary` - Geostationary satellite (requires `projection_params`)
- `lambert_conformal` - Lambert Conformal Conic
- `mercator` - Mercator projection

### `schedule` (required)

Data availability schedule.

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
  poll_interval_secs: 300
  lookback_minutes: 60
```

### `retention` (recommended)

How long to keep ingested data.

```yaml
retention:
  hours: 24    # Keep data for 24 hours
```

### `precaching` (optional)

Grid cache warming configuration.

```yaml
precaching:
  enabled: true
  keep_recent: 3              # Number of recent observations/cycles to cache
  warm_on_ingest: true        # Warm cache when new data ingested
  poll_interval_secs: 60
  parameters: [TMP, UGRD]     # Parameters to precache
```

### `parameters` (required)

List of available parameters/variables.

```yaml
parameters:
  - name: TMP                     # GRIB2 parameter name
    description: "Temperature"
    levels:
      - type: height_above_ground
        value: 2
        display: "2 m above ground"
      - type: isobaric
        values: [1000, 850, 500, 250]
        display_template: "{value} mb"
    style: temperature            # Rendering style
    units: K                      # Native units
    display_units: "C"           # Display units
    conversion: K_to_C            # Unit conversion
```

**Level Types:**
- `surface` - Surface level
- `height_above_ground` - Height in meters above ground
- `height_above_ground_layer` - Layer averaged (e.g., 0-6km)
- `isobaric` - Pressure level (mb/hPa)
- `mean_sea_level` - Mean sea level
- `entire_atmosphere` - Column-integrated
- `low_cloud_layer` / `middle_cloud_layer` / `high_cloud_layer` - Cloud layers
- `cloud_top` - Cloud top
- `top_of_atmosphere` - TOA (satellite)
- `depth_below_surface` - Soil depth
- `boundary_layer` - Planetary boundary layer
- `tropopause` - Tropopause level

**Styles:**
- `default` - Generic colormap
- `temperature` - Temperature colormap
- `wind` - Wind speed colormap
- `precipitation` - Precipitation colormap
- `humidity` - Humidity colormap
- `atmospheric` - Pressure/height colormap
- `cape` - CAPE/convective colormap
- `cloud` - Cloud cover colormap
- `visibility` - Visibility colormap
- `reflectivity` - Radar reflectivity colormap
- `precip_rate` - Precipitation rate colormap
- `goes_visible` - GOES visible imagery
- `goes_ir` - GOES infrared imagery
- `wind_barbs` - Wind barb symbols
- `helicity` - Storm helicity colormap
- `lightning` - Lightning colormap
- `smoke` - Smoke/aerosol colormap
- `radar` - Generic radar colormap

**Unit Conversions:**
- `K_to_C` - Kelvin to Celsius
- `K_to_F` - Kelvin to Fahrenheit
- `Pa_to_hPa` / `Pa_to_mb` - Pascals to hectopascals/millibars
- `m_to_km` - Meters to kilometers
- `m_to_ft` - Meters to feet
- `m_to_kft` - Meters to kilofeet
- `ms_to_kt` - m/s to knots
- `ms_to_mph` - m/s to mph

### `composites` (optional)

Derived layers from multiple parameters.

```yaml
composites:
  - name: WIND_BARBS
    description: "Wind barbs visualization"
    requires: [UGRD, VGRD]    # Required parameters
    renderer: wind_barbs
    style: wind_barbs
```

## Example: Minimal Forecast Model

```yaml
model:
  id: mymodel
  name: "My Weather Model"
  enabled: true

dimensions:
  type: forecast
  run: true
  forecast: true
  elevation: true

source:
  type: aws_s3
  bucket: my-bucket
  region: us-east-1

grid:
  projection: geographic
  resolution: "0.5deg"
  bbox:
    min_lon: -180
    min_lat: -90
    max_lon: 180
    max_lat: 90

schedule:
  cycles: [0, 12]
  forecast_hours:
    start: 0
    end: 72
    step: 6
  poll_interval_secs: 3600

retention:
  hours: 24

parameters:
  - name: TMP
    description: "Temperature"
    levels:
      - type: surface
        display: "surface"
    style: temperature
    units: K
```

## Example: Observation Model

```yaml
model:
  id: myradar
  name: "My Radar Network"
  enabled: true

dimensions:
  type: observation
  time: true
  elevation: false

source:
  type: aws_s3
  bucket: my-radar-bucket
  region: us-east-1

grid:
  projection: latlon
  resolution: "0.01deg"
  bbox:
    min_lon: -130
    min_lat: 20
    max_lon: -60
    max_lat: 55

schedule:
  type: observation
  poll_interval_secs: 120
  lookback_minutes: 30

retention:
  hours: 2

parameters:
  - name: REFL
    description: "Reflectivity"
    levels:
      - type: surface
        display: "composite"
    style: reflectivity
    units: dBZ
```
