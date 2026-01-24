# Data Sources

Weather WMS ingests data from multiple NOAA and research sources, each providing different types of weather and environmental information at various resolutions and update frequencies.

## Source Comparison

### Continuously Updated Sources

These sources are automatically downloaded and ingested on a schedule:

| Source | Type | Coverage | Resolution | Update | Parameters | Format |
|--------|------|----------|------------|--------|------------|--------|
| [GFS](./gfs.md) | Model | Global | 25 km | 6 hours | 129 | GRIB2 |
| [HRRR](./hrrr.md) | Model | CONUS | 3 km | 1 hour | 49 | GRIB2 |
| [MRMS](./mrms.md) | Radar | CONUS | 1 km | 2 min | 2 | GRIB2 |
| [GOES](./goes.md) | Satellite | Hemisphere | 0.5-2 km | 5-10 min | 16 | NetCDF |

### Static Data Sources

These sources require manual download and ingestion:

| Source | Type | Coverage | Resolution | Update | Parameters | Format |
|--------|------|----------|------------|--------|------------|--------|
| [VIIRS](./viirs.md) | Light Pollution | Global | 500 m | Annual | 1 | GeoTIFF |

Static data differs from continuously updated sources:
- **Manual ingestion**: Use `./scripts/deploy-remote.sh --ingest-viirs`
- **No time dimension**: Always returns the latest annual composite
- **Large file size**: VIIRS is ~300-400 MB compressed

## Data Types

### Numerical Weather Prediction (NWP)

**GFS and HRRR** are numerical models that simulate atmospheric physics:
- Temperature, pressure, humidity
- Wind speed and direction  
- Precipitation, cloud cover
- Forecast hours: 0-384 (GFS), 0-48 (HRRR)

### Radar Observations

**MRMS** provides real-time radar observations:
- Reflectivity (storm intensity)
- Precipitation rate
- Composite from 146 radar sites
- Real-time, no forecast

### Satellite Observations

**GOES** provides geostationary satellite imagery:
- Visible, infrared, water vapor channels
- Cloud-top temperature
- Full-disk imagery every 10-15 minutes
- Real-time, no forecast

## Layer Naming Convention

Layers follow the pattern: `{model}_{parameter}_{level}`

**Examples**:
- `gfs_TMP_2m` - GFS temperature at 2 meters
- `hrrr_REFL` - HRRR radar reflectivity composite
- `goes18_CMI_C13` - GOES-18 channel 13 (infrared)
- `mrms_PRECIP_RATE` - MRMS precipitation rate

## Temporal Coverage

| Source | History | Forecast | Total Range |
|--------|---------|----------|-------------|
| GFS | Current cycle | 0-384 hours | 16 days |
| HRRR | Current cycle | 0-48 hours | 2 days |
| MRMS | 2 hours | None | 2 hours |
| GOES | 2 hours | None | 2 hours |

## Download Schedules

- **GFS**: Every 6 hours (00, 06, 12, 18 UTC)
- **HRRR**: Every hour
- **MRMS**: Every 2 minutes
- **GOES**: Continuous (every 5-15 minutes)

## Data Volume

**Daily ingestion** (approximate):
- GFS: ~13 GB per cycle × 4 = 52 GB/day
- HRRR: ~10 GB per hour × 24 = 240 GB/day
- MRMS: ~1 GB per hour × 24 = 24 GB/day
- GOES: ~3 GB per hour × 24 = 72 GB/day

**Total**: ~400 GB/day (with all sources enabled)

## Next Steps

Explore individual data sources:

- [GFS (Global Forecast System)](./gfs.md)
- [HRRR (High-Resolution Rapid Refresh)](./hrrr.md)
- [MRMS (Multi-Radar Multi-Sensor)](./mrms.md)
- [GOES (Geostationary Satellites)](./goes.md)
- [VIIRS (Nighttime Lights / Light Pollution)](./viirs.md)
