# Trail Conditions — Design & Session 1 Implementation Notes

**Status:** design doc (original) + verified amendments from the first
implementation session (2026-09). The original design doc content is
preserved below unedited; this header section records what was verified
against live sources and what shipped, so future sessions don't re-derive
facts that are already settled.

---

## Session 1 summary (what shipped)

Scope: fetch the raw ingredients and expose geometry — **no physics, no
classifier**. See `docs/trail-conditions-frontend.md` for the API handoff.

- **HRRR ingredients** added to the existing `wrfprs` pull (zero new files,
  just more `.idx` byte-ranges): `SOILW` (volumetric soil moisture) at 0/1/4/10/30
  cm, `WEASD`, `SNOD`, `PRATE`, `CRAIN`/`CSNOW`/`CFRZR`/`CICEP`, `DLWRF`.
  Exposed via EDR: `hrrr-soil` extended, new `hrrr-snow` collection.
- **Trail geometry**: new `linear_features` PostGIS table (one row per OSM
  way), generic `feature_class` discriminator (`mtb_trail`/`hiking_trail`/
  `track`/`bridleway`), served as the EDR `trails` feature collection
  (`/radius`, `/area`, `/items?bbox=`, `/items?q=`).
- **Trailheads**: OSM `highway=trailhead` nodes upserted into the existing
  shared `locations` table (`location_type=trailhead`, id `TH<osm_node_id>`)
  — discoverable via the existing generic `/edr/locations` and
  forecast-proxied via the existing generic `/edr/locations/{id}?collections=`,
  both with zero code changes (see "Known gaps" below for name-search).
- **Sync**: `crates/trail-sync`, weekly OSM/Overpass poll
  (`config/trail-sync.yaml`, region list — no code change to add a region),
  soft-deletes (`active=false`) ways missing from the latest pass rather than
  hard-deleting, plus `POST /api/admin/trails/refresh` for an on-demand run.
- **Phase 0 groundwork**: `trail_reports` table (raw label archive, not
  EDR-exposed) + `scripts/scrape_trail_reports.py` (CSV import working now;
  Trailforks mode is a gated skeleton pending ToS verification).
- **Fixed in passing**: GLDAS CF NetCDF files were being tagged model
  `unknown-cf` instead of `gldas-noah` (a second, separate filename-sniffing
  function in `cf_netcdf.rs` didn't know the GLDAS prefix, even though the
  routing-decision function elsewhere did). Now correctly attributed —
  relevant here since GLDAS/NLDAS are the anchor data source for the S1
  bias-correction stage.
- **Closed a real gap found during review**: production Postgres had no
  backup at all besides one manual dump from months earlier. Added
  `scripts/backup_postgres.sh`, nightly cron, local retention. Scoped to
  local-disk protection only — off-NUC copy is a tracked follow-up, not
  implemented.

### Verified against live sources (amends open questions in the original doc)

