# Tarrawarra Validation (Rung 1)

Reproduces the design doc's Rung 1 validation target
(`docs/trail-conditions-design.md`, Section 8): does the Eq. 1 topographic/
transmissivity redistribution (`services/trail-physics/physics/redistribution.py`)
reproduce the published Tarrawarra result (RMSE ≈ 0.0321 m3/m3, beating the
site-mean baseline of 0.0352, improving on ≥9 of 13 sampling dates)?

Per the design doc: **"If you don't land near this, stop and debug. Do this
before anything else."** `run_validation.py` enforces exactly that --
non-zero exit if the target isn't met.

## Status (as of Session 7): FAIL, but closing in -- real, substantial, honestly-earned progress

The full dataset was obtained in Session 4 (see "How the data was acquired"
below) and `run_validation.py` has been run against it for real across
four sessions now (4, 5, 6, 7), testing one concrete, paper-grounded
hypothesis at a time -- Session 7 built and wired the paper's full
two-stage pipeline (Eq. 1 + Eq. 2/7), not just Stage 1. **The gate still
does not pass.** Per the design doc's own instruction, this remains the
right point to report honestly rather than push forward or fabricate a
passing number -- but the gap has narrowed substantially:

```
                                 Session 4    Session 6      Session 7
                                 (builtin)   (best Stage 1)  (+ Stage 2)
Overall baseline RMSE:            0.0370       0.0370          0.0370       (doc target: 0.0352)
Overall Eq. 1[+2] RMSE:           0.1127       0.0597          0.0579       (doc target: 0.0321)
Dates improved:                    0/13         0/13            0/13        (doc target: >= 9)
```

That's a 48.6% reduction in RMSE from where Session 4 started, and one
date (`sm270995`, the wettest/most topographically-driven) now lands
within 0.6% of its own baseline -- the closest any single date has come to
"improved" across all four sessions. Still FAIL, not a pass -- see
"Session 7" below for exactly what Stage 2 added, and why its effect was
smaller than hoped.

### How the data was acquired

`https://people.eng.unimelb.edu.au/aww/tarrawarra/` is unrestricted but
sits behind an Incapsula/Imperva WAF that blocks plain `curl`/automated
fetches (confirmed: a real browser passes it fine over HTTPS; the failure
mode over plain HTTP was a 403 from the origin Apache server directly, a
separate issue from the WAF). The user downloaded the site's own "complete
data archive" (8 zip files, linked from `all_data.html`) through an
ordinary browser and supplied them; everything below was extracted from
those zips, not hand-transcribed through a fetch tool (a real risk in
Session 3 -- see git history -- that this fully resolves).

Committed under `data/`:

```
data/
├── tarrawar.dem       # from siteinfo.zip -- Tarrawarra LOCAL coordinates (NOT UTM, see below)
├── ksat.dat           # from otherdat.zip
├── particle.dat       # from otherdat.zip
├── layer.dat          # from otherdat.zip
├── vegetat.dat        # from otherdat.zip -- NOT used by --with-flux-correction (see Session 7: sigma_f is a swept uniform scalar instead, deliberately)
├── daily.met          # from day_flux.zip -- used by --with-flux-correction (Session 7: Ep via FAO-56 daily Penman-Monteith)
├── neutron.pos        # from siteinfo.zip -- NMM site coordinates
├── docs/              # from tarradoc.zip -- all 9 real Readme.* files
├── tdr/*.tdr           # 13 files, from patterns.zip
└── nmm/tube_*.dat      # 20 files, from neutron.zip
```

Not committed: the zip archives themselves (gitignored -- redundant with
the extracted files above), `fluxdata.zip`'s 6-minute met data (13MB
uncompressed, not needed), `pictures.zip` (site photos, not needed),
`tarrautm.dem` (see below -- deliberately not used).

### Bugs found against real data (this is exactly why Rung 1 exists)

1. **A stray `</content>` tag in the previously-committed `ksat.dat`.**
   Session 3's WebFetch-acquired copy had accidentally included a tool
   -output wrapper artifact as if it were file content. Harmless to the
   parser (skipped as an unparseable line) but embarrassing and now fixed
   by replacing it with the zip archive's clean copy.

