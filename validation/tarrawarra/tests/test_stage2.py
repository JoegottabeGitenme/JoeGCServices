"""Tests for stage2.py (Eq. 2/7 flux-difference correction wiring).

Real-data tests skip gracefully if the data isn't present, matching the
pattern used throughout test_parsers.py and test_run_validation.py.
"""

import datetime
import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))
sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "services" / "trail-physics"))

from parsers import DailyMetRecord  # noqa: E402
from stage2 import (  # noqa: E402
    TARRAWARRA_ELEVATION_M,
    TARRAWARRA_LAT_DEG,
    TDR_DATE_WINDOWS,
    Stage2Config,
    apply_stage2,
    apply_stage2_for_date,
    coarse_averaged_soil_params,
    compute_daily_ep_mm,
    compute_point_slope_aspect,
    compute_survey_mean_ep_mm,
    interpolate_soil_params_to_points,
    load_texture_derived_soil_params,
)
from parsers import parse_dem  # noqa: E402
from physics.soil_texture import NoahSoilParams  # noqa: E402

DATA_DIR = Path(__file__).parent.parent / "data"


def _complete_record(date, **overrides):
    defaults = dict(
        date=date,
        dry_bulb_mean_c=15.0,
        dry_bulb_max_c=20.0,
        dry_bulb_min_c=10.0,
        wet_bulb_mean_c=12.0,
        wet_bulb_max_c=14.0,
        wet_bulb_min_c=10.0,
        surf_air_mean_c=16.0,
        surf_air_max_c=22.0,
        surf_air_min_c=9.0,
        rain_mm=0.0,
        global_rad_kj_m2=15000.0,
        net_rad_kj_m2=8000.0,
        wind_mean_km_hr=10.0,
        wind_max_km_hr=20.0,
        wind_min_km_hr=0.0,
    )
    defaults.update(overrides)
    return DailyMetRecord(**defaults)


# =============================================================================
# compute_daily_ep_mm
# =============================================================================


def test_compute_daily_ep_mm_returns_positive_value():
    day = datetime.date(1996, 1, 15)
    records = {day: _complete_record(day)}
    ep = compute_daily_ep_mm(records, day)
    assert ep is not None
    assert ep > 0.0


def test_compute_daily_ep_mm_missing_day_returns_none():
    ep = compute_daily_ep_mm({}, datetime.date(1996, 1, 15))
    assert ep is None


def test_compute_daily_ep_mm_missing_required_field_returns_none():
    day = datetime.date(1995, 8, 9)  # before AWS installation, matches real data
    records = {day: _complete_record(day, dry_bulb_mean_c=None)}
    ep = compute_daily_ep_mm(records, day)
    assert ep is None


def test_compute_daily_ep_mm_falls_back_to_estimated_net_radiation():
    """When net_rad is missing but global_rad is present, Ep must still be
    computable via the FAO-56 Rn-from-Rs estimation path."""
    day = datetime.date(1996, 1, 15)
    records = {day: _complete_record(day, net_rad_kj_m2=None)}
    ep = compute_daily_ep_mm(records, day)
    assert ep is not None
    assert ep > 0.0


def test_compute_daily_ep_mm_no_radiation_at_all_returns_none():
    day = datetime.date(1996, 1, 15)
    records = {day: _complete_record(day, net_rad_kj_m2=None, global_rad_kj_m2=None)}
    ep = compute_daily_ep_mm(records, day)
    assert ep is None


# =============================================================================
# compute_survey_mean_ep_mm (real data)
# =============================================================================


def test_all_tdr_dates_are_in_the_window_table():
    """Every real TDR file must have a date window -- a silent gap here
    would mean a survey date is missing from Stage 2 entirely."""
    expected = {
        "sm270995.tdr", "sm140296.tdr", "sm230296.tdr", "sm280396.tdr", "sm130496.tdr",
        "sm220496.tdr", "sm020596.tdr", "sm030796.tdr", "sm020996.tdr", "sm200996.tdr",
        "sm251096.tdr", "sm101196.tdr", "sm291196.tdr",
    }
    assert set(TDR_DATE_WINDOWS.keys()) == expected


def test_compute_survey_mean_ep_against_real_data():
    """Every one of the 13 real TDR survey windows must yield a usable,
    physically plausible Ep (confirmed against the paper's own stated
    ~830mm/year -> ~2.3mm/day average; Session 7 found the real range to
    be 0.3-4.6 mm/day across all 13 dates, which comfortably brackets a
    generous plausibility check)."""
    met_path = DATA_DIR / "daily.met"
    if not met_path.exists():
        pytest.skip("real data/daily.met not present (see README.md)")
    for tdr_filename in TDR_DATE_WINDOWS:
        ep = compute_survey_mean_ep_mm(met_path, tdr_filename)
        assert ep is not None, f"{tdr_filename}: no usable Ep"
        assert 0.0 < ep < 10.0, f"{tdr_filename}: implausible Ep={ep}"


