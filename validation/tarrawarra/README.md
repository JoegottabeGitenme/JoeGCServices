# Tarrawarra Validation (Rung 1)

Reproduces the design doc's Rung 1 validation target
(`docs/trail-conditions-design.md`, Section 8): does the Eq. 1 topographic/
transmissivity redistribution (`services/trail-physics/physics/redistribution.py`)
reproduce the published Tarrawarra result (RMSE ≈ 0.0321 m3/m3, beating the
site-mean baseline of 0.0352, improving on ≥9 of 13 sampling dates)?

Per the design doc: **"If you don't land near this, stop and debug. Do this
before anything else."** `run_validation.py` enforces exactly that --
non-zero exit if the target isn't met.

## Status (Session 4): real data acquired, Rung 1 run for real, FAIL -- root cause identified, not fabricated-around

The full dataset was obtained this session (see "How the data was acquired"
below) and `run_validation.py` was run against it for the first time. Three
real bugs were found and fixed along the way (see "Bugs found against real
data"); after fixing them, **the gate still does not pass**, with a
specific, diagnosed, and NOT-yet-resolved root cause (see "Why it fails").
Per the design doc's own instruction, this is exactly the right point to
stop and report honestly rather than push forward or fabricate a passing
number.

```
Overall baseline (site-mean) RMSE: 0.0370  (doc target: 0.0352)   <- very close, as expected
Overall Eq. 1 redistribution RMSE: 0.1127  (doc target: 0.0321)   <- 3x WORSE than baseline
Dates improved: 0/13  (doc target: >= 9)
RUNG 1: FAIL
```

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
├── vegetat.dat        # from otherdat.zip -- for a future --with-flux-correction
├── daily.met          # from day_flux.zip -- for a future --with-flux-correction
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

### Why it fails: still unresolved -- pyDEM was tried directly and it is NOT the (whole) answer

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
does not close the gap. The two candidates that remain most plausible,
neither yet tested:

1. **A resolution-scale mismatch.** `k=13` might be calibrated against TWI
   computed on a coarser "high-resolution" grid (e.g. 30m) than the full
   5m DEM used here -- coarser grids average out small-scale variability
   and would shrink TWI's spread, which is the direction needed (smaller
   deviations -> smaller k needed for the same correction magnitude).
   Untested: would require regridding the DEM and repeating this exact
   comparison at 2-3 coarser resolutions.
2. **pyDEM's non-default options.** `apply_twi_limits` and
   `uca_saturation_limit=32` are both off by default in the version we
   ran; a production GeoWATCH configuration might enable them, compressing
   the upper tail of the TWI distribution. Untested.

Two things ruled out by direct algebra, not tested empirically (no point):
Ks's measurement units (mm/hr vs. any other unit) cancel out of Eq. 1's
`(ln(Ks) - ln(Ks)-bar)` deviation term regardless of choice (a unit
conversion is a uniform multiplicative constant on Ks, hence an additive
constant on ln(Ks), which washes out against the mean); the same argument
rules out "specific catchment area" normalization conventions (per-unit-
contour-length vs. raw area) as an explanation, for the same reason.

**What was deliberately NOT done**: fit a local `k` (64-74, depending on
TWI engine) to make this gate pass. That would defeat the entire purpose
of Rung 1 -- reproducing a literature-published, independently-calibrated
constant, not curve-fitting our own implementation to hit a target number.
`k=13` stays as documented, unmodified, in every configuration tested.

**How to reproduce any of the above**:
```bash
pip install pydem   # optional, heavier dep (rasterio + Cython)
python3 run_validation.py --data-dir ./data                              # builtin D8
python3 run_validation.py --data-dir ./data --twi-engine pydem           # pyDEM, unscaled
python3 run_validation.py --data-dir ./data --twi-engine pydem --twi-scaled  # pyDEM, x10
```

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
