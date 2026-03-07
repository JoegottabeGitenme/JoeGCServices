# GLDAS-2.1 (Global Land Data Assimilation System)

NASA's global land surface model providing worldwide soil, snow, and energy balance data.

## Overview

- **Provider**: NASA GES DISC (Goddard Earth Sciences Data and Information Services Center)
- **Coverage**: Global land (-60°S to 90°N, 180°W to 180°E)
- **Resolution**: 0.25° (~25 km)
- **Grid Size**: 1440 x 600 points
- **Update Frequency**: 3-hourly (00, 03, 06, 09, 12, 15, 18, 21 UTC)
- **Data Latency**: ~33 days (792 hours) - Early Products
- **Format**: CF-compliant NetCDF-4 (.nc4)
- **Authentication**: NASA Earthdata Login required
- **Storage**: Zarr V3 with multi-resolution pyramids

## Product Details

**Product**: GLDAS-2.1 Noah 0.25° 3-Hourly Early Products (EP)  
**Product ID**: `GLDAS_NOAH025_3H_EP.2.1`  
**Model ID**: `gldas-noah`

The "Early Products" designation means data is available with ~33 day latency, before the final quality-controlled version. This provides a good balance between timeliness and accuracy for operational monitoring.

## GLDAS vs NLDAS

| Attribute | GLDAS-2.1 EP | NLDAS-2 |
|-----------|-------------|---------|
| **Coverage** | Global land | CONUS only |
| **Resolution** | 0.25° (~25 km) | 0.125° (~12.5 km) |
| **Temporal** | 3-hourly | Hourly |
| **Latency** | ~33 days | ~4 days |
| **Parameters** | 36 | 37 |
| **File size** | ~22 MB | ~6.5 MB |
| **Grid points** | 864,000 | 103,936 |

Use **NLDAS** for CONUS applications requiring higher resolution and lower latency. Use **GLDAS** for global coverage or regions outside North America.

## Available Parameters (36 total)

### Soil Moisture (5 parameters)

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| SoilMoi0_10cm_inst | `SoilMoi0_10cm_inst` | Soil moisture (0-10 cm) | kg/m² |
| SoilMoi10_40cm_inst | `SoilMoi10_40cm_inst` | Soil moisture (10-40 cm) | kg/m² |
| SoilMoi40_100cm_inst | `SoilMoi40_100cm_inst` | Soil moisture (40-100 cm) | kg/m² |
| SoilMoi100_200cm_inst | `SoilMoi100_200cm_inst` | Soil moisture (100-200 cm) | kg/m² |
| RootMoist_inst | `RootMoist_inst` | Root zone soil moisture | kg/m² |

### Soil Temperature (4 parameters)

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| SoilTMP0_10cm_inst | `SoilTMP0_10cm_inst` | Soil temperature (0-10 cm) | K |
| SoilTMP10_40cm_inst | `SoilTMP10_40cm_inst` | Soil temperature (10-40 cm) | K |
| SoilTMP40_100cm_inst | `SoilTMP40_100cm_inst` | Soil temperature (40-100 cm) | K |
| SoilTMP100_200cm_inst | `SoilTMP100_200cm_inst` | Soil temperature (100-200 cm) | K |

### Snow (2 parameters)

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| SWE_inst | `SWE_inst` | Snow water equivalent | kg/m² |
| SnowDepth_inst | `SnowDepth_inst` | Snow depth | m |

### Surface (3 parameters)

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| AvgSurfT_inst | `AvgSurfT_inst` | Average surface skin temperature | K |
| Albedo_inst | `Albedo_inst` | Surface albedo | % |
| CanopInt_inst | `CanopInt_inst` | Plant canopy surface water | kg/m² |

### Radiation Fluxes (4 parameters)

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| SWdown_f_tavg | `SWdown_f_tavg` | Downward shortwave radiation | W/m² |
| LWdown_f_tavg | `LWdown_f_tavg` | Downward longwave radiation | W/m² |
| Swnet_tavg | `Swnet_tavg` | Net shortwave radiation | W/m² |
| Lwnet_tavg | `Lwnet_tavg` | Net longwave radiation | W/m² |

### Heat Fluxes (3 parameters)

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| Qle_tavg | `Qle_tavg` | Latent heat flux | W/m² |
| Qh_tavg | `Qh_tavg` | Sensible heat flux | W/m² |
| Qg_tavg | `Qg_tavg` | Ground heat flux | W/m² |

### Water Fluxes (7 parameters)

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| Evap_tavg | `Evap_tavg` | Total evapotranspiration | kg/m²/s |
| Qs_acc | `Qs_acc` | Surface runoff (accumulated) | kg/m² |
| Qsb_acc | `Qsb_acc` | Subsurface runoff (accumulated) | kg/m² |
| Qsm_acc | `Qsm_acc` | Snowmelt (accumulated) | kg/m² |
| Snowf_tavg | `Snowf_tavg` | Snowfall rate | kg/m²/s |
| Rainf_tavg | `Rainf_tavg` | Rainfall rate | kg/m²/s |
| Rainf_f_tavg | `Rainf_f_tavg` | Forcing rainfall rate | kg/m²/s |

