"""Eq. 3/4/5 -- vegetation transpiration and direct soil evaporation flux
functions, transcribed from Eylander et al. (2023) Section 2.2.2 (user-
supplied copy, Session 3 -- see docs/trail-conditions-design.md's Session 3
notes; Session 2 had no access and reconstructed this module from adjacent
literature instead).

Session 3 reconciliation against the primary source:

- **Eq. 4 (vegetation transpiration, Et)**: the paper's printed form,
  `sigma_f * Ep * (theta - theta_w)/(theta_ref - theta_w)`, is an EXACT
  match to what Session 2 already implemented from the independently-
  published Ek et al. (2003)/Chen et al. (1996) formulation. No change.

- **Eq. 5 (direct soil evaporation, Edir)**: the paper's printed form is

      Edir(theta) = [Rd + (1-Rd)*iota] * Ep * (1-sigma_f) * (theta-theta_ref)/(theta_s-theta_ref)

  Two things Session 2 got wrong by not having the paper:
  1. **The `[Rd + (1-Rd)*iota]` radiative prefactor was missing entirely.**
     Rd=0.15 is the paper's diffuse-light fraction; iota is the Eq. 6 solar
     view factor (physics/radiation.py). This is now implemented as
     `radiative_factor()`.
  2. Session 2 *replaced* the paper's `(theta-theta_ref)/(theta_s-theta_ref)`
     ratio with the Ek-2003 `(theta-theta_wilt)/(theta_ref-theta_wilt)` form,
     reasoning (from docs/trail-conditions-design.md, written without
     access to the paper) that the printed form must be a transcription
     error since it "does not match the standard Noah formulation and gives
     wrong sign behaviour below field capacity." Having now read the actual
     paper: **it really does print `(theta-theta_ref)/(theta_s-theta_ref)`,
     verbatim, with no clipping mentioned.** Whether that's intentional
     (GeoWATCH's own designed departure from Ek 2003) or an uncaught error
     in the source paper itself is unresolvable from the paper text alone.

     Per this session's decision: **both forms are implemented**,
     selectable via `form=`. `"ek2003"` (Session 2's substitution) remains
     the default since it's provably well-behaved (bounded [0,1], monotonic,
     no sign flip below field capacity); `"geowatch"` is the paper-as-printed
     form, unclipped, for the Tarrawarra harness to test empirically against
     both the 0.0321 (TDR) and 0.030 (NMM) published targets. Whichever
     form actually reproduces the published numbers is the answer to which
     one GeoWATCH really ran -- an empirical question, not an editorial one.

- **Eq. 3**: `F(theta) = -Et(theta) - Edir(theta)` -- a sign convention
  (positive Et/Edir = water removed from the soil, consistent with how
  evapotranspiration is normally expressed; `f_theta()` below applies the
  paper's negation for use directly in Eq. 2/7, see relaxation.py).
"""

from __future__ import annotations

import numpy as np

DEFAULT_RD = 0.15  # per the paper Eq. 5: "default value of 0.15"


def radiative_factor(iota: np.ndarray | float, rd: float = DEFAULT_RD) -> np.ndarray | float:
    """Eq. 5's `[Rd + (1-Rd)*iota]` prefactor -- the fraction of potential
    evaporation actually available given diffuse vs. direct-beam radiative
    exposure. Rd (diffuse fraction, not subject to the solar view factor)
    sets the floor: even a fully self-shadowed point (iota=0) still gets
    `Rd` of potential evaporation from diffuse sky radiation."""
    return rd + (1.0 - rd) * iota


def soil_moisture_stress_factor(
    theta: np.ndarray, theta_wilt: np.ndarray | float, theta_ref: np.ndarray | float
) -> np.ndarray:
    """The Ek et al. (2003)/Noah LSM stress function:
    beta(theta) = clip((theta - theta_wilt) / (theta_ref - theta_wilt), 0, 1).
    Used by Eq. 4 (confirmed exact paper match) and by Eq. 5's `form="ek2003"`
    option (Session 2's substitution for the paper's printed ratio)."""
    denom = theta_ref - theta_wilt
    # Guard degenerate soils where theta_ref == theta_wilt (shouldn't occur
    # with real SSURGO data, but a stray zero must not produce a NaN/inf
    # that silently propagates through the whole flux-correction chain).
    denom = np.where(np.abs(np.asarray(denom)) < 1e-9, 1e-9, denom)
    beta = (theta - theta_wilt) / denom
    return np.clip(beta, 0.0, 1.0)