| Open question | Answer | Where verified |
|---|---|---|
| Does HRRR export usable Ep (PEVPR)? | **No.** Absent from wrfsfc, wrfprs, and wrfnat `.idx` inventories. Penman-Monteith from 2m T/Q, 10m wind, DSWRF/DLWRF, surface pressure (all present) is required for S1/S5's ET terms. | Live `hrrr.tXXz.wrfprsfYY.grib2.idx` grep |
| Does HRRR export greenness fraction (σf)? | **Yes, but only in `wrfsfc`** (`VEG`, `VEGMIN`, `VEGMAX`), not in `wrfprs` (the file this pipeline pulls). A second file-pattern pull would be needed if σf is wanted from HRRR directly, or fall back to VIIRS/MODIS GVF as the doc's Plan B already suggested. | Live `hrrr.tXXz.wrfsfcfYY.grib2.idx` grep |
| Soil ice fraction (for S4, "the highest-value stage")? | **Not present in any public HRRR file** (wrfsfc, wrfprs, or wrfnat). **This weakens S4 as originally scoped.** Practical v1 proxy: `TSOIL <= 273.15 K` at the shallow depths as a frozen/thawed indicator — coarser than an ice fraction but captures the freeze-thaw signal the whole product depends on. Revisit if RAP/HRRR ever exposes SOILL. | Live `.idx` grep across wrfsfc/wrfprs/wrfnat |
| Exact GRIB2 discipline/category/number for the new snow/moisture params | Verified via `eccodes` against live message bytes (not from memory/tables — table lookups from memory are exactly the kind of unverified citation risk flagged in the original doc's references section). `SOILW`=2/0/192 (NCEP local, not the standard "Soil Moisture Content" num=3), `WEASD`=0/1/13, `SNOD`=0/1/11, `PRATE`=0/1/7, `CRAIN`/`CSNOW`/`CFRZR`/`CICEP`=0/1/33..36, `DLWRF`=0/5/3 (category 5, not 4 like `DSWRF`). | `crates/... config/models/hrrr.yaml` comments cite the verification method inline |
| NLDAS/GLDAS anchor data availability | **Already flowing**, not a gap. `nldas-noah`, `nldas-forcing`, and `gldas-noah` model configs were already enabled and ingesting before this session (confirmed live in `config/models/`). The only defect was the model-attribution bug above, now fixed. | `config/models/*.yaml`, live ingester logs |
| Trailforks API ToS for a derived commercial product | **Still unverified** — not resolved this session, deliberately (out of scope; requires a real account/contact). `scrape_trail_reports.py`'s Trailforks mode is gated behind `TRAILFORKS_TOKEN` and is a skeleton, not a working integration, specifically so it can't be accidentally run against the live API before that's settled. | N/A — explicitly deferred |
| A competitor has launched since the doc's ~May 2026 knowledge cutoff | **Not checked this session.** Still an open risk per the original doc's own caveat. | N/A — deferred |

### Scope corrections vs. the original doc

- **Domain is Colorado-wide, not corridor-scoped.** The original doc's §6
  ("you do not need statewide coverage... buffered corridors around ~200
  trail systems") was written before the physics phase's actual intended
  scale was confirmed. The decision made this session: the soil-model
  calculations will run over Colorado statewide, not corridor buffers. This
  changes the calculus for §4.1's static-layer acquisition (3DEP/SSURGO/NLCD)
  when that phase starts — statewide 10m rasters are a meaningfully larger
  one-time fetch/compute than ~200 corridors, though still bounded and
  one-time. Not re-derived in detail this session since that phase hasn't
  started (see "Deferred" below).
- **Trail geometry granularity: one feature per OSM way**, not merged per
  trail system. This was an explicit decision (not in the original doc),
  made because the product goal is showing *part* of a trail as rideable and
  another part not — merging ways would lose exactly that discrimination.
  OSM way ids can change when mappers split/redraw ways; the sync's
  soft-delete (`active=false`) plus re-appearance-on-next-sync handles churn
  without ever hard-deleting geometry a client might be referencing mid-session.
- **Segments carry no conditions yet, by design.** Per this session's
  decision, condition data is computed **app-side**: the app queries
  `hrrr-soil`/`hrrr-snow` along a trail's own geometry (from `/edr/collections/trails/items`)
  and colors segments itself. Server-side per-segment precompute (the
  original doc's S8) is deliberately deferred until the 10m downscaling
  exists — at native HRRR 3km resolution, adjacent OSM ways mostly land in
  the same grid cell, so precomputing per-segment values today would add
  pipeline complexity without adding information over what the app can
  already get by querying the grid directly.
- **No server-side condition class or heuristic in v1.** The EDR collection
  serves geometry + OSM tags only, explicitly labeled as carrying no
  rideability judgment. This was a deliberate call to avoid exactly the
  false-`good` credibility risk the original doc's own risk table (§11)
  flags — an unvalidated heuristic is worse than no heuristic, and the app
  can render raw ingredients meaningfully without one.

### Known gaps / fast-follow items (not blocking, explicitly scoped out)

- **Trailhead name search** (`?q=` finding a trailhead by name) is not wired.
  Trailheads are discoverable via the unfiltered `GET /edr/locations` listing
  and directly by id (`GET /edr/locations/TH<id>`), but not via a search box
  the way populated places and ZIPs are (`/edr/collections/populated/locations?q=`).
  The trails *linestring* collection's own `?q=` (searching way names) is
  fully wired — this gap is specifically about trailhead *points*.
- **`system` (trail-system grouping) is frequently null.** OSM models trail
  systems as route *relations*, not way-level tags; resolving relation
  membership needs a second Overpass pass this session didn't implement.
  `network` (a way-level tag, when present) is used as a weak proxy.
- **Static terrain/soil layers (3DEP/SSURGO/NLCD) are entirely deferred** —
  nothing this session consumes them. The `static/` MinIO prefix convention
  is reserved (verified reaper-safe: outside `CleanupTask`'s catalog-driven
  scope, outside `SyncTask`'s `shredded/`/`raw/`/`grids/` listing, outside
  the ILM expiry rules) for when that phase starts.
- **Postgres backup is local-disk only.** Protects against container/compose
  nukes and bad migrations, not full hardware/disk loss. An off-NUC copy
  (rclone to cloud storage or a workstation) is a tracked follow-up.
- **Competitor landscape (§2 below) not re-verified** against a live search.

### Validation posture (unchanged from the original doc, restated for emphasis)

No physics has been built, so none of the four validation rungs apply yet.
**Phase 0 (report-separability) is next**, once enough `trail_reports` rows
accumulate via `scripts/scrape_trail_reports.py` — either CSV imports from a
partner org, or the Trailforks integration once its API/ToS are resolved.
Until Phase 0 and the Tarrawarra physics-port reproduction (Rung 1) both
pass, no condition/rideability claim should ship, and the collection
description says exactly that.

---

## Session 2 summary (2026-09) — the physics core itself

Scope: build the actual downscaling physics (Eq. 1–7), stand up the
Tarrawarra validation harness for real, and start the WS1–WS8 workstream
plan from the capacity-planning session. **Explicit decision this session:
proceed ahead of Phase 0** (labels aren't accumulating yet; Trailforks
ToS remains unresolved) — a deliberate, acknowledged departure from the
doc's own "Phase 0 first" sequencing, made because the ingest/geometry
substrate and the physics core have value independent of when Phase 0's
report-separability signal arrives.

### What shipped

- **`services/trail-physics/physics/`** — the actual Eq. 1–7 downscaling
  core in Python, 49 unit tests: `redistribution.py` (Eq. 1, fully
  specified in this doc, high confidence), `flux.py` (Eq. 4/5 — implements
  the **correct** Ek et al. 2003 Noah soil-evaporation form, replacing the
  transcription error this doc already flagged in the source paper),
  `radiation.py` (Eq. 6, standard solar-geometry terrain correction),
  `pet.py` (FAO-56 Penman-Monteith, since HRRR has no PEVPR — confirmed
  absent, see Session 1's amendment table), `relaxation.py` (Eq. 2/7 —
  **explicitly unverified reconstruction**, see below), `terrain.py` (D8
  flow accumulation + TWI, small-catchment scale only).
- **Three real bugs found and fixed by the test suite itself**, not by
  inspection: a soil-evaporation-vs-wind test assumption that was
  physically wrong (FAO-56's wind term can legitimately decrease ET under
  humid conditions — a real property, not a bug, once traced through by
  hand); a genuine sign error in `compute_aspect` (north/south swapped,
  caught because the test's own DEM setup was backwards and forced a
  re-derivation); a backwards invariant in a pit-fill test (asserted the
  opposite of what correct pit-filling does). All three are documented
  in-place as regression tests, not silently fixed.
- **`hrrr_grid.py`** — a direct transcription of
  `crates/projection/src/lambert.rs`'s HRRR Lambert Conformal Conic
  projection (not a fresh pyproj-based implementation — the Rust code's
  origin convention is idiosyncratic and a from-scratch reimplementation
  couldn't be verified to align pixel-for-pixel with already-written Zarr
  grids). **Cross-validated against the actual Rust code**: a throwaway
  `cargo run --example` against the real `LambertConformal::hrrr()`
  produced exact reference (i,j) values for three points, asserted
  verbatim in the Python test suite — bit-for-bit agreement confirmed, not
  just internal round-trip consistency.
- **`forcing.py`** — Zarr v3 reader + NaN-aware bilinear sampling. Verified
  against the actual Rust writer's conventions (array shape `[height,
  width]`, `row_origin: south` meaning no row-flip needed for HRRR —
  confirmed by reading `crates/grid-processor`'s source directly, not
  assumed) and tested against a **real local Zarr v3 array** built to the
  same shape/dtype convention. **Not tested against live production
  MinIO** — the S3 API port isn't publicly exposed (only the admin console
  is proxied through the gateway), so there was no reachable endpoint from
  this session's environment.
- **`physics/snow.py`** — snow-lite per the prior session's Q3 resolution
  (canopy interception + enhanced temperature-radiation melt), explicitly
  *without* Winstral Sx wind redistribution. The 16-azimuth Sx layers are
  still planned for WS1 as a diagnostic; actual wind-redistribution logic
  is gated behind Rung 3 residual analysis (validate snow-lite's
  disappearance dates first, check whether errors correlate with wind
  exposure, build the more complex piece only if the data says to).
- **`db.py` / `aggregate.py` / `main.py`** — the orchestration wiring
  (poll `datasets` for new complete HRRR runs, mirroring `ChunkWarmer`'s
  proven pattern; sample forcing at each trail's own OSM vertices — a
  vertex-sampling approximation of true S8 zonal statistics, since WS1/WS2
  don't exist yet but the same `segment_conditions` schema serves either
  approach without a migration). **Not run against live infrastructure**
  (no reachable Postgres from this session's environment either) — every
  piece it calls is independently unit-tested; this file is the untested
  wiring, and a first live run is the intended smoke test.
- **`validation/tarrawarra/`** — real parsers for every documented
  Tarrawarra file format, and a real validation runner reproducing the
  doc's Rung 1 methodology exactly (redistribute each date's own
  catchment-mean to its measurement points via Eq. 1, compare RMSE against
  the site-mean baseline). **Proved end-to-end against a synthetic dataset**
  built to the exact documented format with a known Eq. 1 relationship
  baked in — both the PASS and FAIL verdict paths were exercised and
  behave correctly. **Blocked on the real data**: the dataset is
  unrestricted and hosted at a stable URL
  (`people.eng.unimelb.edu.au/aww/tarrawarra/`), and its documentation
  pages fetched successfully early in the session — then an
  Incapsula/Imperva bot-challenge started blocking every subsequent
  request, including to paths that had just worked. See
  `validation/tarrawarra/README.md` for exact manual-download instructions
  (this is a browser-vs-bot problem, not a data-availability problem).
- **`crates/storage`: `segment_conditions` schema** — the per-segment,
  per-hour output table (Rust migration, follows the `linear_features`
  pattern exactly: no FK to `linear_features.feature_id` since geometry
  churns weekly and a physics run should never be blocked by a missing FK
  target; not wired into `crates/retention`, kept indefinitely as the
  training-label archive per this doc's own Section 6). Read-side
  `SegmentConditionsCatalog` added for the future EDR `?conditions=latest`
  exposure (not built yet).
- **`pipelines/static/`** — static-stack recipe scripts, two of them
  **fully live-verified this session** (not just written): `fetch_3dep.py`
  (USGS 3DEP tile enumeration, confirmed against the real `prd-tnm` public
  bucket) and `fetch_nlcd.py` (NLCD land cover via MRLC's WCS endpoint —
  debugging a real `InvalidAxisLabel` error live led to a working
  `subsettingCrs` parameter, confirmed by an actual GetCoverage request
  returning a real GeoTIFF). `fetch_ssurgo.py`'s download step could
  **not** be verified: gSSURGO's distribution has moved to an
  interactive-only Box folder since this doc was written, no stable
  programmatic URL exists anymore (documented, not worked around).
  `derive_terrain.py` (WhiteboxTools-based, for Front-Range/statewide
  scale — explicitly not this session's small-catchment `physics/terrain.py`)
  is written but not executed (tool not installed, DEM not fetched).
- **`pipelines/corridor/build_corridor_mask.py`** — the vector-buffering
  primitive (buffer active `linear_features` ways by a configurable
  distance into a `trail_corridor` table), real PostGIS SQL, not yet
  including the raster-onto-WS1's-grid step (that grid doesn't have a
  defined origin/extent yet).
- **`web/science.html`** — the public-facing science justification page,
  linked from the splash page's trails section. Every reference link was
  checked to actually resolve this session (not cited from memory
  unverified) — including catching a real citation risk: an initial DOI
  candidate for Coleman & Niemann (2013) turned out to be
  `10.1002/wrcr.20065`, a *withdrawn* duplicate entry; the correct,
  published paper is `10.1002/wrcr.20159`. Three references (FAO-56,
  Winstral & Marks 2002, Walter et al. 2002) are cited from memory without
  independent re-verification this session and are labeled as such on the
  page itself, not presented with unverified confidence.
- **`docker-compose.yml`**: `trail-physics` service added, **profile-gated**
  (`profiles: ["trail-physics"]`, not started by default `docker compose
  up`) — explicitly because Rung 1 hasn't passed and WS1/WS2 don't exist,
  so running this continuously against production data would be premature
  per the doc's own "if you don't land near this, stop and debug" rule.

### What this session did NOT do

- **Did not pass Rung 1** — blocked on data acquisition (see above), not on
  the physics or the harness, both of which are built and proven against
  synthetic data.
- **Did not build WS1** (the actual Colorado static terrain/soil stack) —
  the fetch recipes exist and two are live-verified, but no DEM was
  mosaicked, no terrain derivatives were computed at scale, and no Zarr
  stack was written to `static/`.
- **Did not build WS2's raster corridor mask** — only the vector-buffering
  step, which doesn't depend on WS1's grid existing.
- **Did not run `main.py` against live Postgres/MinIO** — neither was
  reachable from this session's environment. Every module it calls is
  independently tested; the orchestration wiring itself is not.
- **Did not resolve the Winstral Sx / wind-redistribution question** —
  deliberately deferred behind Rung 3 evidence, per the prior session's Q3
  resolution.

---

## Session 3 summary (2026-09, continued) — the primary source, finally

The user supplied `geowatch.pdf` (Eylander et al. 2023) directly, resolving
Session 2's biggest gap: two of the seven governing equations had been
built without ever seeing the paper that specifies them. This session did
a full equation-by-equation audit and fixed what needed fixing.

### Equation-by-equation audit result

| Eq. | What the paper says | Session 2's version | Verdict |
|---|---|---|---|
| 1 (static disaggregation) | `θ* = θws − (1/k)(λ̄−λ) − (1/k)(ln Ks̄ − ln Ks)`, k=13 | Identical | ✅ exact match, no change |
| 3 (total flux) | `F(θ) = −Et(θ) − Edir(θ)` | Sign convention differed trivially | ✅ fine, `f_theta()` added matching Eq. 3's sign |
| 4 (vegetation transpiration) | Chen et al. (1996) form | Identical | ✅ exact match, no change |
| 5 (direct soil evaporation) | `[Rd+(1−Rd)ι]·Ep·(1−σf)·(θ−θref)/(θs−θref)`, Rd=0.15, **unclipped ratio, verbatim** | Ek et al. (2003) form substituted, radiative prefactor missing entirely | ⚠️ two real gaps — see below |
| 6 (solar view factor ι) | daily integral of sun-surface alignment over the sun's azimuth sweep, sunrise to sunset | Not implemented (only instantaneous incidence existed) | ❌ missing, now built |
| 2 (flux-difference correction) | `θ = θ* + Δt·(F(θ*) − F'(θws))`, F' uses weather-scale-**averaged** soil properties | Exponential anomaly decay (physically similar direction, algebraically unrelated) | ❌ replaced entirely |
| 7 (relaxation timestep) | `Δt = δts·(θws−θref)/F(θs)`, `δts = Cts·{1 if θ*<θs, else e^−(θ*−θs)/θs}`, Cts=0.1, clipped [0,30 days] | Session 2 misassigned Rd (Eq. 5's diffuse-light fraction) as a timescale coefficient and invented a different form | ❌ replaced entirely |

**On Eq. 5's discrepancy**: the design doc (written without the paper)
predicted the printed ratio must be a transcription error and should
really be the well-established Ek-2003 form. Having read the paper: it
really does print `(θ−θref)/(θs−θref)`, verbatim, unclipped — this is not
a transcription error between the paper and the doc, it's genuinely what
the paper's text says (whether that's itself an error in the paper is
unresolvable from the text alone). Decision made with the user: implement
**both** forms (`form="ek2003"` default, `form="geowatch"` paper-literal)
in `flux.py`, `relaxation.py`, and let the Tarrawarra harness empirically
determine which one reproduces the published RMSE, once real data is
available.

**Discovered property, not a bug**: `compute_delta_t`'s Eq. 7 evaluates
`F(theta_s)` — the flux AT saturation. Both Eq. 5 forms give
beta/ratio = 1.0 exactly at `theta=theta_s` whenever `theta_s > theta_ref`
(the normal case) — `ek2003` clips there, `geowatch`'s ratio is trivially 1
by construction. So `Δt` itself is form-independent; the two forms only
diverge in Eq. 2's `F(theta*)`/`F'(theta_ws)` terms, evaluated away from
saturation. Documented and asserted via a dedicated test
(`test_compute_delta_t_is_form_independent_when_theta_s_exceeds_theta_ref`)
so a future change that breaks this invariant is caught.

**One genuine open question the paper doesn't resolve**: for Eq. 2's units
to work out (`theta` dimensionless, `delta_t` in days), `F` must already be
a volumetric-fraction-per-day rate, not FAO-56's depth-per-time (mm/day) —
the paper never states the depth-normalization this requires. Flagged in
`relaxation.py`'s module docstring as an open calibration question, on the
same footing as k=13 — something only empirical reproduction against
Tarrawarra can pin down.

**A second, larger methodology question, also newly discovered**: Section
2.2 describes the model as "a two-stage approach" (Eq. 1 static
disaggregation + Eq. 2/7 flux correction) used together "to generate the
higher resolution products," and Section 4.2.1's Tarrawarra validation
text doesn't state whether the published **0.0321 m3/m3** target used
stage 1 alone or the full two-stage pipeline. `run_validation.py`'s Rung 1
gate currently validates stage 1 (Eq. 1) only — the fully-specified part
the design doc scoped Rung 1 around — and now says so explicitly in its
own docstring, rather than silently assuming the published number is
Eq.-1-only. If Eq. 1 alone doesn't land on 0.0321 once real data is
available, that is not necessarily proof Eq. 1 is wrong.

### New validation targets discovered (not previously known)

The paper's Section 4.2.1/4.2.2 documents two additional published
comparisons beyond the 13-date TDR result the design doc already knew
about:

- **NMM (neutron moisture meter)**: same Tarrawarra site, 20 locations per
  date (vs. ~508 for TDR — worse spatial coverage) but **59 dates** (vs.
  13) — denser temporal check. Published target: RMSE 0.040 → 0.030
  m3/m3, improving on 56/59 dates (95%).
- **Shale Hills catchment** (a second, independent, publicly-available
  site): 74 dates, published target RMSE 0.060 → 0.054, 55/74 dates
  improved (74%). Not yet pursued this session (Tarrawarra alone already
  hit the WAF blocker); noted here as a future third validation target.

### What shipped this session

- `physics/flux.py` rewritten: `radiative_factor()`, `geowatch_soil_moisture_ratio()`
  (new, unclipped-by-design), `direct_soil_evaporation()`/`total_actual_et()`
  updated to dispatch on `form=`, `f_theta()` new (Eq. 3). 24 tests.
- `physics/radiation.py` extended: `solar_view_factor()` (Eq. 6), numerically
  integrating the sun's daily azimuth sweep, normalized so flat ground gives
  ι=1.0 by construction (an interpretive normalization choice — the paper
  doesn't state one explicitly). 7 tests, manually sanity-checked against
  physical intuition (winter north-facing slopes stay heavily shadowed,
  ι≈0.11 vs. south-facing ≈1.84).
- `physics/relaxation.py` **completely rewritten**: true Eq. 2/7 (piecewise
  `delta_ts`, `SoilProperties` dataclass kept as two separate required
  fine-vs-coarse arguments specifically so that distinction can't be
  silently dropped, per the parameter-matching discipline
  Section 5.1 already flagged). 15 tests, including the two discovered
  properties above (veg-split invariance under `ek2003`; delta_t
  form-independence at saturation).
- `physics/__init__.py`'s confidence-level docstring updated to reflect
  transcribed-not-reconstructed status for all modules.
- `validation/tarrawarra/parsers.py`: **DEM header format corrected against
  a real downloaded file** (see below) — was an unconfirmed ESRI-grid
  guess, now a confirmed `north:`/`south:`/`east:`/`west:`/`rows:`/`cols:`
  format with derived cellsize (cross-checked two ways, both give 5.0m).
  New `parse_nmm_file()` (Eq.-7-motivated denser validation target, format
  transcribed from `Readme.nmm`, tested against a synthetic fixture only —
  not yet against a real file).
- `run_validation.py`: `--with-flux-correction` added as an honest,
  documented stub (exits with a clear explanation of exactly what
  additional Tarrawarra inputs and modeling choices it needs), rather than
  a fabricated implementation of a pedotransfer function the paper doesn't
  specify.

### A real-data acquisition attempt, partially successful

Retrying the Tarrawarra fetch this session (via this environment's
WebFetch tool rather than direct `curl`, which is immediately WAF-blocked)
found a brief window where the WAF let two real files through:
`sundry/ksat.dat` (fetched completely, 42 rows, now committed at
`validation/tarrawarra/data/ksat.dat` and tested against directly) and
`topodata/tarrautm.dem`'s 7-line header (kept and used to fix the parser;
the 8000-value elevation grid body was fetched too but failed a post-fetch
integrity check — expected 8000 values, found 7878 with malformed rows —
and was discarded rather than committed, since hand-reproducing thousands
of numbers through a chat-mediated fetch tool isn't reliable enough to
trust as scientific ground truth, and a silently-wrong DEM would be worse
than an honestly-missing one). See `validation/tarrawarra/README.md` for
the full story. The 13 TDR files and the DEM body are still needed via
manual browser download to actually run Rung 1.

### What this session did NOT do

- **Still did not pass Rung 1** — still blocked on the TDR files and DEM
  body specifically (not on the physics, which is now transcribed directly
  from the primary source rather than reconstructed).
- **Did not implement `--with-flux-correction`** — deliberately left as a
  documented stub rather than fabricating a pedotransfer function and Ep
  source the paper doesn't specify for Tarrawarra.
- **Did not wire the NMM or Shale Hills targets into `run_validation.py`**
  — parser exists for NMM (untested against real data), no harness logic
  yet for either.
- **Did not resolve whether the published 0.0321 target includes stage 2**
  — flagged as a real, currently-unresolvable-from-the-paper-text
  ambiguity rather than silently picking an assumption.

---

## Session 4 summary (2026-09, continued) — real data, real Rung 1 run, real FAIL

The user obtained the full Tarrawarra dataset via the site's own zip
archives (browser download, bypassing the WAF that blocked automated
fetches all of Session 3) and supplied it. This is the first session Rung
1 was actually run against real data, not synthetic fixtures.

### Five real bugs found and fixed before Rung 1 would even run cleanly

1. Session 3's committed `ksat.dat` had a stray `</content>` tag leaked in
   from a tool-output copy-paste — harmless to the parser, embarrassing,
   fixed by replacing with the zip archive's clean copy.
2. **A coordinate-system bug, and it was Session 3's own fault**: the
   README told future sessions to use `tarrautm.dem` (UTM) "NOT
   tarrawar.dem" — backwards. `Readme.tdr`'s own text says TDR coordinates
   are in "the Tarrawarra coordinate system," not UTM; `ksat.dat`'s header
   says the same. Using the UTM DEM against local-coordinate TDR/ksat data
   produced a *silent* total failure — every point interpolated to NaN,
   caught only by an existing defensive warning, not a crash. Switched to
   `tarrawar.dem`; the wrong file is no longer committed at all, to
   prevent a third session repeating this.
3. A genuine `Ksat = 0.0 mm/hr` measurement in the real data made
   `ln(Ks)` = `-inf`, which poisoned the **domain-wide** mean (not just
   that one point) and corrupted every single prediction. Fixed by
   excluding non-positive conductivity from both the mean and the
   interpolation pool, loudly logged (1 of 42 excluded). A related
   duplication bug (the domain mean was independently recomputed
   elsewhere, unfiltered) was fixed by making one function the single
   source of truth.
4. **A units mismatch**: TDR data is %V/V, the paper's targets are
   fractional m3/m3. Converting only at the final RMSE-reporting step
   would have been wrong — Eq. 1's `k=13` is an additive correction on
   whatever scale theta is expressed in, so the conversion has to happen
   before `redistribute()` is called, not after. Fixing this alone brought
   the baseline (site-mean) RMSE to 0.0370 — strikingly close to the
   paper's own 0.0352, confirming data/coordinates/units were now all
   correct.
5. **Parser bugs in `parse_nmm_file`, `parse_particle_file`,
   `parse_layer_file`**, all only surfaced by testing against real files
   for the first time: NMM dates are `DD-Mon-YY`, not the documented
   `dd/mm/yyyy`; particle.dat gives clay explicitly (a 9th column), not as
   a residual, and the depth-range disambiguation logic mishandled it;
   layer.dat is TAB-delimited with multi-word texture values that a plain
   whitespace `.split()` silently misaligned, plus open-ended depths
   (`">73"`) that can't be a float. All fixed; see
   `validation/tarrawarra/README.md` for full detail and
   `tests/test_parsers.py` for real-file regression tests.

### The actual Rung 1 result: FAIL, with a diagnosed (not guessed) cause

With all five bugs fixed:

```
Overall baseline (site-mean) RMSE: 0.0370  (doc target: 0.0352)
Overall Eq. 1 redistribution RMSE: 0.1127  (doc target: 0.0321)
Dates improved: 0/13  (doc target: >= 9)
```

Eq. 1's correction makes every single date *worse*, not a near-miss.
Diagnosis: per-date correlation between TWI anomaly and observed moisture
anomaly is real and mostly strong (0.07 to 0.62), and the two weakest
dates are the driest of the 13 — matching the paper's *own* stated
behavior ("topography does not strongly influence... extremely dry" or
"extremely wet" conditions) almost exactly. The structural signal is
right. A least-squares fit implies an effective `k≈74`, not the paper's
`k=13` — a ~5-6x magnitude mismatch, not a sign or structural error.

The most likely explanation: the paper computed TWI using **pyDEM**
(Ueckermann et al. 2018, a purpose-built Python package cited by name),
not a from-scratch D8 implementation like `physics/terrain.py`'s. `k=13`
was calibrated against pyDEM's specific numerical output, which was never
checked against ours. **Deliberately not done**: fitting a local `k≈74` to
make the gate pass — that would defeat Rung 1's actual purpose. `k=13`
stays as documented; the gate stays FAIL, honestly reported.

**Concrete next step**: install pyDEM directly and compute TWI through it
instead of `physics/terrain.py`'s own implementation, to get numerically
compatible results. Not "try a different constant" — literally use the
tool the paper's own authors used.

### What this session did NOT do

- **Did not pass Rung 1** — a specific, diagnosed root cause (TWI
  numerical incompatibility with pyDEM) remains unresolved, honestly
  reported rather than papered over with a fitted constant.
- **Did not install or try pyDEM** — identified as the concrete next step,
  not attempted this session.
- **Did not wire NMM into `run_validation.py`** — parser now validated
  against all 20 real files, but harness logic doesn't exist yet, and is
  moot until the Eq. 1/TWI issue above is resolved (per the design doc: no
  building on top of an unresolved Rung 1 failure).
- **Did not implement `--with-flux-correction`** — same reasoning, doubly
  moot now that stage 1 alone doesn't pass.

---

## Session 5 summary (2026-09, continued) — pyDEM tested directly: hypothesis ruled out (mostly)

Session 4 ended with a specific, testable hypothesis: our `compute_twi()`
(from-scratch D8) is numerically incompatible with pyDEM (Ueckermann et
al. 2018), which the paper's Section 2.2 states it used, and that
incompatibility explains the ~5-6x magnitude gap between our best-fit `k`
and the paper's stated `k=13`. This session installed pyDEM and tested
that hypothesis directly rather than leaving it as a plausible-sounding
guess.

### What was built

- `physics/terrain.py::compute_twi_pydem()` — computes TWI via pyDEM's
  `DEMProcessor` (in-memory numpy array input, no GeoTIFF I/O needed),
  with a `scaled` flag for pyDEM's own stored `x10` TWI value vs. its
  plain unscaled return value.
- `run_validation.py --twi-engine {builtin,pydem}` and `--twi-scaled` —
  lets the harness use either TWI implementation without duplicating the
  rest of the pipeline.
- `pydem` added to `services/trail-physics/requirements.txt` as a
  commented-out **optional** dependency (heavier than everything else in
  that file — rasterio + a Cython build step — and not needed by the live
  pipeline, only this cross-check).
- New tests in `tests/test_terrain.py` (skip-if-not-installed pattern):
  physical-property parity with the builtin D8 test, the exact x10 scaling
  relationship (confirmed directly against pyDEM's own source), and a
  clean-DEM no-NaN-propagation check.

### The actual result: the hypothesis was wrong, or at least insufficient

|                        | builtin (D8) | pyDEM (D-infinity) | pyDEM, x10-scaled |
|---|---|---|---|
| Overall Eq. 1 RMSE     | 0.1127       | 0.1089              | 0.9581             |
| Dates improved         | 0/13         | 0/13                | 0/13               |
| Implied best-fit k     | ~74          | ~64                 | (ruled out by construction) |

Switching from D8 to pyDEM's D-infinity flow routing produced only a
marginal improvement — both in RMSE and in per-date correlation with real
observed anomalies (a genuine, small win, e.g. `sm101196`: 0.515 → 0.588)
— but nowhere near enough to close a 5-6x magnitude gap. The x10-scaled
variant (in case a saved GeoWATCH raster, rather than the API return
value, is what `k=13` was calibrated against) is dramatically *worse*,
exactly as basic dimensional reasoning predicts (a 10x larger correction
term) — this specific sub-hypothesis is now decisively ruled out, not just
unlikely.

**Two things ruled out by direct algebra this session, not empirical
testing** (because the math makes empirical testing pointless): Ks's
measurement units cancel out of Eq. 1's `ln(Ks)` deviation term regardless
of unit choice (any unit conversion is a multiplicative constant on Ks,
hence additive on `ln(Ks)`, which washes out against the domain mean); the
same argument rules out "specific catchment area" normalization
convention (per-unit-contour-length vs. raw area) as an explanation.

**Two candidates that remain, neither yet tested**:
1. A resolution-scale mismatch — `k=13` might assume TWI computed on a
   coarser grid than the full 5m DEM (coarser grids reduce TWI's spread,
   which is the direction needed).
2. pyDEM's non-default `apply_twi_limits`/`uca_saturation_limit` capping
   options, which compress the upper tail of the TWI distribution and are
   off by default in what this session ran.

**What was deliberately NOT done, again**: fit a local `k` (64-74) to pass
the gate. `k=13` remains untouched in every tested configuration.

### What this session did NOT do

- **Did not pass Rung 1** — still FAIL, with the leading hypothesis from
  the prior session now tested and found insufficient by itself.
- **Did not test the resolution-scale or capping-options hypotheses** —
  identified as the next concrete steps, not attempted.
- **Did not revisit NMM or `--with-flux-correction`** — both remain moot
  until Rung 1's root cause is actually resolved.

---

## Session 6 summary (2026-09, continued) — three more hypotheses, real progress, still FAIL

Tested the two concrete next steps Session 5 identified, plus one more
found by re-reading the paper's Section 4.2.1 carefully: real, substantial
progress, closing most of the gap, but Rung 1 still does not pass.

### Three hypotheses tested

1. **Resolution mismatch** (`physics.terrain.coarsen_dem`, block-averages
   elevation before recomputing TWI; `--dem-resolution {10,15,30}`). The
   paper's Section 2.4 says GeoWATCH's *global* elevation composite is
   30m — but Section 4.2.1, re-read carefully this session, explicitly
   says Tarrawarra used the site's own 5m DEM "in lieu of its default
   global geospatial inputs." **Ruled out** by the paper's own text, and
   independently confirmed empirically: coarsening to 30m *collapses* the
   TWI/observed correlation (0.465 → 0.193) rather than improving it — the
   ~700m catchment is too small (13-23 cells across at 30m) to resolve
   real terrain structure at that resolution.
2. **Soil texture instead of measured conductivity** (`--ks-source
   texture`). Section 4.2.1 lists "soil texture data," not measured
   conductivity, as an input. Built `physics/soil_texture.py`: a
   zero-dependency USDA texture-triangle classifier (public-domain
   boundary data, transcribed and verified against reference corners with
   zero coverage gaps across the full triangle) feeding Noah's own
   `SOILPARM.TBL` (STAS table, fetched live from `wrf-model/WRF` — the
   same lookup the paper's own Ek-2003/Chen-1996 flux lineage is built
   on). RMSE improved (0.1127 → 0.0930) but diagnosis shows this is mostly
   a magnitude-shrinkage artifact: Tarrawarra's 11 sample sites are nearly
   texturally homogeneous (matching the paper's own description), so
   texture-derived ln(Ks) has ~8x less spread than the noisy measured
   field, and its correlation with real anomalies is indistinguishable
   from zero on every date. The TWI term's own implied-k barely moved
   (74.9 vs. 73.7) — confirming the persistent gap lives in the TWI term,
   not Ks.
3. **pyDEM's non-default capping** (`apply_twi_limits`/
   `uca_saturation_limit=32`, off by default in pyDEM itself;
   `--twi-apply-limits`). Shrinks TWI's standard deviation ~30% and
   reduces RMSE (0.1089 → 0.0909) on its own.

### Combined result: real structural progress

Combining all three (pyDEM + capping + texture-Ks, at native 5m
resolution — coarsening doesn't stack usefully with the others):

| | Session 4 (builtin) | Session 6 (best combo) |
|---|---|---|
| Overall Eq. 1 RMSE | 0.1127 | 0.0597 |
| Implied best-fit k (TWI term) | 73.7 | 43.9 |
| Correlation (TWI anomaly vs. observed) | 0.465 | 0.519 |

This is genuinely structural, not just magnitude convergence: correlation
quality *improved* (didn't just shrink toward the trivial baseline the way
the resolution and texture-Ks-alone experiments partly did), and the
implied-k gap narrowed from 5.7x to 3.4x. One date (`sm270995`, the
wettest, most topographically-driven) now lands within 1% of its own
baseline RMSE. Still FAIL overall (0.0597 vs. target 0.0321, 0/13 dates
improved) — meaningfully closer than any prior session, not a pass.

**What was deliberately NOT done, again**: fit a local `k` to pass the
gate. `k=13` remains untouched in every configuration tested across all
three sessions.

### What remains untested

- Eq. 5's form, Eq. 6's ι normalization, and Eq. 2/7's depth
  normalization — all require Stage 2, which per the design doc's own
  discipline shouldn't be built on a still-failing Stage 1.
- Whether the remaining ~3.4x implied-k gap is closeable by TWI-side fixes
  at all, or requires Stage 2's own damping effect on stage-1 anomalies —
  Session 6's progress makes "a correctly-calibrated but smaller Stage 1,
  followed by Stage 2's damping" a more coherent story than either stage
  alone reproducing 0.0321.
- A different Ks pedotransfer scheme than Noah's STAS table, and whether
  `k=13` was ever meant to be reproduced by Stage 1 in isolation at all
  (the two-stage-pipeline ambiguity documented since Session 3).

### What this session did NOT do

- **Did not pass Rung 1** — closer than ever, still FAIL.
- **Did not build Stage 2** — correctly deferred per the design doc's own
  "don't build on an unresolved Rung 1 failure" rule; Session 6's findings
  make Stage 2 more clearly relevant to the remaining gap than before, but
  building it was out of scope this session.
- **Did not revisit NMM** — still moot until Rung 1 resolves.

---

## Session 7 summary (2026-09, continued) — Stage 2 built for real: a genuine but modest effect

Built and wired the paper's full two-stage pipeline (`--with-flux-correction`
is no longer a stub) to test Session 6's own remaining hypothesis: that
0.0321 represents the full pipeline, not Stage 1 alone, since Eq. 2/7's
sign structure damps Stage 1's anomalies. Result: real, still FAIL, and
the gap's location is now better characterized.

### What was built

- **`services/trail-physics/physics/pet.py`**: daily-timestep FAO-56
  Penman-Monteith (Eq. 6-40 of Allen et al. 1998), each function
  cross-checked against the primary source's own fully worked numerical
  examples (fetched live from fao.org), not just formula transcription —
  10 golden-value tests, all passing exactly.
- **`validation/tarrawarra/parsers.py::parse_daily_met_file`**: 32
  tab-delimited columns, confirmed against the real `daily.met` file.
- **`services/trail-physics/tests/test_solar_view_factor.py`**: 4 new
  southern-hemisphere tests (Tarrawarra: 37.65°S) — a real correctness
  risk worth checking (every existing test used a northern latitude,
  and `solar_position` is pure trigonometry with no hemisphere branch to
  audit) rather than assume. All passed with zero code changes needed.
- **`validation/tarrawarra/stage2.py`** (new module, kept separate from
  `run_validation.py` for independent testability): Ep computation and
  TDR-survey-to-met-date matching, texture-derived soil parameters
  extended to the full theta_wilt/theta_ref/theta_s (not just Ks),
  per-point iota from the native DEM, and the full Eq. 2/7 orchestration.
- `run_validation.py` gained `--with-flux-correction` (now real),
  `--sigma-f`, `--active-layer-depth-mm`, `--eq5-form`.

### A real dimensional-analysis finding, then a bigger one testing it

For Eq. 7's `delta_t` to actually be in *days* (matching the paper's own
"clipped ... 0 to 30 days"), `F(theta_s)` must be a fraction-per-day rate,
not FAO-56's mm/day — resolved by working through the algebra (see
`relaxation.py`'s updated docstring), not assumed. The fix: divide Ep by
an assumed active-layer depth (a standard bucket-model conversion).

Testing the resulting depth sweep (150/300/1000mm) then produced a
surprise: **results were bit-for-bit identical across all three depths.**
Proven algebraically, not just observed: whenever the same Ep drives both
the fine and coarse flux terms (true at Tarrawarra — one weather station),
Eq. 7's `delta_t` is exactly inversely proportional to Ep while Eq. 2's
flux-difference term is exactly directly proportional to Ep — their
product (the actual applied correction) is analytically independent of
Ep's magnitude, and therefore of the depth normalization entirely. This
**fully closes** the depth-normalization question (flagged as open since
Session 3) for this configuration — not "we didn't find a good value" but
"the value provably doesn't matter here." `sigma_f` has a real but
similarly tiny effect for the same underlying reason (visible only at the
5th decimal place per point).

### Results

Stage 2 only activates when `theta_ws < theta_ref` (Eq. 7's `delta_t`
clips to exactly 0 otherwise) — confirmed against real data, **7 of the 13
TDR dates**, not a rare edge case. On those 7, Stage 2 correctly and
consistently pulled Stage 1's over-amplified predictions toward the
baseline (every active date improved, never worsened) — physically
correct signed behavior, modest magnitude. Eq. 5's `geowatch` form beat
`ek2003` by a small but real margin (0.0579 vs 0.0587).

| | Session 6 (Stage 1 only) | Session 7 (Stage 1 + 2) |
|---|---|---|
| Overall RMSE | 0.0597 | 0.0579 |

Combined with Session 4's starting point (0.1127), that's a 48.6%
cumulative reduction. One date (`sm270995`) now lands within 0.6% of its
own baseline — closest ever. Still FAIL (target 0.0321, 0/13 dates
improved).

**What was deliberately NOT done, again**: fit constants to pass. `k=13`
and `Cts=0.1` remain untouched.

### What remains open

- The bulk of the gap (already traced to the TWI term specifically,
  Sessions 4-6) is still unexplained — Stage 2 is a genuine contributor,
  not the missing piece.
- The paper's "modified TWI" sentence (Section 2.2.1) remains the single
  most suspicious unexplained detail across all 4 sessions.
- Whether 0.0321 is Stage-1-only or the full pipeline is still
  undetermined either way.
- Shale Hills (74 dates, public data) is the next planned step, per the
  user's decision to pursue it after Stage 2 rather than in parallel.

### What this session did NOT do

- **Did not pass Rung 1** — best result yet, still FAIL.
- **Did not pursue Shale Hills** — queued for next, per plan.
- **Did not resolve the "modified TWI" question** — still requires either
  the authors or pyDEM's own source/documentation, neither pursued this
  session per the user's explicit decision to skip author contact.

---

## Original design doc (unedited below)

# Trail Conditions — End-to-End Design

**Status:** design draft
**Domain:** Colorado Front Range (expandable)
**Service target:** OGC API - Environmental Data Retrieval, collection `trail-conditions`

---

## 1. Product definition

**The question:** *Are the trails dry enough to ride today, and if not, when?*

**The answer, per trail segment, hourly:**

> **Green Mountain** — Good. Frozen until ~10am, firm through the day. South-facing, decomposed granite.
> **Apex** — Marginal. Upper north-facing sections still holding snow. Lower loop fine.
> **Bear Creek** — Stay off. Clay, thawing from 3" of Tuesday snow. Thursday earliest.

**Why a physics model beats crowdsourced reports:** the crowd tells you what a trail was like when somebody last rode it. It cannot tell you that a trail is frozen at 8am and soup at 1pm, cannot forecast, and cannot distinguish two trailheads 15 miles apart that diverged after the same storm because one is south-facing decomposed granite and the other is north-facing clay under canopy. Those are the three things this product does.

**Differentiators, ranked:**

1. **Freeze–thaw window.** "Frozen until 10am, ride early." Structurally impossible for crowdsourcing to deliver — reports are stale by the time the trail changes state.
2. **Forecast.** 48-hour firm-up time, not a nowcast.
3. **Spatial discrimination.** Segment-level, driven by aspect, soil texture, and canopy rather than a single verdict per trail system.

**Primary user:** trail advocacy organizations and land managers (COMBA, Medicine Wheel, BMA, USFS/county open space) who currently automate closures on a rain-gauge threshold. Secondary: riders, via those orgs' channels.

---

## 2. Competitive landscape

| Who | What they do | Gap |
|---|---|---|
| Trailforks, MTB Project, Singletracks | Rider-submitted condition reports, some recent-precip overlay | No soil model; stale; no forecast |
| Trail org auto-status pages | Rain gauge + 24/48h threshold rule | Ignores aspect, texture, canopy, frozen state |
| Scandinavian forestry trafficability | LiDAR depth-to-water / TWI maps, operational, routes harvest machinery | Same algorithm family — pointed at logging equipment |
| Ag field workability (precision-ag platforms) | Trafficability forecasting for equipment | Same value prop — pointed at farms |
| Northern-state DOT frost/thaw models | Spring load restriction timing | Same freeze–thaw physics — pointed at trucks |

The physics is commercially validated in three adjacent markets. Nobody has pointed it at recreation. Read that as a real but not risk-free gap: recreational users won't pay much, incumbents own rider distribution, and there is liability discomfort in "open" verdicts. Hence the advocacy-org channel.

> **Verify before committing.** This landscape is from knowledge current to ~May 2026 and was not web-verified. A small app could have launched since. Re-check Trailforks feature releases and search for recent entrants.

---

## 3. Architecture overview

```
STATIC (one-time / annual)          DYNAMIC (hourly)              REFERENCE (3-hourly / daily)
  3DEP DEM 1m → 10m                   HRRR analysis + forecast       SMAP L4 / NLDAS-2
  SSURGO / STATSGO                    VIIRS GVF (if needed)          Sentinel-2 / Landsat FSC
  NLCD canopy + land cover                                           Trailforks / MTB Project reports
  Trail geometry (OSM / GPX)
        |                                    |                              |
        v                                    v                              v
  +----------------------------------------------------------------------------+
  |  S0 preprocess     S1 anchor    S2 snow    S3 melt    S4 gate    S5 downscale |
  |                    S6 strength  S7 classify  S8 aggregate-to-segment          |
  +----------------------------------------------------------------------------+
                                     |
                                     v
                       Zarr / COG store (grids)  +  Parquet (segment timeseries)
                                     |
                                     v
                       OGC EDR service — collection `trail-conditions`
```

---

## 4. Inputs

### 4.1 Static

| Layer | Source | Native res | Derived products |
|---|---|---|---|
| Elevation | USGS 3DEP | 1 m | slope, aspect, TWI, sky-view factor, horizon angles, Winstral Sx (16 azimuths) |
| Soil texture | SSURGO, STATSGO fallback | polygon | USDA class, USCS class, Ks, θs, θref, θw |
| Land cover / canopy | NLCD 2021+ | 30 m | cover class, canopy density fraction |
| Trail geometry | Trailforks GPX (preferred), OSM (fallback) | vector | segmented centerlines, trailhead points |

**Working resolution: 10 m.** Do not go to 1 m. Flow routing on 1 m LiDAR generates accumulation artifacts in low-relief terrain, and the STOPMODEL physics has no established validity below ~10 m. 10 m is still 300× finer than HRRR.

**SSURGO gaps:** coverage thins in national forest and wilderness. Where it falls back to STATSGO the soil correction term degrades toward uniform. Flag affected segments in output metadata.

### 4.2 Dynamic — HRRR (hourly)

| Field | Use |
|---|---|
| Soil moisture by level (RUC LSM, 9 levels) | θws input, depth-weighted to 0–5 / 0–10 cm |
| Soil ice fraction by level | infiltration gate, frozen-state classification |
| Soil temperature by level | frost depth, freeze–thaw timing |
| SWE, snow depth | sub-grid snow redistribution |
| Precip rate + type | rain vs. snow accumulation |
| 2 m T/Q, 10 m wind | melt energy, Sx storm weighting, PET |
| DSWRF / DLWRF | radiation melt, PET |
| Surface pressure | PET |

**Two fields to verify in the GRIB inventory before planning around them:**

- **Potential ET (Ep)** — appears in Eq. 4 and 5. HRRR likely does not export a usable PEVPR. Plan to compute Penman-Monteith from the fields above.
- **Greenness fraction (σf)** — HRRRv4 uses a near-real-time VIIRS GVF internally; whether it's in output grids is uncertain. Fallback: pull VIIRS/MODIS GVF directly.

**Use RUC's own soil parameter table**, not a generic USDA lookup — see §5.1.

### 4.3 Reference / validation

- **SMAP L4** (9 km, 3-hourly, 2015–) — primary bias anchor
- **NLDAS-2** (12 km, hourly, 1979–, gauge-corrected precip forcing) — alternate anchor, long record for CDF training
- **Sentinel-2 / Landsat harmonized fractional snow cover** (10–30 m) — snow disappearance date validation
- **SNOTEL, COAgMet, USCRN, SCAN** — in-situ soil moisture (note SNOTEL siting bias, §8)
- **Trailforks / MTB Project condition reports** — the labels for supervised calibration

---

## 5. Processing chain

Eight stages. Each is an independently callable function with a persisted intermediate. This is non-negotiable — validation failures require answering "was it the snow partition or the strength model," and that's only answerable if stages are separately inspectable.

### S1 — Anchor (coarse bias correction)

HRRR soil moisture is tuned to make the atmosphere behave, not to be a validated soil product. It is cycled through HRRRDAS with its own adjustments and drifts to a model-specific attractor.

**Do not use GFS as the reference.** GFS soil moisture is Noah inside a coupled NWP system — same structural situation, correlated biases, wrong in the same direction for the same reason. LSM soil moisture is only nominally volumetric: 0.25 m³/m³ in RUC and in Noah are different physical states. What is comparable across models is the **anomaly**, not the level.

Three options, increasing ambition:

**(a) Seasonal CDF matching.** Empirical CDFs of HRRR vs. reference over the overlap period, stratified by season and by soil-texture × land-cover class. Class-based beats per-pixel with a short record. Constraint: HRRRv4 landed Dec 2020 and changed the RUC soil configuration — training across the v3/v4 boundary violates stationarity, leaving ~5 years.

**(b) Triple collocation.** Three estimates with mutually independent errors (HRRR, SMAP L4, in-situ) solve for each one's error variance without knowing truth. Yields rescaling *and* per-pixel uncertainty. Watch the independence assumption — NLDAS-2 and HRRR share precipitation lineage.

**(c) Use HRRR's rate, not its level.** *Recommended.* HRRR's *state* drifts; its *tendency* (dθ/dt) is driven by precip, radiation, and ET over the last hour — physically constrained forcing at 3 km, which is what HRRR is good at. Anchor absolute level to SMAP L4 / NLDAS-2 at their cadence, integrate HRRR hourly increments forward, re-anchor with a relaxation timescale (not a hard reset, to avoid sawtooth). Generalizes cleanly to the forecast leg: beyond the last analysis you are integrating tendencies from your last good anchor, which is honest about what you know.

#### 5.1 The parameter-matching trap

Rescaling θws into a reference climatology means you **must also adopt that reference's soil parameters** (θs, θref, θw, Ks) throughout Eq. 2–7. Eq. 2 subtracts F′(θws), a flux evaluated at the coarse state using coarse-scale soil properties. Rescale moisture into NLDAS/Noah space while evaluating fluxes with RUC's field capacity and the correction term is computed against a different soil than the state it corrects — a systematic error that looks like a downscaling artifact and is miserable to diagnose.

**Keep moisture climatology and parameter set matched as a single unit. Assert it in code.**

### S2 — Snow partition

Redistribute HRRR grid-cell SWE to 10 m. **Conserves to cell total** — same discipline as Eq. 1, redistribute, never invent mass.

- **Wind:** Winstral Sx (max upwind slope over a search distance) for drift deposition and scour. Standard practice computes Sx for one climatological wind direction. Compute all 16 azimuths as static layers, then weight per storm by actual HRRR wind direction and speed during the accumulation event. Small but real methodological improvement, free given hourly forcing.
- **Canopy:** interception and sublimation from NLCD canopy density. Colorado subalpine losses commonly ~30–40% of seasonal snowfall. Creates sharp forest/clearing contrasts in ground SWE invisible to HRRR.

Front Range foothills: canopy and radiation dominate; wind redistribution matters less than in alpine terrain.

### S3 — Melt

Enhanced temperature-index melt using the sky-view / horizon layers with HRRR DSWRF, DLWRF, and 2 m T. Aspect drives melt-out date differences of weeks at the same elevation. Output: per-pixel melt water input rate.

### S4 — Infiltration gate

**The highest-value stage and the one the source paper names but never implements.** Read HRRR soil ice fraction. Melt water arriving on frozen soil does not infiltrate — it ponds or runs off, saturating the thin thawed layer above the frost. Route rain/melt to infiltration vs. ponding vs. runoff by ice fraction.

This produces a physically distinct **wet-surface-over-frozen-substrate** state invisible to both HRRR and the unmodified GeoWATCH algorithm, and it is exactly the state that wrecks trails.

> **Session 1 amendment:** soil ice fraction is confirmed absent from all public HRRR output (wrfsfc/wrfprs/wrfnat). This stage's implementation will need to substitute a `TSOIL <= 273.15K` frozen/thawed proxy at the shallow depths, which is coarser than a true ice fraction. See the amendment table above.

### S5 — Downscale

GeoWATCH core, from Eylander et al. (2023):

- **Eq. 1** — topographic redistribution: `θ* = θws − (1/k)(λ̄ − λ) − (1/k)(ln(Ks) − ln(Ks)‾)`, k = 13
- **Eq. 2–7** — flux correction for vegetation ET (Eq. 4) and direct soil evaporation (Eq. 5), timestep Δt from Eq. 7, Cts = 0.1, Rd = 0.15, Δt clipped [0, 30 days]
- **Eq. 6** — solar view factor
- S4 flux added as a source term in Eq. 2

**Two transcription errors in the published paper — reconcile before trusting your port:**

- **Eq. 5** reads `(θ − θref)/(θs − θref)`, which does not match the standard Noah / Ek et al. (2003) formulation and gives wrong sign behaviour below field capacity.
- **Eq. 7's** δts expression is mangled in the text.

Check both against Ek et al. (2003) and the Creare podpac notebook (§12).

**Known limitation:** TOPMODEL/STOPMODEL assumes saturation-excess runoff and a water table shallow enough that hydraulic gradient tracks surface slope — a humid temperate catchment. Colorado's high country is steep, thin-soiled, and fractured-bedrock-controlled; the plains are semi-arid and infiltration-excess-dominated. The source paper's own validation shows best results in sandy, low-vegetation sites and significant regressions in silty loam (Mt Vernon bias 0.01 → 0.15, NNSE 0.68 → 0.21). See Coleman & Niemann (EMT) and Ranney (EMT-VS) — both developed on Colorado catchments — for what worked in this exact terrain.

### S6 — Strength / softness index

Volumetric → gravimetric, then Eq. 8: `RCI = exp[c1 − c2·ln(MC)]`.

**Blocker:** c1/c2 are per-USCS-class coefficients from Army FASST / SMSP II. Access is restricted and the paper does not publish them. Eq. 8 gives the form, not the content.

**v1 resolution: skip absolute RCI.** Emit a *relative softness index* calibrated against observed trail conditions. This is not a compromise — a validated four-class index is more useful to a trail manager than an unvalidated PSI number, and it routes around the dependency entirely. Sourcing c1/c2 via ERDC contact, or fitting from cone penetrometer field data, is an upgrade path, not a prerequisite.

### S7 — Classify

Features → ordinal class. Given the Trailforks label supply (§8), this should be **supervised**: model the physics to produce features, let reported conditions train the mapping.

Features per pixel: softness index, soil ice fraction / frost depth, surface SWE, aspect, sky-view, canopy density, USCS class, slope, hours since last precip, 24h freeze–thaw cycle count.

Classes: `frozen-firm` / `good` / `marginal` / `soft-stay-off` / `snow-covered`.

Note `frozen-firm` and `good` are distinct states with the same verdict but different stability — one becomes `soft-stay-off` by afternoon. Do not collapse them.

### S8 — Segment aggregation

Zonal statistics along segmented trail centerlines with a buffer (~15 m). Emit per-segment class, plus the worst-class fraction so a trail can be **partially** open — which matches reality and is more useful than one verdict per system. Derive `firm_until` / `firm_from` transition times from the hourly forecast series.

> **Session 1 amendment:** the trail geometry this stage will aggregate onto already exists (`linear_features`, one row per OSM way — see the ingested collection notes above), so S8 has a real segment set to target once the physics stages produce grid output.

---

## 6. Data model & storage

| Artifact | Format | Notes |
|---|---|---|
| Static terrain/soil stack | Zarr or COG, 10 m | computed once, chunked by trail corridor |
| Hourly gridded intermediates | Zarr, chunked (time, y, x) | S1–S7 outputs, rolling retention (~30 days) |
| Segment timeseries | Parquet, partitioned by date | the EDR service reads primarily from here |
| Segment/trailhead registry | GeoJSON + Postgres | location IDs, geometry, metadata, soil-data-quality flag |

**Domain scoping:** you do not need statewide coverage. You need buffered corridors around ~200 trail systems. That is a small fraction of Colorado's cells — the statewide-precompute scaling problem largely disappears, and the whole domain runs in minutes per cycle.

> **Session 1 amendment: superseded.** The domain was confirmed Colorado-wide during the session that shipped the ingredients/geometry work (this document's header). Statewide 10m static-layer acquisition and precompute cost should be re-scoped against that when the physics phase starts, not against the ~200-corridor assumption above.

**Retention:** keep the full segment timeseries indefinitely (small, and it is your training set). Grids roll off.

---

## 7. OGC EDR service design

Collection: `trail-conditions`. This is a good fit for EDR specifically because `locations` gives you named trailheads and `trajectory` lets a rider query conditions along a planned GPX route — which no competitor can do.

> **Session 1 amendment:** the collection actually shipped is named `trails` (geometry/metadata only, see the ingested-collection notes above), not `trail-conditions` — the latter name is reserved for when a condition parameter actually exists, per the "no server-side condition class in v1" decision. Trailheads are served through the existing `locations`/populated-places-style registry, not through this collection's own `/locations` (which, per the codebase audit that informed this session, is not actually implemented for *any* feature collection today, storm events included, despite being documented in `tornado.yaml`'s header comment).

### 7.1 Collection metadata

`GET /edr/collections/trail-conditions`

```json
{
  "id": "trail-conditions",
  "title": "Colorado Front Range Trail Conditions",
  "description": "Hourly modeled trail surface condition and freeze-thaw state, analysis and 48-hour forecast",
  "extent": {
    "spatial": { "bbox": [[-106.2, 38.6, -104.6, 40.6]], "crs": "EPSG:4326" },
    "temporal": { "interval": [["2026-01-01T00:00Z", null]], "trs": "Gregorian" }
  },
  "data_queries": {
    "locations": { "link": { "href": "/edr/collections/trail-conditions/locations" } },
    "position":  { "link": { "href": "/edr/collections/trail-conditions/position" } },
    "area":      { "link": { "href": "/edr/collections/trail-conditions/area" } },
    "trajectory":{ "link": { "href": "/edr/collections/trail-conditions/trajectory" } },
    "items":     { "link": { "href": "/edr/collections/trail-conditions/items" } }
  },
  "parameter_names": { "...": "see 7.3" },
  "output_formats": ["CoverageJSON", "GeoJSON", "application/json"],
  "crs": ["EPSG:4326"]
}
```

### 7.2 Query patterns

| Query | Example | Serves |
|---|---|---|
| Locations list | `/edr/collections/trail-conditions/locations` | trailhead picker; the ranked "where should I ride today" list |
| Single location | `/edr/collections/trail-conditions/locations/apex?datetime=2026-02-14T00Z/2026-02-16T00Z` | trail detail page with forecast |
| Segment | `/edr/collections/trail-conditions/locations/apex.enchanted-forest` | partial-open detail |
| Position | `/edr/collections/trail-conditions/position?coords=POINT(-105.21 39.73)` | arbitrary point, off-registry |
| Area | `/edr/collections/trail-conditions/area?coords=POLYGON((...))` | land manager, whole jurisdiction |
| **Trajectory** | `/edr/collections/trail-conditions/trajectory?coords=LINESTRINGM(...)` | **conditions along a planned route — the differentiated query** |
| Items | `/edr/collections/trail-conditions/items` | registry enumeration, geometry + static metadata |

Standard EDR parameters apply throughout: `datetime`, `parameter-name`, `crs`, `f`.

**Location ID convention:** `{system}` for a trail system, `{system}.{segment}` for a segment. Slugified, stable, never recycled. Registry maps to Trailforks/OSM IDs for report joins.

> **Session 1 amendment:** the id convention actually shipped is simpler — `feature_id` is the raw OSM way id (an integer, not a slug), since ways are not currently grouped into named systems (see the `system` field gap noted above). Trailheads use `TH<osm_node_id>`, matching the existing `PP<GEOID>`/`ZIP<code>` convention for populated places/ZIPs.

### 7.3 Parameters

| Parameter | Unit | Type | Description |
|---|---|---|---|
| `condition_class` | — | categorical | `frozen-firm` / `good` / `marginal` / `soft-stay-off` / `snow-covered` |
| `softness_index` | 0–1 | continuous | relative; higher = softer. v1 substitute for RCI |
| `frozen_fraction` | 0–1 | continuous | soil ice fraction, top layer |
| `frost_depth` | m | continuous | depth to thaw front |
| `swe` | mm | continuous | sub-grid snow water equivalent |
| `soil_moisture` | m³/m³ | continuous | downscaled volumetric, 0–5 cm |
| `firm_until` | ISO 8601 | time | when the segment leaves `frozen-firm` today |
| `firm_from` | ISO 8601 | time | next time it returns to rideable |
| `confidence` | 0–1 | continuous | from triple collocation + soil-data-quality flag |

`condition_class` and the two transition times are the product. Everything else is exposed because it's cheap and because the follow-on products (§10) are alternate renderings of it.

> **Session 1 amendment:** none of the parameters in this table exist yet — this is entirely future work, gated on the validation rungs in §8. What exists today is raw HRRR/NLDAS ingredients (`hrrr-soil`'s `SOILW`/`TSOIL`, `hrrr-snow`'s `WEASD`/`SNOD`/`PRATE`/precip-type flags) at native 3km resolution and native SI units, plus `trails` geometry (`feature_class`, `name`, `tags`). See `docs/trail-conditions-frontend.md` for the actual current API surface.

### 7.4 Service notes

- **Pre-render everything.** Each HRRR cycle writes the full segment table. The EDR layer serves reads from Parquet/Postgres — no on-demand computation. This is the major simplification over GeoWATCH's architecture, which was on-demand only because it had to serve arbitrary global regions.
- **Cache-Control** keyed to the HRRR cycle. Publish cycle time and model run in response metadata.
- **Disclaimer in collection description and every location response.** This is an advisory model, not a closure authority. Land manager decisions are theirs. Non-negotiable given the liability posture.

---

## 8. Validation

Four rungs, in order, each gating the next.

**Rung 1 — Tarrawarra reproduction.** Public dataset (Western & Grayson 1998; Western et al. 2004). No snow, no HRRR, no Colorado. Proves the Eq. 1–7 port against a published answer. Target: RMSE ≈ 0.0321 m³/m³ against TDR, beating the site-mean's 0.0352, improving on 9 of 13 dates. **If you don't land near this, stop and debug.** Do this before anything else.

**Rung 2 — Scrape a winter of trail reports.** *This is the project go/no-go and it is cheap.* Pull Trailforks / MTB Project Front Range condition reports with timestamps, join to the feature stack per segment-day using historical HRRR, and test whether aspect / texture / canopy / frozen-state separate the reported classes at all.

Two weeks in a notebook. If reported conditions don't separate on your features, you find out before building anything. If they do, you have a trained classifier and a validation set in one pass.

Known biases: popular trails oversampled, reporting skewed toward good conditions and toward weekends, free-text quality varies. Weight accordingly; don't pretend it's a clean label set.

> **Session 1 amendment:** the label-collection mechanism for this rung now exists (`trail_reports` table + `scripts/scrape_trail_reports.py`), but no analysis has been run — this remains entirely future work, blocked on enough reports accumulating (CSV import from a partner org is the fastest path; the Trailforks API mode is a gated skeleton pending ToS verification, see the amendment table above).

**Rung 3 — Snow disappearance date.** Per-pixel against harmonized Landsat/Sentinel-2 fractional snow cover. Free, same resolution as output, good Colorado spring clear-sky frequency. **Do not try to validate SWE.** If melt-out date is right per pixel, melt flux timing is very likely right — a much cleaner and cheaper target than anything the source paper attempted.

Supplement with Airborne Snow Observatory LiDAR SWE where flown, for distribution.

**Rung 4 — Forward test.** Predict, publish, compare against incoming reports. Track class accuracy and, more importantly, the false-`good` rate — the error that damages trails and credibility.

**On SNOTEL:** sites are deliberately placed in sheltered clearings. That is a known sampling bias and makes them poor validation for exactly the wind and canopy effects S2 models. Use them for timing, not spatial skill.

---

## 9. Build phases

| Phase | Scope | Exit criterion |
|---|---|---|
| **0** | Scrape trail reports, join to historical HRRR + static features, test separability | Rung 2 passes — features separate reported classes |
| **1** | Static preprocessing, one trail system. 3DEP → slope/aspect/TWI/sky-view/Sx at 10 m | Static stack renders and looks physically sensible |
| **2** | S1 + S5 port, plain NumPy, single tile | Rung 1 passes |
| **3** | S2–S4 snow and frozen-ground modules | Rung 3 passes |
| **4** | S6–S8, supervised classifier, segment aggregation | Held-out class accuracy acceptable |
| **5** | Operationalize: hourly HRRR ingest, Zarr/Parquet write, EDR endpoints | Collection live, `locations/apex` returns |
| **6** | Forecast leg, firm-up times, partner pilot with one trail org | Rung 4 running; org using it |

**Phase 0 first, not Phase 1.** Pull the reports before writing physics. It is the cheapest possible test of the core hypothesis and it determines whether the rest is worth building.

**Start partner conversations during Phase 0.** They have long lead times and the org relationship is the distribution channel, the credibility, and the ground-truth source simultaneously.

> **Session 1 amendment:** what this session actually delivered doesn't map cleanly onto Phase 0/1 above — it's closer to "Phase 5's ingest half, pulled forward without the physics that would normally justify it," done because the immediate goal was a working EDR surface for a future UI to build against, not the physics validation gate. Phase 0 itself (the actual separability analysis) has not been run. Treat the phase table above as still fully applicable to the physics work; nothing in it has been skipped, only the ingest/geometry substrate has been built ahead of it.

---

## 10. Follow-on products

All are alternate renderings of intermediates already computed. None require new modeling.

- **Snow disappearance date map** (from S3) — fire agencies, May delivery, forecast-lead value for summer fuel dryness
- **Post-fire debris flow antecedent saturation** (from S5) — combine with MTBS/BAER severity; burn scars are steep, small, and unresolved by 3 km
- **Mud-season road trafficability** (from S6) — USFS district offices, county road departments, utility/pipeline access. The original GeoWATCH use case, pointed at terrain where it bites.
- **Wet slab / glide avalanche melt input rate** (from S3) — flag as a CAIC *collaboration*, not a product. Snowpack internal structure is the hard part and you won't have it.

---

## 11. Risks & open questions

| Risk | Severity | Mitigation |
|---|---|---|
| HRRR soil moisture bias propagates through | High | S1 anchoring; test magnitude of CDF correction early — if large, use NLDAS-2/SMAP L4 as backbone with HRRR for forecast increment only |
| Trail reports too noisy to train on | High | Phase 0 answers this before spend |
| TOPMODEL assumptions weak in Front Range terrain | Medium | Validate by soil class; expect worse performance in clay-loam; consider EMT-VS formulation as alternative |
| c1/c2 coefficients unavailable | Medium | Relative index for v1 (§S6) |
| SSURGO gaps in national forest | Medium | Flag in `confidence`; surface data-quality in response metadata |
| Liability on a false `good` | Medium | Advisory framing, org-mediated distribution, track false-`good` rate explicitly |
| A competitor has launched since May 2026 | Unknown | Verify §2 with a live search |

**Open:**
- Does HRRR export usable Ep and σf? (Check GRIB inventory.) — **Answered, see amendment table above: no Ep; σf only in wrfsfc, not wrfprs.**
- What is the actual magnitude of the HRRR–SMAP L4 bias over Front Range soils? — still open, needs the physics phase.
- Does `frozen-firm` vs `good` separate in the report data, or do riders conflate them? — still open, needs Phase 0.
- Trailforks API terms — is a derived commercial product permitted? — still open, deliberately not resolved this session.

---

## 12. References

**Primary:**
Eylander, J., Bieszczad, J., Ueckermann, M., Peters, J., Brooks, C., Audette, W., Ekegren, M. (2023). "Geospatial Weather Affected Terrain Conditions and Hazards (GeoWATCH) description and evaluation." *Environmental Modelling and Software* 160, 105606. Open access, CC BY-NC-ND.

**Code:**
- pyDEM — `github.com/creare-com/pydem` (TWI/slope/aspect; written for this project)
- Downscaling reference notebook — `github.com/creare-com/podpac-examples/blob/main/notebooks/5-datalib/smap/SMAP-downscaling-example-application.ipynb`
- Public GeoWATCH viewer — `mobility.crearecomputing.com`

**Algorithm lineage:**
- Beven & Kirkby (1979) — TOPMODEL
- Walter et al. (2002) — STOPMODEL, shallow subsurface flow
- Ek et al. (2003); Chen et al. (1996) — Noah evaporation formulation (**check Eq. 5 against these**)
- Coleman & Niemann (2013) — EMT; Ranney et al. (2015) — EMT-VS. Colorado-developed alternative lineage.
- Winstral et al. — Sx terrain parameter for snow drift

**Validation datasets:**
- Western & Grayson (1998); Western et al. (2004) — Tarrawarra
- Naithani & Baldwin (2015) — Shale Hills CZO

> Citations above are drawn from the source paper and from memory. I don't have search access — verify anything you plan to cite formally.