### ET Components (4 parameters)

| Parameter | CF Name | Description | Units |
|-----------|---------|-------------|-------|
| PotEvap_tavg | `PotEvap_tavg` | Potential evapotranspiration | W/m² |
| ECanop_tavg | `ECanop_tavg` | Canopy water evaporation | W/m² |
| Tveg_tavg | `Tveg_tavg` | Transpiration | W/m² |
| ESoil_tavg | `ESoil_tavg` | Bare soil evaporation | W/m² |

### Forcing Variables (4 parameters)

| Parameter | CF Name | Description | Units | Level |
|-----------|---------|-------------|-------|-------|
| Wind_f_inst | `Wind_f_inst` | Near-surface wind speed | m/s | 10 m |
| Tair_f_inst | `Tair_f_inst` | Near-surface air temperature | K | 2 m |
| Qair_f_inst | `Qair_f_inst` | Near-surface specific humidity | kg/kg | 2 m |
| Psurf_f_inst | `Psurf_f_inst` | Surface pressure | Pa | surface |

### Variable Naming Convention

GLDAS variable names include temporal suffixes indicating how the value was computed:

| Suffix | Meaning | Example |
|--------|---------|---------|
| `_inst` | Instantaneous value at the timestamp | `SWE_inst` |
| `_tavg` | Time-averaged over the 3-hour period | `Qle_tavg` |
| `_acc` | Accumulated over the 3-hour period | `Qs_acc` |

## Data Source

**NASA GES DISC**:
```
https://hydro1.gesdisc.eosdis.nasa.gov/data/GLDAS/GLDAS_NOAH025_3H_EP.2.1/{YYYY}/{DDD}/GLDAS_NOAH025_3H_EP.A{YYYYMMDD}.{HHMM}.021.nc4
```

**Example**:
```
https://hydro1.gesdisc.eosdis.nasa.gov/data/GLDAS/GLDAS_NOAH025_3H_EP.2.1/2026/010/GLDAS_NOAH025_3H_EP.A20260110.0000.021.nc4
```

### Authentication

Same as NLDAS - requires NASA Earthdata Login. See [NLDAS Authentication](./nldas.md#authentication) for setup instructions.

## File Sizes

- Per file: ~22 MB (NetCDF-4 with all 36 parameters)
- Per day (8 files): ~176 MB
- 30-day retention: ~5.3 GB raw + ~5 GB Zarr pyramids = ~10-11 GB

## Storage Estimate (Combined LIS)

| Model | Files/Day | Size/Day | 30-Day Total |
|-------|-----------|----------|-------------|
| NLDAS Noah | 24 | 156 MB | ~10 GB |
| NLDAS Forcing | 24 | 156 MB | ~10 GB |
| GLDAS Noah | 8 | 176 MB | ~10-11 GB |
| **Total** | **56** | **488 MB** | **~30 GB** |

## Configuration

**Model Config** (`config/models/gldas-noah.yaml`):
```yaml
model:
  id: gldas-noah
  name: "GLDAS-2.1 Noah Land Surface Model (EP)"
  enabled: true

source:
  type: nasa_gesdisc
  base_url: "https://hydro1.gesdisc.eosdis.nasa.gov/data/GLDAS/GLDAS_NOAH025_3H_EP.2.1"

grid:
  projection: geographic
  resolution: "0.25deg"
  bbox:
    min_lon: -179.875
    min_lat: -59.875
    max_lon: 179.875
    max_lat: 89.875

schedule:
  type: observation
  poll_interval_secs: 10800    # 3 hours
  delay_hours: 792             # ~33 days

retention:
  hours: 720                   # 30 days
```

**EDR Config** (`config/edr/gldas-noah.yaml`):

Defines 9 EDR collections: soil moisture, soil temperature, snow, surface variables, energy fluxes, water fluxes, ET components, and forcing variables.

## Typical Uses

- **Global drought monitoring**: Soil moisture anomalies across all continents
- **Water resources**: Runoff, baseflow, and evapotranspiration at global scale
- **Snow monitoring**: Global snowpack for climate and hydrological applications
- **Agricultural assessments**: Soil moisture and temperature outside North America
- **Land-atmosphere coupling**: Energy and water balance studies
- **Climate model validation**: Compare against CMIP outputs

## Related

- [NLDAS-2 (CONUS)](./nldas.md) - Higher-resolution CONUS counterpart
- [Downloader Service](../services/downloader.md) - Earthdata download path
- [Ingester Service](../services/ingester.md) - CF NetCDF ingestion
