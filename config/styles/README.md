# Weather WMS Style Configuration

This directory contains JSON style configuration files that define how weather data is visualized in the WMS/WMTS tile rendering pipeline.

## Quick Start

1. Copy an existing style file as a template
2. Modify the color stops and parameters
3. Validate your changes: `python3 validate_styles.py`
4. Restart the WMS API service to load the new style

## File Structure

Each JSON file contains one or more styles for a specific data category:

```
config/styles/
├── README.md              # This file
├── schema.example.json    # Reference schema with all options documented
├── validate_styles.py     # Validation script
├── temperature.json       # Temperature visualization styles
├── wind.json              # Wind speed gradient styles
├── wind_barbs.json        # Wind barb symbol styles
├── reflectivity.json      # Radar reflectivity styles
├── precipitation.json     # Precipitation styles
└── ...
```

## Style Types

### 1. Gradient (`type: "gradient"`)

Maps continuous data values to colors via linear interpolation. Best for temperature, humidity, wind speed, etc.

```json
{
  "type": "gradient",
  "stops": [
    { "value": 233.15, "color": "#0000FF", "label": "-40C" },
    { "value": 273.15, "color": "#00FF00", "label": "0C" },
    { "value": 313.15, "color": "#FF0000", "label": "40C" }
  ],
  "interpolation": "linear"
}
```

### 2. Filled Contour (`type: "filled_contour"`)

Discrete color bands between threshold values. Best for radar reflectivity, flight categories, etc.

```json
{
  "type": "filled_contour",
  "stops": [
    { "value": -10, "color": "#00000000", "label": "<-10" },
    { "value": 20, "color": "#00FF00", "label": "20 dBZ" },
    { "value": 40, "color": "#FFFF00", "label": "40 dBZ" },
    { "value": 60, "color": "#FF0000", "label": "60 dBZ" }
  ]
}
```

### 3. Contour Lines (`type: "contour"`)

Isolines at regular intervals. Best for pressure, geopotential height, etc.

```json
{
  "type": "contour",
  "contour": {
    "interval": 4,
    "line_width": 1.5,
    "line_color": "#000000",
    "labels": true
  }
}
```

### 4. Wind Barbs (`type: "wind_barbs"`)

Traditional meteorological wind barb symbols.

```json
{
  "type": "wind_barbs",
  "wind": {
    "spacing": 50,
    "size": 25.0,
    "line_width": 1.5,
    "color": "#000000"
  }
}
```

### 5. Wind Arrows (`type: "wind_arrows"`)

Directional arrows optionally colored by speed.

```json
{
  "type": "wind_arrows",
  "wind": {
    "spacing": 40,
    "min_length": 5.0,
    "max_length": 40.0
  },
  "color_by_speed": {
    "enabled": true,
    "stops": [
      { "value": 0, "color": "#808080" },
      { "value": 20, "color": "#FF0000" }
    ]
  }
}
```

## Data Transforms

Use transforms to convert data units before color mapping:

| Transform | Description | Example Use |
|-----------|-------------|-------------|
| `none` | No transformation (default) | Data already in display units |
| `linear` | `output = input * scale + offset` | Custom conversions |
| `k_to_c` | Kelvin to Celsius (`- 273.15`) | Temperature display |
| `pa_to_hpa` | Pascals to hPa (`/ 100`) | Pressure display |
| `mps_to_knots` | m/s to knots (`* 1.94384`) | Wind speed display |
| `m_to_km` | Meters to km (`/ 1000`) | Visibility display |

Example:
```json
{
  "transform": {
    "type": "k_to_c"
  },
  "stops": [
    { "value": -40, "color": "#0000FF", "label": "-40C" },
    { "value": 40, "color": "#FF0000", "label": "40C" }
  ]
}
```

## Color Formats

- **Hex RGB**: `#RRGGBB` (e.g., `#FF0000` for red)
- **Hex RGBA**: `#RRGGBBAA` (e.g., `#FF000080` for 50% transparent red)
- **Transparent**: The string `"transparent"` for fully transparent

## Complete Style File Structure

