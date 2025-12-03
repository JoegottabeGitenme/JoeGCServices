# HRRR (High-Resolution Rapid Refresh)

High-resolution numerical weather model for CONUS providing detailed short-range forecasts.

## Overview

- **Provider**: NOAA NCEP
- **Coverage**: CONUS (Continental United States)
- **Resolution**: 3 km
- **Grid Size**: 1799 × 1059 points (Lambert Conformal projection)
- **Update Frequency**: Every hour
- **Forecast Range**: 0-48 hours
- **Format**: GRIB2

## Forecast Hours

- Hourly forecasts: 0, 1, 2, 3, ..., 48
- Total: 49 files per cycle

## Available Parameters

| Parameter | Level | Description | Units |
|-----------|-------|-------------|-------|
| `TMP` | 2m | Temperature | K |
| `DPT` | 2m | Dewpoint temperature | K |
| `RH` | 2m | Relative humidity | % |
| `UGRD` | 10m | U-component wind | m/s |
| `VGRD` | 10m | V-component wind | m/s |
| `GUST` | surface | Wind gust | m/s |
| `REFL` | entire atmos | Composite reflectivity | dBZ |
| `VIS` | surface | Visibility | m |
| `APCP` | surface | Accumulated precipitation | kg/m² |
| `CAPE` | surface | Convective available potential energy | J/kg |
| `CIN` | surface | Convective inhibition | J/kg |

## Layer Names

Examples:
- `hrrr_TMP_2m` - Surface temperature
- `hrrr_REFL` - Composite reflectivity
- `hrrr_CAPE` - Convective energy

## Data Source

**NOMADS Server**:
```
https://nomads.ncep.noaa.gov/pub/data/nccf/com/hrrr/prod/hrrr.{YYYYMMDD}/conus/hrrr.t{HH}z.wrfsfcf{FF}.grib2
```

**Example**:
```
https://nomads.ncep.noaa.gov/pub/data/nccf/com/hrrr/prod/hrrr.20241203/conus/hrrr.t15z.wrfsfcf00.grib2
```

## File Sizes

- Per file: ~200-300 MB
- Per cycle (49 files): ~10 GB
- Per day (24 cycles): ~240 GB

## Download Script

```bash
./scripts/download_hrrr.sh

# Downloads recent HRRR cycle
# Forecast hours: 0-18 (hourly)
# Total: ~3-4 GB
```

## Typical Uses

- **Severe weather forecasting**: Thunderstorms, tornadoes
- **Short-range temperature forecasts**: Hourly temperatures
- **Convective analysis**: CAPE, storm initiation
- **High-resolution radar simulation**: Reflectivity forecasts
- **Aviation**: Terminal forecasts, turbulence
- **Nowcasting**: 0-6 hour detailed forecasts
