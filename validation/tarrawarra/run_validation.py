#!/usr/bin/env python3
"""Rung 1 validation: reproduce the Tarrawarra topographic-redistribution
result (docs/trail-conditions-design.md Section 8).

For each of the 13 TDR sampling dates: interpolate terrain (TWI) and soil
(ln Ks) predictors to each measurement point, redistribute that date's own
catchment-mean measured moisture to each point via Eq. 1
(physics/redistribution.py -- the trail-physics service's fully-specified,
high-confidence module), and compare the redistributed prediction against
the actual TDR reading. Reports per-date and overall RMSE against two
baselines:

  1. The "site mean" baseline (predict every point = that date's catchment
     mean) -- the design doc's target is 0.0352 %V/V here.
  2. The Eq. 1 redistribution -- the design doc's target is RMSE ~= 0.0321
     %V/V, beating the site mean, improving on >= 9 of the 13 dates.

**Session 3 methodology caveat, worth stating plainly**: having now read
the full GeoWATCH paper (`geowatch.pdf`), Section 2.2 describes the model
as "a two-stage approach" -- Eq. 1 (static topographic disaggregation) AND
Eq. 2/7 (flux-based time-relaxation correction) -- and Section 4.2.1's
Tarrawarra description says "GeoWATCH soil moisture downscaling algorithms
were applied" without stating whether the published 0.0321 m3/m3 figure
used stage 1 alone or the full two-stage pipeline. This script currently
validates **stage 1 (Eq. 1) only** -- the fully-specified, unambiguous part
that the design doc's Rung 1 gate is scoped around ("do this before
anything else"). Applying stage 2 here would require Tarrawarra inputs
this session couldn't fetch (daily meteorological data for Ep, vegetation
greenness fraction, and soil-texture-derived theta_wilt/theta_ref/theta_s
via a pedotransfer function the paper doesn't specify for this site) --
`--with-flux-correction` exists as a documented stub for a future session
(see its help text) rather than a fabricated implementation. If Eq. 1
alone doesn't reach 0.0321, that is not necessarily a sign Eq. 1 is wrong
-- it may mean stage 2 is required to close the gap. Don't over-interpret
a near-miss without accounting for this.

**Requires the actual Tarrawarra data files** (see README.md for how to
obtain them -- Session 4 finally got all of them via the site's zip
archives). This script will refuse to run with a clear message if the
data directory is incomplete, rather than silently produce no output.

**Uses `tarrawar.dem`, not `tarrautm.dem`** -- a real bug caught in Session
4. `readme.tdr` states TDR coordinates are in "the Tarrawarra coordinate
system," not UTM (its own words: UTM only applies "for the transect,"
which isn't used here); `ksat.dat`'s own header says the same
("Coordinates: Tarrawarra coordinates"). `tarrawar.dem`'s extent (x:
732.5-1462.5, y: 752.5-1132.5) matches that local coordinate system;
`tarrautm.dem`'s extent (UTM meters, ~362000/5831000) does not overlap it
at all. Using the wrong DEM produces silent, total interpolation failure
(every point falls "outside" the DEM in griddata's eyes) -- the
`valid.sum() < len(records) * 0.5` warning in `validate_one_date` below
is what caught this, not a crash, so watch that warning if this ever
regresses.

**Session 5: `--twi-engine`**. Session 4's diagnosis (implied k~74 vs. the
paper's stated k=13) hypothesized this module's own from-scratch D8 TWI
implementation was numerically incompatible with pyDEM (Ueckermann et al.
2018), the specific tool the paper's Section 2.2 says it used. Directly
testing that hypothesis (`--twi-engine pydem`, requires `pip install
pydem`): pyDEM's D-infinity TWI has similar (not dramatically different)
standard deviation to this module's D8 TWI, and per-date correlation with
real observed anomalies is only modestly better -- switching flow-routing
algorithms alone does NOT close the ~5x gap. See
physics/terrain.py::compute_twi_pydem's docstring and
validation/tarrawarra/README.md for the full comparison. Kept as an option
since it's still the more paper-faithful method and rules out one concrete
hypothesis, even though it isn't sufficient by itself.

Usage:
    python3 run_validation.py --data-dir ./data
    python3 run_validation.py --data-dir ./data --twi-engine pydem
    python3 run_validation.py --data-dir ./data --twi-engine pydem --twi-scaled  # pyDEM's stored (x10) TWI
    python3 run_validation.py --data-dir ./data --with-flux-correction  # not yet implemented, see --help
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
from scipy.interpolate import griddata

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from parsers import parse_dem, parse_ksat_file, parse_tdr_file  # noqa: E402
from physics.redistribution import redistribute  # noqa: E402
from physics.terrain import compute_twi, compute_twi_pydem  # noqa: E402

TDR_FILENAMES = [
    "sm270995.tdr",
    "sm140296.tdr",
    "sm230296.tdr",
    "sm280396.tdr",
    "sm130496.tdr",
    "sm220496.tdr",
    "sm020596.tdr",
    "sm030796.tdr",
    "sm020996.tdr",
    "sm200996.tdr",
    "sm251096.tdr",
    "sm101196.tdr",
    "sm291196.tdr",
]

TARGET_RMSE = 0.0321  # %V/V-scale (design doc, Section 8)
BASELINE_RMSE = 0.0352
TARGET_DATES_IMPROVED = 9  # of 13


def check_data_available(data_dir: Path) -> None:
    dem_path = data_dir / "tarrawar.dem"
    ksat_path = data_dir / "ksat.dat"
    missing = [p for p in (dem_path, ksat_path) if not p.exists()]
    missing += [
        data_dir / "tdr" / f for f in TDR_FILENAMES if not (data_dir / "tdr" / f).exists()
    ]
    if missing:
        print(
            "Tarrawarra data not found. This validation gate requires the "
            "actual dataset, which was blocked this session by the host's "
            "WAF (see README.md in this directory for the full story and "
            "manual-download instructions -- the data itself is unrestricted, "
            "just currently bot-blocked for automated fetches).\n\n"
            f"Expected but missing ({len(missing)} file(s)):",
            file=sys.stderr,
        )
        for p in missing[:10]:
            print(f"  {p}", file=sys.stderr)
        if len(missing) > 10:
            print(f"  ... and {len(missing) - 10} more", file=sys.stderr)
        sys.exit(1)


def build_terrain_predictors(
    dem_path: Path, ksat_path: Path, twi_engine: str = "builtin", twi_scaled: bool = False
):
    """Compute TWI on the DEM grid, then return a function that interpolates
    (TWI, ln(Ks)) to arbitrary (x, y) points -- since TDR measurement points
    don't sit exactly on DEM grid nodes.

    twi_engine: "builtin" (this repo's own D8 implementation,
        physics.terrain.compute_twi) or "pydem" (Ueckermann et al. 2018 --
        the tool the GeoWATCH paper itself used; requires `pip install
        pydem`). See physics/terrain.py::compute_twi_pydem's docstring.
    twi_scaled: only meaningful for twi_engine="pydem" -- use pyDEM's own
        stored (x10) TWI value instead of the plain unscaled ln() value.
    """
    grid = parse_dem(str(dem_path))
    if twi_engine == "builtin":
        twi_grid = compute_twi(grid.elevation, grid.cellsize)
    elif twi_engine == "pydem":
        twi_grid = compute_twi_pydem(grid.elevation, grid.cellsize, scaled=twi_scaled)
    else:
        raise ValueError(f"Unknown twi_engine {twi_engine!r} -- must be 'builtin' or 'pydem'")

    nrows, ncols = grid.elevation.shape
    # Grid cell centers in the DEM's coordinate system. Row 0 = north edge
    # per the documented convention, so y decreases as row index increases.
    xs = grid.xllcorner + (np.arange(ncols) + 0.5) * grid.cellsize
    ys_from_north = grid.yllcorner + grid.cellsize * nrows - (np.arange(nrows) + 0.5) * grid.cellsize
    xx, yy = np.meshgrid(xs, ys_from_north)

    ksat_records = parse_ksat_file(str(ksat_path))
    # The real ksat.dat (Session 4) contains at least one measured 0.0 mm/hr
    # conductivity (an effectively impermeable point -- a real measurement,
    # not obviously a data error). ln(0) = -inf, which would poison not just
    # that single point but the domain-wide log_ks_mean (mean of any array
    # containing -inf is -inf), corrupting Eq. 1's prediction at EVERY
    # location, not just near the zero-conductivity point. Rather than
    # silently substitute an arbitrary floor value, exclude non-positive
    # measurements from both the domain mean and the interpolation source
    # pool, with a loud count of how many were dropped.
    n_before = len(ksat_records)
    ksat_records = [r for r in ksat_records if r.ksat_mm_hr > 0]
    n_dropped = n_before - len(ksat_records)
    if n_dropped:
        print(
            f"NOTE: excluded {n_dropped}/{n_before} ksat.dat record(s) with "
            f"non-positive conductivity (ln(Ks) undefined) from ln(Ks) "
            f"processing -- see build_terrain_predictors' comment.",
            file=sys.stderr,
        )
    ksat_xy = np.array([(r.x, r.y) for r in ksat_records])
    ksat_ln = np.log(np.array([r.ksat_mm_hr for r in ksat_records]))
    # Computed from the SAME filtered ksat_records used for interpolation
    # above -- deliberately not recomputed separately elsewhere, so there is
    # exactly one place that decides how non-positive conductivity is
    # handled, not two independent (and previously inconsistent) ones.
    log_ks_mean = float(np.mean(ksat_ln))

    def predictors_at(points_xy: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
        twi_at_points = griddata(
            (xx.ravel(), yy.ravel()), twi_grid.ravel(), points_xy, method="linear"
        )
        log_ks_at_points = griddata(ksat_xy, ksat_ln, points_xy, method="nearest")
        return twi_at_points, log_ks_at_points

    return predictors_at, twi_grid, log_ks_mean


def validate_one_date(tdr_path: Path, predictors_at, twi_mean: float, log_ks_mean: float):
    records = parse_tdr_file(str(tdr_path))
    xy = np.array([(r.x, r.y) for r in records])
    # TDR files store %V/V (e.g. 39.1 meaning 39.1%); the design doc's
    # published targets (0.0352, 0.0321) are in fractional m3/m3 (the
    # paper's own units), a factor of 100 different. Converting here --
    # not just when reporting the final RMSE -- matters because Eq. 1's
    # k=13 constant is an additive correction on whatever scale theta is
    # expressed in; applying it to %-scale theta while k was calibrated
    # against fractional-scale theta would apply a correction of the wrong
    # relative magnitude, not just report a wrongly-scaled number.
    observed = np.array([r.moisture_pct / 100.0 for r in records])

    twi_at_points, log_ks_at_points = predictors_at(xy)
    valid = ~(np.isnan(twi_at_points) | np.isnan(log_ks_at_points))
    if valid.sum() < len(records) * 0.5:
        print(
            f"  WARNING: only {valid.sum()}/{len(records)} points had valid "
            f"interpolated predictors (likely outside the DEM/ksat coverage) "
            f"for {tdr_path.name}",
            file=sys.stderr,
        )

    observed = observed[valid]
    twi_at_points = twi_at_points[valid]
    log_ks_at_points = log_ks_at_points[valid]

    theta_coarse = float(observed.mean())
    predicted = redistribute(
        theta_coarse=theta_coarse,
        twi=twi_at_points,
        log_ks=log_ks_at_points,
        twi_mean=twi_mean,
        log_ks_mean=log_ks_mean,
    )

    baseline_rmse = float(np.sqrt(np.mean((observed - theta_coarse) ** 2)))
    model_rmse = float(np.sqrt(np.mean((observed - predicted) ** 2)))
    return baseline_rmse, model_rmse, len(observed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--data-dir",
        default="./data",
        help="Directory containing tarrawar.dem, ksat.dat, and a tdr/ subdirectory "
        "with the 13 sm*.tdr files (see README.md for exact manual-download layout)",
    )
    parser.add_argument(
        "--with-flux-correction",
        action="store_true",
        help="Apply Eq. 2/7 (physics/relaxation.py) on top of Eq. 1, per the "
        "paper's full two-stage pipeline (see module docstring for why this "
        "might be what the published 0.0321 target actually represents). "
        "NOT YET IMPLEMENTED -- requires Tarrawarra daily meteorological "
        "data (for Ep), vegetation greenness fraction, and soil-texture- "
        "derived theta_wilt/theta_ref/theta_s (via a pedotransfer function "
        "the paper doesn't specify for this site), none of which are wired "
        "up yet. Passing this flag currently exits with an explanatory error "
        "rather than silently falling back to Eq. 1 only.",
    )
    parser.add_argument(
        "--twi-engine",
        choices=["builtin", "pydem"],
        default="builtin",
        help="TWI computation to use: 'builtin' (this repo's own D8 "
        "implementation) or 'pydem' (Ueckermann et al. 2018 -- the tool "
        "the GeoWATCH paper itself used; requires `pip install pydem`). "
        "See module docstring's Session 5 note for what this test found.",
    )
    parser.add_argument(
        "--twi-scaled",
        action="store_true",
        help="Only meaningful with --twi-engine pydem: use pyDEM's own "
        "stored TWI value (natural-log TWI x10, 'for better integer "
        "resolution when storing' per pyDEM's own docstring) instead of "
        "the plain unscaled ln() value.",
    )
    args = parser.parse_args()
    data_dir = Path(args.data_dir)

    if args.with_flux_correction:
        print(
            "--with-flux-correction is not yet implemented. It requires "
            "additional Tarrawarra inputs not yet integrated into this "
            "harness:\n"
            "  - met_flux/daily.met (daily meteorological data, for Ep)\n"
            "  - sundry/vegetat.dat (vegetation greenness fraction)\n"
            "  - sundry/layer.dat texture classes -> theta_wilt/theta_ref/"
            "theta_s via a pedotransfer function (not specified by the "
            "paper for this site -- a modeling choice a future session "
            "needs to make deliberately, not guess at here)\n"
            "See run_validation.py's module docstring and README.md's "
            "'Session 3 update' section for the full context.",
            file=sys.stderr,
        )
        sys.exit(1)

    check_data_available(data_dir)

    predictors_at, twi_grid, log_ks_mean = build_terrain_predictors(
        data_dir / "tarrawar.dem",
        data_dir / "ksat.dat",
        twi_engine=args.twi_engine,
        twi_scaled=args.twi_scaled,
    )
    twi_mean = float(np.nanmean(twi_grid))
    print(
        f"TWI engine: {args.twi_engine}"
        + (" (scaled x10)" if args.twi_scaled else ""),
        file=sys.stderr,
    )

    baseline_rmses = []
    model_rmses = []
    dates_improved = 0

    print(f"{'Date':<20} {'n':>5} {'baseline RMSE':>15} {'Eq.1 RMSE':>12} {'improved?':>10}")
    for filename in TDR_FILENAMES:
        tdr_path = data_dir / "tdr" / filename
        baseline_rmse, model_rmse, n = validate_one_date(tdr_path, predictors_at, twi_mean, log_ks_mean)
        baseline_rmses.append(baseline_rmse)
        model_rmses.append(model_rmse)
        improved = model_rmse < baseline_rmse
        dates_improved += improved
        print(f"{filename:<20} {n:>5} {baseline_rmse:>15.4f} {model_rmse:>12.4f} {'yes' if improved else 'no':>10}")

    overall_baseline = float(np.sqrt(np.mean(np.array(baseline_rmses) ** 2)))
    overall_model = float(np.sqrt(np.mean(np.array(model_rmses) ** 2)))

    print()
    print(f"Overall baseline (site-mean) RMSE: {overall_baseline:.4f}  (doc target: {BASELINE_RMSE})")
    print(f"Overall Eq. 1 redistribution RMSE: {overall_model:.4f}  (doc target: {TARGET_RMSE})")
    print(f"Dates improved: {dates_improved}/{len(TDR_FILENAMES)}  (doc target: >= {TARGET_DATES_IMPROVED})")
    print()

    passed = (
        overall_model < overall_baseline
        and overall_model <= TARGET_RMSE * 1.1  # 10% tolerance band
        and dates_improved >= TARGET_DATES_IMPROVED
    )
    if passed:
        print("RUNG 1: PASS -- Eq. 1 port reproduces the design doc's target. Proceed to Rung 2/3.")
    else:
        print(
            "RUNG 1: FAIL -- per the design doc: 'If you don't land near this, "
            "stop and debug.' Do not proceed to build more physics on top of "
            "Eq. 1 until this is resolved."
        )
        sys.exit(1)


if __name__ == "__main__":
    main()
