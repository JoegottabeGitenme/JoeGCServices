# NLDAS-2 (North American Land Data Assimilation System)

NASA's land surface model providing high-resolution soil, vegetation, and energy balance data over CONUS.

## Overview

- **Provider**: NASA GES DISC (Goddard Earth Sciences Data and Information Services Center)
- **Coverage**: CONUS (25°N to 53°N, 125°W to 67°W)
- **Resolution**: 0.125° (~12.5 km)
- **Grid Size**: 464 x 224 points
- **Update Frequency**: Hourly
- **Data Latency**: ~4 days (96 hours)
- **Format**: CF-compliant NetCDF (.nc)
- **Authentication**: NASA Earthdata Login required
- **Storage**: Zarr V3 with multi-resolution pyramids

## Products

Weather WMS ingests two NLDAS-2 products:

### NLDAS-2 Noah Land Surface Model (`nldas-noah`)

The Noah LSM simulates land surface processes including soil moisture, soil temperature, snowpack, and energy fluxes.

**Product ID**: `NLDAS_NOAH0125_H.2.0`

### NLDAS-2 Forcing (`nldas-forcing`)

The forcing dataset provides the meteorological inputs that drive the Noah model: near-surface temperature, humidity, wind, pressure, radiation, and precipitation.

**Product ID**: `NLDAS_FORA0125_H.2.0`

## Available Parameters

### Noah Land Surface Model

#### Soil Moisture

| Parameter | CF Name | Description | Units | Levels |
|-----------|---------|-------------|-------|--------|
| SoilM_0_10cm | `SOILM_0-10cm` | Soil moisture (0-10 cm) | kg/m² | 0-10 cm |
| SoilM_10_40cm | `SOILM_10-40cm` | Soil moisture (10-40 cm) | kg/m² | 10-40 cm |
| SoilM_40_100cm | `SOILM_40-100cm` | Soil moisture (40-100 cm) | kg/m² | 40-100 cm |
| SoilM_100_200cm | `SOILM_100-200cm` | Soil moisture (100-200 cm) | kg/m² | 100-200 cm |
| SoilM_0_100cm | `SOILM_0-100cm` | Soil moisture (0-100 cm, aggregate) | kg/m² | 0-100 cm |
| SoilM_0_200cm | `SOILM_0-200cm` | Soil moisture (0-200 cm, aggregate) | kg/m² | 0-200 cm |

#### Soil Temperature

| Parameter | CF Name | Description | Units | Levels |
|-----------|---------|-------------|-------|--------|
| SoilTMP_0_10cm | `SOILT_0-10cm` | Soil temperature (0-10 cm) | K | 0-10 cm |
| SoilTMP_10_40cm | `SOILT_10-40cm` | Soil temperature (10-40 cm) | K | 10-40 cm |
| SoilTMP_40_100cm | `SOILT_40-100cm` | Soil temperature (40-100 cm) | K | 40-100 cm |
| SoilTMP_100_200cm | `SOILT_100-200cm` | Soil temperature (100-200 cm) | K | 100-200 cm |

#### Surface & Snow

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| AvgSurfT | `AVSFT` | Average surface skin temperature | K |
| Albedo | `ALBDO` | Surface albedo | % |
| SWE | `WEASD` | Snow water equivalent | kg/m² |
| SnowDepth | `SNOD` | Snow depth | m |
| SnowFrac | `SNOWC` | Snow cover fraction | fraction |
| CanopInt | `CNWAT` | Plant canopy surface water | kg/m² |

#### Energy Fluxes

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| SWnet | `DSWRF` | Net shortwave radiation | W/m² |
| LWnet | `DLWRF` | Net longwave radiation | W/m² |
| Qle | `LHTFL` | Latent heat flux | W/m² |
| Qh | `SHTFL` | Sensible heat flux | W/m² |
| Qg | `GFLUX` | Ground heat flux | W/m² |

### Forcing Parameters

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| TMP | `TMP` | Near-surface air temperature (2m) | K |
| SPFH | `SPFH` | Near-surface specific humidity (2m) | kg/kg |
| PRES | `PRES` | Surface pressure | Pa |
| UGRD | `UGRD` | Near-surface U-wind (10m) | m/s |
| VGRD | `VGRD` | Near-surface V-wind (10m) | m/s |
| DLWRF | `DLWRF` | Downward longwave radiation | W/m² |
| DSWRF | `DSWRF` | Downward shortwave radiation | W/m² |
| APCP | `APCP` | Total precipitation | kg/m² |

## WMS Dimensions

NLDAS layers use observation dimensions (no forecast hours):

| Dimension | Description | Example |
|-----------|-------------|---------|
| `TIME` | Observation time | `2026-02-01T12:00:00Z` |
| `ELEVATION` | Depth below surface or surface | `0-10 cm depth`, `surface` |

## Data Source

**NASA GES DISC**:
```
https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/NLDAS_NOAH0125_H.2.0/{YYYY}/{DDD}/NLDAS_NOAH0125_H.A{YYYYMMDD}.{HH}00.020.nc
```

**Example**:
```
https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/NLDAS_NOAH0125_H.2.0/2026/032/NLDAS_NOAH0125_H.A20260201.1200.020.nc
```

### Authentication

NLDAS data requires NASA Earthdata Login credentials. The downloader handles the OAuth2 redirect-based authentication automatically:

1. Initial request to GES DISC is redirected to `urs.earthdata.nasa.gov`
2. HTTP Basic auth with Earthdata username/password
3. Redirect back to GES DISC with authentication cookie
4. File download proceeds

Configure credentials via environment variables:
```bash
EARTHDATA_USERNAME=your_username
EARTHDATA_PASSWORD=your_password
```

Register at [https://urs.earthdata.nasa.gov](https://urs.earthdata.nasa.gov) and authorize the "NASA GESDISC DATA ARCHIVE" application.

## File Sizes

- Per file: ~6.5 MB (NetCDF with all parameters)
- Per day (24 files): ~156 MB
- 30-day retention: ~4.7 GB raw + ~5 GB Zarr pyramids = ~10 GB

## Configuration

**Model Config** (`config/models/nldas-noah.yaml`):
```yaml
model:
  id: nldas-noah
  name: "NLDAS-2 Noah Land Surface Model"
  enabled: true

source:
  type: nasa_gesdisc
  base_url: "https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/NLDAS_NOAH0125_H.2.0"

grid:
  projection: geographic
  resolution: "0.125deg"
  bbox:
    min_lon: -124.9375
    min_lat: 25.0625
    max_lon: -67.0625
    max_lat: 52.9375

schedule:
  type: observation
  poll_interval_secs: 3600
  delay_hours: 96

retention:
  hours: 720
```

**EDR Config** (`config/edr/nldas-noah.yaml`):

Defines EDR collections for soil moisture (6 depth layers), soil temperature (4 layers), snow, surface variables, energy fluxes, water fluxes, and vegetation.

## Typical Uses

- **Agricultural monitoring**: Soil moisture at multiple depths for crop water stress
- **Drought monitoring**: Root zone soil moisture trends
- **Hydrological modeling**: Runoff, baseflow, evapotranspiration
- **Snow monitoring**: Snowpack water equivalent and depth
- **Energy balance studies**: Surface radiation and heat fluxes
- **Climate studies**: Long-term land surface trends (data available from 1979)

## Related

- [GLDAS-2.1 (Global)](./gldas.md) - Global counterpart at 0.25° resolution
- [Downloader Service](../services/downloader.md) - Earthdata download path
- [Ingester Service](../services/ingester.md) - CF NetCDF ingestion
- [EDR Configuration](../configuration/edr.md) - EDR collection setup