2. **Wrong DEM file -- coordinate system mismatch.** Session 3's own
   guidance (in this file!) said to use `tarrautm.dem` (UTM coordinates)
   "NOT tarrawar.dem". This was backwards. `Readme.tdr`'s own text: "x and
   y are the coordinates of the measurement location in the Tarrawarra
   coordinate system (except for the transect which is in universal
   transverse mercator, zone 55)" -- and `ksat.dat`'s own header says
   "Coordinates: Tarrawarra coordinates" too. `tarrawar.dem`'s extent (x:
   732.5-1462.5, y: 752.5-1132.5) matches that local system;
   `tarrautm.dem`'s extent (~362000 / 5831000, real UTM meters) does not
   overlap it at all. Using the wrong DEM produced a **silent** total
   failure -- every single TDR point interpolated to NaN ("outside the
   DEM/ksat coverage"), caught only by `validate_one_date`'s own
   `valid.sum() < len(records) * 0.5` warning, not a crash. `tarrawar.dem`
   is now the one `run_validation.py` uses; `tarrautm.dem` is deliberately
   not committed to avoid a future session repeating this exact mistake.

3. **A genuine `Ksat = 0.0 mm/hr` measurement** in the real `ksat.dat`
   (an effectively impermeable point -- plausible, not obviously a data
   error). `ln(0) = -inf`, which doesn't just break that one point: it
   poisons the DOMAIN-WIDE `log_ks_mean` (mean of any array containing
   `-inf` is `-inf`), corrupting Eq. 1's prediction at every location, not
   just near the zero-conductivity point. Fixed by excluding non-positive
   conductivity measurements from both the domain mean and the
   interpolation source pool, with a loud printed count of how many were
   dropped (1 of 42). A related duplication bug (the domain mean was being
   computed twice, once correctly filtered and once not) was also fixed by
   making `build_terrain_predictors` the single source of truth for this
   filtering, returning `log_ks_mean` directly instead of recomputing it
   in `main()`.

4. **A units mismatch.** TDR data is in %V/V (e.g. `39.1` meaning 39.1%);
   the published targets (0.0352, 0.0321) are in fractional m3/m3. Fixed
   by converting observed TDR values by `/100.0` at the point they're
   loaded -- not just when reporting the final RMSE, because Eq. 1's
   `k=13` constant is an additive correction applied directly to theta on
   whatever scale it's expressed in; applying the correction to %-scale
   theta while `k` was calibrated for fractional-scale theta would apply a
   correction of the wrong relative magnitude, not just report a
   wrongly-scaled final number. This fix alone brought the baseline
   (site-mean) RMSE to 0.0370 -- strikingly close to the paper's own 0.0352
   -- confirming the data, coordinate system, and units are now all
   correct.

5. **NMM/particle/layer parser bugs**, found by testing against the real
   files for the first time (previously only synthetic fixtures existed):
   - `parse_nmm_file`: dates are actually `DD-Mon-YY` (`20-Sep-95`), not
     the `dd/mm/yyyy` `Readme.nmm` documents (matches the TDR files' own
     convention instead). Fixed; all 20 real tube files now parse, 19 of
     20 yielding exactly the paper's stated 59 dates.
   - `parse_particle_file`: the real file gives clay content as an
     explicit 9th numeric column, not as a residual to compute -- and the
     depth-range disambiguation logic was misparsing it as a two-token
     split depth range, producing a nonsense value like `"0-13-0"`. Fixed
     to detect a non-numeric single depth-range token (including
     open-ended `">24"`) and handle an explicit clay column.
   - `parse_layer_file`: the real file is TAB-delimited with texture
     values that can contain internal spaces (`"silt clay"`), which a
     plain `.split()` on whitespace incorrectly broke into extra fields,
     misaligning every column after it. Also, depths can be open-ended
     (`">73"`, the core didn't reach the horizon bottom) or blank (B1
     texture, B2 depth/texture all occasionally missing). Fixed by
     splitting on the literal tab character and changing
     `depth_b1_cm`/`depth_b2_cm` to raw text (preserving `">NN"`) instead
     of float.

All of the above are now covered by dedicated tests in
`tests/test_parsers.py`, including real-file regression tests (skipped
gracefully if the data isn't present, so CI doesn't require the data to be
committed to pass -- though it is committed here).

### Why it fails (Sessions 4-5): pyDEM tested directly, not the (whole) answer

With all five bugs above fixed, Eq. 1's redistribution makes every single
date's prediction **worse** than the site-mean baseline (0.1127 vs.
0.0370 overall) -- not a near-miss, a consistent ~3x degradation across
every date.

