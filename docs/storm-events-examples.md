# Storm Events API — Request & Response Examples

Live base URL: `https://folkweather.com/edr`

> **Narrative backfill note:** Narratives, damage, and source fields are being
> backfilled across the full archive (1995-present) and appear on more events
> every day. Events where re-ingest has not yet run will show `null` on those
> fields — this is temporary and resolves itself without any action.

---

## 1. "This house" — hail within 1 mile of an address

Use this after geocoding the address. Radius is anchored on the **begin point**
of each report.

**Request**
```
GET /edr/collections/hail/radius
  ?coords=POINT(-97.516+35.467)    ← lon lat (note: + = space in URL)
  &within=1
  &within-units=mi                 ← also accepts: km, m, nm
```

```bash
curl 'https://folkweather.com/edr/collections/hail/radius?coords=POINT(-97.516+35.467)&within=1&within-units=mi'
```

**Response** *(trimmed to 1 feature)*
```json
{
  "type": "FeatureCollection",
  "features": [
    {
      "type": "Feature",
      "id": 1216584,
      "geometry": {
        "type": "Point",
        "coordinates": [-97.5235, 35.469]
      },
      "properties": {
        "event_id": 1216584,
        "event_type": "hail",
        "datetime": "2024-09-24T17:40:00+00:00",
        "begin_time": "2024-09-24T17:40:00+00:00",
        "end_time": "2024-09-24T17:40:00+00:00",
        "magnitude": 1.0,
        "magnitude_unit": "in",
        "tor_f_scale": null,
        "state": "OKLAHOMA",
        "county_name": "OKLAHOMA",
        "county_fips": "40109",
        "event_narrative": null,
        "episode_narrative": null,
        "report_source": null,
        "damage_property": null,
        "damage_crops": null,
        "injuries_direct": null,
        "deaths_direct": null,
        "begin_location": null,
        "tor_length_mi": null,
        "tor_width_yd": null
      }
    }
  ]
}
```

> Results are ordered newest-first. Omit `datetime` to get the full history
> back to 1995. Add `&datetime=2010/..` to limit to recent years only.

---

## 2. "This house" — tornado tracks within 5 miles

Tornadoes are `LineString` geometries (start → end). A 5-mile radius is
recommended because the track can extend beyond the report's begin point.

**Request**
```
GET /edr/collections/tornado/radius
  ?coords=POINT(-97.516+35.467)
  &within=5
  &within-units=mi
```

```bash
curl 'https://folkweather.com/edr/collections/tornado/radius?coords=POINT(-97.516+35.467)&within=5&within-units=mi'
```

**Response** *(trimmed to 1 feature)*
```json
{
  "type": "FeatureCollection",
  "features": [
    {
      "type": "Feature",
      "id": 1184053,
      "geometry": {
        "type": "LineString",
        "coordinates": [
          [-97.463, 35.427],
          [-97.451, 35.439]
        ]
      },
      "properties": {
        "event_id": 1184053,
        "event_type": "tornado",
        "datetime": "2024-05-06T22:46:00+00:00",
        "begin_time": "2024-05-06T22:46:00+00:00",
        "end_time": "2024-05-06T22:47:00+00:00",
        "magnitude": 1.0,
        "magnitude_unit": "EF",
        "tor_f_scale": 1,
        "state": "OKLAHOMA",
        "county_name": "OKLAHOMA",
        "county_fips": "40109",
        "event_narrative": null,
        "episode_narrative": null,
        "report_source": null,
        "damage_property": null,
        "damage_crops": null,
        "injuries_direct": null,
        "deaths_direct": null,
        "begin_location": null,
        "tor_length_mi": null,
        "tor_width_yd": null
      }
    }
  ]
}
```

---

## 3. Per-county events — hail (with full narrative, cached)

One call per visible county. Cached for 24 hours with ETag revalidation.
The response includes county metadata alongside the features.

This is the recommended way to render map data — fetch per county, not per viewport.

**Request**
```
GET /edr/collections/hail/counties/40147
```

```bash
curl 'https://folkweather.com/edr/collections/hail/counties/40147'
```

**Response headers** *(set on every county response)*
```
Cache-Control: public, max-age=86400, stale-while-revalidate=604800
ETag: W/"cc7346b050c32a7e"
Content-Type: application/geo+json
```

