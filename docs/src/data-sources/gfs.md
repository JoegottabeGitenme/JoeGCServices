# GFS (Global Forecast System)

NOAA's global numerical weather prediction model providing worldwide forecasts up to 16 days.

## Overview

- **Provider**: NOAA NCEP (National Centers for Environmental Prediction)
- **Coverage**: Global (90°S to 90°N, 180°W to 180°E)
- **Resolution**: 0.25° (~25 km)
- **Grid Size**: 1440 × 721 points
- **Longitude Convention**: 0° to 360° (see [Coordinate Convention](#coordinate-convention) below)
- **Update Frequency**: Every 6 hours (00, 06, 12, 18 UTC)
- **Forecast Range**: 0-384 hours (16 days)
- **Format**: GRIB2
- **Storage**: Zarr V3 with multi-resolution pyramids

## Forecast Hours

- **Hours 0-120**: Every 3 hours (0, 3, 6, 9, ..., 120)
- **Hours 120-384**: Every 12 hours (132, 144, 156, ..., 384)
- **Total**: 129 files per cycle

## Available Parameters

### Surface & Near-Surface

| Parameter | WMS Layer | Level | Description | Units | Style |
|-----------|-----------|-------|-------------|-------|-------|
| TMP | `gfs_TMP` | 2m above ground | Temperature | K → °C | temperature |
| DPT | `gfs_DPT` | 2m above ground | Dew point temperature | K → °C | temperature |
| RH | `gfs_RH` | 2m above ground | Relative humidity | % | humidity |
| UGRD | `gfs_UGRD` | 10m above ground | U-component of wind | m/s | wind |
| VGRD | `gfs_VGRD` | 10m above ground | V-component of wind | m/s | wind |
| GUST | `gfs_GUST` | surface | Wind gust speed | m/s | wind |
| PRMSL | `gfs_PRMSL` | mean sea level | Pressure reduced to MSL | Pa → hPa | mslp |
| VIS | `gfs_VIS` | surface | Visibility | m → km | visibility |

### Precipitation

| Parameter | WMS Layer | Level | Description | Units | Style |
|-----------|-----------|-------|-------------|-------|-------|
| APCP | `gfs_APCP` | surface | Total precipitation (accumulated) | kg/m² → mm | precipitation |
| PWAT | `gfs_PWAT` | entire atmosphere | Precipitable water | kg/m² → mm | humidity |

### Convective/Stability

| Parameter | WMS Layer | Level | Description | Units | Style |
|-----------|-----------|-------|-------------|-------|-------|
| CAPE | `gfs_CAPE` | surface | Convective Available Potential Energy | J/kg | cape |
| CIN | `gfs_CIN` | surface | Convective Inhibition | J/kg | cape |

### Cloud Cover

| Parameter | WMS Layer | Level | Description | Units | Style |
|-----------|-----------|-------|-------------|-------|-------|
| TCDC | `gfs_TCDC` | entire atmosphere | Total cloud cover | % | cloud |
| LCDC | `gfs_LCDC` | low cloud layer | Low cloud cover (0-2km) | % | cloud |
| MCDC | `gfs_MCDC` | middle cloud layer | Middle cloud cover (2-6km) | % | cloud |
| HCDC | `gfs_HCDC` | high cloud layer | High cloud cover (>6km) | % | cloud |

### Upper-Air Parameters

Upper-air parameters are available on multiple pressure levels, exposed via the **ELEVATION** dimension:

| Parameter | WMS Layer | Available Levels (mb) | Description |
|-----------|-----------|----------------------|-------------|
| TMP | `gfs_TMP` | 1000, 975, 950, 925, 900, 850, 700, 500, 300, 250, 200, 100, 70, 50, 30, 20, 10 | Temperature |
| HGT | `gfs_HGT` | 1000, 925, 850, 700, 500, 300, 250, 200, 100, 70, 50, 30, 20, 10 | Geopotential height |
| RH | `gfs_RH` | 1000, 975, 950, 925, 900, 850, 700, 500, 300 | Relative humidity |
| UGRD | `gfs_UGRD` | 1000, 975, 950, 925, 900, 850, 700, 500, 300, 250, 200, 100 | U-wind component |
| VGRD | `gfs_VGRD` | 1000, 975, 950, 925, 900, 850, 700, 500, 300, 250, 200, 100 | V-wind component |

### Composite Layers

| Layer | Requires | Description |
|-------|----------|-------------|
| `gfs_WIND_BARBS` | UGRD, VGRD | Wind direction and speed barbs |

## WMS Dimensions

GFS layers support the following WMS dimensions:

| Dimension | Description | Example |
|-----------|-------------|---------|
| `RUN` | Model initialization time | `2024-12-17T12:00:00Z` |
| `FORECAST` | Hours from run time | `3`, `6`, `12` |
| `ELEVATION` | Pressure level or surface | `500 mb`, `2 m above ground` |

**Example Request**:
```
/wms?SERVICE=WMS&REQUEST=GetMap
  &LAYERS=gfs_TMP
  &ELEVATION=500%20mb
  &RUN=2024-12-17T12:00:00Z
  &FORECAST=6
  &CRS=EPSG:4326
  &BBOX=-90,-180,90,180
  &WIDTH=512&HEIGHT=256
  &FORMAT=image/png
```

## Data Source

**AWS Open Data**:
```
https://noaa-gfs-bdp-pds.s3.amazonaws.com/gfs.{YYYYMMDD}/{HH}/atmos/gfs.t{HH}z.pgrb2.0p25.f{FFF}
```

**Example**:
```
https://noaa-gfs-bdp-pds.s3.amazonaws.com/gfs.20241217/12/atmos/gfs.t12z.pgrb2.0p25.f003
```

**NOMADS (backup)**:
```
https://nomads.ncep.noaa.gov/pub/data/nccf/com/gfs/prod/gfs.{YYYYMMDD}/{HH}/atmos/gfs.t{HH}z.pgrb2.0p25.f{FFF}
```

## File Sizes

- Per file: ~550 MB (full GRIB2 with all parameters)
- Per forecast hour (ingested): ~50 parameters × 2.5 MB = ~125 MB Zarr
- Per cycle (6 forecast hours): ~750 MB Zarr
- Per day (4 cycles): ~3 GB Zarr

## Storage Format

GFS data is stored in Zarr V3 format with multi-resolution pyramids:

```
grids/gfs/20241217_12z/
├── tmp_2_m_above_ground_f003.zarr/
│   ├── zarr.json                # Root metadata
│   ├── 0/                       # Full resolution (1440×721)
│   │   ├── zarr.json
│   │   └── c/0/0, c/0/1, ...   # Compressed chunks (512×512)
│   └── 1/                       # 2x downsampled (720×360)
│       └── ...
├── tmp_500_mb_f003.zarr/
├── ugrd_10_m_above_ground_f003.zarr/
├── hgt_500_mb_f003.zarr/
└── ...
```

## Coordinate Convention

GFS uses the **0-360° longitude convention**, which differs from the standard -180° to 180° used by web maps:

| Location | Standard (WGS84) | GFS Convention |
|----------|------------------|----------------|
| New York | -74° | 286° |
| London | 0° | 0° |
| Tokyo | 140° | 140° |
| Los Angeles | -118° | 242° |

### Grid Structure

```
Column 0    = 0.00° longitude
Column 1    = 0.25° longitude
...
Column 1439 = 359.75° longitude
(No column at exactly 360° - creates a "wrap gap")
```

### Wrap Gap Handling

The 0.25° gap between column 1439 (359.75°) and 360° requires special handling in the rendering pipeline. When tile requests include negative longitudes near 0° (e.g., -0.1°), they normalize to this gap region (359.9°). The renderer detects this and uses wrapping interpolation between the last and first columns to ensure seamless rendering across the prime meridian.

See [Rendering Pipeline - Prime Meridian Wrap Gap](../architecture/rendering-pipeline.md#handling-the-prime-meridian-wrap-gap) for implementation details.

## Configuration

**Model Config** (`config/models/gfs.yaml`):
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

schedule:
  cycles: [0, 6, 12, 18]
  forecast_hours:
    start: 0
    end: 15
    step: 3

parameters:
  - name: TMP
    levels:
      - type: height_above_ground
        value: 2
      - type: isobaric
        values: [1000, 925, 850, 700, 500, 300, 250, 200, 100, 70, 50, 30, 20, 10]
    style: temperature
    units: K
```

**Layer Config** (`config/layers/gfs.yaml`):
```yaml
layers:
  - id: gfs_TMP
    parameter: TMP
    title: "Temperature"
    style_file: temperature.json
    units:
      native: K
      display: "°C"
      conversion: K_to_C
    levels:
      - value: "2 m above ground"
        default: true
      - value: "1000 mb"
      - value: "500 mb"
      # ... more levels
```

## Typical Uses

- **Synoptic weather maps**: Surface pressure, fronts
- **Temperature forecasts**: Daily high/low temperatures
- **Precipitation forecasts**: Rain/snow accumulation
- **Upper-air analysis**: Jet stream, 500mb heights, troughs/ridges
- **Aviation weather**: Winds aloft, visibility, cloud layers
- **Marine forecasts**: Wave height, wind
- **Severe weather**: CAPE, CIN for thunderstorm potential

## Related

- [Layer Configuration](../configuration/parameters.md)
- [Style Configuration](../configuration/styles.md)
- [Rendering Pipeline](../architecture/rendering-pipeline.md)
- [grid-processor](../crates/grid-processor.md) - Zarr storage details
