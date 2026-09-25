#!/usr/bin/env python3
"""NMM holdout validation -- the FIRST generalization check for Session 8's
Rung 1 pass (Tarrawarra TDR, 13 dates, RMSE 0.0332 vs target 0.0321, 9/13
dates improved).

**Why this exists and how it's different from run_validation.py**: Rung 1
passed against the exact 13 TDR dates where the paper's printed-equation-
vs-published-code discrepancy was found and fixed. That is necessary but
not sufficient evidence the fix is real physics rather than a fit to that
one dataset. Tarrawarra's own Neutron Moisture Meter (NMM) record gives a
second, INDEPENDENT measurement instrument at the SAME catchment across
**59 dates the equation form and k=13 have never been checked against**.

**Integrity discipline, stated up front and enforced by construction, not
just described**: the physics configuration below (equation form, TWI
engine, capping, soil-parameter scale, Stage 2 settings, and k=13 itself)
is HARDCODED to Session 8's exact passing Tarrawarra recipe -- see
FROZEN_CONFIG below. There is deliberately NO command-line way to change
any of these for this script; if you want to explore sensitivity, do that
in run_validation.py against TDR (the dataset the exploration happened
against), not here. This script runs ONE configuration, ONE time, and
reports whatever comes out -- pass or fail -- against pass/fail criteria
that were decided and written into this file BEFORE the script was ever
run against real data (see PRESPECIFIED CRITERIA below). If the result is
a fail, the correct response is to say so plainly, not to loosen the
criteria or adjust the frozen physics config after the fact.

**PRESPECIFIED CRITERIA** (decided with the user before this script
existed -- see chat history / commit log for the timestamp; DO NOT edit
these after seeing a result):
    1. Overall model RMSE must beat the overall site-mean baseline RMSE.
    2. At least 30 of the 59 dates must individually improve on their own
       site-mean baseline (a plain majority -- more lenient than TDR's
       >=9/13 ratio, deliberately, since this is the first-ever check of
       whether the fix generalizes at all, not a recalibration target).

The paper's own published NMM target (Section 4.2.1: RMSE 0.040 -> 0.030
m3/m3, 56/59 dates improved) is reported below for CONTEXT ONLY -- it is
NOT this script's pass/fail gate, both because our per-tube depth
combination methodology (mean of 15cm+30cm, chosen to match TDR's own
~30cm sensing depth -- see below) is a documented choice, not necessarily
identical to whatever the paper's own NMM comparison used, and because
holding ourselves to OUR OWN prespecified, less convenient number is the
whole point of a holdout check.

**Per-tube observed value**: mean of the 15cm and 30cm NMM readings (the
paper's own documented depths, present in essentially every real profile
-- see README.md). This is the best available match to TDR's own "average
soil moisture in the top 30cm" sensing volume, which is what k=13 and the
podpac equation form were checked against in Session 8 -- using a
different depth convention here would silently introduce a scale mismatch
having nothing to do with whether the physics generalizes.

**Date grouping**: unlike TDR (13 dates spread over ~14 months, each a
literal field campaign), NMM's 59 dates are exact matching date strings
across all (up to) 20 tube files -- confirmed directly against the real
data (Session 9): 54 of 59 dates have all 20 tubes, 5 dates have 19 (a
missing tube reading -- real-world variability, not a parsing bug, matches
Session 4/5's tube_20.dat finding). No date-window matching is needed
(unlike TDR's `TDR_DATE_WINDOWS`); every tube's date string for a given
survey day is character-identical.

Usage:
    python3 run_nmm_validation.py --data-dir ./data
"""

from __future__ import annotations

import argparse
import datetime
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from parsers import parse_daily_met_file, parse_neutron_pos_file, parse_nmm_file  # noqa: E402
from physics.redistribution import redistribute_podpac  # noqa: E402
from run_validation import build_podpac_predictors  # noqa: E402
from stage2 import (  # noqa: E402
    TARRAWARRA_ELEVATION_M,
    TARRAWARRA_LAT_DEG,
    TARRAWARRA_LON_DEG,
    apply_stage2,
    coarse_averaged_soil_params,
    compute_daily_ep_mm,
    compute_point_slope_aspect,
    interpolate_soil_params_to_points,
    load_texture_derived_soil_params,
)