**Response** *(Washington County, OK — trimmed to 1 feature with full narrative)*
```json
{
  "type": "FeatureCollection",
  "county_fips": "40147",
  "county_name": "Washington",
  "state": "OK",
  "event_type": "hail",
  "numberReturned": 11,
  "features": [
    {
      "type": "Feature",
      "id": 5145974,
      "geometry": {
        "type": "Point",
        "coordinates": [-96.0, 36.75]
      },
      "properties": {
        "event_id": 5145974,
        "event_type": "hail",
        "datetime": "2000-05-08T22:28:00+00:00",
        "begin_time": "2000-05-08T22:28:00+00:00",
        "end_time": "2000-05-08T22:37:00+00:00",
        "magnitude": 4.5,
        "magnitude_unit": "in",
        "tor_f_scale": null,
        "state": "OKLAHOMA",
        "county_name": "WASHINGTON",
        "county_fips": "40147",
        "event_narrative": "Golfball to softball size hail fell in Bartlesville damaging roofs, windows and cars.",
        "episode_narrative": null,
        "report_source": "LAW ENFORCEMENT",
        "damage_property": "2M",
        "damage_crops": null,
        "injuries_direct": 0,
        "deaths_direct": 0,
        "begin_location": "BARTLESVILLE",
        "tor_length_mi": null,
        "tor_width_yd": null
      }
    }
  ]
}
```

**Optional year filter**
```bash
# Single year
curl 'https://folkweather.com/edr/collections/hail/counties/40147?year=2023'

# Year range
curl 'https://folkweather.com/edr/collections/hail/counties/40147?years=2015/2024'
```

---

## 4. Per-county events — tornado (LineStrings, full report fields)

**Request**
```
GET /edr/collections/tornado/counties/40061
```

```bash
curl 'https://folkweather.com/edr/collections/tornado/counties/40061'
```

**Response** *(Haskell County, OK — 2 features shown including EF3 with injuries)*
```json
{
  "type": "FeatureCollection",
  "county_fips": "40061",
  "county_name": "Haskell",
  "state": "OK",
  "event_type": "tornado",
  "numberReturned": 2,
  "features": [
    {
      "type": "Feature",
      "id": 5494328,
      "geometry": {
        "type": "LineString",
        "coordinates": [
          [-95.017, 35.417],
          [-94.892, 35.512]
        ]
      },
      "properties": {
        "event_id": 5494328,
        "event_type": "tornado",
        "datetime": "2006-03-12T21:18:00+00:00",
        "begin_time": "2006-03-12T21:18:00+00:00",
        "end_time": "2006-03-12T21:42:00+00:00",
        "magnitude": 3.0,
        "magnitude_unit": "EF",
        "tor_f_scale": 3,
        "state": "OKLAHOMA",
        "county_name": "DELAWARE",
        "county_fips": "40061",
        "event_narrative": "The first tornado, which touched down in northwestern Cherokee County, continued into southern Delaware County. Damage suggested the tornado widened and strengthened as it moved through southern Delaware County reaching a maximum width of around 1/4 of a mile. The tornado damaged 95 homes, destroying 42 of those homes. Five businesses were also damaged. Numerous trees were snapped or uprooted and about 100 power poles were downed, which resulted in more than 5000 people losing power as a result of the storm. The worst damage from this tornado was found from near Twin Oaks to about 4 miles west-southwest of Colcord. The tornado injured eight people.",
        "episode_narrative": null,
        "report_source": "NWS STORM SURVEY",
        "damage_property": "3M",
        "damage_crops": null,
        "injuries_direct": 8,
        "deaths_direct": 0,
        "begin_location": "2 S LEACH",
        "tor_length_mi": 17.0,
        "tor_width_yd": 440.0
      }
    },
    {
      "type": "Feature",
      "id": 5506383,
      "geometry": {
        "type": "LineString",
        "coordinates": [
          [-95.383, 35.267],
          [-95.359, 35.281]
        ]
      },
      "properties": {
        "event_id": 5506383,
        "event_type": "tornado",
        "datetime": "2006-04-06T18:18:00+00:00",
        "begin_time": "2006-04-06T18:18:00+00:00",
        "end_time": "2006-04-06T18:18:00+00:00",
        "magnitude": 0.0,
        "magnitude_unit": "EF",
        "tor_f_scale": 0,
        "state": "OKLAHOMA",
        "county_name": "HASKELL",
        "county_fips": "40051",
        "event_narrative": "A brief tornado touchdown was reported by multiple storm spotters southwest of Eufaula dam on Lake Eufaula.",
        "episode_narrative": null,
        "report_source": "TRAINED SPOTTER",
        "damage_property": null,
        "damage_crops": null,
        "injuries_direct": 0,
        "deaths_direct": 0,
        "begin_location": "4 N ENTERPRISE",
        "tor_length_mi": 0.1,
        "tor_width_yd": 40.0
      }
    }
  ]
}
```

---

## 5. ETag / 304 Not Modified — zero-cost revalidation

After the first fetch, the browser can revalidate using the ETag. If nothing
has changed, the server returns `304` with no body.

```bash
# First request — get the ETag
curl -I 'https://folkweather.com/edr/collections/hail/counties/40147'
# → ETag: W/"cc7346b050c32a7e"

# Subsequent requests — 304 if unchanged, 0 bytes transferred
curl -I -H 'If-None-Match: W/"cc7346b050c32a7e"' \
     'https://folkweather.com/edr/collections/hail/counties/40147'
# → HTTP/2 304
```

