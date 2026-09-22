"""Eq. 4/5 -- vegetation transpiration and direct soil evaporation flux
functions, F(theta), used by Eq. 2's flux-anomaly correction.

The design doc flags a transcription error in the source paper's own Eq. 5
text: it reads `(theta - theta_ref)/(theta_s - theta_ref)`, which "does not
match the standard Noah / Ek et al. (2003) formulation and gives wrong sign
behaviour below field capacity" (docs/trail-conditions-design.md Section
5). This module implements the CORRECT, standard form instead -- the
well-established Noah land-surface-model soil-moisture stress function
(Chen et al. 1996; Ek et al. 2003), which is independently documented
(not behind the same paywall as the GeoWATCH paper itself):

    beta(theta) = clip((theta - theta_wilt) / (theta_ref - theta_wilt), 0, 1)

Below wilting point, beta=0 (no evaporation possible -- water is held too
tightly by the soil matrix). At or above field capacity, beta=1 (moisture
is not limiting; evaporation proceeds at the potential rate). This is the
"sign behaviour below field capacity" property the doc's erroneous version
gets wrong: the doc's flagged form uses theta_ref/theta_s as the
denominator reference points instead of theta_wilt/theta_ref, which
produces a negative or nonsensical ratio for theta below field capacity --
exactly the regime this function is most often evaluated in.
"""

from __future__ import annotations

import numpy as np


def soil_moisture_stress_factor(
    theta: np.ndarray, theta_wilt: np.ndarray | float, theta_ref: np.ndarray | float
) -> np.ndarray:
    """beta(theta): fraction of potential ET actually achievable given
    current soil moisture, clipped to [0, 1]."""
    denom = theta_ref - theta_wilt
    # Guard degenerate soils where theta_ref == theta_wilt (shouldn't occur
    # with real SSURGO data, but a stray zero must not produce a NaN/inf
    # that silently propagates through the whole flux-correction chain).
    denom = np.where(np.abs(np.asarray(denom)) < 1e-9, 1e-9, denom)
    beta = (theta - theta_wilt) / denom
    return np.clip(beta, 0.0, 1.0)


def direct_soil_evaporation(
    pet: np.ndarray,
    theta: np.ndarray,
    theta_wilt: np.ndarray | float,
    theta_ref: np.ndarray | float,
    green_veg_fraction: np.ndarray | float,
) -> np.ndarray:
    """Eq. 5 -- direct (bare-soil-fraction) evaporation flux."""
    beta = soil_moisture_stress_factor(theta, theta_wilt, theta_ref)
    return (1.0 - green_veg_fraction) * pet * beta


def vegetation_transpiration(
    pet: np.ndarray,
    theta: np.ndarray,
    theta_wilt: np.ndarray | float,
    theta_ref: np.ndarray | float,
    green_veg_fraction: np.ndarray | float,
) -> np.ndarray:
    """Eq. 4 -- vegetated-fraction transpiration flux.

    Uses the same beta(theta) stress function as direct evaporation for
    consistency/simplicity -- a defensible simplification (real Noah
    applies canopy resistance on top of a similar root-zone stress factor,
    which would need root-zone-weighted moisture and a canopy-resistance
    parameterization neither this session's HRRR ingredients nor the
    Tarrawarra dataset carry); not claimed as the primary source's exact
    form.
    """
    beta = soil_moisture_stress_factor(theta, theta_wilt, theta_ref)
    return green_veg_fraction * pet * beta


def total_actual_et(
    pet: np.ndarray,
    theta: np.ndarray,
    theta_wilt: np.ndarray | float,
    theta_ref: np.ndarray | float,
    green_veg_fraction: np.ndarray | float,
) -> np.ndarray:
    """F(theta): combined actual ET flux -- the function Eq. 2 evaluates at
    both the local state (theta_i) and the coarse state (theta_ws) to form
    the flux anomaly that drives the relaxation correction."""
    return direct_soil_evaporation(
        pet, theta, theta_wilt, theta_ref, green_veg_fraction
    ) + vegetation_transpiration(pet, theta, theta_wilt, theta_ref, green_veg_fraction)
