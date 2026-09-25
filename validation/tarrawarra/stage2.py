"""Stage 2 (Eq. 2/7, the flux-difference correction) wiring for the
Tarrawarra validation harness. Session 7. Kept separate from
run_validation.py so this substantial addition is independently testable
and doesn't bloat the main script.

Per the design doc's own discipline ("don't build on top of an unresolved
Rung 1 failure"), Session 4-6 deliberately did NOT build this until Stage
1 alone had been thoroughly diagnosed. This module exists because Session
3's equation transcription showed Stage 2's sign structure systematically
DAMPS Stage 1's anomalies (points wetter than the coarse mean evaporate
faster, pulling them back down; drier points evaporate slower, staying
relatively wetter than they otherwise would) -- and Session 6 left Stage 1
over-correcting by ~3.4x, making "a correctly-scaled Stage 1 combined with
Stage 2's damping" a more coherent hypothesis for the published 0.0321
than either stage in isolation.

Inputs used, each traceable to a specific real Tarrawarra file:
- Ep (potential evapotranspiration): FAO-56 daily Penman-Monteith
  (physics/pet.py), driven by `data/daily.met`.
- sigma_f (vegetation greenness fraction): per Session 6's user decision,
  treated as a spatially-uniform SWEPT scalar (the paper's own Tarrawarra
  input list -- "site average soil moisture, 5-m DEM, and soil texture
  data" -- does NOT include site vegetation data, meaning GeoWATCH used
  its own default global greenness layer here, which is uniform at this
  10.8 ha site's scale; `data/vegetat.dat`'s biomass measurements are NOT
  used, since converting biomass to a greenness fraction would be an
  invented modeling step the paper's own methodology doesn't describe).
- theta_wilt/theta_ref/theta_s (fine, per-point): Noah SOILPARM.TBL via
  USDA texture classification of `data/particle.dat` (physics/soil_texture,
  Session 6) -- reused here for Eq. 4/5's soil-moisture-stress terms, not
  just Eq. 1's ln(Ks) term.
- theta_wilt/theta_ref/theta_s (coarse): simple mean across the same
  texture sample sites -- the paper's own "weather-scale-averaged soil
  properties" for a domain that IS a single weather-scale block
  (Tarrawarra's whole 10.8 ha catchment).
- iota (Eq. 6 solar view factor): computed from the native 5m DEM's
  slope/aspect at each point (physics/radiation.solar_view_factor,
  Session 3/7) -- Session 7 added and verified southern-hemisphere
  correctness (Tarrawarra: 37.65 S) before trusting this for real.
- Active-layer depth for the Ep unit conversion (mm/day -> fraction/day,
  see physics/relaxation.py's module docstring for why this conversion is
  dimensionally REQUIRED, not optional): swept, 300mm primary (TDR's own
  30cm measurement depth), 150/1000mm as sensitivity bounds.
"""

from __future__ import annotations

import datetime
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from scipy.interpolate import griddata

from parsers import parse_daily_met_file, parse_particle_file
from physics.pet import (
    actual_vapor_pressure_from_wetbulb_kpa,
    atmospheric_pressure_kpa,
    daily_reference_et_fao56,
    extraterrestrial_radiation_mj_m2_day,
    net_radiation_daily_mj_m2_day,
)
from physics.radiation import solar_view_factor
from physics.relaxation import SoilProperties, apply_flux_correction, compute_delta_t
from physics.soil_texture import NoahSoilParams, soil_hydraulic_properties
from physics.terrain import compute_aspect, compute_slope

# Site constants, from the paper's own text (Section 4.2.1): "The
# Tarrawarra Catchment is located in southern Victoria, Australia (37 39'
# south, 145 26' east) ... elevation of approximately 100 mAHD."
TARRAWARRA_LAT_DEG = -37.65
TARRAWARRA_LON_DEG = 145.433333
TARRAWARRA_ELEVATION_M = 100.0

