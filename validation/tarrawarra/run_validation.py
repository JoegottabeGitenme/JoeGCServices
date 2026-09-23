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

**Requires the actual Tarrawarra data files.** `data/ksat.dat` is already
present (fetched live in Session 3); the DEM elevation grid and 13 TDR
files still need a manual browser download -- see README.md for exactly
why (an intermittent WAF) and how to supply them. This script will refuse
to run with a clear message if the data directory is incomplete, rather
than silently produce no output.

Usage:
    python3 run_validation.py --data-dir ./data
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
from physics.terrain import compute_twi  # noqa: E402

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
    dem_path = data_dir / "tarrautm.dem"
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


def build_terrain_predictors(dem_path: Path, ksat_path: Path):
    """Compute TWI on the DEM grid, then return a function that interpolates
    (TWI, ln(Ks)) to arbitrary (x, y) points -- since TDR measurement points
    don't sit exactly on DEM grid nodes."""
    grid = parse_dem(str(dem_path))
    twi_grid = compute_twi(grid.elevation, grid.cellsize)

    nrows, ncols = grid.elevation.shape
    # Grid cell centers in the DEM's coordinate system. Row 0 = north edge
    # per the documented convention, so y decreases as row index increases.
    xs = grid.xllcorner + (np.arange(ncols) + 0.5) * grid.cellsize
    ys_from_north = grid.yllcorner + grid.cellsize * nrows - (np.arange(nrows) + 0.5) * grid.cellsize
    xx, yy = np.meshgrid(xs, ys_from_north)

    ksat_records = parse_ksat_file(str(ksat_path))
    ksat_xy = np.array([(r.x, r.y) for r in ksat_records])
    ksat_ln = np.log(np.array([r.ksat_mm_hr for r in ksat_records]))

    def predictors_at(points_xy: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
        twi_at_points = griddata(
            (xx.ravel(), yy.ravel()), twi_grid.ravel(), points_xy, method="linear"
        )
        log_ks_at_points = griddata(ksat_xy, ksat_ln, points_xy, method="nearest")
        return twi_at_points, log_ks_at_points

    return predictors_at, twi_grid


def validate_one_date(tdr_path: Path, predictors_at, twi_mean: float, log_ks_mean: float):
    records = parse_tdr_file(str(tdr_path))
    xy = np.array([(r.x, r.y) for r in records])
    observed = np.array([r.moisture_pct for r in records])

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
        help="Directory containing tarrautm.dem, ksat.dat, and a tdr/ subdirectory "
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

    predictors_at, twi_grid = build_terrain_predictors(data_dir / "tarrautm.dem", data_dir / "ksat.dat")
    twi_mean = float(np.nanmean(twi_grid))

    ksat_records = parse_ksat_file(str(data_dir / "ksat.dat"))
    log_ks_mean = float(np.mean(np.log([r.ksat_mm_hr for r in ksat_records])))

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
