"""Eq. 2 / Eq. 7 -- flux-driven relaxation of the redistributed anomaly
over time.

*** UNVERIFIED AGAINST THE PRIMARY SOURCE. *** Eylander et al. (2023) is
behind a ScienceDirect paywall; every fetch attempt this session (WebFetch
direct, DOI resolver, ACM mirror) returned 400/403/blocked. Do not treat
the exact algebraic form below as validated -- it is a physically-motivated
reconstruction, not a transcription. See physics/__init__.py's module
docstring for the full confidence-level breakdown across this package.

What the design doc *does* specify (Section 5, "S5 -- Downscale"):
- Eq. 2 "subtracts F'(theta_ws), a flux evaluated at the coarse state using
  coarse-scale soil properties" -- i.e. the correction is driven by a flux
  anomaly (local flux minus coarse-scale flux), not just the moisture
  anomaly itself.
- Eq. 7 produces a timestep/timescale bounded to [0, 30] days, using named
  constants Cts=0.1 and Rd=0.15.
- "S4 flux added as a source term in Eq. 2" -- the frozen-ground
  infiltration-gate output (not implemented in this session; HRRR has no
  soil-ice-fraction field, see the design doc's amendment table) would
  enter here as an additional source/sink once it exists.

Reconstruction implemented here: the local anomaly (theta_local -
theta_coarse) decays exponentially toward zero, at a rate set by how
different the local and coarse-scale actual-ET fluxes are -- a larger flux
difference relaxes the anomaly faster (a wetter-than-average pixel
evaporates faster than the mean, drying back toward it; a drier-than-
average pixel evaporates slower, staying dry longer -- both effects pull
the anomaly toward zero, consistent with the Equilibrium Moisture Theory
lineage the design doc names, Coleman & Niemann 2013 / Ranney et al. 2015).
The relaxation TIMESCALE (not the model integration timestep) is bounded to
[0, 30] days per the doc's Eq. 7, using Cts/Rd as tunable coefficients.

Before trusting this module's numerical output beyond qualitative sanity
(anomalies shrink, wetter-than-mean points dry faster than drier-than-mean
points recover): get the primary source (institutional access, a co-author
copy, or interlibrary loan) and reconcile against the actual Eq. 2/6/7 text,
the same way the design doc already reconciled Eq. 5 against Ek et al. 2003.
"""

from __future__ import annotations

import numpy as np

DEFAULT_CTS = 0.1  # per docs/trail-conditions-design.md Section 5 ("S5 -- Downscale")
DEFAULT_RD = 0.15  # per docs/trail-conditions-design.md Section 5 ("S5 -- Downscale")
MAX_RELAXATION_DAYS = 30.0  # per the doc's stated Eq. 7 clip bound


def relaxation_timescale_hours(
    flux_anomaly_mm_per_day: np.ndarray,
    cts: float = DEFAULT_CTS,
    rd: float = DEFAULT_RD,
    max_days: float = MAX_RELAXATION_DAYS,
) -> np.ndarray:
    """Reconstructed Eq. 7. A larger |flux anomaly| -> shorter relaxation
    timescale (faster convergence back to the coarse-scale value); a
    near-zero flux anomaly -> the timescale saturates at `max_days` (the
    anomaly barely relaxes at all, since there's no flux difference driving
    it to)."""
    epsilon = 1e-6  # avoids a divide-by-zero when flux_anomaly is exactly 0
    tau_days = cts / (rd * np.abs(flux_anomaly_mm_per_day) + epsilon)
    tau_days = np.clip(tau_days, 0.0, max_days)
    return tau_days * 24.0


def apply_flux_correction(
    theta_local: np.ndarray,
    theta_coarse: float,
    flux_local_mm_per_day: np.ndarray,
    flux_coarse_mm_per_day: float,
    dt_hours: float,
    cts: float = DEFAULT_CTS,
    rd: float = DEFAULT_RD,
    max_relaxation_days: float = MAX_RELAXATION_DAYS,
) -> np.ndarray:
    """Reconstructed Eq. 2. Relaxes the Eq.-1-redistributed anomaly
    (theta_local - theta_coarse) exponentially toward zero over `dt_hours`,
    at a per-point rate set by `relaxation_timescale_hours`.

    This is the mechanism that lets the redistributed pattern evolve
    forward through a forecast (unlike Eq. 1 alone, which only redistributes
    a single coarse-scale snapshot and is what Tarrawarra's static-pattern
    validation actually tests -- see redistribution.py's docstring).
    """
    anomaly = theta_local - theta_coarse
    flux_anomaly = flux_local_mm_per_day - flux_coarse_mm_per_day
    tau_hours = relaxation_timescale_hours(flux_anomaly, cts, rd, max_relaxation_days)
    decay = np.exp(-dt_hours / np.maximum(tau_hours, 1e-6))
    return theta_coarse + anomaly * decay
