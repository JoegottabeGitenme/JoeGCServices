"""Unit tests for flux.py (Eq. 3/4/5, transcribed from the primary source
in Session 3 -- see flux.py's module docstring for what changed from
Session 2's paper-less reconstruction)."""

import numpy as np
import pytest

from physics.flux import (
    DEFAULT_RD,
    direct_soil_evaporation,
    f_theta,
    geowatch_soil_moisture_ratio,
    radiative_factor,
    soil_moisture_stress_factor,
    total_actual_et,
    vegetation_transpiration,
)

# theta_s (saturated soil moisture) is required in every direct_soil_evaporation/
# total_actual_et call now (Session 3 signature change) even when form="ek2003"
# doesn't use it, so every call site needs a value -- 0.45 is a plausible loam value.
THETA_S = 0.45


# =============================================================================
# radiative_factor (Eq. 5's [Rd + (1-Rd)*iota] prefactor -- new in Session 3)
# =============================================================================


def test_radiative_factor_at_full_exposure_is_one():
    assert radiative_factor(iota=1.0) == pytest.approx(1.0)


def test_radiative_factor_at_full_shadow_equals_rd():
    """Even fully self-shadowed (iota=0), diffuse sky radiation still
    supplies Rd's worth of potential evaporation -- this is the whole
    point of the Rd floor in the prefactor."""
    assert radiative_factor(iota=0.0) == pytest.approx(DEFAULT_RD)


def test_radiative_factor_monotonic_in_iota():
    values = [radiative_factor(iota=i) for i in [0.0, 0.3, 0.6, 1.0]]
    assert all(a <= b for a, b in zip(values, values[1:]))


# =============================================================================
# soil_moisture_stress_factor (Eq. 4's ratio; also Eq. 5's form="ek2003")
# =============================================================================


def test_stress_factor_zero_at_wilting_point():
    beta = soil_moisture_stress_factor(theta=0.10, theta_wilt=0.10, theta_ref=0.30)
    assert beta == pytest.approx(0.0)


def test_stress_factor_one_at_field_capacity():
    beta = soil_moisture_stress_factor(theta=0.30, theta_wilt=0.10, theta_ref=0.30)
    assert beta == pytest.approx(1.0)


def test_stress_factor_clipped_below_wilting_point():
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
    beta = soil_moisture_stress_factor(
        theta=np.array([0.2]), theta_wilt=np.array([0.2]), theta_ref=np.array([0.2])
    )
    assert not np.isnan(beta).any()


# =============================================================================
# geowatch_soil_moisture_ratio (Eq. 5 AS PRINTED -- new in Session 3)
# =============================================================================


def test_geowatch_ratio_is_one_at_saturation():
    ratio = geowatch_soil_moisture_ratio(theta=0.45, theta_ref=0.30, theta_s=0.45)
    assert ratio == pytest.approx(1.0)


def test_geowatch_ratio_is_zero_at_field_capacity():
    ratio = geowatch_soil_moisture_ratio(theta=0.30, theta_ref=0.30, theta_s=0.45)
    assert ratio == pytest.approx(0.0)


def test_geowatch_ratio_goes_negative_below_field_capacity():
    """This is the exact behavior docs/trail-conditions-design.md flagged
    (sight-unseen, in Session 1/2) as suspicious -- confirmed in Session 3
    to be exactly what the paper's printed equation does, deliberately NOT
    patched here since the point is to test it empirically, not assume."""
    ratio = geowatch_soil_moisture_ratio(theta=0.15, theta_ref=0.30, theta_s=0.45)
    assert ratio < 0.0


def test_geowatch_ratio_not_clipped_above_one():
    ratio = geowatch_soil_moisture_ratio(theta=0.60, theta_ref=0.30, theta_s=0.45)
    assert ratio > 1.0


def test_geowatch_ratio_handles_degenerate_soil_without_nan():
    ratio = geowatch_soil_moisture_ratio(
        theta=np.array([0.3]), theta_ref=np.array([0.3]), theta_s=np.array([0.3])
    )
    assert not np.isnan(ratio).any()


# =============================================================================
# direct_soil_evaporation / total_actual_et -- both forms
# =============================================================================


def test_direct_evaporation_scales_with_bare_fraction():
    common = dict(
        pet=np.array([5.0]), theta=np.array([0.25]), theta_wilt=0.10, theta_ref=0.30, theta_s=THETA_S
    )
    bare = direct_soil_evaporation(green_veg_fraction=0.0, **common)
    vegetated = direct_soil_evaporation(green_veg_fraction=0.8, **common)
    assert bare[0] > vegetated[0]