NUM_TUBES = 20

# Session 8's exact passing Tarrawarra Rung 1 configuration -- see
# validation/tarrawarra/README.md's Session 8 section for the full
# results matrix this was selected from. NOT re-tuned here; NOT
# configurable from this script's CLI (see module docstring).
FROZEN_CONFIG = dict(
    twi_engine="pydem",
    twi_apply_limits=True,
    soil_params_scale="fine",
    sigma_f=0.6,
    active_layer_depth_mm=300.0,
    eq5_form="geowatch",
)

# Paper's own published NMM target (Section 4.2.1) -- CONTEXT ONLY, not
# this script's gate. See module docstring for why.
PAPER_NMM_BASELINE_RMSE_CONTEXT = 0.040
PAPER_NMM_TARGET_RMSE_CONTEXT = 0.030
PAPER_NMM_DATES_IMPROVED_CONTEXT = "56/59"

# THIS script's actual, prespecified gate (see module docstring).
MIN_DATES_IMPROVED = 30  # of 59


def check_data_available(data_dir: Path) -> None:
    missing = []
    for p in (data_dir / "tarrawar.dem", data_dir / "particle.dat", data_dir / "daily.met", data_dir / "neutron.pos"):
        if not p.exists():
            missing.append(p)
    nmm_dir = data_dir / "nmm"
    tube_files = [nmm_dir / f"tube_{i}.dat" for i in range(1, NUM_TUBES + 1)]
    missing += [p for p in tube_files if not p.exists()]
    if missing:
        print(
            "NMM validation requires the real Tarrawarra dataset (see README.md).\n"
            f"Expected but missing ({len(missing)} file(s)):",
            file=sys.stderr,
        )
        for p in missing[:10]:
            print(f"  {p}", file=sys.stderr)
        if len(missing) > 10:
            print(f"  ... and {len(missing) - 10} more", file=sys.stderr)
        sys.exit(1)


def nmm_date_to_date(date_str: str) -> datetime.date:
    """NMM tube files use 'DD-Mon-YY' (e.g. '20-Sep-95') -- see
    parsers.py::parse_nmm_file's docstring for the doc/reality mismatch
    this confirms. Distinct from daily.met's own 'd/m/yy' format."""
    return datetime.datetime.strptime(date_str, "%d-%b-%y").date()


def observed_value_for_profile(depths_cm: np.ndarray, moisture_pct: np.ndarray) -> tuple[float | None, int]:
    """Mean of the 15cm and 30cm readings -- see module docstring for why
    this specific depth choice. Falls back to whichever of the two is
    present if only one is (real data has at least one profile missing
    the 15cm reading -- see README.md), and returns (None, 0) only if
    NEITHER is present.

    **Real data bug found (Session 9), same discipline as Session 4's
    ksat.dat zero-conductivity exclusion**: tube_16.dat, 20-Mar-97 (the
    single driest date in the whole record -- that date's site-mean
    baseline RMSE, 0.0525, is the highest of all 59) reports a 30cm
    reading of -8.3 %V/V -- physically impossible (volumetric moisture
    cannot be negative). This is a genuine neutron-probe calibration
    artifact at an extreme dry-down, not a parsing bug -- the raw file
    genuinely contains this value. A NEGATIVE reading at either depth is
    excluded from the average (treated exactly like a missing depth, not
    floored to 0 or silently kept), and the caller is told how many times
    this happened so it isn't a silent, invisible correction.

    Returns (observed_value_or_None, num_negative_readings_excluded).
    """
    values = []
    excluded_negative = 0
    for target_depth in (15.0, 30.0):
        matches = np.isclose(depths_cm, target_depth)
        if matches.any():
            value = float(moisture_pct[matches][0])
            if value < 0:
                excluded_negative += 1
                continue
            values.append(value)
    if not values:
        return None, excluded_negative
    return float(np.mean(values)), excluded_negative


