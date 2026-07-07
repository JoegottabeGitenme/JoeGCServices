# Populated Places — Frontend Guide

The `populated` EDR collection is a registry of **~10,200 US cities**
(population ≥ 1,000) that lets you look up point-forecast data by city instead
of raw coordinates. It replaces external geocoding + forecast API calls: one
EDR call resolves a city and returns weather data.

Live at `https://folkweather.com/edr/collections/populated`.

## Concept

- Cities are addressed by **`PP` + 7-digit Census GEOID** (e.g. `PP3651000` =
  New York, `PP1714000` = Chicago).
- The collection holds **no weather data itself** — it resolves a city to a
  lat/lon and **proxies point forecasts from gridded collections** (default
  `gfs-surface`, selectable via `?collections=`).
- Population is filterable per request with `?min-population=`.

## Three endpoints

| Endpoint | Purpose |
|---|---|
| `GET /collections/populated/locations` | Discover cities (GeoJSON points) |
| `GET /collections/populated/locations/{id}` | Point forecast at one city |
| `GET /collections/populated/radius` | Forecasts for all cities near a point |

---

## 1. Discover cities

```
GET /edr/collections/populated/locations?min-population=500000&limit=5
```

```jsonc
{
  "type": "FeatureCollection",
  "numberReturned": 5,
  "min_population": 500000,
  "features": [
    {
      "type": "Feature",
      "id": "PP3651000",
      "geometry": { "type": "Point", "coordinates": [-73.93868, 40.66271] },
      "properties": {
        "name": "New York",
        "state": "NY",
        "population": 8258035,
        "forecast": "https://folkweather.com/edr/collections/populated/locations/PP3651000",
        "forecast_links": [
          { "collection": "gfs-surface",
            "href": ".../collections/gfs-surface/position?coords=POINT(-73.93868 40.66271)" }
        ]
      }
    }
    // Los Angeles, Chicago, Houston, Phoenix ...
  ]
}
```

- `min-population` — omit to use the default floor of **25,000** (keeps the
  unfiltered list to major cities). Set `min-population=1000` for everything.
- `bbox=min_lon,min_lat,max_lon,max_lat` — restrict to a map viewport.
- `limit` — capped at 500 per request.
- Each feature carries ready-to-use `forecast` (per-city endpoint) and
  `forecast_links` (direct position queries) so you can wire the UI without
  string-building.

---

## 2. Point forecast at a city

```
GET /edr/collections/populated/locations/PP1714000?collections=gfs-surface
```

```jsonc
{
  "type": "Feature",
  "id": "PP1714000",
  "geometry": { "type": "Point", "coordinates": [-87.68494, 41.83705] },
  "properties": {
    "name": "Chicago",
    "state": "IL",
    "population": 2664452,
    "forecasts": {
      "gfs-surface": {
        "reference_time": "2026-07-07T12:00:00+00:00",
        "valid_time": "2026-07-07T18:00:00+00:00",
        "forecast_hour": 6,
        "parameters": {
          "GUST": { "unit": "m/s", "values": { "surface": 3.15 } },
          "CAPE": { "unit": "J/kg", "values": { "surface": 0.0 } }
          // ... all params of the selected collection
        }
      }
    }
  }
}
```

- **Select models** with `collections=` (comma-list, up to 5), e.g.
  `collections=gfs-surface,hrrr-surface`. Each appears as its own object under
  `forecasts`. Default is `gfs-surface`.
- **Pick a time** with `datetime=<ISO8601>` — the server selects the forecast
  hour whose valid time is closest, from the latest run. Omit for
  closest-to-now.

```
GET /edr/collections/populated/locations/PP1714000?collections=gfs-surface&datetime=2026-07-09T12:00:00Z
# -> forecasts.gfs-surface.valid_time = 2026-07-09T12:00:00Z, forecast_hour = 48
```

Values are **native SI units** (Kelvin, m/s, J/kg, …) — the same convention as
every other EDR collection.

---

## 3. Forecasts for all cities in a radius

```
GET /edr/collections/populated/radius?coords=POINT(-96.8 32.78)&within=80km&min-population=100000&collections=gfs-surface
```

Returns a GeoJSON FeatureCollection, one feature per city (ordered by distance
from the point), each with the same `forecasts` structure as above:

```jsonc
{
  "type": "FeatureCollection",
  "numberReturned": 15,
  "min_population": 100000,
  "features": [
    { "id": "PP4819000", "properties": { "name": "Dallas", "state": "TX",
        "population": 1302868,
        "forecasts": { "gfs-surface": { /* ... GUST 2.46 m/s ... */ } } } }
    // Irving, Mesquite, Garland, Plano, Arlington, Fort Worth ...
  ]
}
```

- `coords=POINT(lon lat)` (required). `within=` accepts `100km`, `50000m`,
  `30mi`, or a bare number + `within-units=`.
- `min-population` and `collections` work exactly as above.
- Bounded by the same 500-place / 5-collection caps.

---

## Errors

- Unknown city id → **404** with a hint to list `/locations`.
- Missing `coords` on `/radius` → **400**.
- Too many `collections` (>5) → **400**.
- Cities outside a model's grid simply omit that collection from `forecasts`
  (e.g. HRRR is CONUS-only; Alaska/Hawaii cities won't have `hrrr-*`).

---

## Replacing external calls

Typical pattern to drop a rate-limited geocode+forecast provider:

```bash
# 1. Autocomplete / nearest cities for a dropped pin
curl ".../collections/populated/radius?coords=POINT(-96.8 32.78)&within=25km&collections=gfs-surface"

# 2. Full forecast card for a chosen city (multiple models + a time)
curl ".../collections/populated/locations/PP1714000?collections=gfs-surface,hrrr-surface&datetime=2026-07-08T00:00:00Z"

# 3. City search box backed by the discovery list (filter by population)
curl ".../collections/populated/locations?min-population=50000&bbox=-98,32,-96,34"
```

(URL-encode the space in `POINT(lon lat)` as `%20` if your client doesn't do
it automatically.)

## Notes

- Data: US Census Gazetteer 2023 (coordinates) + Population Estimates 2023.
  Regenerate with `scripts/build_populated_places.py`.
- IDs are stable Census GEOIDs, so they're safe to persist client-side.
- Coverage is US only (50 states + DC).
