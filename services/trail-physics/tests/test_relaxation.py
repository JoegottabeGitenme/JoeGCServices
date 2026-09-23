"""Unit tests for relaxation.py (Eq. 2/7, transcribed from the primary
source in Session 3 -- replacing Session 2's exponential-decay
reconstruction entirely, see that module's docstring for the full story).
"""

import numpy as np
import pytest

from physics.relaxation import (
    DEFAULT_CTS,
    MAX_RELAXATION_DAYS,
    SoilProperties,
    apply_flux_correction,
    compute_delta_t,
    delta_ts,
)

THETA_S = 0.45
THETA_REF = 0.30
THETA_WILT = 0.10


def fine_props(**overrides):
    defaults = dict(
        theta_wilt=THETA_WILT, theta_ref=THETA_REF, theta_s=THETA_S,
        green_veg_fraction=0.3, iota=1.0,
    )
    defaults.update(overrides)
    return SoilProperties(**defaults)


# =============================================================================
# delta_ts -- the piecewise indicator-function part of Eq. 7
# =============================================================================


def test_delta_ts_below_saturation_equals_cts():
    """theta* < theta_s -> the indicator is 1, so delta_ts = Cts exactly."""
    result = delta_ts(theta_star=np.array([0.20]), theta_s=THETA_S)
    assert result[0] == pytest.approx(DEFAULT_CTS)


def test_delta_ts_at_saturation_equals_cts():
    """theta* == theta_s hits the >= branch, exp(0) = 1, so still Cts."""
    result = delta_ts(theta_star=np.array([THETA_S]), theta_s=THETA_S)
    assert result[0] == pytest.approx(DEFAULT_CTS)


def test_delta_ts_above_saturation_decays():
    """theta* > theta_s -> exponential decay branch, strictly less than Cts."""
    result = delta_ts(theta_star=np.array([THETA_S * 1.5]), theta_s=THETA_S)
    assert 0.0 < result[0] < DEFAULT_CTS


def test_delta_ts_custom_cts_scales_linearly():
    below = delta_ts(theta_star=np.array([0.20]), theta_s=THETA_S, cts=0.2)
    assert below[0] == pytest.approx(0.2)


# =============================================================================
# compute_delta_t -- Eq. 7 in full
# =============================================================================


def test_delta_t_positive_when_drier_than_field_capacity():
    """The paper's own physical description: delta_t is the time to dry
    FROM field capacity TO theta_ws. When theta_ws < theta_ref (the common
    case -- weather-scale state is drier than field capacity), this must
    be positive."""
    dt = compute_delta_t(
        theta_star=np.array([0.20]),
        theta_ws=0.15,  # drier than field capacity
        theta_ref=THETA_REF,
        theta_s=THETA_S,
        ep=np.array([5.0]),
        fine_props=fine_props(),
    )
    assert dt[0] > 0.0


def test_delta_t_clipped_to_zero_when_wetter_than_field_capacity():
    """theta_ws > theta_ref: no drying needed from field capacity -- Eq. 7's
    raw value goes negative, clipped to the paper's stated lower bound of 0."""
    dt = compute_delta_t(
        theta_star=np.array([0.35]),
        theta_ws=0.40,  # wetter than field capacity
        theta_ref=THETA_REF,
        theta_s=THETA_S,
        ep=np.array([5.0]),
        fine_props=fine_props(),
    )
    assert dt[0] == pytest.approx(0.0)


def test_delta_t_clipped_to_thirty_days_maximum():
    """A pathologically small saturation flux (near-zero PET) would blow up
    delta_t -- must clip to the paper's stated 30-day bound."""
    dt = compute_delta_t(
        theta_star=np.array([0.20]),
        theta_ws=0.05,  # very dry, large numerator
        theta_ref=THETA_REF,
        theta_s=THETA_S,
        ep=np.array([1e-6]),  # ~zero PET -> ~zero flux at saturation
        fine_props=fine_props(),
    )
    assert dt[0] == pytest.approx(MAX_RELAXATION_DAYS)


def test_delta_t_never_negative():
    rng = np.random.default_rng(11)
    theta_star = rng.uniform(0.05, 0.5, 50)
    theta_ws_values = rng.uniform(0.05, 0.5, 50)
    ep = rng.uniform(0.0, 10.0, 50)
    for tw in theta_ws_values:
        dt = compute_delta_t(
            theta_star=theta_star, theta_ws=tw, theta_ref=THETA_REF, theta_s=THETA_S,
            ep=ep, fine_props=fine_props(),
        )
        assert np.all(dt >= 0.0)


def test_delta_t_does_not_divide_by_exact_zero():
    """Zero PET makes F(theta_s) exactly zero -- must not raise/NaN."""
    dt = compute_delta_t(
        theta_star=np.array([0.20]), theta_ws=0.15, theta_ref=THETA_REF, theta_s=THETA_S,
        ep=np.array([0.0]), fine_props=fine_props(),
    )
    assert not np.isnan(dt).any()


# =============================================================================
# apply_flux_correction -- Eq. 2 in full
# =============================================================================


def test_zero_delta_t_leaves_theta_star_unchanged():
    """delta_t=0 means no correction is applied at all -- theta must equal
    theta* exactly regardless of the flux terms."""
    result = apply_flux_correction(
        theta_star=np.array([0.25]),
        theta_ws=0.20,
        theta_ref_fine=THETA_REF, theta_s_fine=THETA_S, ep_fine=np.array([5.0]),
        fine_props=fine_props(),
        theta_ref_coarse=THETA_REF, theta_s_coarse=THETA_S, ep_coarse=5.0,
        coarse_props=fine_props(),
        delta_t_days=np.array([0.0]),
    )
    assert result[0] == pytest.approx(0.25)


