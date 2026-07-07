# HRRR Smoke, Aerosol & Solar Radiation — Frontend Guide

New HRRR-Smoke and radiation parameters, available via both EDR (point/area
queries) and WMS (map tiles). Live at `https://folkweather.com`.

> **Units convention:** EDR returns **native SI units** (same as TSOIL →
> Kelvin). Smoke mass densities are therefore very small numbers — scale
> client-side for display. WMS map tiles already render in display units
> (µg/m³, mg/m²) via server-side styling.

## Parameter summary

| Param | What | EDR collection | Native unit | Display scale | WMS layer |
|---|---|---|---|---|---|
| `MASSDEN` | Near-surface smoke (PM) | `hrrr-height-agl` (z=8) | kg/m³ | ×1e9 → µg/m³ | `hrrr_MASSDEN` |
| `COLMD` | Column-integrated smoke | `hrrr-atmosphere` | kg/m² | ×1e6 → mg/m² | `hrrr_COLMD` |
| `AOTK` | Aerosol optical depth | `hrrr-atmosphere` | dimensionless | none | `hrrr_AOTK` |
| `DSWRF` | Downward shortwave (solar) | `hrrr-surface` | W/m² | none | `hrrr_DSWRF` |

**On "UV":** HRRR does not output a UV Index. `DSWRF` (downward shortwave
radiation flux, W/m²) is the physical driver of surface solar/UV exposure
and is the intended basis for any UV-style derivation — the frontend decides
how to use it (e.g. clear-sky ratio, a UV proxy formula, or raw solar).

---

## EDR examples (live responses)

### Near-surface smoke — `MASSDEN` (requires `z=8`)

```
GET /edr/collections/hrrr-height-agl/position?coords=POINT(-100 40)&parameter-name=MASSDEN&z=8
```

```jsonc
{
  "type": "Coverage",
  "domain": { "domainType": "Point",
    "axes": { "x": {"values":[-100.0]}, "y": {"values":[40.0]}, "z": {"values":[8.0]} } },
  "parameters": { "MASSDEN": { "unit": { "symbol": "kg/m^3" } } },
  "ranges": { "MASSDEN": { "type": "NdArray", "dataType": "float",
    "values": [2.5250908e-09] } }   // ×1e9 = 2.53 µg/m³
}
```

`z=8` selects the 8 m above-ground level (the only MASSDEN level). To convert
to the familiar AQI unit: `ug_m3 = value * 1e9`.

### Column smoke + aerosol optical depth — `COLMD`, `AOTK`

```
GET /edr/collections/hrrr-atmosphere/position?coords=POINT(-100 40)&parameter-name=AOTK,COLMD
```

```jsonc
{
  "ranges": {
    "AOTK":  { "values": [0.0] },          // dimensionless; 0 = clear
    "COLMD": { "values": [2.4627472e-05] } // kg/m² ; ×1e6 = 24.6 mg/m²
  }
}
```

### Solar radiation — `DSWRF`

```
GET /edr/collections/hrrr-surface/position?coords=POINT(-100 40)&parameter-name=DSWRF
```

```jsonc
{
  "parameters": { "DSWRF": { "unit": { "symbol": "W/m^2" } } },
  "ranges": { "DSWRF": { "values": [446.98154] } }  // W/m² (clear-sky noon ≈ 1000)
}
```

Time series and area/radius queries work identically to other HRRR surface
params (closed `datetime` ranges; valid instants in the collection's
`extent.temporal.values`). HRRR horizon is 18h (48h on 00/06/12/18z cycles),
hourly cycles, CONUS 3 km.

---

## WMS map tiles

Standard OGC WMS 1.3.0 GetMap. Tiles render in **display units** with
purpose-built colormaps (no client scaling needed for the imagery).

```
GET /wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap
    &LAYERS=hrrr_MASSDEN&STYLES=default&CRS=EPSG:3857
    &BBOX=<minx,miny,maxx,maxy>&WIDTH=256&HEIGHT=256&FORMAT=image/png
```

| Layer | Colormap | Range | Notes |
|---|---|---|---|
| `hrrr_MASSDEN` | AQI-aligned (green→yellow→red→purple→maroon) | 0–500 µg/m³ | breakpoints at 12/35/55/150/250 (AQI PM categories) |
| `hrrr_COLMD` | yellow→red | 0–600 mg/m² | thick-plume emphasis |
| `hrrr_AOTK` | blue→yellow→red | 0–3 | clear→hazy→smoky |
| `hrrr_DSWRF` | radiation ramp | 0–1000 W/m² | shared solar-radiation style |

Also available via WMTS (`/wmts/rest/{layer}/default/WebMercatorQuad/{z}/{x}/{y}.png`)
and the XYZ shortcut (`/tiles/{layer}/default/{z}/{x}/{y}`).

Legends: `GetLegendGraphic` is not implemented; the colormap breakpoints
above (and in `config/styles/{smoke,smoke_column,aod,radiation}.json`) are the
source of truth for building a client-side legend.

---

## Quick reference

```bash
# Smoke concentration at a point (µg/m³ = value × 1e9)
curl "https://folkweather.com/edr/collections/hrrr-height-agl/position?coords=POINT(<lon> <lat>)&parameter-name=MASSDEN&z=8"

# Aerosol optical depth + column smoke
curl "https://folkweather.com/edr/collections/hrrr-atmosphere/position?coords=POINT(<lon> <lat>)&parameter-name=AOTK,COLMD"

# Surface solar radiation (UV/solar basis)
curl "https://folkweather.com/edr/collections/hrrr-surface/position?coords=POINT(<lon> <lat>)&parameter-name=DSWRF"

# Smoke map tile
curl "https://folkweather.com/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=hrrr_MASSDEN&STYLES=default&CRS=EPSG:3857&BBOX=-11271098,4300621,-10958012,4613707&WIDTH=256&HEIGHT=256&FORMAT=image/png"
```

(URL-encode the space in `POINT(lon lat)` as `%20` if your HTTP client
doesn't do it automatically.)

---

_Note: a related fix shipped alongside these — HRRR 10 m wind
(`UGRD`/`VGRD` at `z=10` in `hrrr-height-agl`, and WMS `hrrr_UGRD`/`hrrr_VGRD`)
now labels and queries correctly. Previously it was mislabeled as 2 m._