def load_all_nmm_readings(data_dir: Path) -> dict[datetime.date, dict[int, float]]:
    """Returns {date: {site: observed_fractional_moisture}}, pooling all
    20 tube files. Fractional (m3/m3), matching TDR's own %V/V -> /100.0
    conversion convention (see run_validation.py's validate_one_date)."""
    readings: dict[datetime.date, dict[int, float]] = {}
    dropped_no_depth_match = 0
    total_negative_excluded = 0
    for site in range(1, NUM_TUBES + 1):
        tube_path = data_dir / "nmm" / f"tube_{site}.dat"
        profiles = parse_nmm_file(str(tube_path), site=site)
        for profile in profiles:
            value_pct, num_negative = observed_value_for_profile(profile.depths_cm, profile.moisture_pct)
            total_negative_excluded += num_negative
            if value_pct is None:
                dropped_no_depth_match += 1
                continue
            date = nmm_date_to_date(profile.date)
            readings.setdefault(date, {})[site] = value_pct / 100.0
    if dropped_no_depth_match:
        print(
            f"NOTE: {dropped_no_depth_match} tube/date profile(s) had neither a "
            f"15cm nor 30cm reading -- excluded from that tube's contribution "
            f"to that date.",
            file=sys.stderr,
        )
    if total_negative_excluded:
        print(
            f"NOTE: excluded {total_negative_excluded} physically-impossible negative "
            f"moisture reading(s) (a real neutron-probe calibration artifact -- see "
            f"observed_value_for_profile's docstring) from the 15/30cm average.",
            file=sys.stderr,
        )
    return readings


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--data-dir", default="./data", help="Directory containing the real Tarrawarra dataset")
    args = parser.parse_args()
    data_dir = Path(args.data_dir)

    check_data_available(data_dir)

    print(
        f"FROZEN configuration (Session 8's passing Tarrawarra recipe, unmodified): {FROZEN_CONFIG}",
        file=sys.stderr,
    )

    site_locations = {s.site: (s.x, s.y) for s in parse_neutron_pos_file(str(data_dir / "neutron.pos"))}
    readings_by_date = load_all_nmm_readings(data_dir)
    print(f"Parsed {len(readings_by_date)} unique NMM survey dates across {NUM_TUBES} tubes.", file=sys.stderr)

    predictors_at, _twi_grid = build_podpac_predictors(
        data_dir / "tarrawar.dem",
        data_dir / "particle.dat",
        twi_engine=FROZEN_CONFIG["twi_engine"],
        twi_apply_limits=FROZEN_CONFIG["twi_apply_limits"],
        soil_params_scale=FROZEN_CONFIG["soil_params_scale"],
    )

    native_grid_dem_path = data_dir / "tarrawar.dem"
    from parsers import parse_dem

    native_grid = parse_dem(str(native_grid_dem_path))
    site_soil_params = load_texture_derived_soil_params(data_dir / "particle.dat")
    coarse_soil_params = coarse_averaged_soil_params(site_soil_params)

    met_records = parse_daily_met_file(str(data_dir / "daily.met"))
    met_by_date = {r.date: r for r in met_records}

    baseline_rmses = []
    model_rmses = []
    dates_improved = 0
    dates_with_stage2 = 0
    dates_missing_ep = 0

    print(f"{'Date':<14} {'n':>3} {'baseline RMSE':>15} {'model RMSE':>12} {'improved?':>10} {'stage2?':>8}")
    for date in sorted(readings_by_date.keys()):
        site_values = readings_by_date[date]
        sites = sorted(site_values.keys())
        observed = np.array([site_values[s] for s in sites])
        xy = np.array([site_locations[s] for s in sites])

        twi_at_points, theta_s_at_points, theta_wilt_at_points = predictors_at(xy)
        valid = ~(np.isnan(twi_at_points) | np.isnan(theta_s_at_points) | np.isnan(theta_wilt_at_points))
        if valid.sum() < len(sites):
            print(
                f"  WARNING: only {valid.sum()}/{len(sites)} sites had valid interpolated "
                f"predictors on {date} (outside DEM/particle.dat coverage)",
                file=sys.stderr,
            )
        observed = observed[valid]
        xy = xy[valid]
        twi_at_points = twi_at_points[valid]
        theta_s_at_points = theta_s_at_points[valid]
        theta_wilt_at_points = theta_wilt_at_points[valid]

        theta_coarse = float(observed.mean())
        # twi_mean: the SAME whole-catchment TWI grid mean used throughout
        # Stage 1's TDR validation -- not recomputed from just this date's
        # (up to 20) sample points, for the same reason run_validation.py's
        # validate_one_date takes an explicit twi_mean rather than the
        # array's own mean (see redistribution.py's test suite).
        twi_mean = float(np.nanmean(_twi_grid))

        predicted = redistribute_podpac(
            theta_coarse=theta_coarse,
            twi=twi_at_points,
            theta_s=theta_s_at_points,
            theta_wilt=theta_wilt_at_points,
            twi_mean=twi_mean,
        )

        ep_mm_day = compute_daily_ep_mm(met_by_date, date, TARRAWARRA_LAT_DEG, TARRAWARRA_ELEVATION_M)
        stage2_applied = False
        if ep_mm_day is not None:
            fine_slope_tan, fine_aspect_deg = compute_point_slope_aspect(
                native_grid.elevation, native_grid.cellsize, native_grid.xllcorner, native_grid.yllcorner, xy
            )
            fine_theta_wilt, fine_theta_ref, fine_theta_s = interpolate_soil_params_to_points(site_soil_params, xy)
            day_of_year = date.timetuple().tm_yday
            predicted = apply_stage2(
                theta_star=predicted,
                theta_ws=theta_coarse,
                fine_slope_tan=fine_slope_tan,
                fine_aspect_deg=fine_aspect_deg,
                fine_theta_wilt=fine_theta_wilt,
                fine_theta_ref=fine_theta_ref,
                fine_theta_s=fine_theta_s,
                coarse_params=coarse_soil_params,
                ep_mm_day=ep_mm_day,
                active_layer_depth_mm=FROZEN_CONFIG["active_layer_depth_mm"],
                sigma_f=FROZEN_CONFIG["sigma_f"],
                day_of_year=day_of_year,
                form=FROZEN_CONFIG["eq5_form"],
                lat_deg=TARRAWARRA_LAT_DEG,
                lon_deg=TARRAWARRA_LON_DEG,
            )
            stage2_applied = True
            dates_with_stage2 += 1
        else:
            dates_missing_ep += 1

        baseline_rmse = float(np.sqrt(np.mean((observed - theta_coarse) ** 2)))
        model_rmse = float(np.sqrt(np.mean((observed - predicted) ** 2)))
        baseline_rmses.append(baseline_rmse)
        model_rmses.append(model_rmse)
        improved = model_rmse < baseline_rmse
        dates_improved += improved
        print(
            f"{date.isoformat():<14} {len(sites):>3} {baseline_rmse:>15.4f} {model_rmse:>12.4f} "
            f"{'yes' if improved else 'no':>10} {'yes' if stage2_applied else 'no':>8}"
        )

    overall_baseline = float(np.sqrt(np.mean(np.array(baseline_rmses) ** 2)))
    overall_model = float(np.sqrt(np.mean(np.array(model_rmses) ** 2)))
    n_dates = len(readings_by_date)

    print()
    print(f"Overall baseline (site-mean) RMSE: {overall_baseline:.4f}")
    print(f"Overall model RMSE:                {overall_model:.4f}")
    print(f"Dates improved: {dates_improved}/{n_dates}  (this script's prespecified gate: >= {MIN_DATES_IMPROVED})")
    print(f"Stage 2 applied on {dates_with_stage2}/{n_dates} dates ({dates_missing_ep} had no usable met data)")
    print()
    print(
        f"(Context only, not our gate -- paper's own published NMM result: "
        f"RMSE {PAPER_NMM_BASELINE_RMSE_CONTEXT} -> {PAPER_NMM_TARGET_RMSE_CONTEXT}, "
        f"{PAPER_NMM_DATES_IMPROVED_CONTEXT} dates improved)"
    )
    print()

    passed = overall_model < overall_baseline and dates_improved >= MIN_DATES_IMPROVED
    if passed:
        print(
            "NMM HOLDOUT: PASS -- Session 8's frozen configuration generalizes to an "
            "independent instrument/59 unseen dates at the same site."
        )
    else:
        print(
            "NMM HOLDOUT: FAIL -- per this script's own prespecified criteria (written "
            "before this was ever run against real data). Report honestly; do not adjust "
            "the frozen physics configuration or loosen these criteria after the fact."
        )
        sys.exit(1)


if __name__ == "__main__":
    main()
