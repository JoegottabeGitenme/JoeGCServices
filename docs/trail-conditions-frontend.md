# Trail Conditions API — Frontend Handoff

Everything here is served from `https://folkweather.com/edr` (CORS is open to
any origin, including preflight). The machine-readable contract is the
OpenAPI spec at `https://folkweather.com/edr/api` — this document is the
human guide: what the numbers mean, what they don't, and how to build on them.

For the physics and validation behind the numbers, see
`docs/trail-conditions-design.md` and the public explainer at `/science.html`.

## 1. What you get

For each trail segment (one OpenStreetMap way) in the covered region, an
**hourly, terrain-aware soil-moisture estimate** and a frozen-ground signal,
available as:

- **current conditions** merged into any trail query (`?conditions=latest`), and
- a **per-trail hourly series** — recent past plus the forecast — for charts.

Coarse weather-model soil moisture (HRRR, ~3 km) can't tell a creek-bottom
trail from a ridge trail 500 m away. We downscale it with 10 m terrain and
soil data, so the wet drainage and the dry ridge get different values.

### What this is **not** — read before designing the UI

- **Not a rideability verdict.** There is no "good / marginal / closed" field,
  deliberately. Soil moisture and ground state are physical *inputs* to a
  judgment about a trail, not the judgment. A trail can be dry and still be
  closed (wildlife, fire, ownership); a wet one may ride fine (rock, sand).
- **Not an observation.** These are modeled values (a weather-model nowcast),
  not sensor readings at the trail.
- **Validated for the physics, not yet for rider outcomes.** The downscaling
  is validated against real ground-truth soil moisture at independent
  research sites; it has **not** been checked against rider-reported trail
  conditions. Label it an advisory estimate in your UI.
- **Surface matters and is not modeled.** Sand, rock and hardpack respond to
  moisture very differently from clay. We give you soil *wetness*, not a
  trail-surface response.

## 2. Quick start

```bash
# 1. Trails in a map viewport, with current conditions
curl 'https://folkweather.com/edr/collections/trails/items?bbox=-105.42,39.93,-105.36,39.98&conditions=latest&limit=200'

# 2. Find a trail by name
curl 'https://folkweather.com/edr/collections/trails/items?q=forsythe&conditions=latest'

# 3. Forecast series for one trail (feature_id from the responses above)
curl 'https://folkweather.com/edr/collections/trails/items/956937928/conditions'

# 4. Where do conditions exist at all?  (see "Coverage")
curl 'https://folkweather.com/edr/collections/trails' | jq .conditions_coverage
```

```js
const res = await fetch(
  "https://folkweather.com/edr/collections/trails/items" +
  `?bbox=${west},${south},${east},${north}&conditions=latest&limit=1000`
);
const { features } = await res.json();          // GeoJSON FeatureCollection
for (const f of features) {
  const c = f.properties.conditions;            // may be absent -- see below
  const color = c?.saturation != null ? ramp(c.saturation) : GREY;
}
```

## 3. Endpoints

| Request | Returns |
|---|---|
| `GET /collections/trails/items?bbox=…` | Trails in a viewport (GeoJSON `LineString` features) |
| `GET /collections/trails/items?q=<name>` | Name search; takes precedence over `bbox` |
| `GET /collections/trails/area?coords=minLon,minLat,maxLon,maxLat` | Same as bbox, EDR style |
| `GET /collections/trails/radius?coords=POINT(lon lat)&within=5&within-units=km` | Trails near a point |
| `GET /collections/trails/items/{feature_id}/conditions` | Hourly series for one trail (plain JSON) |
| `GET /collections/trails` | Collection metadata incl. `conditions_coverage` |

All three list endpoints (`items`, `area`, `radius`) accept `&conditions=latest`,
`class=mtb_trail\|hiking_trail\|track\|bridleway`, and `limit` (default 1000,
max 5000); `items` also takes `offset`. All three return `numberReturned`, so
**`numberReturned === limit` means the result was truncated** — narrow the
viewport or raise `limit`.

> **History:** `radius` and `area` used to ignore `limit` and `class` (the
> parameters were silently dropped, so `radius?limit=3` returned up to 1000
> features and `class=` had no effect), and returned no `numberReturned`.
> Fixed in Session 14; if you worked around it by using `items?bbox=`, that
> still works and remains a fine choice (it also supports `offset` paging).

## 4. The `conditions` block

With `?conditions=latest`, each feature that has data gets
`properties.conditions`:

```json
"conditions": {
  "run_time":        "2026-10-05T19:00:00+00:00",
  "valid_time":      "2026-10-05T21:00:00+00:00",
  "forecast_hour":   2,
  "soil_moisture":   0.184,
  "saturation":      0.42,
  "frozen_fraction": 0.0,
  "confidence":      1.0,
  "model_version":   "trail-physics-v1"
}
```