# TDR survey date windows, transcribed from Readme.tdr's own "Data
# Availability" table (some surveys span 2 days).
TDR_DATE_WINDOWS: dict[str, tuple[datetime.date, datetime.date]] = {
    "sm270995.tdr": (datetime.date(1995, 9, 25), datetime.date(1995, 9, 27)),
    "sm140296.tdr": (datetime.date(1996, 2, 13), datetime.date(1996, 2, 14)),
    "sm230296.tdr": (datetime.date(1996, 2, 22), datetime.date(1996, 2, 23)),
    "sm280396.tdr": (datetime.date(1996, 3, 28), datetime.date(1996, 3, 28)),
    "sm130496.tdr": (datetime.date(1996, 4, 13), datetime.date(1996, 4, 13)),
    "sm220496.tdr": (datetime.date(1996, 4, 22), datetime.date(1996, 4, 22)),
    "sm020596.tdr": (datetime.date(1996, 5, 2), datetime.date(1996, 5, 3)),
    "sm030796.tdr": (datetime.date(1996, 7, 3), datetime.date(1996, 7, 3)),
    "sm020996.tdr": (datetime.date(1996, 9, 2), datetime.date(1996, 9, 2)),
    "sm200996.tdr": (datetime.date(1996, 9, 20), datetime.date(1996, 9, 20)),
    "sm251096.tdr": (datetime.date(1996, 10, 25), datetime.date(1996, 10, 25)),
    "sm101196.tdr": (datetime.date(1996, 11, 10), datetime.date(1996, 11, 11)),
    "sm291196.tdr": (datetime.date(1996, 11, 29), datetime.date(1996, 11, 29)),
}


def compute_daily_ep_mm(
    met_records_by_date: dict,
    day: datetime.date,
    lat_deg: float = TARRAWARRA_LAT_DEG,
    elevation_m: float = TARRAWARRA_ELEVATION_M,
) -> float | None:
    """FAO-56 daily reference ET (mm/day) for one date. Returns None if
    that day's record is missing or too incomplete (e.g. before the AWS
    was installed -- the first ~2 months of the record have no
    temperature/humidity/radiation data at all)."""
    r = met_records_by_date.get(day)
    if r is None:
        return None
    required = [r.dry_bulb_mean_c, r.dry_bulb_max_c, r.dry_bulb_min_c, r.wet_bulb_mean_c, r.wind_mean_km_hr]
    if any(v is None for v in required):
        return None
    if r.global_rad_kj_m2 is None and r.net_rad_kj_m2 is None:
        return None

    pressure_kpa = atmospheric_pressure_kpa(elevation_m)
    ea = actual_vapor_pressure_from_wetbulb_kpa(
        np.array([r.dry_bulb_mean_c]), np.array([r.wet_bulb_mean_c]), pressure_kpa
    )
    if r.net_rad_kj_m2 is not None:
        rn_mj = np.array([r.net_rad_kj_m2 / 1000.0])
    else:
        doy = day.timetuple().tm_yday
        ra = extraterrestrial_radiation_mj_m2_day(lat_deg, doy)
        rs_mj = np.array([r.global_rad_kj_m2 / 1000.0])
        rn_mj = net_radiation_daily_mj_m2_day(
            rs_mj, np.array([r.dry_bulb_max_c]), np.array([r.dry_bulb_min_c]), ea, ra, elevation_m
        )
    wind_2m_m_s = r.wind_mean_km_hr / 3.6  # km/hr -> m/s; Tarrawarra's anemometer is already at 2m, no height adjustment
    et0 = daily_reference_et_fao56(
        np.array([r.dry_bulb_max_c]), np.array([r.dry_bulb_min_c]), ea, np.array([wind_2m_m_s]), rn_mj, elevation_m
    )
    return float(et0[0])


def compute_survey_mean_ep_mm(
    met_path: Path,
    tdr_filename: str,
    lat_deg: float = TARRAWARRA_LAT_DEG,
    elevation_m: float = TARRAWARRA_ELEVATION_M,
) -> float | None:
    """Mean Ep across a TDR survey's date window (see TDR_DATE_WINDOWS --
    some surveys span 2-3 days). Returns None if no day in the window has
    a usable met record."""
    records = parse_daily_met_file(str(met_path))
    by_date = {r.date: r for r in records}
    start, end = TDR_DATE_WINDOWS[tdr_filename]
    day = start
    values = []
    while day <= end:
        ep = compute_daily_ep_mm(by_date, day, lat_deg, elevation_m)
        if ep is not None:
            values.append(ep)
        day += datetime.timedelta(days=1)
    if not values:
        return None
    return float(np.mean(values))


def load_texture_derived_soil_params(particle_path: Path) -> list[tuple[float, float, NoahSoilParams]]:
    """Like run_validation.load_texture_derived_ksat, but returns the FULL
    NoahSoilParams (theta_wilt/theta_ref/theta_s/Ks) per surface-depth
    sample site, not just Ks -- Stage 1 only ever needed Ks; Stage 2's
    Eq. 4/5 also need theta_wilt/theta_ref/theta_s. Kept here rather than
    generalizing run_validation.py's function, since that one is
    Stage-1-specific call-site-wise and this module is entirely optional
    (Stage 1 works without it)."""
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
        triplets.append((x, y, params))
    if not triplets:
        raise ValueError(f"No surface-depth particle samples found in {particle_path}")
    return triplets


