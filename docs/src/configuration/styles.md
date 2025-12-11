# Style Configuration

Style files define how weather data is visualized as colored tile images. Each style maps numeric data values to colors, enabling the WMS/WMTS service to render meaningful weather maps.

## File Location

Style configuration files are located in:

```
config/styles/
├── schema.example.json    # Reference schema with all options
├── validate_styles.py     # Validation script
├── temperature.json       # Temperature styles
├── wind.json              # Wind speed styles  
├── wind_barbs.json        # Wind barb styles
├── reflectivity.json      # Radar reflectivity styles
├── precipitation.json     # Precipitation styles
├── humidity.json          # Humidity styles
├── atmospheric.json       # Pressure, height, etc.
├── cloud.json             # Cloud cover styles
├── cape.json              # CAPE/convective styles
├── goes_ir.json           # GOES infrared satellite
├── goes_visible.json      # GOES visible satellite
└── ...
```

## Style File Structure

Each JSON file can contain multiple style variants:

```json
{
  "version": "1.0",
  "metadata": {
    "name": "Temperature Styles",
    "description": "Temperature visualization styles"
  },
  "styles": {
    "default": {
      "name": "Temperature",
      "type": "gradient",
      "units": "K",
      "stops": [
        { "value": 233.15, "color": "#0000FF", "label": "-40C" },
        { "value": 273.15, "color": "#00FF00", "label": "0C" },
        { "value": 313.15, "color": "#FF0000", "label": "40C" }
      ]
    },
    "celsius": {
      "name": "Temperature (Celsius)",
      "type": "gradient",
      "transform": { "type": "k_to_c" },
      "stops": [
        { "value": -40, "color": "#0000FF" },
        { "value": 0, "color": "#00FF00" },
        { "value": 40, "color": "#FF0000" }
      ]
    }
  }
}
```

## Style Types

### Gradient

Continuous color interpolation between stops. Use for temperature, humidity, wind speed, etc.

```json
{
  "type": "gradient",
  "stops": [
    { "value": 233.15, "color": "#1E0082", "label": "-40C" },
    { "value": 253.15, "color": "#0096FF", "label": "-20C" },
    { "value": 273.15, "color": "#96FFC8", "label": "0C" },
    { "value": 293.15, "color": "#FF9600", "label": "20C" },
    { "value": 313.15, "color": "#960000", "label": "40C" }
  ],
  "interpolation": "linear",
  "out_of_range": "clamp"
}
```

**Options:**
- `interpolation`: `"linear"` (smooth), `"step"` (discrete), `"nearest"`
- `out_of_range`: `"clamp"` (use edge color), `"extend"` (extrapolate), `"transparent"`

### Filled Contour

Discrete color bands between thresholds. Use for radar reflectivity, flight categories, etc.

```json
{
  "type": "filled_contour",
  "stops": [
    { "value": -10, "color": "#00000000", "label": "None" },
    { "value": 20, "color": "#00FF00", "label": "20 dBZ" },
    { "value": 40, "color": "#FFFF00", "label": "40 dBZ" },
    { "value": 60, "color": "#FF0000", "label": "60 dBZ" }
  ],
  "out_of_range": "transparent"
}
```

### Contour Lines

Isolines at regular intervals. Use for pressure, geopotential height, etc.

```json
{
  "type": "contour",
  "contour": {
    "interval": 4,
    "base": 1000,
    "min_value": 900,
    "max_value": 1100,
    "line_width": 1.5,
    "line_color": "#000000",
    "major_interval": 20,
    "major_line_width": 2.5,
    "labels": true,
    "label_font_size": 10.0
  }
}
```

### Wind Barbs

Traditional meteorological wind barb symbols.

```json
{
  "type": "wind_barbs",
  "transform": { "type": "mps_to_knots" },
  "wind": {
    "spacing": 50,
    "size": 25.0,
    "line_width": 1.5,
    "color": "#000000",
    "direction_from": true,
    "calm_threshold": 2.5
  }
}
```

**Optional color by speed:**
```json
{
  "type": "wind_barbs",
  "wind": { "spacing": 50, "size": 25.0, "line_width": 1.5 },
  "color_by_speed": {
    "enabled": true,
    "stops": [
      { "value": 0, "color": "#808080" },
      { "value": 25, "color": "#00FF00" },
      { "value": 50, "color": "#FFFF00" },
      { "value": 75, "color": "#FF0000" }
    ]
  }
}
```

### Wind Arrows

Directional arrows scaled and colored by speed.

```json
{
  "type": "wind_arrows",
  "wind": {
    "spacing": 40,
    "min_length": 5.0,
    "max_length": 40.0,
    "line_width": 1.5
  },
  "color_by_speed": {
    "enabled": true,
    "stops": [
      { "value": 0, "color": "#808080" },
      { "value": 30, "color": "#FF0000" }
    ]
  }
}
```

## Data Transforms

Transforms convert data units before color mapping:

| Transform | Formula | Use Case |
|-----------|---------|----------|
| `none` | `output = input` | Default, no conversion |
| `linear` | `output = input * scale + offset` | Custom conversions |
| `k_to_c` | `output = input - 273.15` | Kelvin to Celsius |
| `pa_to_hpa` | `output = input / 100` | Pascals to hectoPascals |
| `mps_to_knots` | `output = input * 1.94384` | m/s to knots |
| `m_to_km` | `output = input / 1000` | Meters to kilometers |

**Example with linear transform:**
```json
{
  "transform": {
    "type": "linear",
    "scale": 0.001,
    "offset": 0
  }
}
```