| Field | Meaning |
|---|---|
| `saturation` | **Use this for display.** 0–1: the fraction of the soil's pore space holding water (0 dry → 1 saturated). Unitless, so a color ramp on it means the same thing in sandy and clay soils. `null` where there is no terrain/soil data (no porosity is guessed). |
| `soil_moisture` | Volumetric soil moisture, m³/m³, ~4 cm depth. Physically meaningful but hard to read without the soil's porosity (0.20 is nearly saturated in one soil, bone dry in another). Provided for analysis, not for coloring. |
| `frozen_fraction` | 0–1: fraction of the segment's vertices where soil temperature is ≤ 273.15 K (0 °C). A *fraction*, not a boolean, so a partly frozen segment (shaded north side) stays representable. |
| `confidence` | 0–1: the fraction of the segment's vertices that received real terrain-informed downscaling. **1.0** = fully downscaled. **0.0** = raw 3 km model value, no terrain applied. Between = the segment straddles the edge of coverage. This is a *data-coverage* measure, not a statistical error bar — it does not say how accurate the number is. |
| `valid_time` | The hour the values describe. For `latest`, the most recent hour at or before now that has data — never a future forecast hour. |
| `run_time`, `forecast_hour` | The model run (HRRR initialization) the value came from, and hours from it. Mostly useful for showing "as of". |
| `model_version` | Version of the physics. A new version means the methodology changed: **do not compare values across versions**, and don't cache across them. |

**Absent vs. null.** `conditions` is *absent* (never `null`) on trails with no
data. Within it, `saturation`/`soil_moisture`/etc. can be `null` when
unavailable. Always check both.

## 5. Coverage

Conditions currently exist for the **Front Range foothills** only (Fort
Collins to Colorado Springs, plains edge up through the foothills). Trails
elsewhere in Colorado have geometry but no `conditions`.

`GET /collections/trails` returns `conditions_coverage.bbox`
(`[minLon, minLat, maxLon, maxLat]`) — use it to hide or grey out the
conditions UI for out-of-region viewports **without fetching them first**.

It is deliberately a *rectangle* and is wider than the region's real edge (the
underlying grid is rotated, so its corners have no terrain data). **The
authoritative per-trail signal is the data itself**: no `conditions` block, or
`confidence: 0`, means "no terrain-informed estimate here." Treat
`confidence < 1` as "partial — show it differently, don't hide it."

**How much of the rectangle is real coverage (measured on the live data):** of
the ~66,400 trail segments inside it, about **87% have `confidence: 1`**, a few
hundred are partial, and about **13% have `confidence: 0`** — they sit in the
rectangle's corners where there is no terrain data, so they carry the raw 3 km
model value and `saturation: null`. Expect that mix, and render `confidence: 0`
like "no terrain-informed estimate" rather than as a real reading.

Coverage will grow. Build against the field, not a hard-coded box.

## 6. Freshness and caching

- The model updates **hourly**; the service picks up each new forecast hour
  within about a minute of it landing, and a full pass over the region takes on
  the order of 20 seconds per forecast hour.
- `latest` is a *model nowcast for the current hour*, so `valid_time` is
  normally within the last hour (measured: 30 minutes behind on a live query).
  It is **never a future hour** — an earlier build of this API briefly returned
  forecast hours as "latest"; that was fixed before this contract was published. If the upstream feed stalls it will
  fall behind — show `valid_time`, and treat a `valid_time` more than ~3 hours
  old as stale.
- Responses with `conditions` are cacheable for **5 minutes**
  (`Cache-Control: max-age=300`); geometry-only responses for 1 hour (trail
  geometry changes weekly). Don't cache conditions longer than that.

## 7. The per-trail series

`GET /collections/trails/items/{feature_id}/conditions`

```json
{
  "feature_id": 956937928,
  "name": "Forsythe Canyon Trail",
  "run_time": "2026-10-05T19:00:00+00:00",
  "model_version": "trail-physics-v1",
  "conditions": [
    { "valid_time": "2026-10-05T17:00:00+00:00", "forecast_hour": 3,
      "run_time": "2026-10-05T14:00:00+00:00",
      "soil_moisture": 0.176, "saturation": 0.40, "frozen_fraction": 0.0,
      "confidence": 1.0, "model_version": "trail-physics-v1" },
    "…"
  ]
}
```

- **One point per hour**, ascending `valid_time`, from a few hours ago through
  the end of the forecast horizon. Each point is taken from the **newest
  model run that has a value for that hour** — so the series is stitched
  across runs and each point carries its own `run_time`.
