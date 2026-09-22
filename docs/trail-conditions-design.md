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
