# Soil Temperature & QPE Datasets — Frontend Handoff

Delivery for the Part A EDR dataset spec (frozen-subgrade veto + subgrade
saturation). All endpoints live at `https://folkweather.com/edr`.

## Summary of what shipped vs the spec

| Spec item | Delivered | Notes |
|---|---|---|
| `hrrr-soil` | ✅ `hrrr-soil` | TSOIL at **point depths** (HRRR uses the RUC land model, not layers — see below) |
| `gfs-soil` | ✅ `gfs-soil` + `gfs-soil-latest` | Noah layers incl. 0–0.1 m top layer; **384h horizon** (GFS extended from 120h) |
| `mrms-qpe` | ✅ `mrms-qpe` + `mrms-qpe-latest` | Param names are `QPE_24H` / `QPE_72H` (underscore — not `QPE24H`/`QPE72H`); `QPE_01H` included as a bonus |

## 1. `hrrr-soil`

- **Parameter**: `TSOIL` (soil temperature), units **Kelvin** (convert client-side, same as TMP)
- **Levels**: HRRR's land model (RUC) outputs **point depths**, not 0–0.1 m
  layers. Available depths via `z` (cm below ground): `0, 4, 10, 30, 100`.
  For the frozen-subgrade veto, `z=4` or `z=10` is the closest analog to a
  "0–0.1 m top layer" reading; `z=0` is the soil surface (skin).
- **Horizon**: 18h (48h on 00/06/12/18z cycles), hourly cycles, CONUS 3 km
- **Query contract**: identical to `hrrr-surface` (position, datetime ranges,
  interpolation behavior all inherited)

```
GET /edr/collections/hrrr-soil/position?coords=POINT(-95.5 37.2)&parameter-name=TSOIL&z=10
GET /edr/collections/hrrr-soil/position?coords=POINT(-95.5 37.2)&parameter-name=TSOIL&z=10&datetime=2026-07-02T06:00:00Z/2026-07-03T00:00:00Z
```

Omitting `z` returns the first configured depth (0 cm). Always pass `z`.

## 2. `gfs-soil` (+ `gfs-soil-latest`)

- **Parameter**: `TSOIL`, units **Kelvin**
- **Levels**: Noah land-model **layers**, addressed by layer top in cm via `z`:

| `z` | Layer |
|---|---|
| `0` | 0–0.1 m (the spec's top layer) |
| `10` | 0.1–0.4 m |
| `40` | 0.4–1 m |
| `100` | 1–2 m |

- **Horizon**: **384h (16 days)** — hourly to 120h, 3-hourly beyond. This
  covers the 24–48h cure window far past HRRR's 18h, per the spec's fallback
  requirement. (Note: GFS was extended from 120h to 384h as part of this
  work; runs ingested before 2026-07-02 only reach 120h.)
- Resolution 0.25° (~22 km), 4 cycles/day (00/06/12/18z, ~4h latency)

```
GET /edr/collections/gfs-soil/position?coords=POINT(-95.5 37.2)&parameter-name=TSOIL&z=0&datetime=2026-07-02T00:00:00Z/2026-07-04T00:00:00Z
GET /edr/collections/gfs-soil-latest/position?coords=POINT(-95.5 37.2)&parameter-name=TSOIL&z=0
```

## 3. `mrms-qpe` (+ `mrms-qpe-latest`)

- **Parameters**: `QPE_01H`, `QPE_24H`, `QPE_72H` — rolling MultiSensor
  Pass 2 accumulations, units **mm**. ⚠️ Note the underscores; the spec's
  `QPE24H`/`QPE72H` spellings will 400.
- **Update cadence**: hourly (top of hour), ~1 km CONUS
- **Position queries fully supported** (the "map-tile-only" limitation in the
  spec does not exist). For "single most-recent value per point", use the
  `-latest` collection:

```
GET /edr/collections/mrms-qpe-latest/position?coords=POINT(-95.5 37.2)&parameter-name=QPE_24H,QPE_72H
```

Response (CoverageJSON): `ranges.QPE_24H.values[0]` / `ranges.QPE_72H.values[0]`
in mm. `-9` to `0` range values can occur over radar-sparse terrain (sentinel
handling); treat negatives as 0.

## General notes

- All collections list themselves at `GET /edr/collections`; per-collection
  metadata (temporal extent, vertical levels, parameter names) at
  `GET /edr/collections/{id}`.
- Output default is CoverageJSON; `f=GeoJSON` also supported.
- Vertical extents are discoverable: `extent.vertical.values` on the
  collection document gives the exact valid `z` values.
- Datetime rules are the same as the other gridded collections: instants or
  closed ranges; valid instants are in `extent.temporal.values`.
- Also newly available (same work): NBM-CONUS extended to **264h (11 days)**
  and GFS-Wave to **384h** — relevant if you want blended-guidance or marine
  fallbacks later.
