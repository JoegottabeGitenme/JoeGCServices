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
(`parsers.py` + `tests/test_parsers.py`, 9/9 passing against synthetic files
matching the documented formats). **The actual Tarrawarra files could not be
downloaded this session.**

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
├── ksat.dat                  # sundry/ksat.dat
├── particle.dat              # sundry/particle.dat (not required by run_validation.py yet)
├── layer.dat                 # sundry/layer.dat (not required by run_validation.py yet)
└── tdr/
    ├── sm270995.tdr          # tdr_maps/sm270995.tdr
    ├── sm140296.tdr
    ├── sm230296.tdr
    ├── sm280396.tdr
    ├── sm130496.tdr
    ├── sm220496.tdr
    ├── sm020596.tdr
    ├── sm030796.tdr
    ├── sm020996.tdr
    ├── sm200996.tdr
    ├── sm251096.tdr
    ├── sm101196.tdr
    └── sm291196.tdr
```

Then:

```bash
cd validation/tarrawarra
pip install -r ../../services/trail-physics/requirements.txt scipy
python3 run_validation.py --data-dir ./data
```

### If the DEM parser fails

`parsers.py::parse_dem` assumes a standard ESRI ASCII grid header
(`ncols`/`nrows`/`xllcorner`/`yllcorner`/`cellsize`/`NODATA_value`, one
per line) because Readme.topo's description ("a 6 line header with the
boundaries of the dem and the number of rows and columns") is consistent
with that convention but doesn't spell out the exact key names. If the
real file uses different header text, `parse_dem` raises `ValueError` with
the actual header lines printed -- add the missing key pattern to
`_DEM_HEADER_KEY_PATTERNS` in `parsers.py` (a two-line fix) and re-run.
Every other parser (TDR, ksat, particle, layer) is unambiguous
whitespace-delimited columns per the docs and should need no changes.

## Attribution

Per the dataset's own copyright notice: any publication using this data
must cite Western, A.W. and Grayson, R.B. (1998), "The Tarrawarra data set:
Soil moisture patterns, soil characteristics and hydrological flux
measurements", *Water Resources Research*, 34(10), 2765-2768.
