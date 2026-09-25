# Shale Hills Validation (second generalization check)

The final planned check for Session 8's Rung 1 discovery
(`validation/tarrawarra/README.md`'s Session 8 section): does the real
Creare/GeoWATCH downscaling equation, run with the EXACT same frozen
configuration that passed at Tarrawarra (Session 8) and generalized to
Tarrawarra's own Neutron Moisture Meter holdout (Session 9), also
generalize to a **fully independent research catchment** -- different
continent, terrain, soil, climate, and land cover?

Per the GeoWATCH paper (`geowatch.pdf`, Section 4.2.2): Shale Hills is a
small, forested, V-shaped, shale-derived-soil watershed in central
Pennsylvania, part of the Susquehanna Shale Hills Critical Zone
Observatory (SSHCZO). Published target: site-mean baseline RMSE 0.060 ->
GeoWATCH 0.054 m3/m3, improving on 55 of 74 dates (74%). Citation:
Naithani, K., Baldwin, D., 2015. "CZO Dataset: Shale Hills - Soil
Moisture, Hydropedologic Properties 2006-2015."

## Status: PASS

```
Overall baseline (site-mean) RMSE: 0.0649
Overall model RMSE:                0.0575
Dates improved: 67/76  (this script's prespecified gate: >= 38)
Stage 2 applied on 31/76 dates
```

**SHALE HILLS HOLDOUT: PASS** -- decisively (67 of 76 dates, 88%, well
past the prespecified plain-majority gate of 38). Combined with Session 9's
NMM pass, the Session 8 discovery now has real, positive evidence from
**two independent instruments across two independent, structurally
different research catchments on two different continents**, using the
IDENTICAL frozen configuration (equation form, TWI engine, capping,
Stage 2 settings, `k=13`) throughout -- nothing was re-tuned for this
site. See "Results" below for the full breakdown and the robustness
checks performed before trusting this number.

## How the data was found

The paper's own citation (`http://criticalzone.org/shale-hills/data/dataset/3001/`)
is dead -- the CZO program was succeeded by the Critical Zone Collaborative
Network (CZ Net) in December 2020. The archived listing
(`https://czo-archive.criticalzone.org/shale-hills/data/dataset/3001/`)
still exists and explicitly points to "the updated resource on
HydroShare.org." That HydroShare resource
(`83efb5317d284ba996ddaeb2b74b6a42`, CC-BY licensed) was fetched directly
via HydroShare's own REST API -- no WAF, no manual-download workaround
needed (unlike Tarrawarra's host). The DEM, SSURGO soil survey extract,
and flux-tower meteorology were located the same way, via HydroShare's
public "CZO Shale Hills" group (group id 147).

Committed under `data/`:

```
data/
├── SSHCZO_2010TDRSoilMoisture.xlsx     # TDR soil moisture, 106 sites x 7 depths x 76 dates
├── shalehills_dem_3m_nad83utm18n.tif   # DEM, REPROJECTED to NAD83 UTM 18N -- see below, critical
├── ssurgo/
│   ├── mapunit.txt                     # subsetted to the ~10 map units intersecting the catchment
│   ├── comp.txt                        # (classic SSURGO tabular export, real column schema -- see below)
│   ├── chorizon.txt
│   └── soilmu_subset.{shp,dbf,shx,prj} # subsetted map-unit polygons (12 shapes, was 22920 county-wide)
├── met/
│   └── 20{09..13}_CZO_FluxTowerMeteo.dat  # real Campbell Scientific TOA5 10-minute flux-tower logger files
└── docs/
    ├── ReadMe_TDRSoilMoisture.md
    ├── ReadMe_DEM.md
    └── ReadMe_Meteorology.md
```

## A real, load-bearing coordinate-system discrepancy (found and resolved before writing any interpolation code)

Worth reading before touching anything in this directory. At this site's
scale (an 8ha / ~500x300m catchment), a coordinate-system mistake here
would be far more damaging than at Tarrawarra (10.8ha) -- it would not
just "shift results," it would silently misalign every input entirely.

- The TDR xlsx's own `ReadMe.md` claims **"NAD 1927 State Plane (PA)
  coordinate system."** This is wrong. Decisively confirmed: computing the
  full 106-site coordinate cloud's bounding box under two hypotheses (NAD83
  UTM Zone 18N vs. NAD27 UTM Zone 18N) and comparing against HydroShare's
  OWN independently-stated WGS84 lat/lon bbox for this exact resource --
  the NAD83 UTM 18N interpretation matches to 4 decimal degrees EXACTLY
  (`lon [-77.9071, -77.9020], lat [40.6637, 40.6658]`, both ways); the
  NAD27 interpretation is off by ~0.002 degrees (~200m north-south -- more
  than half the catchment's own extent). **The TDR site coordinates are
  NAD83 UTM Zone 18N (EPSG:26918)**, not State Plane, matching SSURGO's
  own correctly-stated CRS (`readme.txt`: "Coordinate System: UTM Zone 18,
  Northern Hemisphere (NAD 83)").
- The DEM's own `prj.adf` (read directly with `rasterio`, not assumed)
  says **NAD27 UTM Zone 18N (EPSG:26718)** -- a genuinely different datum
  from the TDR/SSURGO data, not a units/naming quibble. The NAD27->NAD83
  shift at this specific location is **(+32m East, +212m North)**,
  confirmed by direct `pyproj` transformation. Left uncorrected, every TDR
  site would silently sample the DEM ~212m away from its true position --
  more than half the catchment's own height, guaranteeing either a "100%
  interpolation failure" (Tarrawarra Session 4's own phrase for this exact
  failure mode) or, worse, a *silent*, subtly-wrong terrain sample that
  wouldn't trip any obvious warning.
- **Resolution**: the committed DEM (`shalehills_dem_3m_nad83utm18n.tif`)
  has ALREADY been reprojected to EPSG:26918 via `rasterio`'s
  warp/reproject (bilinear resampling) -- matching the TDR and SSURGO
  data's real CRS. `parsers.py::parse_dem_geotiff` raises loudly (not
  silently) if ever pointed at a DEM that isn't EPSG:26918, specifically
  to prevent this mistake from being silently reintroduced.

## A second real discrepancy (opposite direction from Tarrawarra)

The TDR xlsx's own values are **already fractional m3/m3** (e.g. 0.2215),
matching the paper's own published RMSE units directly -- no conversion
needed. Tarrawarra's TDR files were %V/V (needed `/100` before use).
Reflexively applying that same `/100` conversion here would have silently
divided every real value by 100 a second time. `parsers.py::parse_tdr_xlsx`
applies no conversion, deliberately.

## Depth choice

Shale Hills' TRIME-T3 tube probes read at 10/20/40/60/80/100cm -- widely
spaced discrete depths, unlike Tarrawarra's TDR (one integrated ~30cm
average) or NMM's closely-spaced 15/30cm pair (which could be meaningfully
averaged). **10cm (the shallowest depth) is used** as the primary
comparison quantity here -- the closest available match to the near-
surface value the real GeoWATCH equation actually downscales (see
`services/trail-physics/physics/redistribution.py`'s module docstring:
the production equation's `theta_SMAP` input is a near-surface satellite/
land-surface-model value, not a full-profile average).

