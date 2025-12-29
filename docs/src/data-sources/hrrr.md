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
| `UGRD` | 2m | U-component wind | m/s |
| `VGRD` | 2m | V-component wind | m/s |
| `GUST` | surface | Wind gust | m/s |
| `PRMSL` | mean sea level | Mean sea level pressure | Pa |
| `APCP` | surface | Accumulated precipitation | kg/m² |
| `VIS` | surface | Visibility | m |
| `TCDC` | entire atmosphere | Total cloud cover | % |

Note: The HRRR configuration focuses on core weather parameters. Convective parameters (CAPE, CIN) and composite reflectivity (REFL) are available in the raw GRIB2 data but not currently exposed as layers.

## Layer Names

Examples:
- `hrrr_TMP` - Surface temperature (2m)
- `hrrr_DPT` - Dewpoint temperature (2m)
- `hrrr_WIND_BARBS` - Wind barbs (composite of UGRD/VGRD)
- `hrrr_PRMSL` - Mean sea level pressure
- `hrrr_TCDC` - Total cloud cover

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

- **Short-range temperature forecasts**: Hourly temperatures at 3km resolution
- **Wind analysis**: Surface and near-surface wind conditions
- **Visibility monitoring**: Aviation and transportation planning
- **Cloud cover forecasting**: Solar energy and outdoor activities
- **Pressure analysis**: Weather system tracking
- **Nowcasting**: 0-6 hour detailed forecasts
