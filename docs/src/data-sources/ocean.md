# Ocean Data Sources

Weather WMS ingests several ocean-related data sources covering wave forecasts, buoy observations, sea surface temperature, and sea ice concentration.

## GFS-Wave (Global Wave Forecast)

NOAA's global wave model providing ocean wave forecasts.

### Overview

- **Provider**: NOAA NCEP
- **Coverage**: Global ocean
- **Resolution**: 0.25° (~25 km)
- **Update Frequency**: Every 6 hours (00, 06, 12, 18 UTC)
- **Forecast Range**: 0-384 hours
- **Format**: GRIB2 (selective download via index files)

### Parameters

| Parameter | Description | Units |
|-----------|-------------|-------|
| HTSGW | Significant wave height | m |
| PERPW | Primary wave period | s |
| DIRPW | Primary wave direction | degrees |
| WVHGT | Wind wave height | m |
| SWELL | Swell height | m |
| SWPER | Swell period | s |

### Data Source

```
https://noaa-gfs-bdp-pds.s3.amazonaws.com/gfs.{YYYYMMDD}/{HH}/wave/gridded/gfswave.t{HH}z.global.0p25.f{FFF}.grib2
```

---

## NDBC (National Data Buoy Center)

Real-time observations from NOAA's network of marine buoys.

### Overview

- **Provider**: NOAA NDBC
- **Coverage**: US coastal waters, Great Lakes, open ocean
- **Stations**: ~900 active buoys and coastal stations
- **Update Frequency**: Every 10 minutes (polling)
- **Data Type**: Point observations (EDR only, no WMS tiles)
- **Format**: Text (parsed latest observations)

### Parameters

| Parameter | Description | Units |
|-----------|-------------|-------|
| wave_height | Significant wave height | m |
| dominant_period | Dominant wave period | s |
| average_period | Average wave period | s |
| mean_wave_direction | Mean wave direction | degrees |
| wind_speed | Wind speed | m/s |
| wind_direction | Wind direction | degrees |
| wind_gust | Wind gust speed | m/s |
| air_pressure | Sea level pressure | hPa |
| air_temperature | Air temperature | °C |
| water_temperature | Sea surface temperature | °C |
| dewpoint | Dew point temperature | °C |
| visibility | Visibility | nmi |

### Data Source

```
https://www.ndbc.noaa.gov/data/latest_obs/latest_obs.txt
```

---

## DART (Deep-ocean Assessment and Reporting of Tsunamis)

Real-time deep-ocean pressure observations from NOAA's tsunami detection network.

### Overview

- **Provider**: NOAA NDBC / PMEL
- **Coverage**: Pacific, Atlantic, Indian, Caribbean basins
- **Stations**: ~60 active DART buoys
- **Update Frequency**: Every 15 minutes (polling)
- **Data Type**: Point observations (EDR only)
- **Format**: Text (realtime2 data files)

### Parameters

| Parameter | Description | Units |
|-----------|-------------|-------|
| water_column_height | Ocean bottom pressure (water column height) | m |
| water_temperature | Water temperature at sensor | °C |

### Operating Modes

DART buoys operate in two modes:
- **Standard mode**: Reports every 15 minutes
- **Event mode**: Triggered by seismic events, reports every 15 seconds to 1 minute

### Data Source

Station list:
```
https://www.ndbc.noaa.gov/activestations.xml
```

Per-station data:
```
https://www.ndbc.noaa.gov/data/realtime2/{STATION_ID}.dart
```

---

## SST (Sea Surface Temperature)

NOAA's daily global sea surface temperature analysis.

### Overview

- **Provider**: NOAA NESDIS
- **Coverage**: Global ocean
- **Resolution**: 0.083° (~9 km)
- **Update Frequency**: Daily
- **Format**: NetCDF

### Parameters

| Parameter | Description | Units |
|-----------|-------------|-------|
| analysed_sst | Sea surface temperature | K |
| analysis_error | Analysis error estimate | K |

---

## Sea Ice

Arctic and Antarctic sea ice concentration.

### Overview

- **Provider**: NOAA NESDIS
- **Coverage**: Global (polar regions)
- **Resolution**: 0.083° (~9 km)
- **Update Frequency**: Daily
- **Format**: NetCDF

### Parameters

| Parameter | Description | Units |
|-----------|-------------|-------|
| ice_concentration | Sea ice area fraction | fraction |

## Storage Estimates

| Source | Files/Day | Size/Day | Retention |
|--------|-----------|----------|-----------|
| GFS-Wave | ~130 | ~2 GB | 2 cycles |
| NDBC | Continuous | ~5 MB | 2 hours |
| DART | Continuous | ~2 MB | 24 hours |
| SST | 1 | ~100 MB | 7 days |
| Sea Ice | 1 | ~50 MB | 7 days |

## Related

- [GFS (Global Forecast System)](./gfs.md) - Parent model for GFS-Wave
- [EDR API](../services/edr-api.md) - Point data access for buoy observations
- [Downloader Service](../services/downloader.md) - Download scheduling
