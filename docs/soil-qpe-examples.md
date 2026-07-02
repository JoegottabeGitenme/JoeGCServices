# Soil & QPE Collections — Live Request/Response Examples

Real requests and (trimmed) real responses captured from
`https://folkweather.com/edr` on 2026-07-02. Companion to
[soil-qpe-frontend-handoff.md](./soil-qpe-frontend-handoff.md).

---

## 1. Discover a collection (extents, valid z values, valid datetimes)

Always start here — the vertical extent gives the exact valid `z` values and
the temporal extent gives the valid `datetime` instants.

```
GET /edr/collections/hrrr-soil
```

```jsonc
{
  "id": "hrrr-soil",
  "title": "HRRR - Soil",
  "extent": {
    "vertical": {
      "interval": [[0.0, 100.0]],
      "values": [0.0, 4.0, 10.0, 30.0, 100.0]   // <- valid z (cm below ground)
    },
    "temporal": {
      "interval": [["2026-07-01T16:00:00Z", "2026-07-04T05:00:00Z"]],
      "values": ["2026-07-01T16:00:00Z", "..."]  // 58 valid instants
    }
  },
  "parameter_names": { "TSOIL": { /* ... */ } },
  "data_queries": ["position", "area", "cube", "trajectory", "corridor", "radius", "locations"]
}
```

`gfs-soil` is identical in shape with `vertical.values: [0, 10, 40, 100]`
and a much longer temporal extent (229 instants — 16-day horizon).

---

## 2. Most-recent QPE at a point (subgrade saturation)

The `-latest` collection returns a single value per parameter — the
frontend's "single most-recent value per point" requirement.

```
GET /edr/collections/mrms-qpe-latest/position?coords=POINT(-81.5 28.5)&parameter-name=QPE_24H,QPE_72H
```

Response (CoverageJSON, captured live — Orlando during active rain):

```jsonc
{
  "type": "Coverage",
  "domain": {
    "type": "Domain",
    "domainType": "Point",
    "axes": { "x": { "values": [-81.5] }, "y": { "values": [28.5] } }
  },
  "parameters": {
    "QPE_24H": { "type": "Parameter", "unit": { "symbol": "mm" } },
    "QPE_72H": { "type": "Parameter", "unit": { "symbol": "mm" } }
  },
  "ranges": {
    "QPE_24H": { "type": "NdArray", "dataType": "float", "values": [3.484117] },
    "QPE_72H": { "type": "NdArray", "dataType": "float", "values": [52.69932] }
  }
}
```

Read the numbers from `ranges.<PARAM>.values[0]` (mm). Treat small negative
values as 0 (sentinel handling over radar-sparse terrain).

⚠️ Param names are `QPE_24H` / `QPE_72H` (underscores). `QPE24H` → 400 with
an `Available: [...]` hint in the error body.

---

## 3. HRRR soil temperature time series (frozen-subgrade veto)

Closed datetime range + `z` selects depth. Same contract as `hrrr-surface`.

```
GET /edr/collections/hrrr-soil/position?coords=POINT(-95.5 37.2)&parameter-name=TSOIL&z=10&datetime=2026-07-02T00:00:00Z/2026-07-02T18:00:00Z
```

Response (trimmed):

```jsonc
{
  "type": "Coverage",
  "domain": {
    "type": "Domain",
    "domainType": "PointSeries",
    "axes": {
      "x": { "values": [-95.5] },
      "y": { "values": [37.2] },
      "t": { "values": [
        "2026-07-02T00:00:00Z",
        "2026-07-02T01:00:00Z",
        "2026-07-02T02:00:00Z"
        // ... hourly steps across the range
      ]}
    }
  },
  "parameters": { "TSOIL": { "unit": { "symbol": "K" } } },
  "ranges": {
    "TSOIL": {
      "type": "NdArray",
      "dataType": "float",
      "axisNames": ["t"],
      "values": [302.52267, 302.52267, 302.52267, 302.03503, 301.53455 /* ... */]
    }
  }
}
```

Values are **Kelvin** — frozen subgrade check is simply `value < 273.15`.

Depth selection (`z`, cm below ground): `0` (soil skin), `4`, `10`, `30`,
`100`. For a "top 10 cm" reading use `z=4` or `z=10` (HRRR outputs point
depths, not layers).

---

## 4. GFS soil — beyond HRRR's 18h horizon

Same query shape; `z` addresses Noah **layers** by layer top:
`z=0` → 0–0.1 m, `z=10` → 0.1–0.4 m, `z=40` → 0.4–1 m, `z=100` → 1–2 m.

Latest single value:

```
GET /edr/collections/gfs-soil-latest/position?coords=POINT(-95.5 37.2)&parameter-name=TSOIL&z=0
```

```jsonc
{
  "type": "Coverage",
  "domain": { "domainType": "Point", "axes": { "x": { "values": [-95.5] }, "y": { "values": [37.2] } } },
  "parameters": { "TSOIL": { "unit": { "symbol": "K" } } },
  "ranges": { "TSOIL": { "values": [299.09064] } }
}
```

Multi-day series for the 24–48h cure window (and far beyond — GFS now runs
to **384h**, hourly to 120h then 3-hourly):

```
GET /edr/collections/gfs-soil/position?coords=POINT(-95.5 37.2)&parameter-name=TSOIL&z=0&datetime=2026-07-02T00:00:00Z/2026-07-05T00:00:00Z
```

---

## 5. Error shapes worth handling

Wrong parameter name (400):

```json
{
  "type": "http://www.opengis.net/def/exceptions/ogcapi-edr-1/1.0/invalid-parameter-value",
  "title": "Bad Request",
  "status": 400,
  "detail": "Parameter 'QPE72H' not available in collection. Available: [\"QPE_01H\", \"QPE_24H\", \"QPE_72H\"]"
}
```

Invalid z (400) — the detail lists the valid values:

```json
{
  "status": 400,
  "detail": "Z coordinate 850 is outside the collection's vertical extent. Must be one of: [0.0, 4.0, 10.0, 30.0, 100.0]"
}
```

Ocean/no-land points return `null` in `ranges.TSOIL.values` (soil params are
land-only) — handle `null` as "no data", not zero.

---

## Quick-reference cheat sheet

```bash
# Is the subgrade frozen right now? (HRRR, 10cm depth)
curl "https://folkweather.com/edr/collections/hrrr-soil/position?coords=POINT(<lon> <lat>)&parameter-name=TSOIL&z=10"

# Will it stay unfrozen through the 48h cure window? (GFS top layer)
curl "https://folkweather.com/edr/collections/gfs-soil/position?coords=POINT(<lon> <lat>)&parameter-name=TSOIL&z=0&datetime=<now>/<now+48h>"

# How saturated is the subgrade? (rolling accumulations, mm)
curl "https://folkweather.com/edr/collections/mrms-qpe-latest/position?coords=POINT(<lon> <lat>)&parameter-name=QPE_24H,QPE_72H"
```

(Remember to URL-encode the space in `POINT(lon lat)` as `%20` — some HTTP
clients do not do this automatically.)
