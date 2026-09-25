"""Unit tests for pet.py (FAO-56 Penman-Monteith).

Reference values are well-known standalone constants from FAO-56 (Allen et
al. 1998) -- es(20degC), Delta(20degC), gamma at sea level, the 10m->2m wind
reduction factor -- not a full worked-example cross-check. Combined with
physical sanity properties (monotonicity, non-negativity, sign of net
radiation day vs. night) for the parts not independently tabulated.
"""

import numpy as np
import pytest

from physics.pet import (
    actual_vapor_pressure_from_rh_kpa,
    actual_vapor_pressure_from_wetbulb_kpa,
    atmospheric_pressure_kpa,
    clear_sky_radiation_mj_m2_day,
    daily_reference_et_fao56,
    extraterrestrial_radiation_mj_m2_day,
    hourly_reference_et_mm,
    net_radiation_daily_mj_m2_day,
    net_radiation_w_m2,
    psychrometric_constant_kpa_per_c,
    saturation_vapor_pressure_kpa,
    vapor_pressure_slope_kpa_per_c,
    wind_speed_2m,
)


def test_saturation_vapor_pressure_at_20c():
    """FAO-56 Table 2.3/worked examples: es(20degC) ~= 2.338 kPa."""
    es = saturation_vapor_pressure_kpa(np.array([293.15]))  # 20 degC
    assert es[0] == pytest.approx(2.338, abs=0.01)


def test_saturation_vapor_pressure_increases_with_temperature():
    temps_k = np.array([273.15, 283.15, 293.15, 303.15])
    es = saturation_vapor_pressure_kpa(temps_k)
    assert np.all(np.diff(es) > 0)


def test_slope_matches_fao56_example_at_20c():
    """FAO-56 Box 6 worked example: Delta(20degC) ~= 0.145 kPa/degC."""
    delta = vapor_pressure_slope_kpa_per_c(np.array([293.15]))
    assert delta[0] == pytest.approx(0.145, abs=0.005)


def test_psychrometric_constant_at_sea_level():
    """FAO-56: gamma ~= 0.0665 kPa/degC at standard sea-level pressure (101.3 kPa)."""
    gamma = psychrometric_constant_kpa_per_c(np.array([101300.0]))
    assert gamma[0] == pytest.approx(0.0665, abs=0.001)


def test_wind_speed_2m_reduces_10m_value():
    """The log-wind-profile factor (4.87/ln(67.8*10-5.42)) is < 1, so 2m
    wind must always be lower than the 10m input for positive wind speeds."""
    u10 = np.array([5.0, 10.0])
    u2 = wind_speed_2m(u10)
    assert np.all(u2 < u10)
    # The reduction factor itself is a known constant (~0.748)
    np.testing.assert_allclose(u2 / u10, 0.7482, atol=0.001)


def test_net_radiation_positive_under_typical_daytime_conditions():
    dswrf = np.array([700.0])  # W/m^2, midday clear-sky-ish
    dlwrf = np.array([320.0])  # W/m^2, typical
    surface_temp_k = np.array([293.15])  # 20 degC
    rn = net_radiation_w_m2(dswrf, dlwrf, surface_temp_k)
    assert rn[0] > 0


def test_net_radiation_can_go_negative_at_night():
    """No shortwave input plus radiative cooling should give net negative
    radiation (the classic nighttime longwave-loss regime)."""
    dswrf = np.array([0.0])
    dlwrf = np.array([250.0])  # cool, clear night sky
    surface_temp_k = np.array([283.15])  # 10 degC
    rn = net_radiation_w_m2(dswrf, dlwrf, surface_temp_k)
    assert rn[0] < 0


def test_hourly_et_is_nonnegative():
    et0 = hourly_reference_et_mm(
        temp_k=np.array([293.15]),
        spfh=np.array([0.010]),
        pressure_pa=np.array([101300.0]),
        wind_speed_10m=np.array([3.0]),
        net_radiation_w_m2_=np.array([-50.0]),  # nighttime, net radiative loss
    )
    assert et0[0] >= 0.0


def test_hourly_et_increases_with_net_radiation():
    """More energy available (higher Rn) at fixed T/humidity/wind must
    increase potential ET -- basic energy-balance sanity check."""
    kwargs = dict(
        temp_k=np.array([293.15]),
        spfh=np.array([0.010]),
        pressure_pa=np.array([101300.0]),
        wind_speed_10m=np.array([3.0]),
    )
    et_low_rn = hourly_reference_et_mm(net_radiation_w_m2_=np.array([100.0]), **kwargs)
    et_high_rn = hourly_reference_et_mm(net_radiation_w_m2_=np.array([600.0]), **kwargs)
    assert et_high_rn[0] > et_low_rn[0]