**What is confirmed real, not a guess:**

- **The correlation structure is real.** Checking `corr(TDR anomaly, TWI
  anomaly)` per date shows values from 0.07 up to 0.68, strongly positive
  for most dates -- and the two weakest dates (`sm140296`, `sm230296`,
  both February/late-summer, driest of the 13) match the paper's *own*
  described behavior almost exactly: "the only times that topography does
  not strongly influence the soil moisture pattern is when the site is
  extremely dry... or extremely wet." Our TWI computation is capturing
  real, physically-correct signal.
- **The magnitude is wrong by roughly 5-6x.** A least-squares fit of the
  TWI-anomaly-to-observed-anomaly relationship across all 13 dates implies
  an effective `k` of about 74, not the paper's stated 13.

**Session 4 hypothesized** this was because `compute_twi()`'s from-scratch
D8 implementation was numerically incompatible with **pyDEM** (Ueckermann
et al. 2018), the specific tool Section 2.2 of the paper says it used.
**Session 5 tested this directly** -- installed pyDEM, computed TWI
through it (`physics/terrain.py::compute_twi_pydem`, run via
`--twi-engine pydem`) on the real Tarrawarra DEM, and re-ran the full
harness. Result:

```
                     builtin (D8)   pydem (D-infinity)   pydem, x10-scaled
Overall Eq. 1 RMSE       0.1127            0.1089              0.9581
Dates improved            0/13              0/13                0/13
```

**The hypothesis was wrong, or at least insufficient.** pyDEM's TWI has
similar standard deviation to `compute_twi()`'s own output (1.43 vs. 1.28
-- not the dramatic difference the ~5-6x gap would need), and per-date
correlation with real observed anomalies is only modestly better (e.g.
`sm230296`: 0.084 -> 0.132; `sm101196`: 0.515 -> 0.588) -- a genuine,
worthwhile improvement, but the implied-k gap barely moves (74 -> 64,
both far from 13). The x10-scaled variant (pyDEM's own stored value, in
case that's what a saved GeoWATCH raster would contain) is dramatically
*worse* (0.958), as basic dimensional reasoning predicts (a 10x larger
correction term, decisively ruled out).

**What this means**: switching flow-routing algorithms (D8 -> D-infinity)
does not close the gap by itself. Two things ruled out by direct algebra,
not tested empirically (no point): Ks's measurement units (mm/hr vs. any
other unit) cancel out of Eq. 1's `(ln(Ks) - ln(Ks)-bar)` deviation term
regardless of choice (a unit conversion is a uniform multiplicative
constant on Ks, hence an additive constant on ln(Ks), which washes out
against the mean); the same argument rules out "specific catchment area"
normalization conventions (per-unit-contour-length vs. raw area) as an
explanation, for the same reason.

### Session 6: three more hypotheses tested -- real, substantial progress, still not passing

Three more concrete, paper-grounded hypotheses were tested this session,
each via a new `run_validation.py` flag:

**1. Resolution mismatch (`--dem-resolution {10,15,30}`, uses
`physics.terrain.coarsen_dem`)**: the paper's Section 2.4 states
GeoWATCH's *global* elevation composite is 30m resolution and "sets the
finest scale at which downscaled soil moisture products can be computed"
-- but Section 4.2.1 (re-read carefully this session) explicitly says the
Tarrawarra comparison used the site's own 5m DEM "in lieu of its default
global geospatial inputs." **Verdict: ruled out by the paper's own text**,
and independently confirmed empirically -- coarsening to 30m *collapses*
the TWI/observed-anomaly correlation (0.465 -> 0.193) rather than
improving it. At only ~13-23 cells across, this ~700m-wide catchment is
simply too small to resolve real terrain structure at 30m. 10m showed a
mild, inconclusive improvement (implied k: 74 -> 65); nothing close to 13
at any resolution.