## Soil texture: dominant-component convention

SSURGO map units at this site are frequently associations/complexes of 2
named soil series -- e.g. **"BMF" = Berks-Weikert association** (Berks
50%, Weikert 30%, remainder unnamed/rock). Berks and Weikert are the
canonical, extensively-studied soil series at THIS specific site in the
Critical Zone literature -- their presence in the real SSURGO extract is
itself a strong correctness signal (locked in as
`test_real_data_mapunit_names_match_known_shale_hills_soil_series`), not
just a schema check.

Following the same "one real, documented choice" discipline as
Tarrawarra's particle.dat "shallowest layer" convention: the **dominant
component** (highest `comppct_r`) per map unit is used, and that
component's **shallowest horizon's** sand%/clay% feeds the *same*
`physics.soil_texture.soil_hydraulic_properties` (USDA triangle -> Noah
SOILPARM.TBL) pipeline Tarrawarra/NMM used -- deliberately the same
pipeline, not a different soil-parameter methodology that happens to also
produce a pass. Real texture variation across the 106 TDR sites turned out
modest (57 of 71 checked sites fall in one dominant map unit, sand~30%/
clay~14%; a smaller cluster of 11 falls in a second, sand~29%/clay~17.5%)
-- meaning the strong result below is primarily a genuine topographic-
wetness effect, the same mechanism validated at Tarrawarra/NMM, not an
artifact of unusually large texture heterogeneity.

