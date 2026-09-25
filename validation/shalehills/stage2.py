"""Stage 2 (Eq. 2/7, the flux-difference correction) wiring for the Shale
Hills validation harness -- Session 10. Structurally parallel to
`validation/tarrawarra/stage2.py`, but adapted to Shale Hills' real data
shapes:

- Meteorology is real Campbell Scientific TOA5 10-minute flux-tower data
  (RH + directly-measured net radiation), not Tarrawarra's own daily.met
  (wet/dry-bulb psychrometer, with an Rn-from-Rs fallback for gaps) -- see
  `parsers.py::parse_meteo_file` and `physics/pet.py::
  actual_vapor_pressure_from_rh_kpa` (new, Session 10).
- Soil parameters come from SSURGO map-unit POLYGONS (a point falls inside
  a mapped area) rather than Tarrawarra's discrete particle.dat SAMPLE
  SITES (nearest-neighbor to a point) -- see `parsers.py::
  assign_mukey_to_points` / `dominant_component_texture_by_mukey`.
- No vegetation data input is used here either, for the same reason as
  Tarrawarra/NMM: sigma_f is kept a swept uniform scalar, FROZEN at
  Session 8's value, not re-derived from this site's own Leaf Area Index
  data (which does exist in the source TDR workbook's 'Leaf Area Index'
  sheet) -- introducing a new vegetation methodology here would confound
  whether a pass/fail reflects the frozen physics generalizing, or a
  different methodology that happens to also work.
"""

from __future__ import annotations

import datetime
from pathlib import Path

import numpy as np

from parsers import (
    DEFAULT_WIND_SPEED_M_S,
    MeteoRecord,
    assign_mukey_to_points,
    dominant_component_texture_by_mukey,
    parse_meteo_file,
    parse_ssurgo_mapunit_polygons,
)
from physics.pet import (
    actual_vapor_pressure_from_rh_kpa,
    atmospheric_pressure_kpa,
    daily_reference_et_fao56,
)
from physics.soil_texture import NoahSoilParams, soil_hydraulic_properties
from physics.terrain import compute_aspect, compute_slope
from scipy.interpolate import griddata

# Site constants. Paper's own text (Section 4.2.2): "central Pennsylvania,
# USA" -- HydroShare's own stated coverage bbox (Session 10) gives the
# precise center; elevation is this project's own DEM's mean (not an
# external guess -- the reprojected DEM is the authoritative source).
SHALEHILLS_LAT_DEG = 40.6647
SHALEHILLS_LON_DEG = -77.9045


def load_all_meteo(met_dir: Path) -> dict[datetime.date, list[MeteoRecord]]:
    """Parse every real 20XX_CZO_FluxTowerMeteo.dat file in met_dir and
    group all 10-minute records by calendar date."""
    by_date: dict[datetime.date, list[MeteoRecord]] = {}
    for path in sorted(Path(met_dir).glob("*_CZO_FluxTowerMeteo.dat")):
        for r in parse_meteo_file(str(path)):
            by_date.setdefault(r.timestamp.date(), []).append(r)
    return by_date


def compute_daily_ep_mm(
    met_by_date: dict[datetime.date, list[MeteoRecord]],
    day: datetime.date,
    elevation_m: float,
    lat_deg: float = SHALEHILLS_LAT_DEG,
) -> float | None:
    """FAO-56 daily reference ET (mm/day) for one date, from real 10-minute
    flux-tower records aggregated to daily Tmax/Tmin/RHmax/RHmin and a
    directly-measured daily net radiation total (no Rs-based fallback
    needed here, unlike Tarrawarra -- Shale Hills' net radiometer has no
    documented outage). Returns None if the day is missing entirely or has
    too few valid scans to trust (fewer than 6 of the expected 144
    ten-minute scans -- a generous floor, not a tight one)."""
    records = met_by_date.get(day)
    if not records:
        return None

    temps = [r.temperature_c for r in records if r.temperature_c is not None]
    rhs = [r.relative_humidity_pct for r in records if r.relative_humidity_pct is not None]
    rns = [r.net_radiation_w_m2 for r in records if r.net_radiation_w_m2 is not None]
    if len(temps) < 6 or len(rhs) < 6 or len(rns) < 6:
        return None

    tmax_c, tmin_c = max(temps), min(temps)
    rh_max, rh_min = max(rhs), min(rhs)
    # Daily mean net radiation (W/m2) -> MJ/m2/day: 1 W/m2 = 0.0864
    # MJ/m2/day (FAO-56 Table 3, Chapter 3 -- exact conversion factor, not
    # rounded here).
    rn_mj_m2_day = float(np.mean(rns)) * 0.0864

    ea = actual_vapor_pressure_from_rh_kpa(
        tmax_c=np.array([tmax_c]),
        tmin_c=np.array([tmin_c]),
        rh_max_pct=np.array([rh_max]),
        rh_min_pct=np.array([rh_min]),
    )
    et0 = daily_reference_et_fao56(
        tmax_c=np.array([tmax_c]),
        tmin_c=np.array([tmin_c]),
        ea_kpa=ea,
        wind_2m_m_s=np.array([DEFAULT_WIND_SPEED_M_S]),
        net_radiation_mj_m2_day=np.array([rn_mj_m2_day]),
        elevation_m=elevation_m,
    )
    return float(et0[0])


