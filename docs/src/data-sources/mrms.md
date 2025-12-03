# MRMS (Multi-Radar Multi-Sensor)

Real-time composite radar mosaic covering CONUS from 146 individual radar sites.

## Overview

- **Provider**: NOAA NSSL (National Severe Storms Laboratory)
- **Coverage**: CONUS
- **Resolution**: 1 km
- **Grid Size**: ~7000 × ~3500 points
- **Update Frequency**: Every 2 minutes
- **Latency**: ~2-3 minutes
- **Format**: GRIB2 (gzip compressed)

## Available Products

### Reflectivity

**`REFL` - MergedReflectivityQCComposite**

- Composite radar reflectivity (dBZ)
- Quality-controlled mosaic from all radars
- Vertical integration of strongest return

**Layer**: `mrms_REFL`

### Precipitation Rate

**`PRECIP_RATE` - PrecipRate**

- Surface precipitation rate (mm/hr)
- Derived from radar + gauge calibration
- Real-time rainfall intensity

**Layer**: `mrms_PRECIP_RATE`

## Reflectivity Scale

| dBZ | Intensity | Precipitation |
|-----|-----------|---------------|
| <20 | Light echo | Drizzle |
| 20-30 | Light rain | Light rain |
| 30-40 | Moderate rain | Moderate rain |
| 40-50 | Heavy rain | Heavy rain |
| 50-60 | Very heavy rain | Very heavy rain/hail |
| >60 | Extreme | Severe storms |

## Data Source

**MRMS Server**:
```
https://mrms.ncep.noaa.gov/data/2D/{PRODUCT}/MRMS_{PRODUCT}.{YYYYMMDD}-{HHMMSS}.grib2.gz
```

**Examples**:
```
https://mrms.ncep.noaa.gov/data/2D/MergedReflectivityQCComposite/MRMS_MergedReflectivityQCComposite.20241203-180000.grib2.gz
https://mrms.ncep.noaa.gov/data/2D/PrecipRate/MRMS_PrecipRate.20241203-180000.grib2.gz
```

## File Sizes

- Per file: ~10-30 MB (compressed)
- Per hour (30 files): ~300 MB
- Per day: ~7 GB

## Download Script

```bash
./scripts/download_mrms.sh

# Downloads recent MRMS composites
# Updates: Every 2 minutes
# Retention: Last 2 hours
```

## Typical Uses

- **Storm tracking**: Real-time severe weather monitoring
- **Rainfall estimation**: Flash flood warnings
- **Aviation**: Convective weather avoidance
- **Nowcasting**: 0-60 minute storm forecasts
- **Public alerts**: Tornado/severe thunderstorm warnings