## SSURGO tabular schema -- extracted from the real template database, not memory

The real classic-SSURGO tabular export (`soildb_US_2002.zip`, a fixed
nationwide MS Access template, no per-file column headers) was **not**
guessed or recalled from memory. The actual bundled
`soildb_US_2002.mdb` template database was read directly (via the
pure-Python `access_parser` package -- no `mdbtools`/ODBC driver
available in this environment) to extract the real, authoritative column
order for `mapunit`/`component`/`chorizon`. This was independently
verified against a real data row via an internal-consistency checksum:
a horizon's `sandtotal_r + silttotal_r + claytotal_r` must sum to ~100;
confirmed EXACTLY (26.3 + 52.7 + 21.0 = 100.0) on the first real row
checked, and re-confirmed as a permanent regression test
(`test_real_data_chorizon_sand_silt_clay_sum_to_100`) against every real
horizon in the committed subset.

The shapefile (`soilmu_a_pa061.shp`, originally 22,920 polygons county-
wide) and the tabular files were subsetted to just the ~10 map units that
actually intersect the Shale Hills catchment, using `shapely` for the
spatial intersection test -- the full county-wide files were never
committed (21MB+ vs. the ~24KB subset actually needed).

## Meteorology

Real Campbell Scientific TOA5-format flux-tower logger files (10-minute
intervals, 2009-04-01 through 2013-05-20 -- only 31 of the 76 TDR dates
fall in this window; the rest fall back to Stage-1-only, exactly like
Tarrawarra/NMM's own per-date graceful fallback). Real, directly-measured
net radiation and relative humidity (no Rn-from-Rs fallback needed here,
unlike Tarrawarra) -- but **no wind speed sensor at all**. FAO-56 (Allen
et al. 1998, Chapter 3, "Estimating missing climatic data") explicitly
sanctions a fixed default for exactly this situation, verified against the
live primary source (not recalled from memory) before use: *"Where no
wind data are available within the region, a value of 2 m/s can be used
as a temporary estimate. This value is the average over 2000 weather
stations around the globe."* Session 10 also added
`physics/pet.py::actual_vapor_pressure_from_rh_kpa` (FAO-56 Eq. 17, using
real daily RHmax/RHmin rather than Tarrawarra's wet/dry-bulb pair),
verified against FAO-56's own worked Example 5 (ea=1.70 kPa).

Real sensor dropouts exist in the raw logger files (e.g. a 2011 record
with `pressure_irga_mean=11.004` kPa -- physically impossible at this
site's ~285m elevation). `parsers.py::parse_meteo_file` rejects records
outside a generous plausible pressure range (70-105 kPa) rather than
feeding sensor noise into the FAO-56 calculation.

## A real data-quality bug found in the TDR data (same discipline as Session 4/9)

Exactly 8 of 4141 readings at 10cm depth are a literal **0.0** --
physically implausible (even bone-dry soil retains residual moisture).
Confirmed as a data-quality artifact, not a real reading, by a clean
discontinuity in the value distribution: the 8 zeros are followed
immediately by a jump to 0.005 and up, with no smooth continuum near zero
the way a genuine dry-limit reading would show. All 8 are concentrated at
just 2 sites (`A3`, `74B`) on specific dates -- consistent with a recorded
probe-fault code, not 8 independent real dry readings. Excluded (treated
like the sheet's own `NA` marker) with a loud, counted warning. Fixing
this changed the final result only marginally (0.0575 vs. 0.0576 RMSE,
67/76 vs. 68/76 dates -- confirms the PASS isn't an artifact of this one
data issue).

## Results

Configuration: Session 8's exact frozen recipe (`FROZEN_CONFIG` in
`run_validation.py`) -- pyDEM TWI with `apply_twi_limits`, per-point
(fine) soil parameters, Stage 2 with the `geowatch` Eq. 5 form, `sigma_f
=0.6`, `k=13` untouched. **No command-line flags exist to change any of
this** -- unlike `validation/tarrawarra/run_validation.py`'s deliberate
sweep-everything design, this harness runs exactly one configuration, by
design, for the same reason `run_nmm_validation.py` does: this is a
holdout check, not another round of hypothesis testing.

```
Overall baseline (site-mean) RMSE: 0.0649   (paper's own context figure: 0.060)
Overall model RMSE:                0.0575   (paper's own context figure: 0.054)
Dates improved: 67/76                        (paper's own context figure: 55/74, 74%)
```

**PASS** against this project's own prespecified gate (overall RMSE beats
baseline AND at least half of compared dates improve -- 38 was the
threshold; 67 was achieved). Written into `run_validation.py` as constants
with a full docstring explanation *before* the script was ever run against
real data.

Sanity/robustness checks performed before trusting this result (a result
this strong warranted real skepticism, not just a celebratory readout):
- **TWI grid statistics are non-degenerate**: mean 6.53, std 1.09, range
  3.6-11.1 across 26,927 valid cells (98.8% of the grid) -- comparable
  magnitude to Tarrawarra's own capped TWI (std ~1.01), not a pathological
  near-zero-variance grid that would trivially "predict the baseline."
- **Texture variation is real but modest** (see "Soil texture" above) --
  rules out "the pass is just texture heterogeneity doing all the work."
- **Per-date RMSE reductions are plausible in magnitude** (typically
  10-30%), not suspiciously uniform or perfect -- consistent with real
  physics, not a bug that trivially collapses predictions toward the
  baseline.
- Re-running after fixing the 0.0-reading bug changed the result by <1%,
  confirming it isn't fragile to that one data issue.

**What this does and doesn't settle**: this is real, strong evidence the
Session 8 discovery is genuine, transferable physics -- not a fit to
Tarrawarra's 13 dates, and not an artifact confined to that one site's
particular terrain/soil/climate. Combined with the NMM holdout (Session
9), the physics has now been checked against 3 independent measurement
records across 2 independent, structurally different catchments, with
zero constants ever adjusted. Per the project's own sequencing decision,
this was deliberately the LAST validation check before beginning the
Colorado static-terrain-stack build (`pipelines/static/`) -- that work is
no longer gated on anything in the validation ladder.

**How to reproduce**:
```bash
pip install -r requirements.txt  # openpyxl, pyshp, shapely, rasterio -- on top of trail-physics' own requirements.txt
python3 run_validation.py --data-dir ./data
```

## Attribution

Per HydroShare's own Data Use Policy and this resource's citation:
"Logistical support and/or data were provided by the NSF-supported Shale
Hills Susquehanna Critical Zone Observatory." Original dataset citation:
Naithani, K., Baldwin, D. (2019). SSHCZO -- Soil Moisture, Hydropedologic
Properties -- Shale Hills -- (2006-2015), HydroShare,
http://www.hydroshare.org/resource/83efb5317d284ba996ddaeb2b74b6a42.
DEM/SSURGO/LiDAR data: HydroShare resource
`cea8dda7b8c64f76aaaf412d8d37f041`. Meteorology: HydroShare resource
`4c3da3f64b2f436cae68f45f77f4e3be` (Davis, K., Shi, Y.).
