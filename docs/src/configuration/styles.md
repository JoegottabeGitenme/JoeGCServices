# Style Configuration

Style files define how weather data is visualized as colored images.

## File Location

`config/styles/{style}.json`

## Example: Temperature Gradient

```json
{
  "name": "temperature",
  "title": "Temperature",
  "type": "gradient",
  "parameter": "TMP",
  "colormap": [
    {"value": 233.15, "color": "#0000FF"},
    {"value": 253.15, "color": "#00FFFF"},
    {"value": 273.15, "color": "#00FF00"},
    {"value": 293.15, "color": "#FFFF00"},
    {"value": 313.15, "color": "#FF0000"}
  ],
  "opacity": 0.7,
  "units": "K"
}
```

## Style Types

### Gradient
```json
{
  "type": "gradient",
  "colormap": [
    {"value": 0, "color": "#0000FF"},
    {"value": 100, "color": "#FF0000"}
  ]
}
```

### Contours
```json
{
  "type": "contours",
  "interval": 10.0,
  "line_width": 2,
  "color": "#000000"
}
```

### Wind Barbs
```json
{
  "type": "wind_barbs",
  "spacing": 32,
  "scale": 1.0,
  "color": "#000000"
}
```

## Color Formats

- Hex: `#RRGGBB` or `#RRGGBBAA`
- RGB array: `[255, 0, 0]` or `[255, 0, 0, 128]`

See `config/styles/` for more examples.
