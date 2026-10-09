# Lightning (GOES GLM) — Frontend Handoff

Near-real-time lightning flashes for the continental US, served from
`https://folkweather.com/edr` (CORS open to any origin). The machine-readable
contract is the OpenAPI spec at `https://folkweather.com/edr/api`; this document
is the human guide: how to poll it, what the numbers mean, and the one trap to
avoid.

Every number below was **measured on the production system** (October 2026), not
estimated.

## 1. What this is — and is not

One GeoJSON `Point` per **lightning flash** detected by the Geostationary
Lightning Mapper (GLM) on GOES-East (GOES-19) and GOES-West (GOES-18), for the
last 24 hours, CONUS only (`-125…-66` lon, `24…50` lat).

- **Not a safety system.** This is a data feed with no guarantees of
  availability or accuracy. Do not present it as a substitute for NWS warnings,
  and do not tell a user they are safe because the feed shows nothing.
- **Optical, from space, not ground strikes.** GLM sees light from cloud tops. It
  does not distinguish cloud-to-ground from in-cloud flashes, and its detection
  efficiency varies with the flash and the viewing angle. Treat counts as
  *relative activity*, not an exact tally.
- **Not instantaneous.** Expect a flash to appear **~30–60 seconds** after it
  happens (§7).
- **Position precision is ~8 km** (GLM pixel size at nadir, coarser at the edges
  of the disk), even though coordinates are given to 4 decimals.

## 2. Quick start

```bash
# Flashes in a map viewport, last 10 minutes (the default window)
curl 'https://folkweather.com/edr/collections/glm-lightning/items?bbox=-106,39,-104,41'

# Last hour, both satellites, within 50 km of a point
curl 'https://folkweather.com/edr/collections/glm-lightning/radius?coords=POINT(-105.27%2040.01)&within=50&within-units=km&window=PT1H'

# Only what's new since the last response (cursor — see §4)
curl 'https://folkweather.com/edr/collections/glm-lightning/items?bbox=-106,39,-104,41&after=123456'
```

| Request | Returns |
|---|---|
| `GET /collections/glm-lightning/items?bbox=minLon,minLat,maxLon,maxLat` | flashes in a viewport (omit `bbox` for all of CONUS) |
| `GET /collections/glm-lightning/area?coords=minLon,minLat,maxLon,maxLat` | same, EDR style |
| `GET /collections/glm-lightning/radius?coords=POINT(lon lat)&within=50&within-units=km` | flashes near a point (default 50 km, max 500 km) |
| `GET /collections/glm-lightning` | collection metadata |

## 3. The one thing to get right: an empty answer is ambiguous

`features: []` can mean **a genuinely quiet sky** — or **a feed that has died**.
A client that shows "No lightning nearby" in both cases will, on the day the
pipeline breaks, tell people it is clear while a storm is overhead.

So **every response says how current the data is**, independent of whether
anything flashed:

```json
{
  "features": [],
  "numberReturned": 0,
  "timeStamp":      "2026-10-07T22:30:12.431Z",
  "dataThrough":    "2026-10-07T22:29:40.000Z",
  "dataAgeSeconds": 32.4,
  "lastId": null
}
```

`dataThrough` is the end of the newest 20-second GLM granule we have processed
for the satellite(s) you asked about (for `satellite=both`, the *older* of the
two). Read it together with `features`:

| `features` | `dataAgeSeconds` | Meaning | Show |
|---|---|---|---|
| empty | small (< ~120) | genuinely quiet | "No lightning detected in the last 10 min" |
| empty | large (> ~180) | **feed is behind** | "Lightning data delayed — last update N min ago" |
| empty | `null` | unknown | "Lightning data unavailable" |
| any | large or `null` | stale / unknown | a warning banner, whatever the flashes say |

**Never treat `null` as current.** It means nothing has been ingested yet, or a
selected satellite has never reported.

Normal `dataAgeSeconds` is **~20–60 s** (measured). We alert on 10 minutes.

## 4. Polling efficiently: the cursor