## Color Formats

- **Hex RGB**: `#RRGGBB` (e.g., `#FF0000` for red)
- **Hex RGBA**: `#RRGGBBAA` (e.g., `#FF000080` for 50% transparent red)
- **Transparent**: Use `"transparent"` for fully transparent

## Using Styles in WMS Requests

The `STYLES` parameter in WMS requests specifies which style to use.

**Forecast model example (GFS, HRRR):**
```
/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap
    &LAYERS=gfs_TMP
    &STYLES=celsius
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
/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap
    &LAYERS=goes16_CMI_C13
    &STYLES=default
    &TIME=2024-01-15T18:00:00Z
    &CRS=EPSG:3857
    &BBOX=-20037508,-20037508,20037508,20037508
    &WIDTH=256&HEIGHT=256
    &FORMAT=image/png
```

**Key parameters:**

| Parameter | Format | Example |
|-----------|--------|---------|
| `LAYERS` | `{model}_{parameter}` | `gfs_TMP`, `hrrr_REFC`, `goes16_CMI_C13` |
| `STYLES` | Style name | `default`, `celsius`, `enhanced` |
| `ELEVATION` | Vertical level | `2 m above ground`, `500 mb`, `surface` |

**Dimension parameters (use based on layer type):**

| Layer Type | Dimensions | Description |
|------------|------------|-------------|
| **Forecast models** (GFS, HRRR) | `RUN` + `FORECAST` | Model run time + forecast hours ahead |
| **Observation data** (GOES, MRMS) | `TIME` | Observation timestamp |

- `RUN` = Model initialization time (ISO8601 timestamp, or `latest` for most recent)
- `FORECAST` = Hours ahead from the run time (integer: `0`, `6`, `12`, `24`, etc.)
- `TIME` = Observation timestamp (ISO8601: `2024-01-15T18:00:00Z`)

The appropriate style config file is automatically selected based on the parameter type:
- Temperature parameters → `temperature.json`
- Wind parameters → `wind.json`
- Reflectivity → `reflectivity.json`
- GOES visible → `goes_visible.json`
- GOES IR → `goes_ir.json`

**Style name examples:**
- `default` → Default style from the auto-selected config
- `celsius` → Celsius temperature scale from `temperature.json`
- `enhanced` → Enhanced radar colors from `reflectivity.json`

## Rendering Pipeline

```
┌────────────────┐
│   Grid Data    │  Raw numeric values from GRIB2/NetCDF
│  (e.g., 288K)  │
└───────┬────────┘
        │
        ▼
┌────────────────┐
│   Transform    │  Apply unit conversion (K → °C)
│  (e.g., 15°C)  │  Based on style's "transform" config
└───────┬────────┘
        │
        ▼
┌────────────────┐
│  Color Mapper  │  Interpolate color from stops
│  (e.g., green) │  Based on style's "stops" array
└───────┬────────┘
        │
        ▼
┌────────────────┐
│   PNG Tile     │  256×256 or 512×512 image
│                │  Returned to client
└────────────────┘
```

## Validation

Validate style files before deployment:

```bash
cd config/styles

# Validate all files
python3 validate_styles.py

# Verbose output
python3 validate_styles.py --verbose
```

The validator checks:
- Valid JSON syntax
- Required fields (`version`, `styles`)
- Valid style types
- Valid transform types
- Proper color formats
- Numeric field types
- Range validity

## Creating New Styles

### Step 1: Choose a Base

Start from an existing style file or `schema.example.json`:

```bash
cp config/styles/temperature.json config/styles/my_parameter.json
```

### Step 2: Define Styles

Edit the JSON file with your color stops and settings:

```json
{
  "version": "1.0",
  "metadata": {
    "name": "My Parameter Styles"
  },
  "styles": {
    "default": {
      "name": "My Parameter",
      "type": "gradient",
      "units": "units",
      "stops": [
        { "value": 0, "color": "#0000FF", "label": "Low" },
        { "value": 100, "color": "#FF0000", "label": "High" }
      ]
    }
  }
}
```

### Step 3: Validate

```bash
python3 config/styles/validate_styles.py --verbose
```

### Step 4: Test

Restart the WMS API and request a tile with your new style:

```bash
# Forecast model (GFS) - uses RUN + FORECAST dimensions
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetMap&LAYERS=gfs_TMP&STYLES=default&RUN=latest&FORECAST=0&ELEVATION=2%20m%20above%20ground&CRS=EPSG:3857&BBOX=-20037508,-20037508,20037508,20037508&WIDTH=256&HEIGHT=256&FORMAT=image/png"

# Observation data (GOES) - uses TIME dimension
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetMap&LAYERS=goes16_CMI_C13&STYLES=default&TIME=2024-01-15T18:00:00Z&CRS=EPSG:3857&BBOX=-20037508,-20037508,20037508,20037508&WIDTH=256&HEIGHT=256&FORMAT=image/png"
```

## Best Practices

1. **Use meaningful color progressions**: Cold→hot for temperature, light→dark for intensity
2. **Consider colorblind users**: Avoid red-green only progressions
3. **Follow meteorological conventions**: Standard radar colors, wind barb rules
4. **Include labels**: Help users interpret the legend
5. **Set appropriate ranges**: Match typical data ranges for the parameter
6. **Test at multiple zoom levels**: Ensure readability at all scales

## Reference

- [Schema Reference](../../config/styles/schema.example.json) - Complete schema with all options
- [Renderer Crate](../crates/renderer.md) - Implementation details
- [WMS API](../services/wms-api.md) - How styles are served