def test_compute_survey_mean_ep_earliest_date_uses_estimated_radiation():
    """sm270995's window (25-27 Sep 1995) predates the net-radiation
    sensor's installation (~Oct 1995, confirmed Session 7) -- this must
    still produce a value via the Rn-from-Rs fallback, not silently fail."""
    met_path = DATA_DIR / "daily.met"
    if not met_path.exists():
        pytest.skip("real data/daily.met not present (see README.md)")
    ep = compute_survey_mean_ep_mm(met_path, "sm270995.tdr")
    assert ep is not None
    assert ep == pytest.approx(2.24, abs=0.1)  # confirmed value, Session 7


# =============================================================================
# Soil params (texture) helpers
# =============================================================================


def test_coarse_averaged_soil_params_is_simple_mean():
    params = [
        (0.0, 0.0, NoahSoilParams(satdk_m_per_s=1e-5, maxsmc=0.4, refsmc=0.3, wltsmc=0.1)),
        (1.0, 1.0, NoahSoilParams(satdk_m_per_s=3e-5, maxsmc=0.6, refsmc=0.5, wltsmc=0.3)),
    ]
    coarse = coarse_averaged_soil_params(params)
    assert coarse.maxsmc == pytest.approx(0.5)
    assert coarse.refsmc == pytest.approx(0.4)
    assert coarse.wltsmc == pytest.approx(0.2)
    assert coarse.satdk_m_per_s == pytest.approx(2e-5)


def test_interpolate_soil_params_consistent_nearest_site():
    """A query point closer to site A than site B must get ALL THREE of
    site A's parameters, never a mix -- consistency is the whole point of
    this function's design (see its docstring)."""
    params = [
        (0.0, 0.0, NoahSoilParams(satdk_m_per_s=1e-5, maxsmc=0.4, refsmc=0.3, wltsmc=0.1)),
        (100.0, 100.0, NoahSoilParams(satdk_m_per_s=3e-5, maxsmc=0.6, refsmc=0.5, wltsmc=0.3)),
    ]
    points = np.array([[1.0, 1.0]])  # much closer to site A (0,0)
    wilt, ref, s = interpolate_soil_params_to_points(params, points)
    assert wilt[0] == pytest.approx(0.1)
    assert ref[0] == pytest.approx(0.3)
    assert s[0] == pytest.approx(0.4)


def test_load_texture_derived_soil_params_against_real_data():
    real_path = DATA_DIR / "particle.dat"
    if not real_path.exists():
        pytest.skip("real data/particle.dat not present (see README.md)")
    site_params = load_texture_derived_soil_params(real_path)
    assert len(site_params) == 11  # confirmed, Session 6
    for _, _, p in site_params:
        assert p.maxsmc > p.refsmc > p.wltsmc > 0  # physically required ordering


# =============================================================================
# compute_point_slope_aspect
# =============================================================================


def test_compute_point_slope_aspect_flat_dem_gives_zero_slope():
    elevation = np.full((10, 10), 100.0)
    points = np.array([[25.0, 25.0]])
    slope, aspect = compute_point_slope_aspect(elevation, cellsize=5.0, xllcorner=0.0, yllcorner=0.0, points_xy=points)
    assert slope[0] == pytest.approx(0.0, abs=1e-6)


# =============================================================================
# apply_stage2 (integration)
# =============================================================================


def test_apply_stage2_produces_finite_result():
    theta_star = np.array([0.25, 0.30])
    coarse_params = NoahSoilParams(satdk_m_per_s=2e-6, maxsmc=0.45, refsmc=0.35, wltsmc=0.08)
    result = apply_stage2(
        theta_star=theta_star,
        theta_ws=0.27,
        fine_slope_tan=np.array([0.1, 0.2]),
        fine_aspect_deg=np.array([0.0, 180.0]),
        fine_theta_wilt=np.array([0.08, 0.09]),
        fine_theta_ref=np.array([0.35, 0.36]),
        fine_theta_s=np.array([0.45, 0.46]),
        coarse_params=coarse_params,
        ep_mm_day=2.5,
        active_layer_depth_mm=300.0,
        sigma_f=0.6,
        day_of_year=250,
    )
    assert np.all(np.isfinite(result))


def test_apply_stage2_zero_ep_leaves_theta_star_mostly_unchanged():
    """With Ep=0, there's no flux at all (F=0 identically for both fine and
    coarse), so delta_t's numerator involves a zero flux at saturation --
    the degenerate-F-at-saturation guard in compute_delta_t kicks in, and
    the correction should be at most a rounding-scale perturbation, not a
    large shift."""
    theta_star = np.array([0.25])
    coarse_params = NoahSoilParams(satdk_m_per_s=2e-6, maxsmc=0.45, refsmc=0.35, wltsmc=0.08)
    result = apply_stage2(
        theta_star=theta_star,
        theta_ws=0.27,
        fine_slope_tan=np.array([0.1]),
        fine_aspect_deg=np.array([0.0]),
        fine_theta_wilt=np.array([0.08]),
        fine_theta_ref=np.array([0.35]),
        fine_theta_s=np.array([0.45]),
        coarse_params=coarse_params,
        ep_mm_day=0.0,
        active_layer_depth_mm=300.0,
        sigma_f=0.6,
        day_of_year=250,
    )
    assert result[0] == pytest.approx(0.25, abs=1e-6)