Granules arrive every 20 seconds, so polling more often than **every 15–20
seconds** gains nothing. Re-downloading the whole window each time is wasteful;
use the cursor instead.

Every feature has a monotonic `id`; every response has `lastId` (the largest `id`
returned, or `null` when empty). Pass it back as `after`:

```js
const BASE = "https://folkweather.com/edr/collections/glm-lightning/items";
const MAX_AGE_S = 600;            // how long a flash stays on the map

let cursor = null;                // last id we've seen (null = need a fresh load)
let flashes = new Map();          // id -> { lon, lat, ageAtReceipt, receivedAt, energy }
let boxKey = null;

async function poll(bbox /* [w,s,e,n] */) {
  const key = bbox.join(",");
  // A cursor is global ("newer than id N"), not per-viewport. If the user pans or
  // zooms, flashes in the NEW area that we haven't seen are older than the cursor,
  // so start over for the new box.
  if (key !== boxKey) { flashes.clear(); cursor = null; boxKey = key; }

  const q = new URLSearchParams({ bbox: key, limit: 2000 });
  cursor === null ? q.set("window", "PT10M") : q.set("after", cursor);

  const r = await (await fetch(`${BASE}?${q}`)).json();
  const receivedAt = performance.now();           // monotonic: never the wall clock

  for (const f of r.features) {
    flashes.set(f.id, {
      lon: f.geometry.coordinates[0], lat: f.geometry.coordinates[1],
      ageAtReceipt: f.properties.age_seconds, receivedAt,
      energy: f.properties.energy_j,
    });
  }
  if (r.lastId !== null) cursor = r.lastId;       // empty page: keep the old cursor

  // Truncated? Keep paging immediately — with a cursor you get the OLDEST page first.
  if (r.numberReturned === 2000) return poll(bbox);

  // Expire by age, computed from the server's ages + our own elapsed time.
  const now = performance.now();
  for (const [id, f] of flashes)
    if (f.ageAtReceipt + (now - f.receivedAt) / 1000 > MAX_AGE_S) flashes.delete(id);

  return { flashes, stale: r.dataAgeSeconds === null || r.dataAgeSeconds > 180, dataAgeSeconds: r.dataAgeSeconds };
}
setInterval(() => poll(map.getBounds().toArray().flat()).then(render), 15000);
```

Notes:

- **Don't use the phone's clock.** `age_seconds` is "seconds between the flash and
  the server's `timeStamp`"; add your own elapsed time since you received it
  (above). A phone clock that is a few minutes off — common — then can't break
  fading or expiry.
- **With a cursor, no window is needed**: the cursor defines the position, and the
  lookback defaults to the full 24 h of retained data. Pass `window=` as well only
  if you want to cap it.
- **Truncation:** `numberReturned == limit` means there may be more. *Without*
  `after` you get the **newest** `limit` flashes; *with* `after` you get the
  **oldest** `limit` after the cursor, so repeating the request with the new
  `lastId` walks forward with no gaps.

## 5. Parameters

| Param | Applies to | Meaning |
|---|---|---|
| `bbox` / `coords` | items / area, radius | the area (see §2). `items` without `bbox` = all CONUS |
| `window` | all | server-relative period, ISO-8601 duration: `PT10M` (default), `PT1H`, … max `PT24H` |
| `datetime` | all | explicit RFC-3339 interval `start/end`; `..` for an open end. A lone instant is rejected. Mutually exclusive with `window` |
| `after` | all | cursor: only flashes with `id` > this (§4) |
| `satellite` | all | `goes-east` (default), `goes-west`, or `both` — **read §6 first** |
| `limit` | all | default 1000, max 10,000 |
| `within`, `within-units` | radius | default 50 km; max 500 km |

Prefer `window` over `datetime`: it is relative to the **server's** clock, so the
phone never has to produce a correct timestamp. Lower bounds older than 24 h are
clamped to 24 h; nothing older exists.

## 6. East vs West: why the default is East only

