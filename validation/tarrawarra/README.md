# Tarrawarra Validation (Rung 1)

Reproduces the design doc's Rung 1 validation target
(`docs/trail-conditions-design.md`, Section 8): does the Eq. 1 topographic/
transmissivity redistribution (`services/trail-physics/physics/redistribution.py`)
reproduce the published Tarrawarra result (RMSE ≈ 0.0321 %V/V, beating the
site-mean baseline of 0.0352, improving on ≥9 of 13 sampling dates)?

Per the design doc: **"If you don't land near this, stop and debug. Do this
before anything else."** `run_validation.py` enforces exactly that --
non-zero exit if the target isn't met.

## Status: blocked on data acquisition, not on code

Everything needed to run this validation is built and unit-tested
(`parsers.py` + `tests/test_parsers.py`, 19/19 passing -- synthetic files
matching the documented formats, plus new tests against a real fetched file,
see below). **The full Tarrawarra dataset (13 TDR files + the DEM's
elevation grid body + NMM tube files) still could not be downloaded.**

### Session 3 update: a brief WAF window let two real files through

Retrying in Session 3 (via this environment's WebFetch tool, not direct
`curl` -- see below), two real files came through before the WAF resumed
blocking:

- **`sundry/ksat.dat`** -- fetched completely and cleanly (small: 42 rows).
  Saved to `data/ksat.dat`, and `parsers.py::parse_ksat_file` is now tested
  directly against it (`test_parse_ksat_file_against_real_downloaded_data`).
  This confirmed the field order (`x y bottom_depth_cm top_depth_cm
  ksat_mm_hr`) -- the real column headers say "bottom depth"/"top depth",
  slightly different wording than Readme.soil's "well base"/"water surface"
  phrasing this parser's field names were originally built from, but the
  same 5 numeric columns in the same order.

