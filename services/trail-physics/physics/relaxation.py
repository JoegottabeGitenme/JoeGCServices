"""Eq. 2 / Eq. 7 -- the flux-difference correction and its timestep,
transcribed directly from Eylander et al. (2023) Section 2.2.2 (user-
supplied copy, Session 3). Session 2 had no access to the paper for this
equation and built a physically-motivated but ultimately WRONG
reconstruction (an exponential anomaly decay) -- replaced entirely below.

**Eq. 7** (the timestep):

    delta_t = delta_ts * (theta_ws - theta_ref) / F(theta_s)

    delta_ts = Cts * { 1                                if theta* < theta_s
                        exp(-(theta* - theta_s)/theta_s) if theta* >= theta_s }

    Cts = 0.1 (the paper's calibration coefficient)

delta_t is clipped to [0, 30] days. The design doc (written without the
paper) called this expression "mangled in the text" -- having read it, it
isn't mangled at all: the two-branch `delta_ts` is a straightforward
piecewise/indicator-function expression once written out plainly, which
Session 2's reconstruction (`Cts / (Rd * |flux_anomaly| + eps)`) got
completely wrong in both form and which named constant does what. Rd
(0.15) is NOT part of Eq. 7 at all -- it's Eq. 5's diffuse-light fraction
(see flux.py). Session 2 conflated the two.

Physical reading of Eq. 7, confirmed against the paper's own text: "Delta_t
is the time required for the soil moisture to drop from field capacity
(theta_ref) to the weather-scale soil moisture value theta_ws," using
F(theta_s) -- the flux evaluated AT SATURATION, i.e. the fastest possible
drying rate -- as a reference rate. This is a linear-rate time-to-dry
estimate, not a physical time-integration in the ODE sense.

**Eq. 2** (the correction itself):

    theta = theta* + delta_t * (F(theta*) - F'(theta_ws))

F(theta*) is evaluated with FINE-resolution soil/vegetation properties at
the point being corrected. F'(theta_ws) is evaluated with WEATHER-SCALE-
AVERAGED soil/vegetation properties at the coarse state -- the paper is
explicit about this ("the weather-scale flux terms use soil properties
averaged to weather-scale resolution soil"), which is exactly the
parameter-matching discipline docs/trail-conditions-design.md Section 5.1
already flagged as critical (mixing a fine-resolution state with
coarse-resolution parameters, or vice versa, silently corrupts the
correction). `SoilProperties` below exists specifically so a caller cannot
accidentally pass one properties bundle where two distinct ones (fine vs.
coarse-averaged) are required.

**Units -- resolved by dimensional analysis (Session 7), value still an open
calibration question.** For `delta_t = delta_ts * (theta_ws-theta_ref) /
F(theta_s)` to actually come out in days (matching the paper's own explicit
"clipped between 0 and ... 30 days"), F(theta_s) must have units of
[1/day] -- a fractional-soil-moisture-per-day rate -- not a depth-per-time
rate like FAO-56's mm/day. Passing `Ep` in mm/day directly (as
`services/trail-physics/physics/pet.py` naturally produces) would silently
give `delta_t` units of day/mm instead of days; the two Eq. 2 flux terms
happen to still cancel those units correctly in the final correction
(`delta_t [day/mm] * F_diff [mm/day]` = dimensionless), so the bug would
NOT show up as an obviously-wrong result -- only as a `delta_t` whose
*numerical value* has been implicitly calibrated for depth=1mm, an
arbitrary and almost certainly wrong choice, especially since it's the
[0,30]-day clip that determines whether corrections ever saturate.

**The fix**: convert `Ep` from mm/day to fraction/day by dividing by an
assumed active/root-zone depth in mm before calling any function in this
module (`ep_fraction_per_day = ep_mm_per_day / depth_mm`) -- a standard
bucket-model unit conversion, not a hack. The depth ITSELF remains a real
calibration unknown the paper doesn't state; validation/tarrawarra's
Stage-2 harness sweeps it (300mm primary, matching TDR's own 30cm
measurement depth, with 150/1000mm as sensitivity bounds), same footing as
k=13 (Eq. 1) -- something only empirical reproduction against Tarrawarra
can pin down, not something to silently assume a value for here.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from . import flux

DEFAULT_CTS = 0.1  # per the paper Eq. 7: "Cts = 0.1 is a calibration coefficient"
MAX_RELAXATION_DAYS = 30.0  # per the paper: "clipped between 0 and a maximum allowed value (e.g. 30 days)"


@dataclass
class SoilProperties:
    """A bundle of the soil/vegetation properties Eq. 2/5 need, kept
    together specifically so a caller can't accidentally mix a fine-
    resolution property with a coarse-resolution one -- see this module's
    docstring on the parameter-matching requirement."""

    theta_wilt: np.ndarray | float
    theta_ref: np.ndarray | float
    theta_s: np.ndarray | float
    green_veg_fraction: np.ndarray | float
    iota: np.ndarray | float = 1.0  # Eq. 6 solar view factor; 1.0 = no self-shading info yet


def delta_ts(
    theta_star: np.ndarray, theta_s: np.ndarray | float, cts: float = DEFAULT_CTS
) -> np.ndarray:
    """The piecewise indicator-function part of Eq. 7."""
    theta_star = np.asarray(theta_star, dtype=np.float64)
    theta_s = np.asarray(theta_s, dtype=np.float64)
    below_saturation = theta_star < theta_s
    decay = np.exp(-(theta_star - theta_s) / theta_s)
    return cts * np.where(below_saturation, 1.0, decay)


def compute_delta_t(
    theta_star: np.ndarray,
    theta_ws: float,
    theta_ref: np.ndarray | float,
    theta_s: np.ndarray | float,
    ep: np.ndarray | float,
    fine_props: SoilProperties,
    cts: float = DEFAULT_CTS,
    max_days: float = MAX_RELAXATION_DAYS,
    form: str = "ek2003",
) -> np.ndarray:
    """Eq. 7. `F(theta_s)` -- the flux at saturation -- uses the FINE-
    resolution properties (`fine_props`), evaluated at `theta_s` itself
    (not at theta*), per the paper's literal `F(theta_s)` notation.

    Discovered property, worth knowing rather than being surprised by:
    whenever theta_s > theta_ref (the physically normal case), BOTH Eq. 5
    forms give beta/ratio = 1.0 exactly at theta=theta_s -- "ek2003" clips
    its ratio to 1 there (since (theta_s-theta_wilt)/(theta_ref-theta_wilt)
    exceeds 1 whenever theta_s>theta_ref), and "geowatch"'s ratio is
    trivially 1 by construction (numerator equals denominator when
    theta=theta_s). This means `compute_delta_t`'s result is form-
    independent in the normal case -- the two Eq. 5 forms only diverge in
    `apply_flux_correction`, where F is evaluated at theta* and theta_ws
    (generally not at saturation). Not a bug; a consequence of both forms
    agreeing at the boundary condition theta=theta_s by construction.
    """
    f_at_saturation = flux.f_theta(
        ep,
        theta_s,
        fine_props.theta_wilt,
        theta_ref,
        theta_s,
        fine_props.green_veg_fraction,
        fine_props.iota,
        flux.DEFAULT_RD,
        form,
    )
    dts = delta_ts(theta_star, theta_s, cts)
    # f_at_saturation is <= 0 (drying) in the normal case; guard the
    # degenerate f_at_saturation == 0 case (e.g. zero PET, night-time)
    # rather than dividing by exactly zero.
    f_safe = np.where(np.abs(np.asarray(f_at_saturation)) < 1e-12, -1e-12, f_at_saturation)
    dt_days = dts * (theta_ws - theta_ref) / f_safe
    return np.clip(dt_days, 0.0, max_days)


def apply_flux_correction(
    theta_star: np.ndarray,
    theta_ws: float,
    theta_ref_fine: np.ndarray | float,
    theta_s_fine: np.ndarray | float,
    ep_fine: np.ndarray | float,
    fine_props: SoilProperties,
    theta_ref_coarse: float,
    theta_s_coarse: float,
    ep_coarse: float,
    coarse_props: SoilProperties,
    delta_t_days: np.ndarray,
    form: str = "ek2003",
) -> np.ndarray:
    """Eq. 2: theta = theta* + delta_t * (F(theta*) - F'(theta_ws)).

    F(theta*) uses `fine_props` (this point's own soil/vegetation
    properties). F'(theta_ws) uses `coarse_props` -- the weather-scale-
    AVERAGED properties, per the paper's explicit statement that the
    weather-scale flux term uses weather-scale-averaged soil properties.
    Passing the same `SoilProperties` instance for both is almost always
    wrong; the two are kept as separate required arguments (not one
    optional one) so that mistake is a visible call-site choice, not a
    silent default.
    """
    f_fine = flux.f_theta(
        ep_fine,
        theta_star,
        fine_props.theta_wilt,
        theta_ref_fine,
        theta_s_fine,
        fine_props.green_veg_fraction,
        fine_props.iota,
        flux.DEFAULT_RD,
        form,
    )
    f_coarse = flux.f_theta(
        ep_coarse,
        theta_ws,
        coarse_props.theta_wilt,
        theta_ref_coarse,
        theta_s_coarse,
        coarse_props.green_veg_fraction,
        coarse_props.iota,
        flux.DEFAULT_RD,
        form,
    )
    return theta_star + delta_t_days * (f_fine - f_coarse)