- **The horizon is whatever the data reaches** — read the last `valid_time`;
  don't assume a fixed number of hours. It is typically **around a day** (we
  measured 24.5 h ahead on a live trail) and varies with which model runs have
  landed: the 6-hourly runs reach further than the hourly ones. The far end of
  the series comes from older runs than the near end — that's expected, and
  each point's `run_time` says so.
- To mark "now" on a chart, use `valid_time`, not `forecast_hour` (which is
  relative to each point's own run).
- Unknown `feature_id` → **404**. A known trail with no data (outside
  coverage) → **200 with `"conditions": []`**. Handle both.

## 8. Suggested UI patterns

- **Color trails by `saturation`** with a sequential ramp (dry → wet). Pick
  your own thresholds and **say they're yours** ("wetter than ~60%"), since the
  API intentionally supplies none.
- **Overlay frozen ground** when `frozen_fraction > 0` (hatching or an icon).
  Frozen-and-firm and thawed-and-soft are very different trail states that the
  same saturation can hide — it's why this is a separate field.
- **De-emphasize low `confidence`** (dashed or translucent line), don't hide it.
- **No `conditions` → neutral grey**, labeled "no conditions data here."
- **Show "as of `valid_time`"** and the advisory caveat from §1 near the legend.
- **Gate by zoom.** A bbox query for 5000 trails with geometry and conditions is
  a large payload. Fetch conditions at viewport zoom levels where individual
  trails are legible; use the series endpoint only on a selected trail.

## 9. Operational notes

- **Errors** are JSON exception bodies with standard HTTP codes: `400` bad
  params, `404` unknown collection/trail, `500` server error.
- **Paging:** `limit` + `offset` on `items`. Bbox results are not guaranteed to
  be spatially sorted.
- **Stability:** new fields may be added to `conditions` at any time — ignore
  fields you don't know. Existing fields won't change meaning without a new
  `model_version`.
- **No auth, no key**, currently no published rate limit — be reasonable, and
  tell us before building something that polls aggressively.
- **Send a real `User-Agent` from scripts and servers.** The gateway returns
  `403` to default scripting agents (e.g. Python's `Python-urllib`). Browsers are
  unaffected; `curl` works as-is.

## 10. Trail metadata and its gaps

Each feature also carries OSM-derived properties: `feature_id`, `feature_class`
(`mtb_trail` \| `hiking_trail` \| `track` \| `bridleway`), `name`, `system`,
`region`, `active`, `updated_at`, and raw OSM `tags`.

- **`name` is often `null`** — a large share of OSM ways are unnamed (spurs,
  connectors, un-surveyed paths). Don't assume a label; fall back to
  `feature_class` or the raw `tags`.
- `feature_class` is derived from OSM tags; expect occasional misclassification
  on ambiguous tagging.
- `system` (trail-system grouping) is usually `null` — OSM doesn't reliably tag
  it per way. Don't depend on it.
- **Surface, difficulty and length are not parsed or normalized.** If OSM has
  them they are inside the raw `tags` (`surface`, `mtb:scale`, …); we don't
  clean or compute them.
- One feature per OSM way, **not merged per named trail** — a long trail is
  many features. This is intentional (part of a trail can be wet while another
  part is dry), but a "trail" in your UI is probably a group of features you'll
  need to assemble by `name`.
- `active: false` means the way vanished from the latest weekly OSM sync.

## 11. Raw weather ingredients (optional)

If you want to build your own logic instead of (or beside) the trail
conditions, the underlying HRRR fields are queryable at any point:

```
GET /collections/hrrr-soil/position?coords=POINT(-105.2 39.7)&parameter-name=SOILW&z=4
GET /collections/hrrr-soil/position?coords=POINT(-105.2 39.7)&parameter-name=TSOIL&z=4
GET /collections/hrrr-snow/position?coords=POINT(-105.2 39.7)&parameter-name=SNOD
```

All values are native SI units (`TSOIL` Kelvin, `SOILW` fraction 0–1, `SNOD`
metres, `WEASD` kg/m² ≈ mm SWE, `PRATE` kg/m²/s). These are the **raw 3 km**
values — the trail `conditions` above are these, downscaled with terrain.

## 12. Not available yet (don't build against it)

- A rideability / condition class, or "firm until X" times — gated on
  validation against rider reports.
- Snow depth/cover per trail (the raw `hrrr-snow` fields exist; a per-trail
  product does not).
- Coverage beyond the Front Range foothills.
- Trailhead *name search* (trailheads exist as `location_type=trailhead` in
  `/edr/locations`, but `?q=` isn't wired for them).