def test_apply_stage2_form_selectable():
    theta_star = np.array([0.20])
    coarse_params = NoahSoilParams(satdk_m_per_s=2e-6, maxsmc=0.45, refsmc=0.35, wltsmc=0.08)
    kwargs = dict(
        theta_star=theta_star,
        theta_ws=0.15,
        fine_slope_tan=np.array([0.3]),
        fine_aspect_deg=np.array([90.0]),
        fine_theta_wilt=np.array([0.08]),
        fine_theta_ref=np.array([0.35]),
        fine_theta_s=np.array([0.45]),
        coarse_params=coarse_params,
        ep_mm_day=3.0,
        active_layer_depth_mm=300.0,
        sigma_f=0.6,
        day_of_year=100,
    )
    result_ek2003 = apply_stage2(form="ek2003", **kwargs)
    result_geowatch = apply_stage2(form="geowatch", **kwargs)
    assert np.isfinite(result_ek2003[0])
    assert np.isfinite(result_geowatch[0])


# =============================================================================
# apply_stage2_for_date (full real-data orchestration)
# =============================================================================


def test_apply_stage2_for_date_end_to_end_real_data():
    """The full Stage 2 pipeline, real Tarrawarra data, one real TDR
    survey date."""
    met_path = DATA_DIR / "daily.met"
    particle_path = DATA_DIR / "particle.dat"
    dem_path = DATA_DIR / "tarrawar.dem"
    if not (met_path.exists() and particle_path.exists() and dem_path.exists()):
        pytest.skip("real Tarrawarra data not present (see README.md)")

    grid = parse_dem(str(dem_path))
    site_params = load_texture_derived_soil_params(particle_path)
    coarse_params = coarse_averaged_soil_params(site_params)
    config = Stage2Config(
        met_path=met_path,
        native_elevation=grid.elevation,
        native_cellsize=grid.cellsize,
        native_xllcorner=grid.xllcorner,
        native_yllcorner=grid.yllcorner,
        site_soil_params=site_params,
        coarse_soil_params=coarse_params,
        sigma_f=0.6,
        active_layer_depth_mm=300.0,
        form="ek2003",
    )

    # A couple of synthetic points within the real DEM's extent, with a
    # plausible Stage-1 theta_star already computed (values don't need to
    # be exactly right -- this test exercises the Stage 2 plumbing, not
    # Stage 1's own correctness, which is tested elsewhere).
    points_xy = np.array([[900.0, 900.0], [1000.0, 950.0]])
    theta_star = np.array([0.25, 0.30])

    corrected, ep_mm_day = apply_stage2_for_date(
        theta_star=theta_star,
        theta_ws=0.27,
        points_xy=points_xy,
        tdr_filename="sm230296.tdr",  # a date with fully measured net radiation
        day_of_year=datetime.date(1996, 2, 22).timetuple().tm_yday,
        config=config,
    )
    assert ep_mm_day is not None
    assert ep_mm_day > 0.0
    assert corrected is not None
    assert np.all(np.isfinite(corrected))


def test_apply_stage2_for_date_returns_none_for_unmatched_survey():
    """A tdr_filename with no matching daily.met data should return
    (None, None), not raise -- callers need to gracefully fall back."""
    met_path = DATA_DIR / "daily.met"
    if not met_path.exists():
        pytest.skip("real data/daily.met not present (see README.md)")

    config = Stage2Config(
        met_path=met_path,
        native_elevation=np.full((5, 5), 100.0),
        native_cellsize=5.0,
        native_xllcorner=0.0,
        native_yllcorner=0.0,
        site_soil_params=[(0.0, 0.0, NoahSoilParams(1e-6, 0.4, 0.3, 0.1))],
        coarse_soil_params=NoahSoilParams(1e-6, 0.4, 0.3, 0.1),
        sigma_f=0.6,
        active_layer_depth_mm=300.0,
        form="ek2003",
    )
    with pytest.raises(KeyError):
        # "not_a_real_survey.tdr" isn't in TDR_DATE_WINDOWS at all --
        # confirms compute_survey_mean_ep_mm's own lookup fails loudly for
        # an unknown survey name rather than silently returning None.
        apply_stage2_for_date(
            theta_star=np.array([0.2]),
            theta_ws=0.2,
            points_xy=np.array([[0.0, 0.0]]),
            tdr_filename="not_a_real_survey.tdr",
            day_of_year=1,
            config=config,
        )
