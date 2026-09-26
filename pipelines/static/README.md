# WS1 — Static Terrain/Soil Stack

Preprocessing: elevation → terrain derivatives (TWI, slope, aspect), soil
texture → hydraulic properties (θs, θwilt), assembled into a single Zarr
stack, for the physics core (`services/trail-physics`) to sample per trail
vertex.

## Status (Session 11): Boulder-area PILOT built and verified, real data throughout

The full chain — fetch → TWI → soil params → Zarr assembly — was run for
real, against real live public data, for a pilot region (not yet the full
Colorado build; see "Scope" below for why and what changing that requires).
**Every number in this README is from an actual run this session, not a
plan.**

```
Region: Boulder-area foothills (Golden, Boulder, Lyons, Nederland)
Grid: EPSG:5070 (NAD83 Conus Albers), 10m, 4542 x 3762 = 17,087,004 cells
Elevation: 1506.3m - 3729.3m (real, matches this region's known relief)
TWI (pyDEM, apply_twi_limits=True): mean 8.77, std 1.76
Slope: mean 0.220 (tan), max 4.562 (near-vertical rock faces)
theta_s: 0.404 - 0.476 (porosity, texture-derived)
theta_wilt: 0.028 - 0.138 (wilting point, texture-derived)
Coarse HRRR-cell lambda_bar: 186 distinct HRRR cells covered
Wall-clock: DEM ~7s, TWI+slope+aspect ~2min, soil ~17s, assembly ~seconds
Output size: ~470MB of intermediates + Zarr (not committed -- see below)
```

## Why a pilot, not statewide, first

Per the user's own decision: prove the full chain (fetch → TWI → Zarr →
main.py → EDR) on a smaller, real area before committing to a statewide
build. At 10m, statewide Colorado is ~3 BILLION cells (this README's
earlier capacity estimate of "304M cells" was wrong by roughly an order
of magnitude — corrected here now that a real run exists to calibrate
against). The Boulder-area pilot (~17M cells) is Colorado's highest-
traffic Front Range trail corridor and a genuinely varied testbed (steep
granite/gneiss foothills, mixed aspect, 1500-3700m relief) — not a toy
example chosen for convenience.

**Expanding coverage later requires no code changes** — only widening
`grid_spec.PILOT_BBOX_WGS84` (and, for a genuinely statewide build,
re-verifying pyDEM's scaling holds or adding real tiling with overlap
margins — see "TWI scaling" below for why that wasn't needed at pilot
scale but likely would be at 100x the cell count).

## Pipeline (run in this order)

```bash
pip install -r requirements.txt  # on top of services/trail-physics/requirements.txt

python3 build_dem.py --output ./data/static/pilot_dem.tif
python3 derive_terrain_pydem.py --dem-path ./data/static/pilot_dem.tif --output-dir ./data/static
python3 derive_coarse_twi.py --twi-path ./data/static/pilot_twi.tif --output ./data/static/pilot_twi_bar.npz
python3 fetch_polaris.py --output-dir ./data/static
python3 derive_soil_params.py --sand-path ./data/static/pilot_sand_pct.tif --clay-path ./data/static/pilot_clay_pct.tif --output-dir ./data/static
python3 assemble_static_stack.py --data-dir ./data/static --output ./data/static/colorado-10m-pilot.zarr

# Requires real SSH access to the production NUC (not available in this
# sandboxed environment -- see upload_static_stack.sh's own header):
./upload_static_stack.sh ./data/static/colorado-10m-pilot.zarr colorado-10m/pilot
```

