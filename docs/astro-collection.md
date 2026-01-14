# Astro Collection - Astronomical Data API

The `astro` collection provides on-demand astronomical data for any location and time. Unlike other collections that serve pre-computed weather model data, the astro collection computes solar and lunar positions in real-time using astronomical algorithms.

## Overview

- **Collection ID**: `astro`
- **Data Type**: Computed (no storage required)
- **Coverage**: Global (any lat/lon)
- **Temporal Coverage**: Any past, present, or future date
- **Supported Queries**: Position only
- **Output Formats**: CoverageJSON, GeoJSON

## Available Parameters

| Parameter | Type | Unit | Description |
|-----------|------|------|-------------|
| `sunrise` | Unix timestamp | seconds | Time of sunrise. Returns `null` in polar regions during polar night. |
| `sunset` | Unix timestamp | seconds | Time of sunset. Returns `null` in polar regions during midnight sun. |
| `solar_noon` | Unix timestamp | seconds | Time when the sun reaches its highest point in the sky. |
| `sun_altitude` | Float | degrees | Sun's elevation angle above the horizon (-90° to +90°). |
| `sun_azimuth` | Float | degrees | Sun's compass direction (0°=North, 90°=East, 180°=South, 270°=West). |
| `moonrise` | Unix timestamp | seconds | Time of moonrise. Returns `null` when moon doesn't rise. |
| `moonset` | Unix timestamp | seconds | Time of moonset. Returns `null` when moon doesn't set. |
| `moon_phase` | Categorical | - | Current moon phase (0-7): new_moon, waxing_crescent, first_quarter, waxing_gibbous, full_moon, waning_gibbous, last_quarter, waning_crescent |
| `moon_illumination` | Float | fraction | Fraction of moon disk illuminated (0.0 = new moon, 1.0 = full moon). |
| `moon_age` | Float | days | Days since last new moon (0-29.5 approximately). |

### Notes on Sunrise/Sunset

⚠️ **Current Limitation**: The `sunrise`, `sunset`, `moonrise`, and `moonset` parameters currently return `null`. Accurate calculation of rise/set times requires complex transit time algorithms that account for atmospheric refraction, observer elevation, and coordinate precession. This is marked as a TODO for future enhancement.

The sun and moon position parameters (`sun_altitude`, `sun_azimuth`) are simplified calculations and will be enhanced in future versions for higher accuracy.

## API Endpoints

### Get Collection Metadata

```bash
GET /edr/collections/astro
```

Returns collection information including available parameters, extent, and supported queries.

### Position Query

```bash
GET /edr/collections/astro/position
```

Query astronomical data for a specific point location.

**Required Parameters:**
- `coords`: Point location as WKT POINT or lon,lat

**Optional Parameters:**
- `datetime`: ISO 8601 datetime or interval (default: current time)
- `parameter-name`: Comma-separated list of parameters (default: all parameters)
- `step`: ISO 8601 duration for time series (default: PT1H when using datetime interval)
- `f`: Output format (default: application/vnd.cov+json)

## Example Queries

### 1. Current Astronomical Data

Get current sun and moon data for San Francisco:

```bash
curl "http://localhost:8083/edr/collections/astro/position?coords=POINT(-122.4 37.8)"
```

**Response** (CoverageJSON):
```json
{
  "type": "Coverage",
  "domain": {
    "type": "Domain",
    "domainType": "Point",
    "axes": {
      "x": { "values": [-122.4] },
      "y": { "values": [37.8] },
      "t": { "values": ["2026-01-15T19:30:00Z"] }
    }
  },
  "parameters": {
    "sun_altitude": {
      "type": "Parameter",
      "observedProperty": {
        "label": { "en": "Sun Altitude" }
      },
      "unit": { "symbol": "deg" }
    },
    "moon_phase": {
      "type": "Parameter",
      "observedProperty": {
        "label": { "en": "Moon Phase" },
        "categories": [
          { "id": "0", "label": { "en": "new_moon" } },
          { "id": "1", "label": { "en": "waxing_crescent" } },
          ...
        ]
      }
    },
    ...
  },
  "ranges": {
    "sun_altitude": { "type": "NdArray", "values": [25.3] },
    "moon_phase": { "type": "NdArray", "values": [6] },
    "moon_illumination": { "type": "NdArray", "values": [0.87] },
    ...
  }
}
```

### 2. Historical Data at Specific Time

Get astronomical data for a specific date and time:

```bash
curl "http://localhost:8083/edr/collections/astro/position?\
coords=POINT(-97.5 35.2)&\
datetime=2026-01-15T12:00:00Z"
```