def soil_params_for_points(
    points_xy: np.ndarray, mapunit_path: Path, comp_path: Path, chorizon_path: Path, shp_path: Path
) -> list[NoahSoilParams]:
    """Per-point NoahSoilParams via: which SSURGO map-unit polygon the
    point falls in -> that map unit's dominant component's shallowest
    horizon's (sand%, clay%) -> the SAME USDA-triangle -> Noah SOILPARM.TBL
    pipeline Tarrawarra/NMM used (physics.soil_texture), not a different
    methodology."""
    texture_by_mukey = dominant_component_texture_by_mukey(str(mapunit_path), str(comp_path), str(chorizon_path))
    polygons = parse_ssurgo_mapunit_polygons(str(shp_path))
    mukeys = assign_mukey_to_points(polygons, points_xy)

    result = []
    for mukey in mukeys:
        if mukey not in texture_by_mukey:
            raise ValueError(
                f"No texture data for mukey {mukey!r} -- check chorizon.txt has a valid "
                f"shallowest-horizon sand%/clay% for this map unit's dominant component."
            )
        sand_pct, clay_pct = texture_by_mukey[mukey]
        result.append(soil_hydraulic_properties(sand_pct, clay_pct))
    return result


def coarse_averaged_soil_params(site_params: list[NoahSoilParams]) -> NoahSoilParams:
    """Simple mean across all per-point soil params for a given date's
    active sites -- the same "weather-scale-averaged soil properties"
    convention Tarrawarra used (there, an unweighted mean across texture
    sample sites; here, an unweighted mean across whichever map units this
    date's reporting sites happen to fall in)."""
    return NoahSoilParams(
        satdk_m_per_s=float(np.mean([p.satdk_m_per_s for p in site_params])),
        maxsmc=float(np.mean([p.maxsmc for p in site_params])),
        refsmc=float(np.mean([p.refsmc for p in site_params])),
        wltsmc=float(np.mean([p.wltsmc for p in site_params])),
    )


def compute_point_slope_aspect(
    elevation: np.ndarray,
    cellsize: float,
    xllcorner: float,
    yllcorner: float,
    points_xy: np.ndarray,
) -> tuple[np.ndarray, np.ndarray]:
    """Interpolate slope (tan) and aspect (degrees) from the DEM to
    arbitrary points -- identical pattern to
    validation/tarrawarra/stage2.py's own function of the same name
    (duplicated, not imported, to keep this module independently
    testable and free of a dependency on Tarrawarra's own module)."""
    slope_grid = compute_slope(elevation, cellsize)
    aspect_grid = compute_aspect(elevation, cellsize)

    nrows, ncols = elevation.shape
    xs = xllcorner + (np.arange(ncols) + 0.5) * cellsize
    ys_from_north = yllcorner + cellsize * nrows - (np.arange(nrows) + 0.5) * cellsize
    xx, yy = np.meshgrid(xs, ys_from_north)

    slope_tan = griddata((xx.ravel(), yy.ravel()), slope_grid.ravel(), points_xy, method="linear")
    aspect_deg = griddata((xx.ravel(), yy.ravel()), aspect_grid.ravel(), points_xy, method="linear")
    return slope_tan, aspect_deg