def coarse_averaged_soil_params(site_params: list[tuple[float, float, NoahSoilParams]]) -> NoahSoilParams:
    """Simple mean across all sample sites -- the paper's "weather-scale-
    averaged soil properties" for a domain that IS a single weather-scale
    block (Tarrawarra's whole catchment)."""
    return NoahSoilParams(
        satdk_m_per_s=float(np.mean([p.satdk_m_per_s for _, _, p in site_params])),
        maxsmc=float(np.mean([p.maxsmc for _, _, p in site_params])),
        refsmc=float(np.mean([p.refsmc for _, _, p in site_params])),
        wltsmc=float(np.mean([p.wltsmc for _, _, p in site_params])),
    )


def interpolate_soil_params_to_points(
    site_params: list[tuple[float, float, NoahSoilParams]], points_xy: np.ndarray
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Nearest-neighbor interpolate theta_wilt/theta_ref/theta_s from the
    texture sample sites to arbitrary points. Returns (theta_wilt,
    theta_ref, theta_s) arrays. Uses the SAME nearest-site assignment for
    all three fields (nearest-neighbor depends only on xy proximity, not
    on which value array is being interpolated), so a point's three
    parameters are always self-consistent (all from one real sample site,
    never an inconsistent mix)."""
    site_xy = np.array([(x, y) for x, y, _ in site_params])
    wilt_vals = np.array([p.wltsmc for _, _, p in site_params])
    ref_vals = np.array([p.refsmc for _, _, p in site_params])
    s_vals = np.array([p.maxsmc for _, _, p in site_params])
    theta_wilt = griddata(site_xy, wilt_vals, points_xy, method="nearest")
    theta_ref = griddata(site_xy, ref_vals, points_xy, method="nearest")
    theta_s = griddata(site_xy, s_vals, points_xy, method="nearest")
    return theta_wilt, theta_ref, theta_s


def compute_point_slope_aspect(
    elevation: np.ndarray,
    cellsize: float,
    xllcorner: float,
    yllcorner: float,
    points_xy: np.ndarray,
) -> tuple[np.ndarray, np.ndarray]:
    """Interpolate slope (tan) and aspect (degrees) from the DEM to
    arbitrary points -- the same grid-cell-center/griddata pattern
    run_validation.py's build_terrain_predictors already uses for TWI,
    duplicated here (not imported) because it's a small, self-contained
    piece of coordinate bookkeeping, and this module intentionally has no
    dependency on run_validation.py to stay independently testable.
    Uses the NATIVE (uncoarsened) DEM always -- terrain shading is a
    genuinely different physical quantity from TWI, and should use the
    best-available resolution regardless of whatever coarsening a Stage-1
    --dem-resolution experiment applied."""
    slope_grid = compute_slope(elevation, cellsize)
    aspect_grid = compute_aspect(elevation, cellsize)

    nrows, ncols = elevation.shape
    xs = xllcorner + (np.arange(ncols) + 0.5) * cellsize
    ys_from_north = yllcorner + cellsize * nrows - (np.arange(nrows) + 0.5) * cellsize
    xx, yy = np.meshgrid(xs, ys_from_north)

    slope_tan = griddata((xx.ravel(), yy.ravel()), slope_grid.ravel(), points_xy, method="linear")
    aspect_deg = griddata((xx.ravel(), yy.ravel()), aspect_grid.ravel(), points_xy, method="linear")
    return slope_tan, aspect_deg


@dataclass
class Stage2Config:
    """Bundles everything run_validation.py's per-date loop needs to call
    apply_stage2, computed ONCE outside that loop (soil params, the native
    DEM) or swept by the CLI (sigma_f, active_layer_depth_mm, form)."""

    met_path: Path
    native_elevation: np.ndarray
    native_cellsize: float
    native_xllcorner: float
    native_yllcorner: float
    site_soil_params: list[tuple[float, float, NoahSoilParams]]
    coarse_soil_params: NoahSoilParams
    sigma_f: float
    active_layer_depth_mm: float
    form: str


def apply_stage2(
    theta_star: np.ndarray,
    theta_ws: float,
    fine_slope_tan: np.ndarray,
    fine_aspect_deg: np.ndarray,
    fine_theta_wilt: np.ndarray,
    fine_theta_ref: np.ndarray,
    fine_theta_s: np.ndarray,
    coarse_params: NoahSoilParams,
    ep_mm_day: float,
    active_layer_depth_mm: float,
    sigma_f: float,
    day_of_year: int,
    form: str = "ek2003",
    lat_deg: float = TARRAWARRA_LAT_DEG,
    lon_deg: float = TARRAWARRA_LON_DEG,
) -> np.ndarray:
    """Apply the full Eq. 2/7 flux-difference correction on top of Stage
    1's per-point predictions (`theta_star`).

    ep_mm_day is converted to fraction/day by dividing by
    active_layer_depth_mm -- see physics/relaxation.py's module docstring
    for why this unit conversion is dimensionally required, not optional.
    The SAME (uniform, site-mean) Ep is used for both the fine and coarse
    flux terms -- Tarrawarra has one weather station, not a spatially
    resolved met field; the only per-point spatial variation in the flux
    terms comes through `iota` (terrain shading) and the texture-derived
    soil parameters, not through Ep itself.
    """
    ep_fraction_per_day = ep_mm_day / active_layer_depth_mm

    iota_fine = solar_view_factor(lat_deg, lon_deg, day_of_year, fine_slope_tan, fine_aspect_deg)

    fine_props = SoilProperties(
        theta_wilt=fine_theta_wilt,
        theta_ref=fine_theta_ref,
        theta_s=fine_theta_s,
        green_veg_fraction=sigma_f,
        iota=iota_fine,
    )
    # Coarse iota = 1.0: at weather-scale resolution there's no sub-grid
    # terrain to self-shade against -- the coarse cell's own average slope
    # over a ~10km block is negligible, so "no additional shading beyond
    # flat ground" is the correct coarse-scale default, not an
    # approximation of convenience.
    coarse_props = SoilProperties(
        theta_wilt=coarse_params.wltsmc,
        theta_ref=coarse_params.refsmc,
        theta_s=coarse_params.maxsmc,
        green_veg_fraction=sigma_f,
        iota=1.0,
    )

    delta_t = compute_delta_t(
        theta_star,
        theta_ws,
        fine_theta_ref,
        fine_theta_s,
        ep_fraction_per_day,
        fine_props,
        form=form,
    )
    return apply_flux_correction(
        theta_star,
        theta_ws,
        fine_theta_ref,
        fine_theta_s,
        ep_fraction_per_day,
        fine_props,
        coarse_params.refsmc,
        coarse_params.maxsmc,
        ep_fraction_per_day,
        coarse_props,
        delta_t,
        form=form,
    )


def apply_stage2_for_date(
    theta_star: np.ndarray,
    theta_ws: float,
    points_xy: np.ndarray,
    tdr_filename: str,
    day_of_year: int,
    config: Stage2Config,
) -> tuple[np.ndarray, float] | tuple[None, None]:
    """Full per-date orchestration: compute the survey's mean Ep, interpolate
    per-point slope/aspect and soil parameters, apply Stage 2. Returns
    (corrected_theta, ep_mm_day), or (None, None) if that survey's date
    window has no usable meteorological data (missing/incomplete
    daily.met record -- see compute_survey_mean_ep_mm) so the caller can
    decide how to handle it (e.g. fall back to Stage-1-only for that date,
    with a warning -- this function does not make that policy choice).
    """
    ep_mm_day = compute_survey_mean_ep_mm(config.met_path, tdr_filename)
    if ep_mm_day is None:
        return None, None

    fine_slope_tan, fine_aspect_deg = compute_point_slope_aspect(
        config.native_elevation, config.native_cellsize, config.native_xllcorner, config.native_yllcorner, points_xy
    )
    fine_theta_wilt, fine_theta_ref, fine_theta_s = interpolate_soil_params_to_points(
        config.site_soil_params, points_xy
    )

    corrected = apply_stage2(
        theta_star=theta_star,
        theta_ws=theta_ws,
        fine_slope_tan=fine_slope_tan,
        fine_aspect_deg=fine_aspect_deg,
        fine_theta_wilt=fine_theta_wilt,
        fine_theta_ref=fine_theta_ref,
        fine_theta_s=fine_theta_s,
        coarse_params=config.coarse_soil_params,
        ep_mm_day=ep_mm_day,
        active_layer_depth_mm=config.active_layer_depth_mm,
        sigma_f=config.sigma_f,
        day_of_year=day_of_year,
        form=config.form,
    )
    return corrected, ep_mm_day
