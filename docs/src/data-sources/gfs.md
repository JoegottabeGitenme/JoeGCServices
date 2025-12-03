# GFS (Global Forecast System)

NOAA's global numerical weather prediction model providing worldwide forecasts up to 16 days.

## Overview

- **Provider**: NOAA NCEP (National Centers for Environmental Prediction)
- **Coverage**: Global (90°S to 90°N, 180°W to 180°E)
- **Resolution**: 0.25° (~25 km)
- **Grid Size**: 1440 × 721 points
- **Update Frequency**: Every 6 hours (00, 06, 12, 18 UTC)
- **Forecast Range**: 0-384 hours (16 days)
- **Format**: GRIB2

## Forecast Hours

- **Hours 0-120**: Every 3 hours (0, 3, 6, 9, ..., 120)
- **Hours 120-384**: Every 12 hours (132, 144, 156, ..., 384)
- **Total**: 129 files per cycle

## Available Parameters

### Surface Parameters

| Parameter | Level | Description | Units |
|-----------|-------|-------------|-------|
| `TMP` | 2m | Temperature at 2 meters | K |
| `RH` | 2m | Relative humidity at 2m | % |
| `UGRD` | 10m | U-component wind at 10m | m/s |
| `VGRD` | 10m | V-component wind at 10m | m/s |
| `GUST` | surface | Wind gust | m/s |
| `PRMSL` | MSL | Pressure reduced to MSL | Pa |
| `APCP` | surface | Total precipitation | kg/m² |
| `TCDC` | entire atmos | Total cloud cover | % |

### Upper-Air Parameters

| Parameter | Levels | Description |
|-----------|--------|-------------|
| `TMP` | 1000-10 mb | Temperature |
| `HGT` | 1000-10 mb | Geopotential height |
| `RH` | 1000-200 mb | Relative humidity |
| `UGRD/VGRD` | 1000-10 mb | Wind components |

**Standard Levels**: 1000, 975, 950, 925, 900, 850, 800, 750, 700, 650, 600, 550, 500, 450, 400, 350, 300, 250, 200, 150, 100, 70, 50, 30, 20, 10 mb

## Layer Names

Examples:
- `gfs_TMP_2m` - Surface temperature
- `gfs_UGRD_10m` - U-wind component at 10m
- `gfs_PRMSL` - Mean sea level pressure
- `gfs_HGT_500mb` - 500mb geopotential height

## Data Source

**NOMADS Server**:
```
https://nomads.ncep.noaa.gov/pub/data/nccf/com/gfs/prod/gfs.{YYYYMMDD}/{HH}/atmos/gfs.t{HH}z.pgrb2.0p25.f{FFF}
```

**Example**:
```
https://nomads.ncep.noaa.gov/pub/data/nccf/com/gfs/prod/gfs.20241203/00/atmos/gfs.t00z.pgrb2.0p25.f000
```

## File Sizes

- Per file: ~100-150 MB (compressed GRIB2)
- Per cycle (129 files): ~13 GB
- Per day (4 cycles): ~52 GB

## Download Script

```bash
./scripts/download_gfs.sh

# Downloads:
# - Latest GFS cycle (00, 06, 12, or 18 UTC)
# - Forecast hours 0-120 (every 3h)
# - Total: ~4 GB per run
```

## Typical Uses

- **Synoptic weather maps**: Surface pressure, fronts
- **Temperature forecasts**: Daily high/low temperatures
- **Precipitation forecasts**: Rain/snow accumulation
- **Upper-air analysis**: Jet stream, troughs/ridges
- **Aviation weather**: Winds aloft, turbulence
- **Marine forecasts**: Wave height, wind
