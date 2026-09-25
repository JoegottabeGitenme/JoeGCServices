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

**Session 8: `--redistribution-form podpac`**. The paper's own Software and
Data Availability section links a Creare/PODPAC notebook the authors
describe as reproducing "the downscaling algorithm" -- and its actual code
is NOT the paper's printed Eq. 1. See physics/redistribution.py's module
docstring (redistribute_podpac) for the full discovery writeup. This is
now the RECOMMENDED form to validate against; `geowatch-paper` (the
default, for backward compatibility with Sessions 4-7's documented
reproduction-attempt history) remains available.

Usage:
    python3 run_validation.py --data-dir ./data
    python3 run_validation.py --data-dir ./data --twi-engine pydem
    python3 run_validation.py --data-dir ./data --twi-engine pydem --twi-scaled  # pyDEM's stored (x10) TWI
    python3 run_validation.py --data-dir ./data --with-flux-correction  # Eq. 2/7 on top of Stage 1
    python3 run_validation.py --data-dir ./data --redistribution-form podpac  # Session 8: the REAL production equation
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
from scipy.interpolate import griddata

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from parsers import parse_dem, parse_ksat_file, parse_particle_file, parse_tdr_file  # noqa: E402
from physics.redistribution import redistribute, redistribute_podpac  # noqa: E402
from physics.soil_texture import soil_hydraulic_properties  # noqa: E402
from physics.terrain import coarsen_dem, compute_twi, compute_twi_pydem  # noqa: E402

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


def check_data_available(data_dir: Path, require_stage2: bool = False) -> None:
    dem_path = data_dir / "tarrawar.dem"
    ksat_path = data_dir / "ksat.dat"
    missing = [p for p in (dem_path, ksat_path) if not p.exists()]
    missing += [
        data_dir / "tdr" / f for f in TDR_FILENAMES if not (data_dir / "tdr" / f).exists()
    ]
    if require_stage2:
        # Stage 2 (--with-flux-correction) additionally needs met data (for
        # Ep) and particle-size data (for texture-derived soil parameters).
        missing += [p for p in (data_dir / "daily.met", data_dir / "particle.dat") if not p.exists()]
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


def load_texture_derived_ksat(particle_path: Path) -> list[tuple[float, float, float]]:
    """Derive Ks per particle.dat sample SITE (x, y) via USDA texture
    classification -> Noah SOILPARM.TBL lookup, instead of using measured
    conductivity (ksat.dat). Session 6 motivation: the paper's own
    Tarrawarra methodology (Section 4.2.1) lists "soil texture data" as an
    input, not measured saturated hydraulic conductivity -- this is a
    direct test of that alternative.

    Each site has 3-4 particle-size samples at different depths (e.g.
    "0-13", "13-24", ">24"); the SHALLOWEST layer (starting at depth 0) is
    used, to match TDR's own measurement depth ("the average soil moisture
    in the top 30 cm," per Readme.tdr) -- deeper, more clay-rich horizons
    at this site (see README.md) are not what TDR is actually sensing.

    Returns (x, y, satdk_m_per_s) triplets, one per site, suitable for the
    same nearest-neighbor interpolation `build_terrain_predictors` already
    uses for measured ksat.dat.
    """
    records = parse_particle_file(str(particle_path))
    surface_by_site: dict[tuple[float, float], object] = {}
    for r in records:
        if not r.depth_range_cm.startswith("0-") and r.depth_range_cm != "0":
            continue
        surface_by_site[(r.x, r.y)] = r

    triplets = []
    for (x, y), r in surface_by_site.items():
        sand = r.coarse_sand_pct + r.fine_sand_pct
        clay = r.clay_pct
        params = soil_hydraulic_properties(sand, clay)
        triplets.append((x, y, params.satdk_m_per_s))
    if not triplets:
        raise ValueError(f"No surface-depth (0-...) particle samples found in {particle_path}")
    return triplets