```javascript
// Browser does this automatically — just fetch normally.
// After the first load, every subsequent request for the same county
// is either served from browser cache (no network) or validated via
// a 304 (tiny round-trip, zero payload).
const response = await fetch(
  `https://folkweather.com/edr/collections/hail/counties/${fips}`
);
```

---

## 6. County aggregate — choropleth counts + boundaries

Returns event counts per county for the whole state, by year. Use this to
power a choropleth ("heat map by county").

**Request**
```
GET /edr/collections/hail/counties
  ?state=OK
  &datetime=2018/2024
  &geometry=false           ← omit boundary polygons for smaller payload
```

```bash
curl 'https://folkweather.com/edr/collections/hail/counties?state=OK&datetime=2018/2024&geometry=false'
```

**Response** *(top 4 Oklahoma counties by hail count, 2018-2024)*
```json
{
  "type": "FeatureCollection",
  "features": [
    {
      "type": "Feature",
      "id": "40109",
      "geometry": null,
      "properties": {
        "county_fips": "40109",
        "name": "Oklahoma",
        "state": "OK",
        "event_type": "hail",
        "total": 251,
        "by_year": {
          "2018": 12,
          "2019": 38,
          "2020": 19,
          "2021": 11,
          "2022": 13,
          "2023": 82,
          "2024": 76
        }
      }
    },
    {
      "type": "Feature",
      "id": "40027",
      "geometry": null,
      "properties": {
        "county_fips": "40027",
        "name": "Cleveland",
        "state": "OK",
        "event_type": "hail",
        "total": 220,
        "by_year": {
          "2018": 6,
          "2019": 18,
          "2020": 43,
          "2021": 17,
          "2022": 11,
          "2023": 92,
          "2024": 33
        }
      }
    },
    {
      "type": "Feature",
      "id": "40143",
      "geometry": null,
      "properties": {
        "county_fips": "40143",
        "name": "Tulsa",
        "state": "OK",
        "event_type": "hail",
        "total": 155,
        "by_year": {
          "2018": 3,
          "2019": 18,
          "2020": 32,
          "2021": 8,
          "2022": 17,
          "2023": 20,
          "2024": 57
        }
      }
    },
    {
      "type": "Feature",
      "id": "40031",
      "geometry": null,
      "properties": {
        "county_fips": "40031",
        "name": "Comanche",
        "state": "OK",
        "event_type": "hail",
        "total": 134,
        "by_year": {
          "2018": 4,
          "2019": 20,
          "2020": 32,
          "2021": 6,
          "2022": 8,
          "2023": 45,
          "2024": 19
        }
      }
    }
  ]
}
```

**With boundary geometry** — add `&geometry=true` (returns simplified polygons):
```bash
curl 'https://folkweather.com/edr/collections/hail/counties?state=OK&geometry=true'
# geometry field becomes a MultiPolygon instead of null
```

---

## Quick reference

| Endpoint | Use case |
|---|---|
| `/collections/hail/radius?coords=POINT(lon+lat)&within=1&within-units=mi` | Hail hits near an address |
| `/collections/wind/radius?coords=POINT(lon+lat)&within=1&within-units=mi` | Wind damage near an address |
| `/collections/tornado/radius?coords=POINT(lon+lat)&within=5&within-units=mi` | Tornado tracks near an address |
| `/collections/{type}/counties/{fips}` | All events for a county (cached 24h) |
| `/collections/{type}/counties/{fips}?year=2023` | Single year for a county |
| `/collections/{type}/counties/{fips}?years=2015/2024` | Year range for a county |
| `/collections/{type}/counties?state=OK&geometry=false` | Choropleth count index |
| `/collections/{type}/items?bbox={w},{s},{e},{n}&limit=500` | Events in viewport bbox |

### Magnitude units

| Collection | `magnitude_unit` | Meaning |
|---|---|---|
| `hail` | `"in"` | Hail diameter in inches |
| `wind` | `"kt"` | Wind speed in knots |
| `tornado` | `"EF"` | EF scale (0–5), also in `tor_f_scale` |

### Damage string format

`damage_property` and `damage_crops` are raw strings: `"2M"`, `"120.00K"`, `"0.00K"`.

```javascript
function parseDollars(raw) {
  if (!raw) return null;
  const m = raw.match(/^([\d.]+)([KMB]?)$/i);
  if (!m) return null;
  return parseFloat(m[1]) * ({ K: 1e3, M: 1e6, B: 1e9 }[m[2].toUpperCase()] ?? 1);
}
// parseDollars("2M")       → 2000000
// parseDollars("120.00K")  → 120000
// parseDollars("0.00K")    → 0
```