Both satellites see most of CONUS. We **measured** the overlap on live data: about
**51 % of East's flashes are also reported by West, and 75 % of West's by East**
(same flash: within 0.25 s and 30 km). So `satellite=both` draws roughly half of
all flashes twice. There is **no cross-satellite de-duplication**.

- Default `goes-east` is a good single view for the central and eastern US,
  including Colorado.
- Use `goes-west` for the Pacific coast and far west, where West has the better
  viewing angle.
- Use `both` only if you de-duplicate yourself (e.g. merge flashes within ~0.25 s
  and ~30 km), or you specifically want the union to improve coverage.

## 7. How current is it, really? (measured)

| Stage | Measured |
|---|---|
| A flash happens somewhere in a 20 s observation window | 0–20 s (avg 10) |
| NOAA publishes the granule ~14 s after the window closes; we poll every 15 s and ingest | window end → stored: **median 20 s, p90 35 s, max 66 s** |
| **Flash → visible in this API** | **typically 30–60 s** |

`dataAgeSeconds` is measured from the *end of the window* to the response, so a
healthy value is ~20–60 s.

## 8. Response reference

```json
{
  "type": "FeatureCollection",
  "features": [{
    "type": "Feature",
    "id": 123456,
    "geometry": { "type": "Point", "coordinates": [-105.2517, 40.0183] },
    "properties": {
      "flash_time": "2026-10-07T22:29:31.482Z",
      "age_seconds": 41.0,
      "satellite": "goes-east",
      "energy_j": 4.7e-14,
      "quality": 0
    }
  }],
  "numberReturned": 1,
  "timeStamp": "2026-10-07T22:30:12.431Z",
  "lastId": 123456,
  "dataThrough": "2026-10-07T22:29:40.000Z",
  "dataAgeSeconds": 32.4
}
```

| Field | Notes |
|---|---|
| `id` | monotonic change-feed cursor; features are returned **oldest first** |
| `flash_time` | time of the flash's first optical event, UTC, ms precision |
| `age_seconds` | flash → `timeStamp`, by the **server's** clock (§4) |
| `energy_j` | optical radiant energy, joules (~1e-15 – 1e-12); `null` if missing. Useful as a *relative* intensity for sizing/colour; it is not calibrated lightning current |
| `quality` | `0` good; `1`, `3`, `5` degraded (events out of order / too many events / duration exceeded). Degraded flashes are real and are **included** — dim them if you like, don't drop them |
| `lastId` | largest `id` in this response; `null` when empty — keep your previous cursor |

## 9. Sizing

| | Measured |
|---|---|
| bytes per feature (uncompressed JSON) | **224** |
| on the wire (gzip, which the API serves) | **~23 bytes/feature** — 564 flashes = 12.8 KB |
| response time | ~0.1–0.25 s |
| CONUS flash rate, East (October evening) | ~49 per 20 s ≈ **150/min** |
| CONUS flash rate, West | ~35 per 20 s ≈ 105/min |

A CONUS-wide 10-minute query on East alone already returns ~1,300–1,500 flashes (1,346 when measured) —
**more than the default `limit` of 1000, so it is truncated**. In summer storm
season expect several times that. Therefore:

- **Query the viewport** (`bbox`), not all of CONUS.
- Keep `limit` ≥ what a viewport can plausibly hold, and handle `numberReturned == limit`.
- Poll with the cursor (§4): steady-state responses are a few hundred bytes.

## 10. Errors

JSON exception bodies, standard codes: `400` bad parameter (message says which:
unknown `satellite`, malformed or oversized `window`, `datetime` that is a lone
instant or combined with `window`, negative `after`, inverted `bbox`, radius > 500
km), `404` unknown collection, `500` server error. A `5xx` or network failure is
**not** a quiet sky: keep the previous data, flag it as stale, and retry.

## 11. Stability

New fields may be added to features or the envelope at any time — ignore fields
you don't know. Existing fields won't change meaning. The polling interface in
this document is the supported one; a push stream may be added later without
removing it.

Anything not yet available: lightning outside CONUS, ground-strike
classification, flash duration/area, history beyond 24 h.