**2. Soil texture instead of measured conductivity (`--ks-source
texture`)**: Section 4.2.1 lists "soil texture data" (not measured
conductivity) as a Tarrawarra input. Built `physics/soil_texture.py`: a
zero-dependency USDA texture-triangle classifier (boundary data
transcribed from the public-domain USDA Soil Survey Manual scheme, cross
-checked for zero coverage gaps across the full valid triangle) feeding
into Noah's own `SOILPARM.TBL` (STAS table, fetched live from
`wrf-model/WRF`) for Ks/theta_wilt/theta_ref/theta_s per texture class --
the same lookup table the paper's own Ek-2003/Chen-1996 flux lineage is
built on. **Result: real RMSE improvement (0.1127 -> 0.0930)**, but
diagnosis shows this is mostly a magnitude-shrinkage effect, not better
physics: Tarrawarra's 11 sample sites are texturally close to homogeneous
(matching the paper's own description, "the Tarrawara catchment site does
not show much variability in land cover, vegetation, or soil type"), so
texture-derived ln(Ks) has ~8x less spread than the noisy measured field
(std 0.14 vs 1.19) and its correlation with real anomalies is
statistically indistinguishable from zero (|r| < 0.11 on every date). The
TWI term's own implied-k is essentially unchanged (74.9, matching the
measured-Ks baseline of 73.7) -- confirming the persistent gap lives in
the TWI term, not the Ks term.

**3. pyDEM's non-default capping (`--twi-apply-limits`)**: `apply_twi_limits`/
`uca_saturation_limit=32` are off by default in pyDEM itself; a production
GeoWATCH configuration might enable them. **Result: shrinks TWI's standard
deviation by ~30% (1.43 -> 1.01) and reduces RMSE (0.1089 -> 0.0909)** on
its own.

**Combining all three (pyDEM + capping + texture-Ks, at native 5m
resolution -- resolution coarsening does NOT stack usefully with the
others) gives the best result found across every session so far:**

```
                                          builtin (Session 4)   pydem+limits+texture-Ks (Session 6)
Overall Eq. 1 RMSE                              0.1127                       0.0597
Implied best-fit k (TWI term)                    73.7                        43.9
Correlation (TWI anomaly vs. observed)           0.465                       0.519
```

This is real, structural progress, not just magnitude convergence: the
implied-k gap narrowed from 5.7x to 3.4x, and correlation quality
genuinely *improved* (didn't just shrink toward the trivial baseline, as
the resolution-coarsening and texture-Ks-alone experiments partly did).
One date (`sm270995`, the wettest, hence most topographically-driven) now
lands at 0.0494 model RMSE vs. 0.0490 baseline -- a near-exact tie, right
at the edge of "improved." Still FAIL overall (0.0597 vs. target 0.0321,
0/13 dates improved), but meaningfully closer than any prior session.

**What was deliberately NOT done, again**: fit a local `k` to pass the
gate. `k=13` remains untouched in every configuration tested, across all
three sessions now.

**How to reproduce any of the above**:
```bash
pip install pydem   # optional, heavier dep (rasterio + Cython)
python3 run_validation.py --data-dir ./data                                     # builtin D8, baseline
python3 run_validation.py --data-dir ./data --twi-engine pydem                  # pyDEM, unscaled
python3 run_validation.py --data-dir ./data --twi-engine pydem --twi-scaled     # pyDEM, x10 (ruled out)
python3 run_validation.py --data-dir ./data --dem-resolution 30                 # resolution sweep (ruled out)
python3 run_validation.py --data-dir ./data --ks-source texture                 # texture-derived Ks
python3 run_validation.py --data-dir ./data --twi-engine pydem --twi-apply-limits  # pyDEM capping
python3 run_validation.py --data-dir ./data --twi-engine pydem --twi-apply-limits --ks-source texture  # best combo
```