1. **`grid_spec.py`** — the pinned grid definition (EPSG:5070, 10m, the
   pilot bbox). Previously undefined anywhere in the repo (`build_corridor_
   mask.py`'s own docstring: "that grid's exact origin/extent doesn't
   exist yet") — this module is that definition, and every other script
   here reads it rather than re-deriving its own.

2. **`build_dem.py`** — real USGS 3DEP 1/3 arc-second elevation, read via
   `/vsicurl/` HTTP range requests (confirmed live: rasterio's bundled
   GDAL can window-read directly from
   `https://prd-tnm.s3.amazonaws.com/.../USGS_13_n40w106.tif` without
   downloading the full ~413MB tile) — mosaicked across the pilot's two
   covering tiles (`n40w106`, `n41w106`, split at the real 40N seam) and
   reprojected onto the pinned grid. `fetch_3dep.py`'s own whole-tile
   download remains the right tool for an eventual full-state build
   (where nearly the whole tile is needed anyway regardless); this
   windowed approach is specifically for the pilot's much smaller
   footprint relative to a full tile.

3. **`derive_terrain_pydem.py`** — TWI via pyDEM with `apply_twi_limits=
   True`, **the exact configuration validated three times over**
   (Tarrawarra TDR/NMM, Shale Hills TDR — see `validation/*/README.md`).
   **Supersedes `derive_terrain.py`'s original WhiteboxTools plan for TWI
   specifically** — using a different TWI implementation in production
   than in validation would silently deploy unvalidated methodology (see
   that file's own updated header note; its sky-view-factor/Winstral-Sx
   steps remain relevant, unaffected, and still deferred behind Rung 3).
   Slope/aspect via this project's own `physics.terrain` (Horn's method),
   the same functions used at every validation site.

   **TWI scaling, verified empirically before running the full pilot, not
   assumed**: timed on progressively larger real crops of the actual
   pilot DEM — 40K cells (0.4s) → 1M cells (2.1s, ~480K cells/sec) → 9M
   cells (40.7s, ~220K cells/sec, mildly superlinear) — before running the
   full ~17M-cell grid in ONE call, no tiling needed at this scale: 1m56s
   wall-clock. A statewide build (~150-300x this pilot's cell count) would
   very likely need real tiling with overlap margins; that machinery
   doesn't exist yet and wasn't needed here.

   **Two real findings from this run, not assumed from the small
   catchments**: (1) the source DEM has ~17% nodata cells at the pilot's
   edges — reprojecting geographic 3DEP tiles onto a rotated Albers
   rectangle leaves real corner gaps outside the actual tile coverage, an
   expected reprojection artifact, not a bug; TWI adds only 685 additional
   invalid cells beyond the DEM's own nodata (out of ~14.2M valid DEM
   cells). (2) Slope/aspect's nodata footprint is a **superset** of the
   DEM's own (not identical to it) — Horn's method's 3x3 kernel produces
   NaN at any cell whose neighborhood touches a real nodata cell, so
   16,599 cells with valid elevation still have an undefined slope/aspect
   because they border a data gap. Both are checked by real regression
   tests, not just described here.

4. **`derive_coarse_twi.py`** — lambda_bar, the real Creare/GeoWATCH
   production equation's actual coarse term (Session 8's discovery,
   `physics/redistribution.py`'s module docstring: the PODPAC notebook
   reprojects TWI onto the SAME coarse grid as the coarse soil-moisture
   input). Every validation site was smaller than one HRRR cell, so
   "domain mean" and "this cell's own mean" were the same number there by
   construction — at Colorado scale this distinction becomes real for the
   first time. Output: a small lookup table (one entry per HRRR cell the
   stack actually overlaps — 186 for this pilot — not a full 1799x1059
   HRRR-grid-shaped raster, which would be almost entirely empty for a
   single-region stack).