def test_hourly_et_increases_with_wind_speed_under_dry_air():
    """More wind -> more aerodynamic vapor transport -> higher ET, but ONLY
    when there's a real vapor pressure deficit to exploit (dry air). This
    is a genuine, textbook property of Penman-Monteith, not a corner case:
    the wind term appears in BOTH the numerator (scaled by es-ea, the vapor
    deficit) and the denominator (a pure resistance term). Under
    near-saturated air (small es-ea) the denominator's wind penalty can
    outweigh the numerator's small vapor-deficit-driven gain, and ETo can
    actually *decrease* with wind -- discovered while writing this test
    (see test_hourly_et_can_decrease_with_wind_under_humid_air below), not
    assumed away. Uses spfh=0.003 (dry air, large deficit) specifically to
    isolate the "textbook" regime this test's name describes.
    """
    kwargs = dict(
        temp_k=np.array([293.15]),
        spfh=np.array([0.003]),  # dry air -> large es-ea vapor deficit
        pressure_pa=np.array([101300.0]),
        net_radiation_w_m2_=np.array([400.0]),
    )
    et_calm = hourly_reference_et_mm(wind_speed_10m=np.array([1.0]), **kwargs)
    et_windy = hourly_reference_et_mm(wind_speed_10m=np.array([10.0]), **kwargs)
    assert et_windy[0] > et_calm[0]


def test_hourly_et_can_decrease_with_wind_under_humid_air():
    """The flip side of the property above: under near-saturated air (small
    vapor deficit), the denominator's pure aerodynamic-resistance term can
    dominate over the numerator's small vapor-deficit-driven gain, and ETo
    *decreases* with wind. This is correct Penman-Monteith behavior (not a
    bug) -- asserted explicitly so a future change to the formula that
    "fixes" this into blanket monotonicity gets caught as a regression."""
    kwargs = dict(
        temp_k=np.array([293.15]),
        spfh=np.array([0.010]),  # more humid air -> smaller vapor deficit
        pressure_pa=np.array([101300.0]),
        net_radiation_w_m2_=np.array([400.0]),
    )
    et_calm = hourly_reference_et_mm(wind_speed_10m=np.array([1.0]), **kwargs)
    et_windy = hourly_reference_et_mm(wind_speed_10m=np.array([10.0]), **kwargs)
    assert et_windy[0] < et_calm[0]


# =============================================================================
# Session 7: daily-timestep FAO-56 functions, tested against the primary
# source's OWN fully worked numerical examples (fetched live from
# fao.org/4/x0490e/, not from memory) -- these are golden reference values
# from the standard itself, not independently-derived expectations.
# =============================================================================


def test_atmospheric_pressure_matches_fao56_example_2():
    """FAO-56 Example 2: z=1800m -> P=81.8 kPa."""
    assert atmospheric_pressure_kpa(1800.0) == pytest.approx(81.8, abs=0.05)


def test_atmospheric_pressure_matches_fao56_example_18():
    """FAO-56 Example 18 (Brussels): z=100m -> P=100.1 kPa."""
    assert atmospheric_pressure_kpa(100.0) == pytest.approx(100.1, abs=0.05)


def test_actual_vapor_pressure_from_wetbulb_matches_fao56_example_4():
    """FAO-56 Example 4: Tdry=25.6, Twet=19.5, z=1200m -> ea=1.91 kPa
    (via P=87.9 kPa and the ventilated-psychrometer coefficient)."""
    pressure_kpa = atmospheric_pressure_kpa(1200.0)
    assert pressure_kpa == pytest.approx(87.9, abs=0.05)
    ea = actual_vapor_pressure_from_wetbulb_kpa(
        dry_bulb_c=np.array([25.6]), wet_bulb_c=np.array([19.5]), pressure_kpa=pressure_kpa
    )
    assert ea[0] == pytest.approx(1.91, abs=0.01)