def build_terrain_predictors(
    dem_path: Path,
    ksat_path: Path,
    twi_engine: str = "builtin",
    twi_scaled: bool = False,
    dem_coarsen_factor: int = 1,
    ks_source: str = "measured",
    particle_path: Path | None = None,
    twi_apply_limits: bool = False,
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
    dem_coarsen_factor: block-average the DEM to a coarser resolution
        before computing TWI (see physics.terrain.coarsen_dem). Session 6
        hypothesis: the paper's k=13 was calibrated against its own 30m
        global elevation composite (Section 2.4), not the 5m site DEM used
        for Tarrawarra's site-specific validation -- factor=6 approximates
        that (5m * 6 = 30m). (This specific hypothesis was tested and
        found NOT to help -- see README.md -- but the machinery is kept.)
    ks_source: "measured" (ksat.dat, the field-measured well-permeameter
        conductivity used since Session 4) or "texture" (derive Ks from
        USDA soil texture classification of particle.dat samples via Noah
        SOILPARM.TBL -- see load_texture_derived_ksat's docstring for why:
        the paper's own Tarrawarra methodology, Section 4.2.1, lists "soil
        texture data" as an input, not measured conductivity).
    particle_path: required if ks_source="texture".
    twi_apply_limits: only meaningful for twi_engine="pydem" -- see
        compute_twi_pydem's docstring (Session 6 finding: shrinks TWI std
        by ~30% on the real Tarrawarra DEM).
    """
    grid = parse_dem(str(dem_path))
    elevation, cellsize = coarsen_dem(grid.elevation, grid.cellsize, dem_coarsen_factor)
    if twi_engine == "builtin":
        twi_grid = compute_twi(elevation, cellsize)
    elif twi_engine == "pydem":
        twi_grid = compute_twi_pydem(
            elevation, cellsize, scaled=twi_scaled, apply_twi_limits=twi_apply_limits
        )
    else:
        raise ValueError(f"Unknown twi_engine {twi_engine!r} -- must be 'builtin' or 'pydem'")

    nrows, ncols = elevation.shape
    # Grid cell centers in the DEM's coordinate system. Row 0 = north edge
    # per the documented convention, so y decreases as row index increases.
    # xllcorner/yllcorner are unaffected by coarsening (block-averaging
    # starts from the same corner); only cellsize and the row/col counts
    # change.
    xs = grid.xllcorner + (np.arange(ncols) + 0.5) * cellsize
    ys_from_north = grid.yllcorner + cellsize * nrows - (np.arange(nrows) + 0.5) * cellsize
    xx, yy = np.meshgrid(xs, ys_from_north)

    if ks_source == "measured":
        ksat_records = parse_ksat_file(str(ksat_path))
        # The real ksat.dat (Session 4) contains at least one measured 0.0
        # mm/hr conductivity (an effectively impermeable point -- a real
        # measurement, not obviously a data error). ln(0) = -inf, which
        # would poison not just that single point but the domain-wide
        # log_ks_mean (mean of any array containing -inf is -inf),
        # corrupting Eq. 1's prediction at EVERY location, not just near
        # the zero-conductivity point. Rather than silently substitute an
        # arbitrary floor value, exclude non-positive measurements from
        # both the domain mean and the interpolation source pool, with a
        # loud count of how many were dropped.
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
        ksat_values = np.array([r.ksat_mm_hr for r in ksat_records])
    elif ks_source == "texture":
        if particle_path is None:
            raise ValueError("particle_path is required when ks_source='texture'")
        triplets = load_texture_derived_ksat(particle_path)
        ksat_xy = np.array([(x, y) for x, y, _ in triplets])
        ksat_values = np.array([satdk for _, _, satdk in triplets])
        # satdk from Noah SOILPARM.TBL is m/s; always > 0 by construction
        # (no zero-conductivity texture class exists), so no filtering
        # needed here, unlike the measured path above.
    else:
        raise ValueError(f"Unknown ks_source {ks_source!r} -- must be 'measured' or 'texture'")

    ksat_ln = np.log(ksat_values)
    # Computed from the SAME filtered/derived values used for interpolation
    # above -- deliberately not recomputed separately elsewhere, so there is
    # exactly one place that decides how conductivity is sourced and
    # filtered, not two independent (and previously inconsistent) ones.
    log_ks_mean = float(np.mean(ksat_ln))

    def predictors_at(points_xy: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
        twi_at_points = griddata(
            (xx.ravel(), yy.ravel()), twi_grid.ravel(), points_xy, method="linear"
        )
        log_ks_at_points = griddata(ksat_xy, ksat_ln, points_xy, method="nearest")
        return twi_at_points, log_ks_at_points

    return predictors_at, twi_grid, log_ks_mean


def build_podpac_predictors(
    dem_path: Path,
    particle_path: Path,
    twi_engine: str = "builtin",
    twi_scaled: bool = False,
    dem_coarsen_factor: int = 1,
    twi_apply_limits: bool = False,
    soil_params_scale: str = "fine",
):
    """Session 8: TWI + theta_s/theta_wilt predictor builder for the REAL
    Creare/GeoWATCH production equation (physics.redistribution.
    redistribute_podpac), discovered in the paper's own linked notebook --
    see that function's docstring. Structurally parallel to
    build_terrain_predictors, but supplies (theta_s, theta_wilt) instead of
    ln(Ks) -- the podpac equation has no conductivity term.

    soil_params_scale: "fine" (nearest-neighbor per-point theta_s/theta_wilt
        from particle.dat's texture sample sites -- matches the notebook's
        porosity/wilt nodes being evaluated at each output coordinate) or
        "coarse" (a single site-wide mean, the alternative reading of what
        a coarse global soil-constants layer would look like at Tarrawarra's
        10.8 ha scale, where it would be effectively uniform regardless).
        Both are legitimate interpretations of the notebook's construction
        (see redistribute_podpac's docstring) -- kept as an explicit,
        documented choice rather than picking one silently.
    """
    from stage2 import (
        coarse_averaged_soil_params,
        interpolate_soil_params_to_points,
        load_texture_derived_soil_params,
    )

    grid = parse_dem(str(dem_path))
    elevation, cellsize = coarsen_dem(grid.elevation, grid.cellsize, dem_coarsen_factor)
    if twi_engine == "builtin":
        twi_grid = compute_twi(elevation, cellsize)
    elif twi_engine == "pydem":
        twi_grid = compute_twi_pydem(
            elevation, cellsize, scaled=twi_scaled, apply_twi_limits=twi_apply_limits
        )
    else:
        raise ValueError(f"Unknown twi_engine {twi_engine!r} -- must be 'builtin' or 'pydem'")

    nrows, ncols = elevation.shape
    xs = grid.xllcorner + (np.arange(ncols) + 0.5) * cellsize
    ys_from_north = grid.yllcorner + cellsize * nrows - (np.arange(nrows) + 0.5) * cellsize
    xx, yy = np.meshgrid(xs, ys_from_north)

    site_params = load_texture_derived_soil_params(particle_path)
    if soil_params_scale == "coarse":
        coarse = coarse_averaged_soil_params(site_params)
        coarse_theta_s = coarse.maxsmc
        coarse_theta_wilt = coarse.wltsmc
    elif soil_params_scale != "fine":
        raise ValueError(
            f"Unknown soil_params_scale {soil_params_scale!r} -- must be 'fine' or 'coarse'"
        )

    def predictors_at(points_xy: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
        twi_at_points = griddata(
            (xx.ravel(), yy.ravel()), twi_grid.ravel(), points_xy, method="linear"
        )
        if soil_params_scale == "coarse":
            theta_s_at_points = np.full(len(points_xy), coarse_theta_s)
            theta_wilt_at_points = np.full(len(points_xy), coarse_theta_wilt)
        else:
            theta_wilt_at_points, _theta_ref, theta_s_at_points = interpolate_soil_params_to_points(
                site_params, points_xy
            )
        return twi_at_points, theta_s_at_points, theta_wilt_at_points

    return predictors_at, twi_grid


def validate_one_date(
    tdr_path: Path,
    predictors_at,
    twi_mean: float,
    log_ks_mean: float | None = None,
    stage2_config=None,
    redistribution_form: str = "geowatch-paper",
):
    """stage2_config: a stage2.Stage2Config, or None (default) for
    Stage 1 (Eq. 1) only. When provided, Stage 2 (Eq. 2/7) is applied on
    top of Stage 1's per-point prediction for every point, using that
    survey's own mean Ep -- unless the survey's date window has no usable
    meteorological data, in which case Stage 2 is skipped for that date
    ONLY (falls back to Stage 1's own prediction) with a loud warning,
    rather than silently applying Stage 1-only for every date without
    saying so.

    redistribution_form: "geowatch-paper" (Eq. 1 as printed, TWI + ln(Ks)
        terms, `predictors_at` returns (twi, log_ks)) or "podpac" (the
        REAL Creare production equation discovered in Session 8 --
        physics.redistribution.redistribute_podpac's docstring --
        `predictors_at` returns (twi, theta_s, theta_wilt) instead).
    """
    records = parse_tdr_file(str(tdr_path))
    xy = np.array([(r.x, r.y) for r in records])
    # TDR files store %V/V (e.g. 39.1 meaning 39.1%); the design doc's
    # published targets (0.0352, 0.0321) are in fractional m3/m3 (the
    # paper's own units), a factor of 100 different. Converting here --
    # not just when reporting the final RMSE -- matters because Eq. 1's
    # k=13 constant is an additive correction on whatever scale theta is
    # expressed in; applying it to %-scale theta while k was calibrated
    # against fractional-scale theta would apply a correction of the wrong
    # relative magnitude, not just report a wrongly-scaled final number.
    observed = np.array([r.moisture_pct / 100.0 for r in records])

    if redistribution_form == "podpac":
        twi_at_points, theta_s_at_points, theta_wilt_at_points = predictors_at(xy)
        valid = ~(
            np.isnan(twi_at_points)
            | np.isnan(theta_s_at_points)
            | np.isnan(theta_wilt_at_points)
        )
    else:
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
    xy = xy[valid]

    theta_coarse = float(observed.mean())

    if redistribution_form == "podpac":
        theta_s_at_points = theta_s_at_points[valid]
        theta_wilt_at_points = theta_wilt_at_points[valid]
        predicted = redistribute_podpac(
            theta_coarse=theta_coarse,
            twi=twi_at_points,
            theta_s=theta_s_at_points,
            theta_wilt=theta_wilt_at_points,
            twi_mean=twi_mean,
        )
    else:
        log_ks_at_points = log_ks_at_points[valid]
        predicted = redistribute(
            theta_coarse=theta_coarse,
            twi=twi_at_points,
            log_ks=log_ks_at_points,
            twi_mean=twi_mean,
            log_ks_mean=log_ks_mean,
        )

    if stage2_config is not None:
        from stage2 import TDR_DATE_WINDOWS, apply_stage2_for_date

        survey_start_date = TDR_DATE_WINDOWS[tdr_path.name][0]
        day_of_year = survey_start_date.timetuple().tm_yday
        corrected, ep_mm_day = apply_stage2_for_date(
            theta_star=predicted,
            theta_ws=theta_coarse,
            points_xy=xy,
            tdr_filename=tdr_path.name,
            day_of_year=day_of_year,
            config=stage2_config,
        )
        if corrected is None:
            print(
                f"  NOTE: {tdr_path.name}: no usable meteorological data for "
                f"Stage 2 (Ep) -- falling back to Stage 1 only for this date.",
                file=sys.stderr,
            )
        else:
            predicted = corrected

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
        help="Apply Eq. 2/7 (physics/relaxation.py, wired via stage2.py) on "
        "top of Eq. 1, per the paper's full two-stage pipeline. Session 7: "
        "implemented for real -- Ep from FAO-56 daily Penman-Monteith "
        "(data/daily.met), theta_wilt/theta_ref/theta_s from USDA texture "
        "classification (data/particle.dat, Noah SOILPARM.TBL), iota from "
        "the native DEM's slope/aspect. sigma_f is a swept uniform scalar "
        "(see --sigma-f) since the paper's own Tarrawarra input list "
        "doesn't include site vegetation data.",
    )
    parser.add_argument(
        "--sigma-f",
        type=float,
        default=0.6,
        help="Vegetation greenness fraction for Stage 2 (Eq. 4/5), treated "
        "as spatially uniform -- see --with-flux-correction's help and "
        "stage2.py's module docstring for why. Default 0.6 (a mid-range "
        "grazed-pasture value); sweep with e.g. 0.4 and 0.8 to check "
        "sensitivity.",
    )
    parser.add_argument(
        "--active-layer-depth-mm",
        type=float,
        default=300.0,
        help="Depth (mm) used to convert Ep from mm/day to fraction/day "
        "for Stage 2 -- see physics/relaxation.py's module docstring for "
        "why this conversion is dimensionally required. Default 300mm "
        "matches TDR's own 30cm measurement depth; try 150 or 1000 for "
        "sensitivity.",
    )
    parser.add_argument(
        "--eq5-form",
        choices=["ek2003", "geowatch"],
        default="ek2003",
        help="Which Eq. 5 (direct soil evaporation) form Stage 2 uses -- "
        "see physics/flux.py's module docstring for the Session 3 finding "
        "that the paper's own printed form ('geowatch') differs from the "
        "well-established Ek et al. (2003) form used by default here.",
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
    parser.add_argument(
        "--dem-resolution",
        type=float,
        default=5.0,
        help="Block-average the DEM to this resolution in meters before "
        "computing TWI (default 5.0 = the site's native resolution, no "
        "coarsening). Must be a multiple of 5.0 (the native cell size), "
        "e.g. 10, 15, or 30. Session 6 hypothesis: the paper's k=13 was "
        "calibrated against its own 30m global elevation composite "
        "(Section 2.4 of the paper), not Tarrawarra's native 5m site DEM.",
    )
    parser.add_argument(
        "--twi-apply-limits",
        action="store_true",
        help="Only meaningful with --twi-engine pydem: enable pyDEM's "
        "apply_twi_limits/apply_twi_limits_on_uca options (off by default "
        "in pyDEM itself). Session 6: caps the upper tail of the TWI "
        "distribution, shrinking its standard deviation by about 30%% on "
        "the real Tarrawarra DEM.",
    )
    parser.add_argument(
        "--ks-source",
        choices=["measured", "texture"],
        default="measured",
        help="Source of saturated hydraulic conductivity: 'measured' "
        "(ksat.dat, field well-permeameter measurements) or 'texture' "
        "(USDA soil texture classification of particle.dat -> Noah "
        "SOILPARM.TBL lookup). Session 6: the paper's own Tarrawarra "
        "methodology (Section 4.2.1) lists 'soil texture data' as an "
        "input, not measured conductivity -- see "
        "load_texture_derived_ksat's docstring. Ignored when "
        "--redistribution-form=podpac (that equation has no ln(Ks) term).",
    )
    parser.add_argument(
        "--redistribution-form",
        choices=["geowatch-paper", "podpac"],
        default="geowatch-paper",
        help="Which Eq. 1 form to use. 'geowatch-paper' (default) is the "
        "form as printed in the paper (TWI + ln(Ks) terms, flat 1/k "
        "amplitude) -- what Sessions 4-7 validated against. 'podpac' is "
        "the REAL Creare production equation, discovered in Session 8 by "
        "reading the paper's own linked reproduction notebook (Software "
        "and Data Availability section): theta = theta_coarse + "
        "(theta_s - theta_wilt)/k * (twi - twi_bar) -- a soil-water-"
        "holding-range-scaled amplitude and NO ln(Ks) term at all. See "
        "physics/redistribution.py's module docstring for the full "
        "discovery writeup and why it also resolves the paper's own "
        "unexplained 'volumetric vs relative soil moisture' sentence "
        "(Section 2.2.1).",
    )
    parser.add_argument(
        "--soil-params-scale",
        choices=["fine", "coarse"],
        default="fine",
        help="Only meaningful with --redistribution-form=podpac: whether "
        "theta_s/theta_wilt are per-point (nearest texture sample site) or "
        "a single site-wide mean. See build_podpac_predictors's docstring.",
    )
    args = parser.parse_args()
    data_dir = Path(args.data_dir)

    if args.redistribution_form == "podpac" and args.ks_source == "texture":
        print(
            "NOTE: --ks-source is ignored with --redistribution-form=podpac "
            "(that equation has no ln(Ks) term).",
            file=sys.stderr,
        )

    check_data_available(data_dir, require_stage2=args.with_flux_correction)

    stage2_config = None
    if args.with_flux_correction:
        from stage2 import Stage2Config, coarse_averaged_soil_params, load_texture_derived_soil_params

        native_grid = parse_dem(str(data_dir / "tarrawar.dem"))
        site_soil_params = load_texture_derived_soil_params(data_dir / "particle.dat")
        coarse_soil_params = coarse_averaged_soil_params(site_soil_params)
        stage2_config = Stage2Config(
            met_path=data_dir / "daily.met",
            native_elevation=native_grid.elevation,
            native_cellsize=native_grid.cellsize,
            native_xllcorner=native_grid.xllcorner,
            native_yllcorner=native_grid.yllcorner,
            site_soil_params=site_soil_params,
            coarse_soil_params=coarse_soil_params,
            sigma_f=args.sigma_f,
            active_layer_depth_mm=args.active_layer_depth_mm,
            form=args.eq5_form,
        )
        print(
            f"Stage 2 (flux correction) ENABLED: sigma_f={args.sigma_f}, "
            f"active_layer_depth={args.active_layer_depth_mm}mm, "
            f"Eq.5 form={args.eq5_form}",
            file=sys.stderr,
        )

    NATIVE_RESOLUTION_M = 5.0
    factor_float = args.dem_resolution / NATIVE_RESOLUTION_M
    dem_coarsen_factor = round(factor_float)
    if dem_coarsen_factor < 1 or abs(factor_float - dem_coarsen_factor) > 1e-6:
        print(
            f"--dem-resolution {args.dem_resolution} must be a positive "
            f"integer multiple of the native {NATIVE_RESOLUTION_M}m cell "
            f"size (e.g. 5, 10, 15, 30).",
            file=sys.stderr,
        )
        sys.exit(1)

    log_ks_mean = None
    if args.redistribution_form == "podpac":
        predictors_at, twi_grid = build_podpac_predictors(
            data_dir / "tarrawar.dem",
            data_dir / "particle.dat",
            twi_engine=args.twi_engine,
            twi_scaled=args.twi_scaled,
            dem_coarsen_factor=dem_coarsen_factor,
            twi_apply_limits=args.twi_apply_limits,
            soil_params_scale=args.soil_params_scale,
        )
        ks_source_note = f"soil params scale: {args.soil_params_scale} (no ln(Ks) term)"
    else:
        predictors_at, twi_grid, log_ks_mean = build_terrain_predictors(
            data_dir / "tarrawar.dem",
            data_dir / "ksat.dat",
            twi_engine=args.twi_engine,
            twi_scaled=args.twi_scaled,
            dem_coarsen_factor=dem_coarsen_factor,
            ks_source=args.ks_source,
            particle_path=data_dir / "particle.dat",
            twi_apply_limits=args.twi_apply_limits,
        )
        ks_source_note = f"Ks source: {args.ks_source}"
    twi_mean = float(np.nanmean(twi_grid))
    twi_valid_count = int(np.sum(~np.isnan(twi_grid)))
    print(
        f"Redistribution form: {args.redistribution_form}, TWI engine: {args.twi_engine}"
        + (" (scaled x10)" if args.twi_scaled else "")
        + f", DEM resolution: {args.dem_resolution}m (factor {dem_coarsen_factor}x), "
        f"{twi_valid_count} valid TWI cells, {ks_source_note}",
        file=sys.stderr,
    )

    baseline_rmses = []
    model_rmses = []
    dates_improved = 0

    print(f"{'Date':<20} {'n':>5} {'baseline RMSE':>15} {'Eq.1 RMSE':>12} {'improved?':>10}")
    for filename in TDR_FILENAMES:
        tdr_path = data_dir / "tdr" / filename
        baseline_rmse, model_rmse, n = validate_one_date(
            tdr_path,
            predictors_at,
            twi_mean,
            log_ks_mean,
            stage2_config=stage2_config,
            redistribution_form=args.redistribution_form,
        )
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
