"""Unit tests for physics/snow.py (snow-lite: canopy interception,
enhanced temperature-radiation melt, frozen-ground gate)."""

import numpy as np
import pytest

from physics.snow import (
    canopy_adjusted_swe,
    enhanced_temperature_index_melt,
    is_frozen_ground,
    route_water_input,
)


def test_canopy_adjustment_reduces_swe_under_full_canopy():
    swe = np.array([100.0])
    adjusted = canopy_adjusted_swe(swe, canopy_fraction=np.array([1.0]))
    # 35% max interception loss at full canopy
    assert adjusted[0] == pytest.approx(65.0)


def test_canopy_adjustment_no_change_in_open_ground():
    swe = np.array([100.0])
    adjusted = canopy_adjusted_swe(swe, canopy_fraction=np.array([0.0]))
    assert adjusted[0] == pytest.approx(100.0)


def test_canopy_adjustment_scales_linearly_with_density():
    swe = np.array([100.0, 100.0])
    adjusted = canopy_adjusted_swe(swe, canopy_fraction=np.array([0.5, 1.0]))
    loss_half = 100.0 - adjusted[0]
    loss_full = 100.0 - adjusted[1]
    assert loss_full == pytest.approx(loss_half * 2, abs=1e-6)


def test_is_frozen_ground_at_and_below_freezing():
    tsoil = np.array([272.0, 273.15, 274.0])
    frozen = is_frozen_ground(tsoil)
    np.testing.assert_array_equal(frozen, [True, True, False])


def test_route_water_input_thawed_ground_all_infiltrates():
    infil, runoff = route_water_input(
        rain_mm=np.array([5.0]), melt_mm=np.array([3.0]), frozen=np.array([False])
    )
    assert infil[0] == pytest.approx(8.0)
    assert runoff[0] == pytest.approx(0.0)


def test_route_water_input_frozen_ground_all_runs_off():
    """This is the design doc's 'wet-surface-over-frozen-substrate' state
    -- the whole reason S4 exists."""
    infil, runoff = route_water_input(
        rain_mm=np.array([5.0]), melt_mm=np.array([3.0]), frozen=np.array([True])
    )
    assert infil[0] == pytest.approx(0.0)
    assert runoff[0] == pytest.approx(8.0)


def test_route_water_input_conserves_mass():
    """Whichever bucket it goes in, infiltration + runoff must equal the
    total input -- S4 routes water, it doesn't create or destroy it."""
    rng = np.random.default_rng(1)
    rain = rng.uniform(0, 10, 50)
    melt = rng.uniform(0, 10, 50)
    frozen = rng.random(50) > 0.5
    infil, runoff = route_water_input(rain, melt, frozen)
    np.testing.assert_allclose(infil + runoff, rain + melt)


def test_melt_zero_below_freezing():
    melt = enhanced_temperature_index_melt(
        temp_k=np.array([260.0]),
        terrain_corrected_shortwave_w_m2=np.array([500.0]),
        swe_mm=np.array([50.0]),
    )
    assert melt[0] == pytest.approx(0.0)


def test_melt_increases_with_temperature():
    common = dict(terrain_corrected_shortwave_w_m2=np.array([300.0]), swe_mm=np.array([100.0]))
    cool_melt = enhanced_temperature_index_melt(temp_k=np.array([274.0]), **common)
    warm_melt = enhanced_temperature_index_melt(temp_k=np.array([285.0]), **common)
    assert warm_melt[0] > cool_melt[0]


def test_melt_increases_with_radiation():
    """The mechanism behind aspect-driven melt-out differences: more
    incoming shortwave (e.g. a south-facing slope) -> more melt, holding
    temperature fixed."""
    common = dict(temp_k=np.array([280.0]), swe_mm=np.array([100.0]))
    shaded_melt = enhanced_temperature_index_melt(
        terrain_corrected_shortwave_w_m2=np.array([50.0]), **common
    )
    sunny_melt = enhanced_temperature_index_melt(
        terrain_corrected_shortwave_w_m2=np.array([800.0]), **common
    )
    assert sunny_melt[0] > shaded_melt[0]


def test_melt_capped_at_available_swe():
    """Cannot melt more snow than exists -- a hot, sunny timestep with
    thin snowpack should be capped at the remaining SWE, not the
    uncapped potential melt rate."""
    melt = enhanced_temperature_index_melt(
        temp_k=np.array([300.0]),  # very warm
        terrain_corrected_shortwave_w_m2=np.array([1000.0]),  # full sun
        swe_mm=np.array([0.5]),  # almost no snow left
    )
    assert melt[0] == pytest.approx(0.5)
