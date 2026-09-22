# Trail Conditions — Frontend Handoff (Session 1: ingredients + geometry)

All endpoints live at `https://folkweather.com/edr`. See
`docs/trail-conditions-design.md` for the full design and what's deferred.

**Important framing for the UI:** nothing shipped this session computes or
judges rideability. This is geometry + raw weather ingredients. The app is
expected to combine them (query the grid along the trail's own geometry,
apply your own thresholds/coloring) until a validated server-side condition
parameter exists — see the design doc's validation section for why that's a
deliberate sequencing choice, not a missing feature.

## 1. Trail geometry — `trails` collection

One feature per OSM way (not merged per trail system) — so a rider can see
one part of a trail rideable and another not, once your app layers condition
data on top.

```
GET /edr/collections/trails/items?bbox=-105.3,39.6,-105.1,39.8
GET /edr/collections/trails/items?q=apex
GET /edr/collections/trails/items?q=apex&class=mtb_trail
GET /edr/collections/trails/area?coords=-105.3,39.6,-105.1,39.8
GET /edr/collections/trails/radius?coords=POINT(-105.2 39.7)&within=5&within-units=km
```

Response is a standard GeoJSON `FeatureCollection` of `LineString` features:

```json
{
  "type": "Feature",
  "id": 123456789,
  "geometry": { "type": "LineString", "coordinates": [[-105.21, 39.73], ...] },
  "properties": {
    "feature_id": 123456789,
    "feature_class": "mtb_trail",
    "name": "Enchanted Forest",
    "system": null,
    "region": "colorado",
    "active": true,
    "updated_at": "2026-09-20T04:12:33Z",
    "tags": { "highway": "path", "surface": "dirt", "mtb:scale": "2", "...": "..." }
  }
}
```

- **`feature_class`**: `mtb_trail` | `hiking_trail` | `track` | `bridleway` — derived from OSM tags (see design doc for the exact rule). Not user-curated; expect occasional misclassification on ambiguous OSM tagging.
- **`system`**: usually `null`. OSM doesn't reliably tag trail-system grouping at the way level; don't build UI that depends on it being populated.
- **`active`**: `false` means the way was missing from the most recent weekly sync (deleted/retagged/redrawn upstream). Not currently filterable out of `bbox`/`radius`/`area` results — **only `?q=` and `bbox` items queries currently return active-only** (bbox/area/radius all filter `active=true` server-side already; this note is just to flag that the field exists in properties for your own use, e.g. graying out a feature that just went inactive rather than having it vanish instantly).
- **`?q=`** does substring/prefix name search (same normalization as city search — accent/punctuation-insensitive), ranked exact > prefix > substring. Takes precedence over `bbox` when both are given.
- **No `datetime`, no temporal extent** — trails aren't time-series data.
- **Limits**: default 1000 features per response, max 5000. Bump `?limit=` if a bbox query gets truncated.
- **No `/locations` endpoint on this collection** (a pre-existing EDR-API limitation, not new to trails — storm events have the same gap despite documenting `/locations` in their config comments). Use `/items` for everything.

## 2. Trailheads — the shared `locations` registry

Trailheads are points, stored alongside populated places/ZIPs/airports in the
same registry, with `location_type=trailhead` and id `TH<osm_node_id>`.

```
GET /edr/locations                              # includes trailheads, unfiltered listing
GET /edr/locations/TH123456?collections=hrrr-soil,hrrr-snow   # forecast proxy
```

- **Name search (`?q=`) is NOT wired for trailheads** in this session — only
  the unfiltered listing and direct-by-id lookup work. If you need "type a
  trailhead name, get a match," that's a small fast-follow (see design doc's
  "Known gaps"), not yet built. For now, get trailhead ids from a `trails`
  bbox query area you already know, or from the unfiltered `/edr/locations`
  list client-side.
- The `/edr/locations/{id}?collections=...` forecast-proxy works exactly
  like it does for any other location type (airports, cities) — no code
  change was needed, it's generic. Pass whatever `hrrr-*` collections you want.

## 3. Weather ingredients — `hrrr-soil` and `hrrr-snow`

**All values are native SI units — convert client-side** (same house rule as every other collection in this API).

### `hrrr-soil` (extended this session)

| Parameter | Units | Levels (`z`, cm below ground) |
|---|---|---|
| `TSOIL` (soil temperature) | Kelvin | 0, 4, 10, 30, 100 |
| `SOILW` (volumetric soil moisture) | fraction 0–1 | 0, 1, 4, 10, 30 |

Note the two parameters have **different depth sets** — don't assume `z=100`
works for `SOILW` (it doesn't; max is 30 for that parameter).

**Frozen-state proxy** (soil ice fraction isn't available from HRRR — see
design doc): treat `TSOIL <= 273.15` (0°C) at the shallow depths (`z=0` or
`z=4`) as frozen. This is what a future server-side condition class will
also be built on, so it's a reasonable client-side approximation today.

```
GET /edr/collections/hrrr-soil/position?coords=POINT(-105.2 39.7)&parameter-name=SOILW&z=4
GET /edr/collections/hrrr-soil/position?coords=POINT(-105.2 39.7)&parameter-name=TSOIL&z=4&datetime=2026-09-22T00Z/2026-09-24T00Z
```

### `hrrr-snow` (new this session)

| Parameter | Units | Notes |
|---|---|---|
| `WEASD` | kg/m² | Numerically equals mm of snow water equivalent |
| `SNOD` | m | Snow depth |
| `PRATE` | kg/m²/s | Numerically equals mm/s; multiply by 3600 for mm/hr |
| `CRAIN` | 0 or 1 | Categorical rain flag |
| `CSNOW` | 0 or 1 | Categorical snow flag |
| `CFRZR` | 0 or 1 | Categorical freezing rain flag |
| `CICEP` | 0 or 1 | Categorical ice pellets flag |
| `DLWRF` | W/m² | Downward longwave radiation (already had `DSWRF` on `hrrr-surface`) |

All at `surface` level (no `z` param needed — omit it).

```
GET /edr/collections/hrrr-snow/position?coords=POINT(-105.2 39.7)&parameter-name=SNOD
GET /edr/collections/hrrr-snow/position?coords=POINT(-105.2 39.7)&parameter-name=PRATE&datetime=2026-09-22T00Z/2026-09-24T00Z
```

- **Horizon**: 48h forecast, hourly, same cadence/query contract as `hrrr-surface`/`hrrr-soil`.
- The precip-type flags are mutually exclusive per HRRR's own diagnostic scheme — you generally only need to check which one is `1`.

## 4. Recommended app pattern: color a trail by querying along its geometry

Since there's no per-segment precompute yet, the intended pattern is:

1. `GET /edr/collections/trails/items?bbox=<viewport>` → get LineString features in view.
2. For each feature (or a sampled midpoint if you want to batch fewer requests — HRRR is 3km native, so adjacent points on the same way will usually return identical values anyway), query `hrrr-soil`/`hrrr-snow` at that point.
3. Apply your own thresholds (e.g. `TSOIL < 273.15` → frozen, `SOILW > X` → wet) and color the LineString accordingly.

This is explicitly how the design intends v1 to work — see
`docs/trail-conditions-design.md`'s "Scope corrections" section for why
server-side per-segment precompute was deliberately deferred rather than
missed.

## 5. What's coming later (not yet available, don't build against it)

- Any `condition_class`, `softness_index`, `firm_until`/`firm_from` parameter — gated on physics validation (design doc §8).
- Trail-system grouping (multiple ways under one named system).
- Trailhead name search.
- Any statewide precomputed condition grid.
