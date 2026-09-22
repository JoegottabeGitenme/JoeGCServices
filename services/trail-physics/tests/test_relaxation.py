"""Unit tests for relaxation.py (Eq. 2/7 -- UNVERIFIED reconstruction, see
that module's docstring).

Deliberately tests only the qualitative properties actually claimed
(anomalies shrink over time, larger flux differences relax faster, output
never diverges) -- NOT exact numerical agreement with a primary source we
don't have access to. Do not add a test asserting a specific numeric output
here without first reconciling this module against the actual Eylander et
al. (2023) Eq. 2/7 text.
"""

import numpy as np
import pytest

from physics.relaxation import (
    MAX_RELAXATION_DAYS,
    apply_flux_correction,
    relaxation_timescale_hours,
)


def test_zero_flux_anomaly_saturates_at_max_timescale():
    """No difference between local and coarse flux means nothing is
    driving the anomaly to relax -- the timescale should sit at its
    upper bound."""
    tau = relaxation_timescale_hours(np.array([0.0]))
    assert tau[0] == pytest.approx(MAX_RELAXATION_DAYS * 24.0)


def test_larger_flux_anomaly_relaxes_faster():
    """A bigger local-vs-coarse flux difference must produce a SHORTER
    relaxation timescale (faster convergence back to the coarse value)."""
    small_anomaly_tau = relaxation_timescale_hours(np.array([0.5]))
    large_anomaly_tau = relaxation_timescale_hours(np.array([5.0]))
    assert large_anomaly_tau[0] < small_anomaly_tau[0]


def test_timescale_bounded_to_thirty_days():
    """Per the design doc's stated Eq. 7 clip bound."""
    tau_hours = relaxation_timescale_hours(np.array([1e-9, 1e9]))
    assert np.all(tau_hours <= MAX_RELAXATION_DAYS * 24.0 + 1e-6)
    assert np.all(tau_hours >= 0.0)


def test_anomaly_shrinks_toward_coarse_value_over_time():
    """The core qualitative property Eq. 2 must have: a positive anomaly
    (wetter than the coarse mean) shrinks, not grows, under a nonzero flux
    difference driving it back toward the mean."""
    theta_coarse = 0.20
    theta_local = np.array([0.30])  # 0.10 above coarse
    result = apply_flux_correction(
        theta_local=theta_local,
        theta_coarse=theta_coarse,
        flux_local_mm_per_day=np.array([6.0]),  # wetter pixel ET's faster
        flux_coarse_mm_per_day=4.0,
        dt_hours=6.0,
    )
    anomaly_before = theta_local[0] - theta_coarse
    anomaly_after = result[0] - theta_coarse
    assert 0 <= anomaly_after < anomaly_before


def test_zero_anomaly_stays_zero():
    """If local already equals coarse, there's nothing to relax -- the
    corrected value must stay exactly at the coarse value regardless of
    flux difference."""
    result = apply_flux_correction(
        theta_local=np.array([0.20]),
        theta_coarse=0.20,
        flux_local_mm_per_day=np.array([5.0]),
        flux_coarse_mm_per_day=3.0,
        dt_hours=6.0,
    )
    assert result[0] == pytest.approx(0.20)


def test_longer_timestep_relaxes_more():
    """A longer dt_hours must move the anomaly at least as far toward the
    coarse value as a shorter one, all else equal (monotonic decay)."""
    kwargs = dict(
        theta_local=np.array([0.30]),
        theta_coarse=0.20,
        flux_local_mm_per_day=np.array([6.0]),
        flux_coarse_mm_per_day=4.0,
    )
    result_1h = apply_flux_correction(dt_hours=1.0, **kwargs)
    result_24h = apply_flux_correction(dt_hours=24.0, **kwargs)
    anomaly_1h = result_1h[0] - 0.20
    anomaly_24h = result_24h[0] - 0.20
    assert anomaly_24h < anomaly_1h