def test_identical_fine_and_coarse_conditions_leave_theta_star_unchanged():
    """If the fine-scale point has EXACTLY the same state and properties as
    the coarse average (theta*==theta_ws, same props), F(theta*) and
    F'(theta_ws) are identical and the correction must be exactly zero
    regardless of delta_t -- there's no flux DIFFERENCE to correct for."""
    same_theta = 0.20
    result = apply_flux_correction(
        theta_star=np.array([same_theta]),
        theta_ws=same_theta,
        theta_ref_fine=THETA_REF, theta_s_fine=THETA_S, ep_fine=np.array([5.0]),
        fine_props=fine_props(),
        theta_ref_coarse=THETA_REF, theta_s_coarse=THETA_S, ep_coarse=5.0,
        coarse_props=fine_props(),
        delta_t_days=np.array([10.0]),
    )
    assert result[0] == pytest.approx(same_theta, abs=1e-9)


def test_varying_only_veg_split_does_not_change_total_flux_under_ek2003():
    """Discovered property, asserted deliberately rather than silently
    relied on: under form='ek2003', Et and Edir share the SAME beta(theta)
    stress function, so total ET = Ep*beta*(green_veg_fraction +
    (1-green_veg_fraction)) = Ep*beta regardless of the vegetation split.
    Changing only green_veg_fraction (with identical theta/Ep/iota on both
    sides) must therefore leave the correction at exactly zero -- a real
    flux difference needs a difference in Ep, theta, or iota, not just the
    veg/bare split."""
    result = apply_flux_correction(
        theta_star=np.array([0.25]),
        theta_ws=0.25,
        theta_ref_fine=THETA_REF, theta_s_fine=THETA_S, ep_fine=np.array([10.0]),
        fine_props=fine_props(green_veg_fraction=0.0),
        theta_ref_coarse=THETA_REF, theta_s_coarse=THETA_S, ep_coarse=10.0,
        coarse_props=fine_props(green_veg_fraction=0.8),
        delta_t_days=np.array([5.0]),
    )
    assert result[0] == pytest.approx(0.25)


def test_fine_point_with_higher_pet_gets_additional_drying_correction():
    """A fine-scale point with MORE potential evaporation than the coarse
    average (e.g. a sunnier aspect) should dry out further still, beyond
    Eq. 1's pure topographic redistribution -- this is Eq. 2's actual
    purpose (see the test above for what does NOT trigger a correction)."""
    result = apply_flux_correction(
        theta_star=np.array([0.25]),
        theta_ws=0.25,
        theta_ref_fine=THETA_REF, theta_s_fine=THETA_S, ep_fine=np.array([15.0]),  # sunnier -> higher PET
        fine_props=fine_props(),
        theta_ref_coarse=THETA_REF, theta_s_coarse=THETA_S, ep_coarse=10.0,  # coarse average
        coarse_props=fine_props(),
        delta_t_days=np.array([5.0]),
    )
    # Fine point dries faster than the coarse reference -> F(theta*) more
    # negative than F'(theta_ws) -> correction term is negative -> theta < theta*.
    assert result[0] < 0.25


def test_compute_delta_t_is_form_independent_when_theta_s_exceeds_theta_ref():
    """Discovered property (documented in compute_delta_t's docstring):
    F(theta_s) reduces to the same value under both Eq. 5 forms whenever
    theta_s > theta_ref, since both forms' beta/ratio equal 1.0 exactly at
    theta=theta_s. delta_t itself is therefore form-independent in the
    normal case -- asserted here so a future change that breaks this
    invariant is caught, rather than silently drifting."""
    dt_ek2003 = compute_delta_t(
        theta_star=np.array([0.20]), theta_ws=0.15, theta_ref=THETA_REF, theta_s=THETA_S,
        ep=np.array([5.0]), fine_props=fine_props(), form="ek2003",
    )
    dt_geowatch = compute_delta_t(
        theta_star=np.array([0.20]), theta_ws=0.15, theta_ref=THETA_REF, theta_s=THETA_S,
        ep=np.array([5.0]), fine_props=fine_props(), form="geowatch",
    )
    assert dt_ek2003[0] == pytest.approx(dt_geowatch[0])


def test_apply_flux_correction_form_selection_can_diverge():
    """Unlike compute_delta_t, apply_flux_correction evaluates F at theta*
    and theta_ws (generally not at saturation), where the two Eq. 5 forms
    genuinely can disagree -- e.g. below field capacity, where 'geowatch'
    goes negative and 'ek2003' clips to 0."""
    below_field_capacity = 0.15  # < THETA_REF (0.30)
    result_ek2003 = apply_flux_correction(
        theta_star=np.array([below_field_capacity]), theta_ws=below_field_capacity,
        theta_ref_fine=THETA_REF, theta_s_fine=THETA_S, ep_fine=np.array([10.0]),
        fine_props=fine_props(),
        theta_ref_coarse=THETA_REF, theta_s_coarse=THETA_S, ep_coarse=5.0,
        coarse_props=fine_props(), delta_t_days=np.array([5.0]), form="ek2003",
    )
    result_geowatch = apply_flux_correction(
        theta_star=np.array([below_field_capacity]), theta_ws=below_field_capacity,
        theta_ref_fine=THETA_REF, theta_s_fine=THETA_S, ep_fine=np.array([10.0]),
        fine_props=fine_props(),
        theta_ref_coarse=THETA_REF, theta_s_coarse=THETA_S, ep_coarse=5.0,
        coarse_props=fine_props(), delta_t_days=np.array([5.0]), form="geowatch",
    )
    assert result_ek2003[0] != pytest.approx(result_geowatch[0])