def test_direct_evaporation_scales_with_radiative_exposure():
    """More solar exposure (higher iota) -> more direct evaporation --
    the mechanism that couples Eq. 6 (solar view factor) into Eq. 5."""
    common = dict(
        pet=np.array([5.0]), theta=np.array([0.25]), theta_wilt=0.10, theta_ref=0.30,
        theta_s=THETA_S, green_veg_fraction=0.2,
    )
    shaded = direct_soil_evaporation(iota=0.0, **common)
    sunny = direct_soil_evaporation(iota=1.0, **common)
    assert sunny[0] > shaded[0]


def test_direct_evaporation_ek2003_form_ignores_theta_s():
    """form='ek2003' shouldn't care what theta_s is -- only the geowatch
    form's ratio uses it."""
    common = dict(
        pet=np.array([5.0]), theta=np.array([0.25]), theta_wilt=0.10, theta_ref=0.30,
        green_veg_fraction=0.2, form="ek2003",
    )
    result_a = direct_soil_evaporation(theta_s=0.40, **common)
    result_b = direct_soil_evaporation(theta_s=0.60, **common)
    np.testing.assert_allclose(result_a, result_b)


def test_direct_evaporation_geowatch_form_can_go_negative():
    """Below field capacity, the unclipped geowatch ratio goes negative,
    and that propagates through -- direct_soil_evaporation must NOT clip
    it back to zero (that would silently turn 'geowatch' into 'ek2003
    with different reference points')."""
    result = direct_soil_evaporation(
        pet=np.array([5.0]), theta=np.array([0.15]), theta_wilt=0.05, theta_ref=0.30,
        theta_s=THETA_S, green_veg_fraction=0.2, form="geowatch",
    )
    assert result[0] < 0.0


def test_direct_evaporation_unknown_form_raises():
    with pytest.raises(ValueError):
        direct_soil_evaporation(
            pet=np.array([5.0]), theta=np.array([0.25]), theta_wilt=0.10, theta_ref=0.30,
            theta_s=THETA_S, green_veg_fraction=0.2, form="not-a-real-form",
        )


def test_transpiration_scales_with_green_fraction():
    common = dict(pet=np.array([5.0]), theta=np.array([0.25]), theta_wilt=0.10, theta_ref=0.30)
    sparse = vegetation_transpiration(green_veg_fraction=0.1, **common)
    dense = vegetation_transpiration(green_veg_fraction=0.9, **common)
    assert dense[0] > sparse[0]


def test_total_et_equals_sum_of_components():
    kwargs = dict(
        pet=np.array([5.0]), theta=np.array([0.25]), theta_wilt=0.10, theta_ref=0.30,
        theta_s=THETA_S, green_veg_fraction=0.6,
    )
    total = total_actual_et(**kwargs)
    direct = direct_soil_evaporation(**kwargs)
    veg = vegetation_transpiration(
        pet=kwargs["pet"], theta=kwargs["theta"], theta_wilt=kwargs["theta_wilt"],
        theta_ref=kwargs["theta_ref"], green_veg_fraction=kwargs["green_veg_fraction"],
    )
    np.testing.assert_allclose(total, direct + veg)


def test_total_et_ek2003_never_exceeds_pet_at_full_exposure():
    """With form='ek2003' and iota=1.0 (radiative factor exactly 1), actual
    ET is capped by potential ET -- beta in [0,1], bare/vegetated fractions
    sum to 1."""
    rng = np.random.default_rng(7)
    theta = rng.uniform(0.0, 0.5, size=100)
    pet = np.full(100, 5.0)
    total = total_actual_et(
        pet, theta, theta_wilt=0.10, theta_ref=0.30, theta_s=THETA_S, green_veg_fraction=0.5
    )
    assert np.all(total <= pet + 1e-9)


def test_f_theta_is_negative_of_total_et():
    kwargs = dict(
        pet=np.array([5.0]), theta=np.array([0.25]), theta_wilt=0.10, theta_ref=0.30,
        theta_s=THETA_S, green_veg_fraction=0.6,
    )
    f = f_theta(**kwargs)
    total = total_actual_et(**kwargs)
    np.testing.assert_allclose(f, -total)


def test_f_theta_is_negative_for_positive_et():
    """F(theta) represents drying -- with any positive ET, F must be negative."""
    f = f_theta(
        pet=np.array([5.0]), theta=np.array([0.30]), theta_wilt=0.10, theta_ref=0.30,
        theta_s=THETA_S, green_veg_fraction=0.5,
    )
    assert f[0] < 0.0