### Session 7: Stage 2 (Eq. 2/7) built and wired for real -- a real but smaller-than-hoped effect

Session 6 left one open hypothesis unaddressed: that the published 0.0321
represents the FULL two-stage pipeline, not Stage 1 alone, since Eq. 2/7's
sign structure systematically damps Stage 1's (currently over-amplified)
anomalies. Session 7 built Stage 2 for real -- `--with-flux-correction` is
no longer a stub -- and tested it.

**What was built** (all in `stage2.py`, kept separate from
`run_validation.py` for independent testability):

- **Ep (potential evapotranspiration)**: FAO-56 daily Penman-Monteith
  (`physics/pet.py`, new daily-timestep functions -- Eq. 6-40 of Allen et
  al. 1998, each cross-checked against the primary source's OWN fully
  worked numerical examples, not just formula transcription). Driven by
  `data/daily.met` (a real parser added: 32 tab-delimited columns,
  confirmed against the file directly). Actual vapor pressure comes from
  wet/dry-bulb psychrometry (Tarrawarra's own instrumentation), not
  relative humidity. Net radiation is measured directly when available and
  falls back to FAO-56's own Rs-based estimate for the 2 dates (of 13)
  whose survey window predates the net-radiation sensor's installation or
  falls in a real, isolated sensor outage (Feb 1996).
- **sigma_f (vegetation greenness fraction)**: per the user's own decision,
  a spatially-uniform swept scalar (0.4/0.6/0.8) -- the paper's own
  Tarrawarra input list ("site average soil moisture, 5-m DEM, and soil
  texture data") does NOT include site vegetation data, meaning GeoWATCH
  used its own default global greenness layer here, uniform at this 10.8
  ha site's scale. `data/vegetat.dat`'s biomass measurements are
  deliberately NOT used -- converting biomass to a greenness fraction
  would be an invented modeling step the paper doesn't describe.
- **theta_wilt/theta_ref/theta_s**: reused Session 6's texture
  classification (fine = per-point nearest-site lookup; coarse = simple
  mean across all 11 sites, the paper's "weather-scale-averaged soil
  properties" for a domain that IS one weather-scale block).
- **iota (Eq. 6)**: computed from the native 5m DEM's slope/aspect at each
  point. Required a real correctness check first: Tarrawarra is at
  37.65 S, and every existing `solar_view_factor` test used a
  northern-hemisphere latitude. Added 4 explicit southern-hemisphere
  tests (mirroring the existing northern ones) -- all passed with NO code
  changes needed. `solar_position`/`local_incidence_cosine` are pure
  trigonometry with no hemisphere-specific branch, so this was a real risk
  worth checking (exactly the kind of directional bug Session 2's
  aspect-sign error was), not just a formality.
- **Active-layer depth** (converts Ep from mm/day to fraction/day -- see
  below): swept 150/300/1000mm.

**A real dimensional-analysis finding, resolving relaxation.py's
long-standing "open calibration question"**: for `delta_t` (Eq. 7) to
actually come out in *days* (matching the paper's explicit "clipped ...
0 to 30 days"), `F(theta_s)` must be a fraction-per-day rate, not mm/day --
confirmed by working through the algebra, not assumed. The fix (dividing
Ep by an assumed active-layer depth) is a standard bucket-model
conversion, not a hack -- but then a second, more surprising result
emerged when actually testing the depth sweep:

**The active-layer depth turns out not to matter AT ALL** (confirmed
bit-for-bit identical results across 150/300/1000mm, then proven exactly
via algebra): whenever the SAME Ep drives both the fine and coarse flux
terms (true here -- one weather station, not a spatially-resolved met
field), Eq. 7's `delta_t` is *exactly* inversely proportional to Ep, while
Eq. 2's flux-difference term is *exactly* directly proportional to Ep --
their product (the actual correction applied) is analytically independent
of Ep's absolute magnitude, and therefore of the depth normalization
entirely. This **fully closes** the depth-normalization question for any
site/configuration where fine and coarse Ep are equal -- not just "we
didn't find a good value," but "the value provably doesn't matter here."
sigma_f has a real but tiny effect (visible only at the 5th decimal place
per-point; below display precision in the aggregate RMSE) for the same
underlying reason (it appears in both the correction's numerator and
Eq. 7's denominator in closely -- though not exactly -- canceling ways).

**Eq. 5 form does matter, modestly**: `geowatch` (paper-literal, unclipped
ratio) beat `ek2003` by a small but real margin (0.0579 vs. 0.0587 on the
best combined configuration).

**Results**: Stage 2 only ever activates on dates where `theta_ws <
theta_ref` (Eq. 7's `delta_t` clips to exactly 0 otherwise) -- confirmed
against real Tarrawarra data, this is **7 of the 13 dates**, not a rare
edge case. On those 7 dates, Stage 2 correctly and consistently pulled
Stage 1's over-amplified predictions toward the baseline (every single
active date improved, never made worse) -- physically correct signed
behavior -- but by a modest amount. Combining Stage 2 with Session 6's
best Stage 1 configuration (pyDEM + `--twi-apply-limits` + `--ks-source
texture`) gives the best result found across all four sessions:

```
                                  Session 6         Session 7
                              (Stage 1 only)   (Stage 1 + Stage 2)
Overall Eq. 1[+2] RMSE            0.0597            0.0579
```

**How to reproduce**:
```bash
python3 run_validation.py --data-dir ./data --twi-engine pydem --twi-apply-limits \
    --ks-source texture --with-flux-correction --eq5-form geowatch
```

### What remains open

- **Whether the remaining gap needs a different TWI calibration, a
  different Stage 2 detail, or both.** Stage 2's real-but-modest effect
  means it's a genuine contributor, not the single missing piece -- the
  bulk of the gap (which Session 4-6 already traced to the TWI term
  specifically) is still unexplained.
- **The "modified TWI" sentence** (Section 2.2.1: "the GeoWATCH calculation
  of the TWI was modified to use volumetric soil moisture instead of
  relative soil moisture") remains the single most suspicious unexplained
  detail in the paper's own text -- still not resolved by anything tested
  across 4 sessions.
- **Whether 0.0321 is Stage-1-only or the full pipeline** -- Session 7's
  result doesn't resolve this either way: Stage 2 helped, but not enough
  to flip the conclusion.
- Shale Hills (the paper's second validation site, 74 dates, public data)
  remains the next planned arbiter, per the user's own decision to pursue
  it after Stage 2 rather than in parallel.

### If the DEM parser fails

`parsers.py::parse_dem` tries the confirmed-real Tarrawarra header format
first (`north:`/`south:`/`east:`/`west:`/`rows:`/`cols:`, one "key: value"
pair per line, cellsize derived), falling back to an originally-guessed
(never independently confirmed for any real file) ESRI ASCII grid header.
If somehow neither matches, `parse_dem` raises `ValueError` with the
actual header lines printed.

## NMM validation target (denser, not yet wired into run_validation.py)

Per the GeoWATCH paper (Section 4.2.1): the Tarrawarra dataset also
includes Neutron Moisture Meter (NMM) readings -- only 20 locations per
date (vs. ~508 for TDR, worse for spatial structure) but across **59
dates** (vs. 13 for TDR), a much denser temporal check. Published target:
RMSE 0.040 -> 0.030 m3/m3, improving on 56/59 dates (95%).

`parsers.py::parse_nmm_file` is now validated against all 20 real files
(19 of 20 yield exactly 59 profiles; `tube_20.dat` yields 54 -- plausible
real-world missing-measurement variability, not a parsing bug). **Not yet
implemented**: the `run_validation.py` wiring, which needs a different
shape than TDR's one-file-per-date (here, one file per *site*; a "date"
means grouping matching date/time entries across all 20 files). Left for a
future session, and moot until the Eq. 1 / TWI issue above is resolved
first anyway -- per the design doc, don't build more on top of an
unresolved Rung 1 failure.

## Attribution

Per the dataset's own copyright notice: any publication using this data
must cite Western, A.W. and Grayson, R.B. (1998), "The Tarrawarra data set:
Soil moisture patterns, soil characteristics and hydrological flux
measurements", *Water Resources Research*, 34(10), 2765-2768.
