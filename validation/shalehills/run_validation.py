#!/usr/bin/env python3
"""Shale Hills validation -- the SECOND and final generalization check for
Session 8's Rung 1 pass (Tarrawarra TDR: RMSE 0.0332 vs target 0.0321,
9/13 dates; Session 9's NMM holdout: RMSE 0.0295 vs baseline 0.0345,
45/59 dates) -- a fully INDEPENDENT research catchment (different
continent, terrain, soil, climate, land cover) with its own published
target (geowatch.pdf Section 4.2.2: RMSE 0.060 -> 0.054 m3/m3, 55/74
dates improved, citing Naithani & Baldwin 2015).

**Why this is the most important remaining check**: Tarrawarra and NMM
both validate the Session 8 discovery at the SAME site the discovery was
made at -- real evidence of temporal/instrument generalization, but not
evidence the physics transfers to a DIFFERENT catchment's terrain, soil,
and vegetation. Shale Hills is a forested, V-shaped, shale-derived-soil
watershed in Pennsylvania -- about as different from Tarrawarra's grazed-
pasture Australian catchment as two small research watersheds get.

**Integrity discipline, identical to run_nmm_validation.py**: the physics
configuration below (FROZEN_CONFIG) is Session 8's exact passing Tarrawarra
recipe, HARDCODED with no command-line way to change it. k=13 is untouched.
Pass/fail criteria (below) were decided and written into this file BEFORE
it was ever run against the real Shale Hills data.

**PRESPECIFIED CRITERIA** (decided before this script was run against real
data -- DO NOT edit after seeing a result):
    1. Overall model RMSE must beat the overall site-mean baseline RMSE.
    2. At least half of the dates with a usable comparison (a plain
       majority) must individually improve on their own site-mean
       baseline -- the same "first-generalization-check" standard used for
       the NMM holdout (Session 9), not the paper's own 74% ratio (we have
       no license to tune towards matching their specific number).

The paper's own published Shale Hills target (0.060 -> 0.054 m3/m3,
55/74 dates, 74%) is reported for CONTEXT ONLY, not this script's gate,
for the same reason as NMM: our own real-data acquisition and processing
choices (10cm depth as the comparison quantity, dominant-SSURGO-component
texture, FAO-56's own documented 2 m/s default wind speed) are documented,
reasoned choices, not necessarily identical to whatever the paper's own
comparison used.

Usage:
    python3 run_validation.py --data-dir ./data
"""

from __future__ import annotations

import argparse
import datetime
import sys
from pathlib import Path

import numpy as np
from scipy.interpolate import griddata

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from parsers import parse_dem_geotiff, parse_tdr_xlsx  # noqa: E402
from physics.radiation import solar_view_factor  # noqa: E402
from physics.redistribution import redistribute_podpac  # noqa: E402
from physics.relaxation import SoilProperties, apply_flux_correction, compute_delta_t  # noqa: E402
from physics.terrain import compute_twi_pydem  # noqa: E402
from stage2 import (  # noqa: E402
    SHALEHILLS_LAT_DEG,
    SHALEHILLS_LON_DEG,
    coarse_averaged_soil_params,
    compute_daily_ep_mm,
    compute_point_slope_aspect,
    load_all_meteo,
    soil_params_for_points,
)

# Session 8's exact passing Tarrawarra Rung 1 configuration (see
# validation/tarrawarra/README.md's Session 8 section) -- NOT re-tuned
# here, NOT configurable from this script's CLI (see module docstring).
FROZEN_CONFIG = dict(
    twi_apply_limits=True,
    sigma_f=0.6,
    active_layer_depth_mm=300.0,
    eq5_form="geowatch",
)

TDR_DEPTH_CM = 10  # see parsers.py's module docstring for why

# Paper's own published Shale Hills target -- CONTEXT ONLY, not this
# script's gate. See module docstring.
PAPER_BASELINE_RMSE_CONTEXT = 0.060
PAPER_TARGET_RMSE_CONTEXT = 0.054
PAPER_DATES_IMPROVED_CONTEXT = "55/74"

MIN_SITES_PER_DATE = 5  # a date with fewer reporting sites isn't a meaningful spatial comparison


def check_data_available(data_dir: Path) -> None:
    required = [
        data_dir / "shalehills_dem_3m_nad83utm18n.tif",
        data_dir / "SSHCZO_2010TDRSoilMoisture.xlsx",
        data_dir / "ssurgo" / "mapunit.txt",
        data_dir / "ssurgo" / "comp.txt",
        data_dir / "ssurgo" / "chorizon.txt",
        data_dir / "ssurgo" / "soilmu_subset.shp",
        data_dir / "met",
    ]
    missing = [p for p in required if not p.exists()]
    if missing:
        print("Shale Hills data not found. See README.md for how it was acquired.\n", file=sys.stderr)
        for p in missing:
            print(f"  {p}", file=sys.stderr)
        sys.exit(1)


