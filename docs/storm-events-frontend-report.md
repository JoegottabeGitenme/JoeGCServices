# Storm Events API — Frontend Integration Report

**Deployed:** June 5, 2026  
**Base URL:** `https://folkweather.com/edr`  
**Data source:** NOAA Storm Events Database, 1995–present

---

## What's available

Three event-type collections, each a GeoJSON FeatureCollection:

| Collection | Geometry | Record count |
|---|---|---|
| `hail` | `Point` | ~175k events |
| `wind` | `Point` | ~175k events |
| `tornado` | `LineString` (or `Point` if no end coords) | ~17k events |

The downloader re-pulls data daily. The full historical archive is still being
backfilled in the background and grows by ~20k–30k events per year as each year
completes.

---

## Every event feature now carries these properties

```json
{
  "type": "Feature",
  "id": 10334009,
  "geometry": {
    "type": "LineString",
    "coordinates": [[-96.13, 40.89], [-96.12, 40.91]]
  },
  "properties": {
    "event_id":          10334009,
    "event_type":        "tornado",
    "datetime":          "1995-05-08T23:15:00+00:00",
    "begin_time":        "1995-05-08T23:15:00+00:00",
    "end_time":          "1995-05-08T23:20:00+00:00",

    "magnitude":         0,
    "magnitude_unit":    "EF",
    "tor_f_scale":       0,

    "state":             "NEBRASKA",
    "county_name":       "CASS",
    "county_fips":       "31025",

    "event_narrative":   "Cass County Emergency Services had 40 to 50 spotters throughout the county with several reports of brief tornadoes touching down near Greenwood and Murdock.",
    "episode_narrative": "A squall line moved through the region producing multiple brief tornadoes across southeast Nebraska.",
    "report_source":     "Trained Spotter",
    "damage_property":   "10.00K",
    "damage_crops":      "0.00K",
    "injuries_direct":   0,
    "deaths_direct":     0,
    "begin_location":    "2.5 SW GREENWOOD",
    "tor_length_mi":     0.1,
    "tor_width_yd":      10.0
  }
}
```

### Field reference

| Field | Type | Notes |
|---|---|---|
| `event_narrative` | `string \| null` | The per-event comment text — **same text shown on the NOAA/SPC Storm Events site** when you click an event. Present on most events from 1996+. |
| `episode_narrative` | `string \| null` | Storm-system-level narrative (shared across all events in the same storm episode). |
| `report_source` | `string \| null` | Who reported it: `"Trained Spotter"`, `"ASOS"`, `"Law Enforcement"`, `"NWS Employee"`, `"911 Call Center"`, etc. |
| `damage_property` | `string \| null` | Raw property damage string as NOAA recorded it, e.g. `"120.00K"`, `"2.50M"`, `"0.00K"`. Parse the suffix client-side if you need a number (`K`=×1000, `M`=×1000000, `B`=×1000000000). |
| `damage_crops` | `string \| null` | Raw crop damage string, same format. |
| `injuries_direct` | `number \| null` | Direct injuries (integer). |
| `deaths_direct` | `number \| null` | Direct deaths (integer). |
| `begin_location` | `string \| null` | Human-readable begin location, e.g. `"3.94 ESE BELKNAP"` (range + compass bearing + nearest city). |
| `tor_length_mi` | `number \| null` | Tornado path length in **miles**. Tornadoes only. |
| `tor_width_yd` | `number \| null` | Tornado path width in **yards**. Tornadoes only. |

**Nullability:** All new fields are `null` on events that predate the narrative
columns in the CSV format (pre-1996 records often lack narratives/damage), and
temporarily `null` on events ingested before today's re-ingest completes. The
background re-ingest fills them in via `ON CONFLICT (event_id) DO UPDATE`, so
they appear without any app downtime.

---

## Endpoints

### The "this house" flow

Three calls per address (geocode first, then):

```
GET /edr/collections/hail/radius?coords=POINT({lon}+{lat})&within=1&within-units=mi
GET /edr/collections/wind/radius?coords=POINT({lon}+{lat})&within=1&within-units=mi
GET /edr/collections/tornado/radius?coords=POINT({lon}+{lat})&within=5&within-units=mi
```