5. **`fetch_polaris.py`** — soil texture (sand%/clay%, 0-30cm thickness-
   weighted mean across POLARIS's 0-5/5-15/15-30cm bins). **Why POLARIS,
   not gSSURGO**: gSSURGO's distribution moved to a JS-rendered Box folder
   with no stable download URL (`fetch_ssurgo.py`'s own docstring,
   confirmed dead again this session). POLARIS (Chaney et al. 2019) is a
   public, direct-HTTP, 30m probabilistic SSURGO-derived product —
   confirmed live this session, read via the same `/vsicurl/` windowed
   pattern as the DEM. Only sand%/clay% are fetched — POLARIS's own
   theta_s/theta_r are deliberately NOT used directly (theta_r is not the
   same quantity as wilting point; bypassing the texture-triangle/Noah-
   lookup step would be a different, unvalidated soil-parameter
   methodology).

6. **`derive_soil_params.py`** — theta_s/theta_wilt via the **same**
   USDA-texture-triangle → Noah SOILPARM.TBL pipeline
   (`physics.soil_texture`) used at every validation site — not a
   different soil-parameter methodology that happens to also produce
   plausible output. Uses a precomputed (sand%, clay%) → (theta_s,
   theta_wilt) lookup table (5,151 valid integer-percent pairs, built once
   in 11ms) applied via vectorized numpy indexing across the full ~14M
   valid cells, rather than a 17-million-iteration Python loop calling the
   classifier directly. A real physical sanity check (theta_s > theta_wilt
   everywhere) is run and must pass before the script exits successfully
   — not just asserted in a docstring.

7. **`assemble_static_stack.py`** — all six layers + the coarse lambda_bar
   lookup into one Zarr v3 group, with the grid spec AND full data
   provenance (sources, processing choices, the frozen `k=13`) written
   into the group's own attrs — so a future consumer reads the grid
   definition from the data itself, never re-deriving or assuming it.

8. **`upload_static_stack.sh`** — rsync the local Zarr to the NUC, then
   `mc mirror` it into MinIO under the reaper-safe `static/` prefix
   (verified: outside CleanupTask's catalog-driven scope, SyncTask's
   listing, and the ILM expiry rules — see
   `docs/trail-conditions-design.md`), following the same "run `mc` in a
   throwaway container on the compose network" pattern already
   established in `scripts/setup_minio_lifecycle.sh`. **Written, not
   executed this session** — this sandboxed environment has no SSH access
   to the production NUC (confirmed this session, consistent with every
   prior session's own deployment story); this script is meant to be run
   from a context that does have that access.

## Data not committed to git — reproducible from live sources instead

Unlike Tarrawarra/Shale Hills' primary source data (committed to git
because it was hard to reacquire — a WAF-blocked manual download, or a
migrated/dead original host), everything under `data/` here (~470MB: 8
intermediate GeoTIFFs + the assembled Zarr) is fully and quickly
reproducible from stable, live, public sources by re-running the numbered
pipeline above (~3 minutes total, dominated by the TWI step). Not worth
the git history weight for a regenerable artifact — see `.gitignore`. The
actual deployed deliverable belongs in MinIO (via `upload_static_stack.sh`),
the same place every other grid already lives, not duplicated into git.

## What's deferred, and why (unchanged from the original plan, restated for clarity)

- **NLCD tree canopy**: `fetch_nlcd.py`'s `TCC_SOURCE_URL` remains
  unresolved (unchanged this session) — not needed for v1 (soil moisture +
  frozen flag only; snow/canopy physics is out of scope until Rung 3).
- **Winstral Sx / sky-view factor**: `derive_terrain.py`'s WhiteboxTools
  steps for these remain relevant and un-superseded (only its TWI step was
  superseded) — still gated on WhiteboxTools being installed and verified,
  still deferred behind Rung 3 (wind-driven snow redistribution isn't v1
  scope per the design doc).
- **Corridor-mask rasterization** (`pipelines/corridor/build_corridor_
  mask.py`): still blocked on this exact grid spec existing — it does
  now, but wiring the rasterization step remains future work; vertex
  sampling (the current `main.py` approach) is the documented v1 path,
  with corridor-mask zonal stats as a stated upgrade path, not a v1
  requirement.
- **Statewide build**: needs (a) real tiled TWI processing with overlap
  margins (untested at this scale, unlike the pilot's single-call
  approach) and (b) a decision on whether 3 billion cells' worth of
  compute happens as one long-running batch job or a sharded/parallel
  approach — neither designed yet, deliberately, since proving the pilot
  chain end-to-end (through main.py and EDR) is the more valuable next
  step before scaling up the data volume.

## Next steps (not this session's scope)

Per the plan: Phase B (wire `main.py` to sample this real stack via
`redistribute_podpac`, fix the `valid_time`/`forecast_hour` upsert bug,
first live smoke test on the NUC) and Phase C (EDR exposure) follow in
separate sessions, gated on this pilot stack existing — which it now
does.