def geowatch_soil_moisture_ratio(
    theta: np.ndarray, theta_ref: np.ndarray | float, theta_s: np.ndarray | float
) -> np.ndarray:
    """Eq. 5 AS PRINTED in Eylander et al. (2023): (theta-theta_ref)/(theta_s-theta_ref).

    Deliberately NOT clipped to [0,1] -- the paper doesn't mention clipping
    this ratio, and clipping it would be adding physics the paper doesn't
    state. This means the ratio CAN go negative when theta < theta_ref
    (below field capacity) or exceed 1 when theta > theta_s (which
    shouldn't happen physically but isn't guarded against here either,
    matching the paper). This is exactly the "wrong sign behaviour below
    field capacity" docs/trail-conditions-design.md flagged sight-unseen --
    now confirmed to be what the paper actually prints, not a transcription
    error introduced somewhere between the paper and the design doc.
    """
    denom = theta_s - theta_ref
    denom = np.where(np.abs(np.asarray(denom)) < 1e-9, 1e-9, denom)
    return (theta - theta_ref) / denom


def direct_soil_evaporation(
    pet: np.ndarray,
    theta: np.ndarray,
    theta_wilt: np.ndarray | float,
    theta_ref: np.ndarray | float,
    theta_s: np.ndarray | float,
    green_veg_fraction: np.ndarray | float,
    iota: np.ndarray | float = 1.0,
    rd: float = DEFAULT_RD,
    form: str = "ek2003",
) -> np.ndarray:
    """Eq. 5 -- direct (bare-soil-fraction) evaporation flux, with the
    radiative prefactor now included (see module docstring for what
    Session 2 was missing).

    `theta_s` (local saturated soil moisture) is required in the signature
    now even for `form="ek2003"` (which doesn't use it) so callers -- and
    especially relaxation.py's Eq. 7, which evaluates F at theta_s -- don't
    need two different call conventions depending on which form is active.

    `iota` defaults to 1.0 (full solar exposure, i.e. the radiative
    prefactor reduces to 1.0 regardless of `rd`) for callers that haven't
    computed Eq. 6 yet -- NOT a physics claim, just a safe default matching
    "assume no self-shading until told otherwise."
    """
    factor = radiative_factor(iota, rd)
    if form == "ek2003":
        beta = soil_moisture_stress_factor(theta, theta_wilt, theta_ref)
    elif form == "geowatch":
        beta = geowatch_soil_moisture_ratio(theta, theta_ref, theta_s)
    else:
        raise ValueError(f"Unknown form {form!r}, expected 'ek2003' or 'geowatch'")
    return factor * pet * (1.0 - green_veg_fraction) * beta


def vegetation_transpiration(
    pet: np.ndarray,
    theta: np.ndarray,
    theta_wilt: np.ndarray | float,
    theta_ref: np.ndarray | float,
    green_veg_fraction: np.ndarray | float,
) -> np.ndarray:
    """Eq. 4 -- vegetated-fraction transpiration flux. Confirmed (Session 3,
    with the actual paper in hand) to be an EXACT match to what Session 2
    already implemented from the independently-published Ek et al. (2003)/
    Chen et al. (1996) formulation -- no change needed."""
    beta = soil_moisture_stress_factor(theta, theta_wilt, theta_ref)
    return green_veg_fraction * pet * beta


def total_actual_et(
    pet: np.ndarray,
    theta: np.ndarray,
    theta_wilt: np.ndarray | float,
    theta_ref: np.ndarray | float,
    theta_s: np.ndarray | float,
    green_veg_fraction: np.ndarray | float,
    iota: np.ndarray | float = 1.0,
    rd: float = DEFAULT_RD,
    form: str = "ek2003",
) -> np.ndarray:
    """Et(theta) + Edir(theta): combined actual ET flux (positive = water
    removed from the soil)."""
    return vegetation_transpiration(
        pet, theta, theta_wilt, theta_ref, green_veg_fraction
    ) + direct_soil_evaporation(
        pet, theta, theta_wilt, theta_ref, theta_s, green_veg_fraction, iota, rd, form
    )


def f_theta(
    pet: np.ndarray,
    theta: np.ndarray,
    theta_wilt: np.ndarray | float,
    theta_ref: np.ndarray | float,
    theta_s: np.ndarray | float,
    green_veg_fraction: np.ndarray | float,
    iota: np.ndarray | float = 1.0,
    rd: float = DEFAULT_RD,
    form: str = "ek2003",
) -> np.ndarray:
    """Eq. 3: F(theta) = -Et(theta) - Edir(theta) -- the signed flux Eq. 2
    and Eq. 7 (relaxation.py) actually use. Negative = net drying."""
    return -total_actual_et(
        pet, theta, theta_wilt, theta_ref, theta_s, green_veg_fraction, iota, rd, form
    )
