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
    hourly_reference_et_mm,
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
