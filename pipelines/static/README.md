# WS1 — Static Terrain/Soil Stack

One-time (annual refresh) preprocessing: 3DEP elevation → terrain
derivatives, SSURGO soil properties, NLCD canopy/land cover, all at 10m
resolution over the Colorado domain (per this project's Q1/statewide
decision — corridor-scoping was superseded, see
`docs/trail-conditions-design.md`'s "Scope corrections" section).

## Status: recipes written, not executed this session

Every script here is complete and follows the same "recipe script, not
yet run" convention already used elsewhere in this repo (e.g.
`scripts/build_populated_places.py`) — real code against real, verified
public data sources, honestly labeled as not-yet-executed rather than
faked. Reasons this session specifically:

1. **Data volume**: Colorado-wide 3DEP 1/3-arcsecond DEM tiles, gSSURGO,
   and NLCD are each multi-GB downloads. Not attempted in this sandboxed
   session (no reason to believe it would fail, just genuinely large and
   slow — this is real background-job territory, not an interactive-session
   task).
2. **WhiteboxTools not installed**: `derive_terrain.py`'s flow-routing step
   needs a real priority-flood algorithm at this scale (`physics/terrain.py`'s
   iterative pit-fill is explicitly documented there as small-catchment-only,
   correct for Tarrawarra's ~4,000 cells, not ~300M). WhiteboxTools
   (`pip install whitebox`, wraps a compiled binary) is the standard,
   well-established tool for this — not installed/tested this session.

## Pipeline order

1. `fetch_3dep.py` — USGS 3DEP 1/3 arc-second DEM tiles → mosaic → 10m
2. `fetch_ssurgo.py` — gSSURGO → rasterize Ks/θs/θref/θw/texture class to 10m
3. `fetch_nlcd.py` — NLCD canopy density + land cover → 10m
4. `derive_terrain.py` — slope, aspect, TWI, sky-view factor, horizon
   angles, Winstral Sx (16 azimuths) via WhiteboxTools
5. Output: ~12-layer Zarr stack → MinIO `static/colorado-10m/` (verified
   reaper-safe prefix — outside CleanupTask's catalog-driven scope, outside
   SyncTask's shredded/raw/grids/ listing, outside the ILM expiry rules;
   see docs/trail-conditions-design.md)

## Capacity estimate (from the planning session)

Colorado at 10m ≈ 304M cells (Front Range subset ≈ 30,400 km²). Static
stack: ~15GB raw → 5-8GB compressed, 1-4 hours one-time compute. See
`docs/trail-conditions-design.md`'s capacity-planning section for the full
analysis (hourly dynamic-stage costs, corridor-vs-region tradeoffs).