### 3. Daily Moon Phase for a Month

Get daily moon phase data for January 2026 in New York:

```bash
curl "http://localhost:8083/edr/collections/astro/position?\
coords=POINT(-74.0 40.7)&\
datetime=2026-01-01T00:00:00Z/2026-01-31T23:59:59Z&\
step=P1D&\
parameter-name=moon_phase,moon_illumination,moon_age"
```

This returns 31 data points (one per day) with just the requested lunar parameters.

### 4. Hourly Sun Position for a Day

Track the sun's position throughout the summer solstice in London:

```bash
curl "http://localhost:8083/edr/collections/astro/position?\
coords=POINT(0 51.5)&\
datetime=2026-06-21T00:00:00Z/2026-06-21T23:59:59Z&\
step=PT1H&\
parameter-name=sun_altitude,sun_azimuth"
```

Returns 24 hourly data points showing how the sun moves across the sky.

### 5. 5-Minute Intervals for Animation

Generate smooth animation data with 5-minute intervals:

```bash
curl "http://localhost:8083/edr/collections/astro/position?\
coords=POINT(-122.4 37.8)&\
datetime=2026-01-15T06:00:00Z/2026-01-15T18:00:00Z&\
step=PT5M&\
parameter-name=sun_altitude,sun_azimuth"
```

Returns 145 data points for a 12-hour period (sunrise to sunset approximate window).

### 6. Moon Data Only

Get just the lunar parameters:

```bash
curl "http://localhost:8083/edr/collections/astro/position?\
coords=POINT(-122.4 37.8)&\
parameter-name=moon_phase,moon_illumination,moon_age,moonrise,moonset"
```

### 7. Solar Noon Calculations

Get solar noon times for a week:

```bash
curl "http://localhost:8083/edr/collections/astro/position?\
coords=POINT(-122.4 37.8)&\
datetime=2026-01-15T00:00:00Z/2026-01-21T23:59:59Z&\
step=P1D&\
parameter-name=solar_noon"
```

## Use Cases

### Weather Application Integration

Combine astro data with weather forecasts:
- Display sunrise/sunset times on weather dashboard
- Show moon phase alongside tidal forecasts
- Calculate daylight hours for outdoor activity planning

### Solar Energy Applications

- Solar panel positioning optimization using sun azimuth
- Energy production estimates based on sun altitude
- Day length calculations for solar energy planning

### Astronomy & Education

- Moon phase calendars
- Lunar cycle visualization
- Sun path diagrams for any location and date

### Photography Planning

- Golden hour calculations (sun altitude < 6°)
- Blue hour timing (sun altitude -6° to -4°)
- Moon visibility for night photography

### Agriculture & Horticulture

- Frost risk assessment using sunrise times
- Lunar gardening calendar generation
- Photoperiod calculations for crop planning

## Technical Implementation

### Architecture

The astro collection is unique in the EDR API:
- **No data storage**: All values computed on-demand
- **No catalog dependency**: Doesn't query the database
- **No instances**: Data isn't versioned or updated
- **Global coverage**: Works for any coordinates without pre-processing

### Computation Engine

Uses the `astro` crate (v2.0), which implements algorithms from Jean Meeus's "Astronomical Algorithms":
- Julian Day conversions
- Sun geocentric ecliptic position
- Moon geocentric ecliptic position  
- Lunar illumination fraction
- Moon age calculation from lunation cycles

### Performance

- **Response time**: ~10-50ms for single point
- **Throughput**: Can handle hundreds of concurrent requests
- **Time series**: Linear scaling with number of time steps
- **Max time steps**: 1000 per request (configurable)

### Coordinate Systems

- **Input**: WGS84 geographic coordinates (CRS:84 / EPSG:4326)
- **Output**: All angular measurements in degrees
- **Time**: UTC (ISO 8601 format)
- **Timestamps**: Unix time (seconds since 1970-01-01T00:00:00Z)

## Limitations & Future Enhancements

### Current Limitations

1. **Simplified Calculations**
   - Sun/moon positions use basic geocentric ecliptic calculations
   - No atmospheric refraction correction
   - No topocentric parallax correction
   - Rise/set times not yet implemented

2. **No Vertical Level Support**
   - Observer elevation not considered
   - Always assumes sea level

3. **Limited Accuracy**
   - Suitable for general applications
   - Not suitable for precision astronomical work
   - Accuracy: ~0.01° for modern dates

### Planned Enhancements

