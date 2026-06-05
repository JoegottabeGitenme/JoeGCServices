# Storm Events API Reference

Historical severe convective reports (hail, thunderstorm wind, tornado) from the
NOAA Storm Events Database (1995 – present), served via EDR at
**`https://folkweather.com/edr`**.

All responses are **GeoJSON FeatureCollections**. No API key required.

---

## Collections

Three collections, one per event type:

| Collection | Description | Geometry |
|---|---|---|
| `hail` | Hail reports | `Point` (report location) |
| `wind` | Thunderstorm wind reports | `Point` (report location) |
| `tornado` | Tornado tracks | `LineString` (start → end), or `Point` when end coords unavailable |

```
GET /edr/collections/hail
GET /edr/collections/wind
GET /edr/collections/tornado
```

---

## Common response shape

Every event feature looks like this:

```json
{
  "type": "Feature",
  "id": 887406,
  "geometry": {
    "type": "Point",
    "coordinates": [-97.56, 35.51]
  },
  "properties": {
    "event_id":       887406,
    "event_type":     "hail",
    "datetime":       "2020-04-22T11:04:00+00:00",
    "begin_time":     "2020-04-22T11:04:00+00:00",
    "end_time":       "2020-04-22T11:04:00+00:00",
    "magnitude":      1.5,
    "magnitude_unit": "in",
    "tor_f_scale":    null,
    "state":          "OKLAHOMA",
    "county_name":    "OKLAHOMA",
    "county_fips":    "40109"
  }
}
```

**Magnitude units by type:**
- `hail` → inches (`"in"`)
- `wind` → knots (`"kt"`)
- `tornado` → EF scale 0-5 (`"EF"`) via `tor_f_scale`

---

## Endpoints

### 1. Radius — events near a point ("this house")

Returns all events within a radius of a lat/lon. The primary query for the
"did my house get hit?" use case.

```
GET /edr/collections/{type}/radius
  ?coords=POINT({lon} {lat})
  &within={radius}
  &within-units=km          # km | mi | m | nm
  &datetime={start}/{end}   # optional ISO8601 interval
```

**Example — hail within 25 km of Oklahoma City, 2010-present:**
```
GET /edr/collections/hail/radius?coords=POINT(-97.516+35.467)&within=25km&datetime=2010-01-01/2026-12-31
```

**Response:**
```json
{
  "type": "FeatureCollection",
  "features": [
    {
      "type": "Feature",
      "id": 887406,
      "geometry": { "type": "Point", "coordinates": [-97.56, 35.51] },
      "properties": {
        "event_id": 887406,
        "event_type": "hail",
        "datetime": "2020-04-22T11:04:00+00:00",
        "begin_time": "2020-04-22T11:04:00+00:00",
        "end_time": "2020-04-22T11:04:00+00:00",
        "magnitude": 1.5,
        "magnitude_unit": "in",
        "tor_f_scale": null,
        "state": "OKLAHOMA",
        "county_name": "OKLAHOMA",
        "county_fips": "40109"
      }
    }
    ...
  ]
}
```

**Tips:**
- Default radius when `within` is omitted: **100 km**
- Omit `datetime` to get all years (1995–present)
- Results ordered newest-first

---

### 2. Items — events in a bounding box (map viewport)

GeoJSON items query, great for rendering events in the current map viewport.
Supports pagination via `limit` / `offset`.

```
GET /edr/collections/{type}/items
  ?bbox={minLon},{minLat},{maxLon},{maxLat}
  &datetime={start}/{end}
  &limit={n}          # default 2000, max 10000
  &offset={n}         # for pagination
```

**Example — tornado tracks in an OKC-area bbox, all years:**
```
GET /edr/collections/tornado/items?bbox=-98.5,34.8,-96.5,36.2
```

**Response (tornado LineStrings):**
```json
{
  "type": "FeatureCollection",
  "numberReturned": 183,
  "timeStamp": "2026-06-05T18:00:00+00:00",
  "features": [
    {
      "type": "Feature",
      "id": 900025,
      "geometry": {
        "type": "LineString",
        "coordinates": [
          [-97.676, 35.262],
          [-97.351, 35.449]
        ]
      },
      "properties": {
        "event_id": 900025,
        "event_type": "tornado",
        "datetime": "2013-05-20T19:56:00+00:00",
        "begin_time": "2013-05-20T19:56:00+00:00",
        "end_time": null,
        "magnitude": 5.0,
        "magnitude_unit": "EF",
        "tor_f_scale": 5,
        "state": "OKLAHOMA",
        "county_name": "CLEVELAND",
        "county_fips": "40051"
      }
    }
    ...
  ]
}
```

**Tips:**
- Tornado tracks that **cross** the bbox edge are included (not just those whose start point is inside)
- `numberReturned` tells you how many came back; use `offset` to page
- Omit `bbox` to query globally (combine with tight `datetime` to avoid huge responses)

---

### 3. Area — events in a bbox (EDR style)

Same spatial filter as `items` but uses the EDR `coords` convention. Equivalent
to `items` without pagination metadata.

```
GET /edr/collections/{type}/area
  ?coords={minLon},{minLat},{maxLon},{maxLat}
  &datetime={start}/{end}
```

**Example — wind events in a central Oklahoma area, 2022-2024:**
```
GET /edr/collections/wind/area?coords=-97.5,35.0,-96.5,36.0&datetime=2022-01-01/2024-12-31
```

