"""Eq. 1 -- topographic/transmissivity redistribution of a coarse-scale
(catchment- or grid-cell-mean) soil moisture to individual points.

**Session 8 finding, worth reading before trusting either function below**:
the GeoWATCH paper's own Software and Data Availability section (which
Sessions 3-7 had noted but never actually opened) publishes a Creare/PODPAC
notebook that the authors describe as reproducing "the downscaling
algorithm":
https://github.com/creare-com/podpac-examples/blob/main/notebooks/5-datalib/smap/SMAP-downscaling-example-application.ipynb

That notebook's actual production code is NOT the paper's printed Eq. 1
(reproduced below as `redistribute`, kept for the documented reproduction
history). It is:

    theta = theta_SMAP + (theta_s - theta_wilt)/k * (lambda - lambda_bar)

    podpac.algorithm.Arithmetic(A=smap, B=twi, C=twi_bar, D=porosity,
        E=wilt, eqn='A + (D - E) / 13.0 * (B - C)')

Two differences from the paper's printed form, both consequential:
1. The amplitude is (theta_s - theta_wilt)/k -- the soil's plant-available
   water-holding RANGE divided by k -- not a flat 1/k. At Tarrawarra
   (theta_s~0.47, theta_wilt~0.09 from texture), that's ~0.029 per unit
   TWI, about 2.6x SMALLER than the paper's flat 1/13~0.077. This lines up
   almost exactly with the ~3x-too-large correction magnitude diagnosed in
   Session 4 and never resolved by any hypothesis tested in Sessions 5-7.
2. There is NO ln(Ks) term at all. This also retroactively explains why
   Sessions 4-6 found essentially zero independent signal from ln(Ks) at
   Tarrawarra (implied k for that term alone was ~260, i.e. "no effect") --
   the real production system never had that term to begin with.

This also resolves the paper's own unexplained Section 2.2.1 sentence,
flagged as unresolved since Session 4: "the GeoWATCH calculation of the TWI
was modified to use volumetric soil moisture instead of relative soil
moisture." Classic TOPMODEL/STOPMODEL redistributes a dimensionless
relative-saturation deficit; multiplying by (theta_s - theta_wilt) is
exactly the conversion from that relative index into volumetric (m3/m3)
units. The sentence was describing this amplitude term the whole time.

`redistribute_podpac` below implements this form. k=13 is kept UNCHANGED
(DEFAULT_K) -- this is a correction to the equation's STRUCTURE, discovered
by reading the paper's own published code, not a fit to any target number.

---

Original (paper-printed, Eq. 1) form, from
docs/trail-conditions-design.md Section 5 ("S1 -- Anchor" / "S5 --
Downscale"), kept for the documented reproduction-attempt history
(Sessions 4-7 all validated against this form and are described as such in
README.md):

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


def redistribute_podpac(
    theta_coarse: float,
    twi: np.ndarray,
    theta_s: np.ndarray | float,
    theta_wilt: np.ndarray | float,
    twi_mean: float | None = None,
    k: float = DEFAULT_K,
) -> np.ndarray:
    """Apply the REAL Creare/GeoWATCH production equation, as published in
    the paper's own linked notebook (see module docstring's Session 8
    finding) -- not the paper's printed Eq. 1.

        theta* = theta_coarse + (theta_s - theta_wilt)/k * (twi - twi_mean)

    Args:
        theta_coarse: coarse-scale (e.g. SMAP-analog / catchment-mean)
            moisture, same units as the desired output (fractional
            volumetric, to match theta_s/theta_wilt's own units).
        twi: per-point topographic wetness index.
        theta_s: saturated soil moisture (porosity), per-point array or a
            single scalar (site-uniform) -- the notebook's `porosity` node,
            evaluated at each output point; whether that's meaningfully
            per-point or effectively uniform at a given site depends on the
            resolution of whatever coarse soil-constants layer is used
            (SMAPPorosity in the original; texture-derived Noah params
            here). Pass either shape; both are legitimate per the
            notebook's own construction.
        theta_wilt: wilting point, same shape rules as theta_s.
        twi_mean: catchment/domain mean TWI. If omitted, computed as
            mean(twi) (only correct when `twi` IS the whole catchment).
        k: the notebook's own literal constant (13.0) -- unchanged from the
            paper-printed form's DEFAULT_K. This function does not
            introduce a second constant; it corrects the amplitude's
            STRUCTURE (soil-water-holding-range-scaled, not flat), not its
            calibrated value.

    Returns:
        Per-point redistributed moisture, same shape as `twi`.

    Note there is deliberately no ln(Ks) term -- the real production
    equation doesn't have one (see module docstring).
    """
    if twi_mean is None:
        twi_mean = float(np.mean(twi))
    theta_s = np.asarray(theta_s, dtype=np.float64)
    theta_wilt = np.asarray(theta_wilt, dtype=np.float64)
    amplitude = (theta_s - theta_wilt) / k
    return theta_coarse + amplitude * (twi - twi_mean)
