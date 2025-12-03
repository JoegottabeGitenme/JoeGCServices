# Model Configuration

Model configuration files define data sources, projections, and download schedules.

## File Location

`config/models/{model}.yaml`

## Example: GFS

```yaml
# config/models/gfs.yaml
name: gfs
description: Global Forecast System
source: NOAA NCEP
format: grib2

projection: latlon
resolution: 0.25  # degrees

update_frequency: 6h  # hours
forecast_hours: [0, 3, 6, 9, 12, ..., 384]

parameters:
  - TMP_2m
  - UGRD_10m
  - VGRD_10m
  - RH_2m
  - PRMSL

download:
  url_template: "https://nomads.ncep.noaa.gov/pub/data/nccf/com/gfs/prod/gfs.{date}/{cycle}/atmos/gfs.t{cycle}z.pgrb2.0p25.f{hour:03d}"
  schedule: "0 */6 * * *"  # Cron expression
  retention_hours: 168  # Keep for 7 days
```

## Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Model identifier |
| `description` | string | Human-readable name |
| `source` | string | Data provider |
| `format` | string | `grib2` or `netcdf` |
| `projection` | string | Projection type |
| `resolution` | float | Grid resolution |
| `update_frequency` | string | Update interval |
| `forecast_hours` | array | Available forecast hours |
| `parameters` | array | Available parameters |
| `download.url_template` | string | Download URL pattern |
| `download.schedule` | string | Cron schedule |
| `download.retention_hours` | int | Data retention period |

See existing files in `config/models/` for more examples.
