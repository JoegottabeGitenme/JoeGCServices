"""Unit tests for flux.py (Eq. 4/5 -- the Ek et al. 2003-consistent
soil-moisture stress function, replacing the design doc's flagged
transcription error)."""

import numpy as np
import pytest

from physics.flux import (
    direct_soil_evaporation,
    soil_moisture_stress_factor,
    total_actual_et,
    vegetation_transpiration,
)


def test_stress_factor_zero_at_wilting_point():
    beta = soil_moisture_stress_factor(theta=0.10, theta_wilt=0.10, theta_ref=0.30)
    assert beta == pytest.approx(0.0)


def test_stress_factor_one_at_field_capacity():
    beta = soil_moisture_stress_factor(theta=0.30, theta_wilt=0.10, theta_ref=0.30)
    assert beta == pytest.approx(1.0)


def test_stress_factor_clipped_below_wilting_point():
    """This is exactly the 'wrong sign behaviour below field capacity' the
    design doc says the source paper's erroneous Eq. 5 exhibits -- the
    corrected form must clip to zero, never go negative, for theta below
    wilting point."""
    beta = soil_moisture_stress_factor(theta=0.05, theta_wilt=0.10, theta_ref=0.30)
    assert beta == pytest.approx(0.0)


def test_stress_factor_clipped_above_field_capacity():
    beta = soil_moisture_stress_factor(theta=0.45, theta_wilt=0.10, theta_ref=0.30)
    assert beta == pytest.approx(1.0)


def test_stress_factor_monotonically_increasing():
    theta = np.linspace(0.05, 0.45, 20)
    beta = soil_moisture_stress_factor(theta, theta_wilt=0.10, theta_ref=0.30)
    assert np.all(np.diff(beta) >= 0)


def test_stress_factor_handles_degenerate_soil_without_nan():
    """theta_ref == theta_wilt would otherwise divide by zero -- must not
    silently produce NaN that poisons the rest of the flux-correction chain."""
    beta = soil_moisture_stress_factor(
        theta=np.array([0.2]), theta_wilt=np.array([0.2]), theta_ref=np.array([0.2])
    )
    assert not np.isnan(beta).any()


def test_direct_evaporation_scales_with_bare_fraction():
    """More bare ground (lower green_veg_fraction) -> more direct
    evaporation, holding PET and moisture fixed."""
    common = dict(pet=np.array([5.0]), theta=np.array([0.25]), theta_wilt=0.10, theta_ref=0.30)
    bare = direct_soil_evaporation(green_veg_fraction=0.0, **common)
    vegetated = direct_soil_evaporation(green_veg_fraction=0.8, **common)
    assert bare[0] > vegetated[0]


def test_transpiration_scales_with_green_fraction():
    common = dict(pet=np.array([5.0]), theta=np.array([0.25]), theta_wilt=0.10, theta_ref=0.30)
    sparse = vegetation_transpiration(green_veg_fraction=0.1, **common)
    dense = vegetation_transpiration(green_veg_fraction=0.9, **common)
    assert dense[0] > sparse[0]


def test_total_et_equals_sum_of_components():
    kwargs = dict(
        pet=np.array([5.0]),
        theta=np.array([0.25]),
        theta_wilt=0.10,
        theta_ref=0.30,
        green_veg_fraction=0.6,
    )
    total = total_actual_et(**kwargs)
    direct = direct_soil_evaporation(**kwargs)
    veg = vegetation_transpiration(**kwargs)
    np.testing.assert_allclose(total, direct + veg)


def test_total_et_never_exceeds_pet():
    """Actual ET is capped by potential ET -- beta in [0,1] and the bare/
    vegetated fractions sum to 1, so total_actual_et <= pet always."""
    rng = np.random.default_rng(7)
    theta = rng.uniform(0.0, 0.5, size=100)
    pet = np.full(100, 5.0)
    total = total_actual_et(pet, theta, theta_wilt=0.10, theta_ref=0.30, green_veg_fraction=0.5)
    assert np.all(total <= pet + 1e-9)