def build_podpac_predictors(dem_path: Path, ssurgo_dir: Path):
    """TWI (pyDEM + apply_twi_limits, Session 8's frozen recipe) + per-point
    theta_s/theta_wilt (from SSURGO map-unit polygons, see stage2.py). No
    `--twi-engine`/`--ks-source`/etc. options here -- unlike
    run_validation.py (Tarrawarra), this harness runs exactly one frozen
    configuration, by design (see module docstring)."""
    dem = parse_dem_geotiff(str(dem_path))
    twi_grid = compute_twi_pydem(dem.elevation, dem.cellsize, apply_twi_limits=FROZEN_CONFIG["twi_apply_limits"])

    nrows, ncols = dem.elevation.shape
    xs = dem.xllcorner + (np.arange(ncols) + 0.5) * dem.cellsize
    ys_from_north = dem.yllcorner + dem.cellsize * nrows - (np.arange(nrows) + 0.5) * dem.cellsize
    xx, yy = np.meshgrid(xs, ys_from_north)

    def predictors_at(points_xy: np.ndarray):
        twi_at_points = griddata((xx.ravel(), yy.ravel()), twi_grid.ravel(), points_xy, method="linear")
        params = soil_params_for_points(
            points_xy,
            ssurgo_dir / "mapunit.txt",
            ssurgo_dir / "comp.txt",
            ssurgo_dir / "chorizon.txt",
            ssurgo_dir / "soilmu_subset.shp",
        )
        theta_s = np.array([p.maxsmc for p in params])
        theta_wilt = np.array([p.wltsmc for p in params])
        return twi_at_points, theta_s, theta_wilt, params

    return predictors_at, twi_grid, dem