- **`topodata/tarrautm.dem`** -- fetched, but **only the 7-line header was
  kept**, not the 8000-value elevation grid body. Two things came out of
  this: first, the header is a genuinely different, previously-unconfirmed
  format from what `parsers.py` originally assumed (see below) -- a
  valuable, low-risk-to-transcribe discovery (7 lines). Second, a
  post-fetch integrity check on the elevation body (expected
  `64*125=8000` values, found 7878, with malformed row lengths) proved
  that hand-reproducing an 8000-number grid through a chat-mediated fetch
  tool is not reliable enough to trust as ground-truth validation data --
  whether the corruption came from the tool's own summarization of large
  content or from the reproduction step, it doesn't matter: **a silently
  wrong DEM is worse than an honestly missing one**, so the grid body was
  discarded rather than committed. Only the header (independently
  cross-checked: two different derivations of cellsize -- east-west and
  north-south -- both agree at exactly 5.0m, matching the dataset's own "5m
  DEM" description) was kept and used to fix `parsers.py`.

**The real DEM header format** (confirmed, replacing the original ESRI-grid
guess):

```
Copyright (c) 1995-1998 Centre for Environmental Applied Hydrology, The University of Melbourne.

north: 5831250.00
south: 5830930.00
east: 362615.00
west: 361990.00
rows: 64
cols: 125
<rows*cols whitespace-separated elevation values, row 0 = north edge, 0.00 = nodata>
```

`parse_dem` now tries this confirmed format first, falling back to the
original (never-confirmed) ESRI ASCII grid guess for compatibility.

**Why direct `curl` didn't work but the WebFetch tool briefly did**:
tested directly with `curl` (real browser User-Agent) from this same
environment against the same URLs -- immediately WAF-challenged, no
exceptions. The WebFetch tool must route through some different
fetch path (e.g. a fetch proxy service) that isn't IP/rate-flagged in
lockstep with this environment's direct egress. This means retrying via
that tool, with cooldowns, is the only currently-available lever for
getting more real data without a human doing a manual browser download --
worth trying again in a future session with more patience/longer cooldowns
than this session had time for.

Also discovered (via the same WebFetch route, `Readme.nmm`): the NMM
(neutron moisture meter) data lives at `nmm_data/tube_1.dat` through
`tube_20.dat` (one file per site, 20 sites, 59 dates each) -- see
"NMM validation target" below.

### What happened

The dataset is real, unrestricted, and hosted at
`https://people.eng.unimelb.edu.au/aww/tarrawarra/` (confirmed via its own
`datapage.html`, which explicitly grants "Permission to use, copy and
distribute this data ... for non-commercial purposes ... without fee").

Early in the session, fetching the documentation pages worked fine
(`Readme.1st`, `Readme.topo`, `Readme.tdr`, `Readme.soil` were all
successfully retrieved -- their content is what `parsers.py`'s format
assumptions are built from). Partway through, every subsequent request --
including to paths that had just succeeded moments earlier -- started
returning an Incapsula/Imperva "Pardon Our Interruption" bot-challenge page,
for both the documentation pages AND the raw data files (`.dem`, `.dat`,
`.tdr`). This matches the same pattern seen with the Overpass API earlier in
the broader trail-conditions project (a public service temporarily
rate-limiting or bot-flagging after a burst of automated requests) --
plain-HTTP and a several-minute cooldown-then-retry did not help, and no
Wayback Machine snapshot of the raw data files exists (only the HTML pages
were ever crawled).

**This is an environment/networking limitation, not a data-availability
problem.** The data is a handful of small ASCII files (the largest listed
is 94 KB) served from a page that works fine in an ordinary browser --
there is no reason to expect this blocks a human downloading it directly.

### How to unblock this

Download these files through a normal web browser (the WAF challenge is
trivially passed by real browser JS execution) from
`https://people.eng.unimelb.edu.au/aww/tarrawarra/`, and place them as:

```
validation/tarrawarra/data/
├── tarrautm.dem              # topodata/tarrautm.dem (UTM coords -- NOT tarrawar.dem)
├── ksat.dat                  # sundry/ksat.dat -- ALREADY PRESENT, fetched live in Session 3
├── particle.dat              # sundry/particle.dat (not required by run_validation.py yet)
├── layer.dat                 # sundry/layer.dat (not required by run_validation.py yet)
├── tdr/
│   ├── sm270995.tdr          # tdr_maps/sm270995.tdr
│   ├── sm140296.tdr
│   ├── sm230296.tdr
│   ├── sm280396.tdr
│   ├── sm130496.tdr
│   ├── sm220496.tdr
│   ├── sm020596.tdr
│   ├── sm030796.tdr
│   ├── sm020996.tdr
│   ├── sm200996.tdr
│   ├── sm251096.tdr
│   ├── sm101196.tdr
│   └── sm291196.tdr
└── nmm/                      # for the denser NMM validation target -- see below
    ├── tube_1.dat            # nmm_data/tube_1.dat
    ├── tube_2.dat
    ├── ...
    └── tube_20.dat           # 20 files total (nmm_data/tube_1.dat .. tube_20.dat)
```

`data/ksat.dat` is already committed (fetched live -- see above); everything
else still needs the manual download.

Then:

```bash
cd validation/tarrawarra
pip install -r ../../services/trail-physics/requirements.txt scipy
python3 run_validation.py --data-dir ./data
```

### If the DEM parser fails

`parsers.py::parse_dem` now tries the **confirmed-real** Tarrawarra header
format first (`north:`/`south:`/`east:`/`west:`/`rows:`/`cols:`, one
"key: value" pair per line, cellsize derived -- see above), falling back to
the originally-guessed (never independently confirmed) ESRI ASCII grid
header. If somehow neither matches, `parse_dem` raises `ValueError` with
the actual header lines printed -- add a new key pattern to
`_TARRAWARRA_HEADER_KEY_PATTERNS` in `parsers.py`. Every other parser (TDR,
ksat, particle, layer) is unambiguous whitespace-delimited columns per the
docs (and `ksat.dat`'s format is now additionally confirmed against a real
downloaded file, see above) and should need no changes.

## NMM validation target (denser, not yet wired into run_validation.py)

Per the GeoWATCH paper (`geowatch.pdf`, Section 4.2.1, read in Session 3):
the Tarrawarra dataset also includes Neutron Moisture Meter (NMM) readings
-- only 20 locations per date (vs. ~508 for TDR, so worse for spatial
structure) but across **59 dates** (vs. 13 for TDR), giving a much denser
temporal check. The paper's published target using this data: GeoWATCH
downscaling reduces RMS error from **0.040 to 0.030 m3/m3**, improving on
**56 of 59 dates (95%)** -- a notably higher improvement rate than the TDR
target's 9/13 (69%).

Format (per `Readme.nmm`, fetched live this session): 20 files,
`tube_1.dat` .. `tube_20.dat`, one per site, each with a header (site ID,
coordinates, depth to bedrock, profile description) followed by repeated
blocks of:

```
date(dd/mm/yyyy)   time(hhmm, AEST)
depth(cm)   moisture(%V/V)
depth(cm)   moisture(%V/V)
...
```

(one blank line between profiles). Site coordinates are in a separate file,
`topodata/neutron.pos`.

`parsers.py::parse_nmm_file` implements this format (tested against a
synthetic fixture; **not yet validated against a real tube_N.dat file**,
still blocked on manual download). **Not yet implemented**: the
`run_validation.py` wiring, which needs a different shape than TDR's
one-file-per-date -- here each "date" means grouping matching date/time
entries across all 20 `tube_N.dat` files (one file per *site*, not per
date). Left for a future session once the base Eq. 1-7 chain is wired and
the TDR target is actually reproducible.

## Attribution

Per the dataset's own copyright notice: any publication using this data
must cite Western, A.W. and Grayson, R.B. (1998), "The Tarrawarra data set:
Soil moisture patterns, soil characteristics and hydrological flux
measurements", *Water Resources Research*, 34(10), 2765-2768.
