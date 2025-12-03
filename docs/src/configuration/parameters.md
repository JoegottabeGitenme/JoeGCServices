# Parameter Tables

Parameter tables map GRIB2 discipline/category/number codes to parameter names.

## File Location

`config/parameters/{table}.yaml`

## Example: GRIB2 NCEP Table

```yaml
# config/parameters/grib2_ncep.yaml
parameters:
  - discipline: 0
    category: 0
    number: 0
    abbrev: TMP
    name: Temperature
    units: K
    
  - discipline: 0
    category: 2
    number: 2
    abbrev: UGRD
    name: U-component of wind
    units: m/s
    
  - discipline: 0
    category: 2
    number: 3
    abbrev: VGRD
    name: V-component of wind
    units: m/s
```

## Fields

| Field | Description |
|-------|-------------|
| `discipline` | GRIB2 discipline (0=meteorological) |
| `category` | Parameter category |
| `number` | Parameter number within category |
| `abbrev` | Short name (e.g., TMP, UGRD) |
| `name` | Long description |
| `units` | Physical units |

## Common Parameters

| Discipline | Category | Number | Abbrev | Name |
|------------|----------|--------|--------|------|
| 0 | 0 | 0 | TMP | Temperature |
| 0 | 1 | 1 | RH | Relative Humidity |
| 0 | 2 | 2 | UGRD | U-wind |
| 0 | 2 | 3 | VGRD | V-wind |
| 0 | 3 | 0 | PRES | Pressure |
| 0 | 3 | 1 | PRMSL | Pressure (MSL) |

See [WMO GRIB2 Tables](https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/) for complete reference.