### Per-county viewport rendering (cache-optimised)

One call per visible county, cached 24 h with ETag revalidation:

```
GET /edr/collections/{hail|wind|tornado}/counties/{fips}
GET /edr/collections/{hail|wind|tornado}/counties/{fips}?year=2023
GET /edr/collections/{hail|wind|tornado}/counties/{fips}?years=2015/2024
```

Response includes county metadata alongside features:
```json
{
  "type": "FeatureCollection",
  "county_fips": "40051",
  "county_name": "Cleveland",
  "state": "OK",
  "event_type": "tornado",
  "numberReturned": 47,
  "features": [...]
}
```

**Cache headers:** `Cache-Control: public, max-age=86400, stale-while-revalidate=604800`  
**ETag:** Weak ETag from `max(ingested_at)` for the county — supports `If-None-Match` → `304 Not Modified`

### County aggregate choropleth index

```
GET /edr/collections/{type}/counties?state=OK&geometry=false     # counts only
GET /edr/collections/{type}/counties?state=OK&geometry=true      # + simplified boundaries
```

### Viewport bbox queries

```
GET /edr/collections/{type}/items?bbox={w},{s},{e},{n}&limit=500
GET /edr/collections/{type}/items?bbox={w},{s},{e},{n}&limit=500&offset=500
```

### Datetime filtering

| Format | Example | Meaning |
|---|---|---|
| Bare years | `2015/2024` | Year range |
| RFC3339 interval | `2013-05-20T00:00:00Z/2013-05-21T00:00:00Z` | Precise range |
| Single year | `year=2023` | Per-county endpoint only |
| Open end | `2020/..` | 2020 to present |
| Omitted | — | Full archive (1995–present) |

---

## Live sample queries

```bash
# Hail events near Oklahoma City, last 5 years
curl 'https://folkweather.com/edr/collections/hail/radius?coords=POINT(-97.5+35.5)&within=25km&datetime=2020/..'

# Tornado tracks in Cleveland County OK, full history
curl 'https://folkweather.com/edr/collections/tornado/counties/40051'

# County aggregate — hail counts in Oklahoma, 2010-2024
curl 'https://folkweather.com/edr/collections/hail/counties?state=OK&datetime=2010/2024&geometry=false'

# 1995 Cass County NE tornado events with narratives (verified live)
curl 'https://folkweather.com/edr/collections/tornado/counties/31025?years=1995/1995'
```

---

## Client-side tips

**Parse damage amounts:**
```javascript
function parseDamage(raw) {
  if (!raw || raw === '0.00K') return 0;
  const m = raw.match(/^([\d.]+)([KMB]?)$/i);
  if (!m) return null;
  const n = parseFloat(m[1]);
  return n * ({ K: 1e3, M: 1e6, B: 1e9 }[m[2].toUpperCase()] ?? 1);
}
// parseDamage("120.00K") → 120000
// parseDamage("2.50M")   → 2500000
```

**Filter tornadoes by EF scale:**
```javascript
const significant = features.filter(f => (f.properties.tor_f_scale ?? -1) >= 2);
```

**Filter by hail size:**
```javascript
const golfBall = features.filter(f => (f.properties.magnitude ?? 0) >= 1.75);
const baseball = features.filter(f => (f.properties.magnitude ?? 0) >= 2.75);
```

**Show narrative with fallback:**
```javascript
const text = feature.properties.event_narrative
  ?? feature.properties.episode_narrative
  ?? 'No narrative available for this event.';
```

---

## Data availability notes

- **Narratives populated:** Events ingested/updated after June 5, 2026 have narratives.
  A full historical re-ingest is running in the background (daily, ~1 GB total).
  All years will have narratives within ~1–2 days.
- **Pre-1996 records:** Older records often have sparse narratives and damage
  amounts in the source data — this is a NOAA data limitation, not a bug.
- **ETag invalidation:** Per-county ETag changes after a monthly data refresh,
  triggering automatic revalidation for browsers/CDNs that cached the county.

---

*Source code: `main` branch — commit `7a68505` and earlier.*  
*Full API reference: `docs/storm-events-api.md`*