def apply_stage2(
    theta_star: np.ndarray,
    theta_ws: float,
    fine_slope_tan: np.ndarray,
    fine_aspect_deg: np.ndarray,
    fine_params,
    coarse_params,
    ep_mm_day: float,
    day_of_year: int,
) -> np.ndarray:
    """Same Eq. 2/7 orchestration as validation/tarrawarra/stage2.py's own
    apply_stage2 -- duplicated (not imported) since the two sites' fine
    soil-param object shapes differ (list[NoahSoilParams] here vs. a
    theta_wilt/theta_ref/theta_s array triple there)."""
    ep_fraction_per_day = ep_mm_day / FROZEN_CONFIG["active_layer_depth_mm"]
    fine_theta_wilt = np.array([p.wltsmc for p in fine_params])
    fine_theta_ref = np.array([p.refsmc for p in fine_params])
    fine_theta_s = np.array([p.maxsmc for p in fine_params])

    iota_fine = solar_view_factor(
        SHALEHILLS_LAT_DEG, SHALEHILLS_LON_DEG, day_of_year, fine_slope_tan, fine_aspect_deg
    )
    fine_props = SoilProperties(
        theta_wilt=fine_theta_wilt,
        theta_ref=fine_theta_ref,
        theta_s=fine_theta_s,
        green_veg_fraction=FROZEN_CONFIG["sigma_f"],
        iota=iota_fine,
    )
    coarse_props = SoilProperties(
        theta_wilt=coarse_params.wltsmc,
        theta_ref=coarse_params.refsmc,
        theta_s=coarse_params.maxsmc,
        green_veg_fraction=FROZEN_CONFIG["sigma_f"],
        iota=1.0,
    )
    delta_t = compute_delta_t(
        theta_star, theta_ws, fine_theta_ref, fine_theta_s, ep_fraction_per_day, fine_props,
        form=FROZEN_CONFIG["eq5_form"],
    )
    return apply_flux_correction(
        theta_star, theta_ws, fine_theta_ref, fine_theta_s, ep_fraction_per_day, fine_props,
        coarse_params.refsmc, coarse_params.maxsmc, ep_fraction_per_day, coarse_props, delta_t,
        form=FROZEN_CONFIG["eq5_form"],
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--data-dir", default="./data", help="Directory containing the real Shale Hills dataset")
    args = parser.parse_args()
    data_dir = Path(args.data_dir)

    check_data_available(data_dir)

    print(f"FROZEN configuration (Session 8's passing Tarrawarra recipe, unmodified): {FROZEN_CONFIG}", file=sys.stderr)

    readings = parse_tdr_xlsx(str(data_dir / "SSHCZO_2010TDRSoilMoisture.xlsx"), depth_cm=TDR_DEPTH_CM)
    by_date: dict[datetime.date, list] = {}
    for r in readings:
        by_date.setdefault(r.date, []).append(r)
    print(f"Parsed {len(by_date)} unique TDR survey dates at {TDR_DEPTH_CM}cm depth.", file=sys.stderr)

    predictors_at, twi_grid, dem = build_podpac_predictors(
        data_dir / "shalehills_dem_3m_nad83utm18n.tif", data_dir / "ssurgo"
    )
    twi_mean = float(np.nanmean(twi_grid))
    station_elevation_m = float(np.nanmean(dem.elevation))

    met_by_date = load_all_meteo(data_dir / "met")

    baseline_rmses = []
    model_rmses = []
    dates_improved = 0
    dates_with_stage2 = 0
    dates_skipped_too_few_sites = 0
    n_dates_compared = 0

    print(f"{'Date':<14} {'n':>3} {'baseline RMSE':>15} {'model RMSE':>12} {'improved?':>10} {'stage2?':>8}")
    for date in sorted(by_date.keys()):
        site_readings = by_date[date]
        if len(site_readings) < MIN_SITES_PER_DATE:
            dates_skipped_too_few_sites += 1
            continue

        observed = np.array([r.moisture_frac for r in site_readings])
        xy = np.array([(r.x, r.y) for r in site_readings])

        twi_at_points, theta_s_at_points, theta_wilt_at_points, fine_params = predictors_at(xy)
        valid = ~(np.isnan(twi_at_points) | np.isnan(theta_s_at_points) | np.isnan(theta_wilt_at_points))
        if valid.sum() < len(site_readings) * 0.5:
            print(f"  WARNING: only {valid.sum()}/{len(site_readings)} valid predictors for {date}", file=sys.stderr)
        observed = observed[valid]
        xy = xy[valid]
        twi_at_points = twi_at_points[valid]
        theta_s_at_points = theta_s_at_points[valid]
        theta_wilt_at_points = theta_wilt_at_points[valid]
        fine_params = [p for p, v in zip(fine_params, valid) if v]
        if len(observed) < MIN_SITES_PER_DATE:
            dates_skipped_too_few_sites += 1
            continue

        theta_coarse = float(observed.mean())
        predicted = redistribute_podpac(
            theta_coarse=theta_coarse,
            twi=twi_at_points,
            theta_s=theta_s_at_points,
            theta_wilt=theta_wilt_at_points,
            twi_mean=twi_mean,
        )

        ep_mm_day = compute_daily_ep_mm(met_by_date, date, elevation_m=station_elevation_m)
        stage2_applied = False
        if ep_mm_day is not None:
            fine_slope_tan, fine_aspect_deg = compute_point_slope_aspect(
                dem.elevation, dem.cellsize, dem.xllcorner, dem.yllcorner, xy
            )
            coarse_params = coarse_averaged_soil_params(fine_params)
            day_of_year = date.timetuple().tm_yday
            predicted = apply_stage2(
                theta_star=predicted,
                theta_ws=theta_coarse,
                fine_slope_tan=fine_slope_tan,
                fine_aspect_deg=fine_aspect_deg,
                fine_params=fine_params,
                coarse_params=coarse_params,
                ep_mm_day=ep_mm_day,
                day_of_year=day_of_year,
            )
            stage2_applied = True
            dates_with_stage2 += 1

        baseline_rmse = float(np.sqrt(np.mean((observed - theta_coarse) ** 2)))
        model_rmse = float(np.sqrt(np.mean((observed - predicted) ** 2)))
        baseline_rmses.append(baseline_rmse)
        model_rmses.append(model_rmse)
        improved = model_rmse < baseline_rmse
        dates_improved += improved
        n_dates_compared += 1
        print(
            f"{date.isoformat():<14} {len(observed):>3} {baseline_rmse:>15.4f} {model_rmse:>12.4f} "
            f"{'yes' if improved else 'no':>10} {'yes' if stage2_applied else 'no':>8}"
        )

    overall_baseline = float(np.sqrt(np.mean(np.array(baseline_rmses) ** 2)))
    overall_model = float(np.sqrt(np.mean(np.array(model_rmses) ** 2)))
    min_dates_improved = (n_dates_compared + 1) // 2  # plain majority, see module docstring

    print()
    print(f"Dates skipped (< {MIN_SITES_PER_DATE} reporting sites): {dates_skipped_too_few_sites}")
    print(f"Dates compared: {n_dates_compared}")
    print(f"Overall baseline (site-mean) RMSE: {overall_baseline:.4f}")
    print(f"Overall model RMSE:                {overall_model:.4f}")
    print(f"Dates improved: {dates_improved}/{n_dates_compared}  (this script's prespecified gate: >= {min_dates_improved})")
    print(f"Stage 2 applied on {dates_with_stage2}/{n_dates_compared} dates")
    print()
    print(
        f"(Context only, not our gate -- paper's own published Shale Hills result: "
        f"RMSE {PAPER_BASELINE_RMSE_CONTEXT} -> {PAPER_TARGET_RMSE_CONTEXT}, "
        f"{PAPER_DATES_IMPROVED_CONTEXT} dates improved)"
    )
    print()

    passed = overall_model < overall_baseline and dates_improved >= min_dates_improved
    if passed:
        print(
            "SHALE HILLS HOLDOUT: PASS -- Session 8's frozen configuration generalizes to a "
            "fully independent catchment (different continent, terrain, soil, vegetation)."
        )
    else:
        print(
            "SHALE HILLS HOLDOUT: FAIL -- per this script's own prespecified criteria (written "
            "before this was ever run against real data). Report honestly; do not adjust the "
            "frozen physics configuration or loosen these criteria after the fact."
        )
        sys.exit(1)


if __name__ == "__main__":
    main()
