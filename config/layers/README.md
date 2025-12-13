# Layer Configuration

This directory contains WMS/WMTS layer definitions for each data model. These configs define what layers are exposed in GetCapabilities and how they map to styles.

## File Format

Each YAML file defines layers for a specific model:

```yaml
model: gfs                              # Model ID (matches catalog)
display_name: "GFS - Global Forecast"   # Human-readable name

default_bbox:                           # Default bounding box (can override per layer)
  west: -180.0
  south: -90.0
  east: 180.0
  north: 90.0

dimension_type: forecast                # "forecast" or "observation"

layers:
  - id: gfs_TMP                         # Layer ID in WMS/WMTS (model_PARAM)
    parameter: TMP                      # Parameter code in data files
    title: "Temperature"                # Display title
    abstract: "Air temperature..."      # Description
    style_file: temperature.json        # Style file in config/styles/
    units:
      native: K                         # Units in source data
      display: "°C"                     # Display units
      conversion: K_to_C                # Conversion function (optional)
    levels:                             # Available elevation levels
      - value: "2 m above ground"
        default: true                   # Default level
      - value: "850 mb"
      - value: "500 mb"
```

## Layer Properties

| Property | Required | Description |
|----------|----------|-------------|
| `id` | Yes | Unique layer identifier (typically `{model}_{parameter}`) |
| `parameter` | Yes | Parameter code matching GRIB2/NetCDF data |
| `title` | Yes | Human-readable title for GetCapabilities |
| `abstract` | No | Longer description |
| `style_file` | Yes | Name of style file in `config/styles/` |
| `units.native` | Yes | Native data units |
| `units.display` | No | Display units for legends |
| `units.conversion` | No | Unit conversion function name |
| `levels` | No | Available vertical levels |
| `composite` | No | True if layer combines multiple parameters |
| `requires` | No | Required parameters for composite layers |
| `accumulation` | No | True for accumulated values (precipitation) |

## Style File Reference

The `style_file` property references a JSON file in `config/styles/`. The style file contains all available visualization styles for the layer:

```json
{
  "styles": {
    "default": { ... },
    "gradient": { ... },
    "isolines": { ... },
    "numbers": { ... }
  }
}
```

All styles defined in the file are automatically exposed in WMS/WMTS GetCapabilities.

## Files

| File | Model | Coverage | Type |
|------|-------|----------|------|
| `gfs.yaml` | GFS | Global | Forecast |
| `hrrr.yaml` | HRRR | CONUS | Forecast |
| `mrms.yaml` | MRMS | CONUS | Observation |
| `goes16.yaml` | GOES-16 | East | Observation |
| `goes18.yaml` | GOES-18 | West | Observation |

## Unit Conversions

Supported conversion functions:
- `K_to_C` - Kelvin to Celsius (subtract 273.15)
- `Pa_to_hPa` - Pascals to hectopascals (divide by 100)
- `m_to_km` - Meters to kilometers (divide by 1000)
- `m_to_kft` - Meters to kilofeet (divide by 304.8)