- ✅ Basic sun and moon position
- ✅ Moon phase and illumination
- ⏳ Accurate rise/set times (transit calculations)
- ⏳ Atmospheric refraction corrections
- ⏳ Observer elevation support
- ⏳ Topocentric corrections
- ⏳ Additional solar parameters (equation of time, declination)
- ⏳ Additional lunar parameters (libration, distance)
- ⏳ Planet positions (Venus, Mars, Jupiter, Saturn)
- ⏳ Solar/lunar eclipses
- ⏳ Twilight times (civil, nautical, astronomical)

## Testing

### Unit Tests

The astro module includes comprehensive unit tests:

```bash
cargo test --package edr-api --lib astro
```

Tests cover:
- Julian Day conversions
- DateTime round-trip accuracy
- Moon phase determination
- Solar data validation
- Lunar data validation
- Parameter value ranges

### Compliance Tests

Web-based compliance tests available at `/web/edr-compliance.html`:

1. **Collection Exists**: Verifies astro collection in listings
2. **Current Data**: Tests real-time data retrieval
3. **Datetime Parameter**: Tests historical/future queries
4. **Time Series**: Tests date range with step parameter
5. **Parameter Filtering**: Tests selective parameter requests
6. **Moon Phase Categories**: Validates categorical encoding
7. **Data Ranges**: Ensures values within valid bounds

### Manual Testing

Start the EDR API service:

```bash
cargo run --bin edr-api
```

Test endpoints:

```bash
# List all collections (astro should be included)
curl http://localhost:8083/edr/collections | jq '.collections[] | select(.id=="astro")'

# Get astro collection metadata
curl http://localhost:8083/edr/collections/astro | jq

# Query current data
curl "http://localhost:8083/edr/collections/astro/position?coords=POINT(-122.4 37.8)" | jq

# Query with time series
curl "http://localhost:8083/edr/collections/astro/position?coords=POINT(0 51.5)&datetime=2026-01-01/2026-01-07&step=P1D" | jq
```

## API Response Reference

### CoverageJSON Structure

```json
{
  "type": "Coverage",
  "domain": {
    "type": "Domain",
    "domainType": "Point" | "PointSeries",
    "axes": {
      "x": { "values": [<longitude>] },
      "y": { "values": [<latitude>] },
      "t": { "values": [<time_iso8601>, ...] }
    }
  },
  "parameters": {
    "<param_name>": {
      "type": "Parameter",
      "description": { "en": "<description>" },
      "observedProperty": {
        "label": { "en": "<label>" },
        "description": { "en": "<description>" },
        "categories": [...]  // For moon_phase only
      },
      "unit": {
        "label": { "en": "<unit_label>" },
        "symbol": "<unit_symbol>"
      }
    }
  },
  "ranges": {
    "<param_name>": {
      "type": "NdArray",
      "values": [<value1>, <value2>, ...],
      "shape": [<array_length>],
      "axisNames": ["t"]
    }
  }
}
```

### Error Responses

**400 Bad Request** - Invalid parameters:
```json
{
  "code": "InvalidParameterValue",
  "description": "Missing required parameter: coords"
}
```

**400 Bad Request** - Too many time steps:
```json
{
  "code": "InvalidParameterValue",
  "description": "Too many time steps requested: 1500. Maximum is 1000."
}
```

## Comparison with Other Collections

| Feature | Weather Collections | Astro Collection |
|---------|-------------------|------------------|
| Data Source | Pre-computed models | Real-time calculation |
| Storage | MinIO/S3 (Zarr arrays) | None |
| Database | PostgreSQL catalog | None |
| Coverage | Model-specific regions | Global |
| Temporal | Model run times | Any time |
| Instances | Multiple runs/versions | N/A (always current) |
| Update Frequency | Hourly/daily ingestion | N/A |
| Parameters | ~10-50 per model | 10 astronomical |
| Vertical Levels | Multiple (surface, isobaric) | N/A |
| Query Types | All (position, area, radius, etc.) | Position only |

## Contributing

To add new astronomical parameters or improve calculations:

1. **Add calculation** to `services/edr-api/src/astro.rs`
2. **Update structs** (`SolarData` or `LunarData`)
3. **Add parameter** to handler in `services/edr-api/src/handlers/astro.rs`
4. **Add tests** to the `#[cfg(test)]` module
5. **Update compliance tests** in `web/edr-compliance.js`
6. **Update this documentation**

## References

- **Astronomical Algorithms**: Jean Meeus, "Astronomical Algorithms" (2nd edition, 1998)
- **Astro Crate**: https://docs.rs/astro/2.0.0/astro/
- **OGC EDR Spec**: https://docs.ogc.org/is/19-086r6/19-086r6.html
- **CoverageJSON**: https://covjson.org/spec/

## License

Part of the weather-wms project. See main project LICENSE.