**Response:**
```json
{
  "type": "FeatureCollection",
  "features": [
    {
      "type": "Feature",
      "id": 1031949,
      "geometry": { "type": "Point", "coordinates": [-97.49, 36.0] },
      "properties": {
        "event_id": 1031949,
        "event_type": "wind",
        "datetime": "2022-05-14T22:30:00+00:00",
        "begin_time": "2022-05-14T22:30:00+00:00",
        "end_time": "2022-05-14T22:30:00+00:00",
        "magnitude": 52.0,
        "magnitude_unit": "kt",
        "tor_f_scale": null,
        "state": "OKLAHOMA",
        "county_name": "LOGAN",
        "county_fips": "40083"
      }
    }
  ]
}
```

---

### 4. Counties — aggregate counts + boundaries (choropleth)

Returns per-county event counts joined to county boundary polygons. Use this
to power choropleth maps showing "which counties get hit the most."

The counts come from a monthly-refreshed materialized view — they're stable
within a calendar month and are fast (no live scan needed).

```
GET /edr/collections/{type}/counties
  ?state={abbr}             # optional: filter by state (e.g. OK, TX, KS)
  &datetime={start}/{end}   # optional: year range (bare years like 2015/2024 work)
  &bbox={minLon},{minLat},{maxLon},{maxLat}  # optional: spatial filter
  &geometry=true|false      # include boundary polygon? default true
  &simplify={degrees}       # geometry simplification tolerance, default 0.01 (~1km)
```

**Example — hail counts in Oklahoma, 2015-2024, with boundaries:**
```
GET /edr/collections/hail/counties?state=OK&datetime=2015/2024&geometry=true
```

**Response:**
```json
{
  "type": "FeatureCollection",
  "features": [
    {
      "type": "Feature",
      "id": "40109",
      "geometry": {
        "type": "Polygon",
        "coordinates": [[
          [-97.674043, 35.573501],
          [-97.674026, 35.72597],
          [-97.141066, 35.724401],
          [-97.142114, 35.37686],
          ...
        ]]
      },
      "properties": {
        "county_fips": "40109",
        "name": "Oklahoma",
        "state": "OK",
        "event_type": "hail",
        "total": 3,
        "by_year": {
          "2015": 1,
          "2020": 1,
          "2023": 1
        }
      }
    }
  ]
}
```

**Tips:**
- `geometry=false` returns counts-only (much smaller payload — good for initial load)
- `by_year` is a sparse map — years with 0 events are omitted
- The `id` on each Feature is the 5-digit county FIPS code (joinable to Census data)
- For a national choropleth, omit `state`; for the full archive omit `datetime`

---

## Datetime formats

All `datetime` parameters accept:

| Format | Example | Meaning |
|---|---|---|
| Interval | `2015-01-01/2024-12-31` | Events between two dates |
| Bare years | `2015/2024` | Same as above (year boundaries) |
| Open end | `2020/..` | 2020 to present |
| Open start | `../2020` | Everything before 2020 |
| Single instant | `2013-05-20T19:56:00Z` | Exact moment |
| Omitted | _(nothing)_ | All years (1995–present) |

---

## Practical recipes

### "This house" report (the main HailTrace flow)

Geocode the address to lat/lon, then:

```
# Step 1: all hail events within 1 mile, all time
GET /edr/collections/hail/radius?coords=POINT(-97.516+35.467)&within=1&within-units=mi

# Step 2: all wind events within 1 mile, all time
GET /edr/collections/wind/radius?coords=POINT(-97.516+35.467)&within=1&within-units=mi

# Step 3: tornado tracks within 5 miles (tracks are lines so use larger radius)
GET /edr/collections/tornado/radius?coords=POINT(-97.516+35.467)&within=5&within-units=mi
```

### County choropleth (national map, counts only, fast)

```
# All hail counts, national, all years — no geometry for initial load
GET /edr/collections/hail/counties?geometry=false

# Then fetch geometry on demand per county, or re-fetch with bbox of viewport
GET /edr/collections/hail/counties?bbox=-104,36,-94,37&geometry=true
```

### Map viewport rendering (tornado tracks as user pans)

```
# Tornado tracks in current map viewport — use items for pagination control
GET /edr/collections/tornado/items?bbox={west},{south},{east},{north}&limit=500

# Page 2
GET /edr/collections/tornado/items?bbox={west},{south},{east},{north}&limit=500&offset=500
```

### Recent events only (last 5 years)

```
GET /edr/collections/hail/radius?coords=POINT(-97.5+35.5)&within=50km&datetime=2020/2026
```

### Filter by EF scale client-side

The API doesn't have a `min_ef` query param — filter `tor_f_scale >= N` on the
response. Significant tornadoes (EF2+) are a small fraction of results so this
is fast client-side:

```javascript
const significant = features.filter(f => f.properties.tor_f_scale >= 2);
```

### Filter by hail size client-side

```javascript
const largeHail = features.filter(f => f.properties.magnitude >= 1.0); // ≥ 1 inch
const baseball  = features.filter(f => f.properties.magnitude >= 2.75); // ≥ baseball
```

---

## Live data status

- **Coverage:** 1995-01-06 through ~2026 (growing daily)
- **Update cadence:** downloader re-fetches NOAA data daily; county aggregate refreshes monthly
- **Records:** ~340k events currently ingested, full archive ~1M when backfill completes
- **Source:** [NOAA Storm Events Database (NCEI)](https://www.ncei.noaa.gov/pub/data/swdi/stormevents/csvfiles/)

---

## Base URL

```
https://folkweather.com/edr
```

All endpoints also accessible locally in dev at `http://localhost:8083/edr`.
