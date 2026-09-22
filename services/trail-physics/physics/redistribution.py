"""Eq. 1 -- topographic/transmissivity redistribution of a coarse-scale
(catchment- or grid-cell-mean) soil moisture to individual points.

Fully specified in docs/trail-conditions-design.md Section 5 ("S1 -- Anchor"
/ "S5 -- Downscale"):

    theta* = theta_ws - (1/k)(lambda_bar - lambda) - (1/k)(ln(Ks) - ln(Ks)_bar)

This is the equation Rung 1 (Tarrawarra) validates: for each of the 13
sampling dates, redistribute that date's own catchment-mean TDR moisture to
every measurement point using each point's topographic wetness index
(lambda, from terrain.py) and log-transformed saturated hydraulic
conductivity (ln(Ks), from soil data), then compare the redistributed
values against the actual per-point TDR measurements.

Physical sign check (both terms should make hydrological sense, not just
algebraic sense):
- A point with lambda_i > lambda_bar (higher TWI -- more convergent
  upslope area relative to its local slope, e.g. a valley bottom or
  footslope) should be WETTER than the catchment mean.
  -(lambda_bar - lambda_i) = (lambda_i - lambda_bar) > 0 -- correct sign.
- A point with ln(Ks)_i > ln(Ks)_bar (more permeable/free-draining soil)
  should be DRIER than the catchment mean (water moves through faster).
  -(ln(Ks)_i - ln(Ks)_bar) < 0 when Ks_i > Ks_bar -- correct sign.
"""

from __future__ import annotations

import numpy as np

DEFAULT_K = 13.0  # per docs/trail-conditions-design.md Section 5 ("S5 -- Downscale")


def redistribute(
    theta_coarse: float,
    twi: np.ndarray,
    log_ks: np.ndarray,
    twi_mean: float | None = None,
    log_ks_mean: float | None = None,
    k: float = DEFAULT_K,
) -> np.ndarray:
    """Apply Eq. 1 pointwise.

    Args:
        theta_coarse: the coarse-scale (catchment-mean, for Tarrawarra; grid-
            cell, for the live HRRR-forced pipeline) moisture value, in the
            same units the caller wants the output in (Tarrawarra validation
            uses %V/V to match TDR directly; the live pipeline uses
            fractional volumetric to match SOILW).
        twi: per-point topographic wetness index (terrain.compute_twi).
        log_ks: per-point ln(saturated hydraulic conductivity). Units must
            match whatever `k` was calibrated against -- Tarrawarra's own
            ksat.dat is in mm/hr; k=13 (the doc's given value) is assumed
            calibrated for that combination since it reproduces the doc's
            target Tarrawarra RMSE.
        twi_mean / log_ks_mean: catchment/domain means. If omitted, computed
            as the mean of the `twi`/`log_ks` arrays themselves (the natural
            choice when every point in the array IS the catchment).
        k: the doc's redistribution sensitivity constant (default 13.0).

    Returns:
        Per-point redistributed moisture, same shape as `twi`.
    """
    if twi_mean is None:
        twi_mean = float(np.mean(twi))
    if log_ks_mean is None:
        log_ks_mean = float(np.mean(log_ks))

    return (
        theta_coarse
        - (1.0 / k) * (twi_mean - twi)
        - (1.0 / k) * (log_ks - log_ks_mean)
    )
