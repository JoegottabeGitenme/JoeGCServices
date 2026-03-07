# Data Sources

Weather WMS ingests data from multiple NOAA and research sources, each providing different types of weather and environmental information at various resolutions and update frequencies.

## Source Comparison

### Continuously Updated Sources

These sources are automatically downloaded and ingested on a schedule:

| Source | Type | Coverage | Resolution | Update | Parameters | Format |
|--------|------|----------|------------|--------|------------|--------|
| [GFS](./gfs.md) | Forecast Model | Global | 25 km | 6 hours | 129 | GRIB2 |
| [HRRR](./hrrr.md) | Forecast Model | CONUS | 3 km | 1 hour | 49 | GRIB2 |
| [MRMS](./mrms.md) | Radar | CONUS | 1 km | 2 min | 2 | GRIB2 |
| [GOES](./goes.md) | Satellite | Hemisphere | 0.5-2 km | 5-10 min | 16 | NetCDF |
| [NLDAS-2 Noah](./nldas.md) | Land Surface | CONUS | 12.5 km | 1 hour | 37 | NetCDF |
| [NLDAS-2 Forcing](./nldas.md) | Land Surface | CONUS | 12.5 km | 1 hour | 8 | NetCDF |
| [GLDAS-2.1 Noah](./gldas.md) | Land Surface | Global land | 25 km | 3 hours | 36 | NetCDF-4 |
| [GFS-Wave](./ocean.md#gfs-wave-global-wave-forecast) | Wave Forecast | Global ocean | 25 km | 6 hours | 6 | GRIB2 |
| [SST](./ocean.md#sst-sea-surface-temperature) | Observation | Global ocean | 9 km | Daily | 2 | NetCDF |
| [Sea Ice](./ocean.md#sea-ice) | Observation | Polar | 9 km | Daily | 1 | NetCDF |
| [NDBC](./ocean.md#ndbc-national-data-buoy-center) | Buoy Obs | US coastal | Point | 10 min | 12 | Text |
| [DART](./ocean.md#dart-deep-ocean-assessment-and-reporting-of-tsunamis) | Tsunami Buoy | Deep ocean | Point | 15 min | 2 | Text |

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

### Land Surface Models (LIS)

**NLDAS-2 and GLDAS-2.1** are land data assimilation systems driven by meteorological forcing:
- Soil moisture at multiple depth layers
- Soil temperature profiles
- Snowpack (water equivalent, depth)
- Energy fluxes (radiation, latent/sensible heat)
- Evapotranspiration components
- Requires NASA Earthdata Login for download

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

### Ocean Data

**GFS-Wave, NDBC, DART, SST, and Sea Ice** provide marine observations and forecasts:
- Wave height, period, and direction (GFS-Wave)
- Buoy observations: wind, waves, pressure, temperature (NDBC)
- Deep-ocean tsunami detection (DART)
- Daily sea surface temperature and sea ice concentration

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
| NLDAS-2 | 30 days | None | 30 days |
| GLDAS-2.1 | 30 days | None | 30 days (~33 day latency) |
| GFS-Wave | Current cycle | 0-384 hours | 16 days |
| NDBC | 2 hours | None | 2 hours |
| DART | 24 hours | None | 24 hours |
| SST | 7 days | None | 7 days |
| Sea Ice | 7 days | None | 7 days |

## Download Schedules

- **GFS**: Every 6 hours (00, 06, 12, 18 UTC)
- **HRRR**: Every hour
- **MRMS**: Every 2 minutes
- **GOES**: Continuous (every 5-15 minutes)
- **NLDAS-2**: Every hour (96-hour latency)
- **GLDAS-2.1**: Every 3 hours (33-day latency)
- **GFS-Wave**: Every 6 hours
- **NDBC**: Every 10 minutes
- **DART**: Every 15 minutes
- **SST / Sea Ice**: Daily

## Data Volume

**Daily ingestion** (approximate):
- GFS: ~13 GB per cycle x 4 = 52 GB/day
- HRRR: ~10 GB per hour x 24 = 240 GB/day
- MRMS: ~1 GB per hour x 24 = 24 GB/day
- GOES: ~3 GB per hour x 24 = 72 GB/day
- NLDAS-2 (both): ~312 MB/day
- GLDAS-2.1: ~176 MB/day
- GFS-Wave: ~2 GB/day
- Ocean obs (NDBC, DART, SST, Sea Ice): ~160 MB/day

**Total**: ~390 GB/day (with all sources enabled)

## Next Steps

Explore individual data sources:

- [GFS (Global Forecast System)](./gfs.md)
- [HRRR (High-Resolution Rapid Refresh)](./hrrr.md)
- [MRMS (Multi-Radar Multi-Sensor)](./mrms.md)
- [GOES (Geostationary Satellites)](./goes.md)
- [NLDAS-2 (Land Surface - CONUS)](./nldas.md)
- [GLDAS-2.1 (Land Surface - Global)](./gldas.md)
- [Ocean Data (GFS-Wave, NDBC, DART, SST, Sea Ice)](./ocean.md)
- [VIIRS (Nighttime Lights / Light Pollution)](./viirs.md)
