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

**`REFL` - SeamlessHSR (Seamless Hybrid Scan Reflectivity)**

- Fully composited radar mosaic covering CONUS
- Eliminates individual radar coverage circles
- Optimized 3D compositing for best representation at each point
- Same continuous display as Windy, NWS, and other weather services

**Layer**: `mrms_REFL`

> **Note**: Earlier versions used `MergedReflectivityQC_00.50` which showed individual radar circles. SeamlessHSR (`SeamlessHSR_00.00`) provides a true nationwide mosaic.

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

**AWS S3 Bucket** (primary):
```
s3://noaa-mrms-pds/CONUS/{PRODUCT}/{YYYYMMDD}/MRMS_{PRODUCT}.{YYYYMMDD}-{HHMMSS}.grib2.gz
```

**Examples**:
```
s3://noaa-mrms-pds/CONUS/SeamlessHSR_00.00/20241203/MRMS_SeamlessHSR_00.00.20241203-180000.grib2.gz
s3://noaa-mrms-pds/CONUS/PrecipRate_00.00/20241203/MRMS_PrecipRate_00.00.20241203-180000.grib2.gz
```

**MRMS HTTP Server** (alternative):
```
https://mrms.ncep.noaa.gov/data/2D/{PRODUCT}/MRMS_{PRODUCT}.{YYYYMMDD}-{HHMMSS}.grib2.gz
```

## MRMS Product Reference

| Product | Description | Best For |
|---------|-------------|----------|
| `SeamlessHSR_00.00` | Fully merged reflectivity mosaic | General radar display |
| `MergedReflectivityQC_XX.XX` | Single elevation scan at XX.XX° | Raw radar analysis |
| `PrecipRate_00.00` | Instantaneous rain rate (mm/hr) | Real-time intensity |
| `MultiSensor_QPE_01H_Pass2` | 1-hour precipitation total | Short-term accumulation |
| `MultiSensor_QPE_24H_Pass2` | 24-hour precipitation total | Daily rainfall totals |

## Sentinel Values

MRMS uses `-999` as a sentinel value for missing/invalid data. During ingestion, these values are converted to NaN to ensure proper rendering. The valid data range for reflectivity is typically -30 to 80 dBZ.

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