```json
{
  "version": "1.0",
  "metadata": {
    "name": "My Custom Styles",
    "description": "Description of these styles"
  },
  "styles": {
    "default": {
      "name": "Default Style Name",
      "description": "What this style shows",
      "type": "gradient",
      "units": "K",
      "range": { "min": 233.15, "max": 313.15 },
      "transform": { "type": "none" },
      "stops": [
        { "value": 233.15, "color": "#0000FF", "label": "-40C" },
        { "value": 313.15, "color": "#FF0000", "label": "40C" }
      ],
      "interpolation": "linear",
      "out_of_range": "clamp",
      "legend": {
        "title": "Temperature",
        "labels": ["-40C", "0C", "40C"]
      }
    },
    "alternate": {
      "...": "another style variant"
    }
  }
}
```

## Validation

Always validate your style files before deploying:

```bash
# Validate all style files
python3 validate_styles.py

# Verbose output
python3 validate_styles.py --verbose
```

The validation script checks:
- Valid JSON syntax
- Required fields present
- Valid style types and options
- Correct color formats
- Proper numeric values

## How Styles are Used

### In WMS Requests

Styles are referenced in WMS GetMap requests:

**Forecast model example (GFS, HRRR):**
```
/wms?SERVICE=WMS&REQUEST=GetMap
    &LAYERS=gfs_TMP
    &STYLES=default
    &RUN=2024-01-15T12:00:00Z
    &FORECAST=6
    &ELEVATION=2 m above ground
    &CRS=EPSG:3857
    &BBOX=-20037508,-20037508,20037508,20037508
    &WIDTH=256&HEIGHT=256
    &FORMAT=image/png
```

**Observation data example (GOES, MRMS):**
```
/wms?SERVICE=WMS&REQUEST=GetMap
    &LAYERS=goes16_CMI_C13
    &STYLES=default
    &TIME=2024-01-15T18:00:00Z
    &CRS=EPSG:3857
    &BBOX=-20037508,-20037508,20037508,20037508
    &WIDTH=256&HEIGHT=256
    &FORMAT=image/png
```

**Key parameters:**
- `LAYERS` = `{model}_{parameter}` (underscore-separated, e.g., `gfs_TMP`, `hrrr_REFC`)
- `STYLES` = Style name from the style config (e.g., `default`, `celsius`, `enhanced`)
- `ELEVATION` = Vertical level (e.g., `2 m above ground`, `500 mb`, `surface`)

**Dimension parameters (mutually exclusive):**

| Layer Type | Dimensions | Example |
|------------|------------|---------|
| **Forecast models** (GFS, HRRR) | `RUN` + `FORECAST` | `RUN=2024-01-15T12:00:00Z&FORECAST=6` |
| **Observation data** (GOES, MRMS) | `TIME` | `TIME=2024-01-15T18:00:00Z` |

- `RUN` = Model initialization time (ISO8601, or `latest`)
- `FORECAST` = Hours ahead from the run time (integer: `0`, `6`, `12`, etc.)
- `TIME` = Observation timestamp (ISO8601)

The style is automatically loaded from the appropriate JSON file based on the parameter type (temperature, wind, reflectivity, etc.).

### In the Rendering Pipeline

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Grid Data   │────►│   Transform  │────►│ Color Mapper │────► PNG Tile
│  (raw vals)  │     │  (units)     │     │  (style)     │
└──────────────┘     └──────────────┘     └──────────────┘
                            │                    │
                            ▼                    ▼
                     transform config      stops/colors
                     from style JSON       from style JSON
```

1. **Grid data** is loaded from storage (GRIB2, NetCDF)
2. **Transform** converts units if specified (e.g., K → °C)
3. **Color mapper** interpolates colors based on stops
4. **PNG encoder** outputs the final tile image

## Adding a New Style

1. **Identify the parameter**: What data will this style visualize?

2. **Choose a style type**: 
   - Continuous data → `gradient`
   - Categorical/threshold data → `filled_contour`
   - Vector data → `wind_barbs` or `wind_arrows`
   - Isolines → `contour`

3. **Design color stops**: Consider colorblind accessibility and meteorological conventions

4. **Create the JSON file** or add to existing category file

5. **Validate**: `python3 validate_styles.py`

6. **Test**: Request a tile with your new style and verify rendering

## See Also

- [schema.example.json](./schema.example.json) - Complete schema reference with all options
- [Style Configuration Docs](https://your-docs-url/configuration/styles.html) - Full documentation
- [Renderer Crate](https://your-docs-url/crates/renderer.html) - Implementation details