def test_actual_vapor_pressure_from_rh_matches_fao56_example_5():
    """FAO-56 Example 5: Tmin=18C & RHmax=82%, Tmax=25C & RHmin=54% ->
    ea=1.70 kPa (Session 10, added for Shale Hills' flux tower, which
    reports RH directly rather than a wet/dry-bulb pair)."""
    ea = actual_vapor_pressure_from_rh_kpa(
        tmax_c=np.array([25.0]),
        tmin_c=np.array([18.0]),
        rh_max_pct=np.array([82.0]),
        rh_min_pct=np.array([54.0]),
    )
    assert ea[0] == pytest.approx(1.70, abs=0.01)


def test_extraterrestrial_radiation_matches_fao56_example_8_southern_hemisphere():
    """FAO-56 Example 8: 3 September at 20 S -> Ra=32.2 MJ/m2/day. This is
    also the test that matters most for Tarrawarra (37.65 S) -- confirms
    the southern-hemisphere sign convention (negative latitude) is handled
    correctly, not just the northern-hemisphere case."""
    ra = extraterrestrial_radiation_mj_m2_day(lat_deg=-20.0, day_of_year=246)
    assert ra == pytest.approx(32.2, abs=0.1)


def test_extraterrestrial_radiation_matches_fao56_example_18_northern_hemisphere():
    """FAO-56 Example 18 (Brussels): 6 July at 50.8 N -> Ra=41.09 MJ/m2/day."""
    ra = extraterrestrial_radiation_mj_m2_day(lat_deg=50.8, day_of_year=187)
    assert ra == pytest.approx(41.09, abs=0.1)


def test_clear_sky_radiation_matches_fao56_example_18():
    """FAO-56 Example 18: Ra=41.09, z=100m -> Rso=30.90 MJ/m2/day."""
    rso = clear_sky_radiation_mj_m2_day(ra_mj_m2_day=41.09, elevation_m=100.0)
    assert rso == pytest.approx(30.90, abs=0.05)


def test_net_radiation_daily_matches_fao56_example_18():
    """FAO-56 Example 18 (Brussels, full Rn-from-Rs pipeline): Rs=22.07,
    Tmax=21.5, Tmin=12.3, ea=1.409, Ra=41.09, z=100m -> Rn=13.28 MJ/m2/day
    (via Rns=17.00, Rnl=3.71)."""
    rn = net_radiation_daily_mj_m2_day(
        rs_mj_m2_day=np.array([22.07]),
        tmax_c=np.array([21.5]),
        tmin_c=np.array([12.3]),
        ea_kpa=np.array([1.409]),
        ra_mj_m2_day=41.09,
        elevation_m=100.0,
    )
    assert rn[0] == pytest.approx(13.28, abs=0.1)


def test_daily_reference_et_matches_fao56_example_18_end_to_end():
    """FAO-56 Example 18 (Brussels, 6 July, full daily ETo calculation):
    Tmax=21.5, Tmin=12.3, ea=1.409 kPa (from RH), u2=2.078 m/s (already
    converted from the example's own 10m measurement -- the point of this
    test is the daily ETo equation itself, not the wind-height conversion,
    which is a separate, already-tested function), Rn=13.28 MJ/m2/day,
    z=100m -> ETo=3.9 mm/day (published as "3.88 -> 3.9")."""
    et0 = daily_reference_et_fao56(
        tmax_c=np.array([21.5]),
        tmin_c=np.array([12.3]),
        ea_kpa=np.array([1.409]),
        wind_2m_m_s=np.array([2.078]),
        net_radiation_mj_m2_day=np.array([13.28]),
        elevation_m=100.0,
    )
    assert et0[0] == pytest.approx(3.88, abs=0.02)


def test_daily_reference_et_is_nonnegative():
    et0 = daily_reference_et_fao56(
        tmax_c=np.array([5.0]),
        tmin_c=np.array([2.0]),
        ea_kpa=np.array([0.8]),
        wind_2m_m_s=np.array([1.0]),
        net_radiation_mj_m2_day=np.array([-2.0]),  # net radiative loss (winter, low sun)
        elevation_m=100.0,
    )
    assert et0[0] >= 0.0


def test_daily_reference_et_increases_with_net_radiation():
    kwargs = dict(
        tmax_c=np.array([20.0]),
        tmin_c=np.array([10.0]),
        ea_kpa=np.array([1.0]),
        wind_2m_m_s=np.array([2.0]),
        elevation_m=100.0,
    )
    et_low = daily_reference_et_fao56(net_radiation_mj_m2_day=np.array([5.0]), **kwargs)
    et_high = daily_reference_et_fao56(net_radiation_mj_m2_day=np.array([20.0]), **kwargs)
    assert et_high[0] > et_low[0]
