"""Snow-lite (S2/S3, scoped per this project's Q3 decision): canopy
interception + enhanced temperature-radiation-index melt, WITHOUT
Winstral Sx wind redistribution.

Rationale (from the session that made this call): Front Range trail miles
are overwhelmingly below treeline where "canopy and radiation dominate;
wind redistribution matters less than in alpine terrain" (the design doc's
own words). Sx's 16-azimuth static layers are still computed in WS1 (nearly
free at preprocessing time) so they exist as a diagnostic, but wind-driven
mass redistribution is deferred behind an explicit evidence gate: run
snow-lite, validate against Landsat/Sentinel-2 snow-disappearance dates
(Rung 3), and stratify the residuals by wind exposure. If errors correlate
with exposure, that's the data justifying the extra work; if not, snow-lite
was the right call and the work was saved.

S4 (the frozen-ground infiltration gate) is included here since it's a
simple, cheap function of the same TSOIL data snow-lite already needs
-- not the full S4 design (soil ice fraction, which HRRR doesn't export;
see the design doc's amendment table), just the fallback proxy: melt/rain
arriving when TSOIL <= 273.15K at the shallow depth does not infiltrate.
"""

from __future__ import annotations

import numpy as np

FREEZING_POINT_K = 273.15


def canopy_adjusted_swe(swe_open_mm: np.ndarray, canopy_fraction: np.ndarray) -> np.ndarray:
    """Reduce open-ground SWE (HRRR's WEASD) by canopy interception loss.

    `canopy_fraction` is NLCD tree-canopy density (0-1), from the WS1
    static stack (not yet built this session -- pass a zeros array to
    disable this adjustment until it exists). The design doc cites
    "Colorado subalpine losses commonly ~30-40% of seasonal snowfall" for
    forest canopy; this uses that range's midpoint (35%) as a linear
    scaling with canopy density, at canopy_fraction=1.0 (fully closed
    canopy). This is a simplification (real interception depends on storm
    intensity, canopy type, temperature) but matches the doc's own stated
    magnitude, not an invented number.
    """
    max_interception_loss_fraction = 0.35
    loss_fraction = canopy_fraction * max_interception_loss_fraction
    return swe_open_mm * (1.0 - loss_fraction)


def is_frozen_ground(tsoil_k: np.ndarray, threshold_k: float = FREEZING_POINT_K) -> np.ndarray:
    """S4 fallback proxy: HRRR has no soil-ice-fraction field (confirmed
    absent from wrfsfc/wrfprs/wrfnat during the ingredients session), so a
    binary TSOIL<=0degC gate stands in for the real infiltration-gate
    physics. Coarser than a true ice fraction -- see the design doc's
    amendment table for why."""
    return tsoil_k <= threshold_k


def route_water_input(
    rain_mm: np.ndarray, melt_mm: np.ndarray, frozen: np.ndarray
) -> tuple[np.ndarray, np.ndarray]:
    """S4: split combined rain+melt water input into infiltration vs.
    runoff/ponding, gated by frozen ground. Returns (infiltration_mm,
    runoff_mm). On thawed ground, everything infiltrates (the redistribution/
    relaxation physics upstream handles moisture routing); on frozen
    ground, nothing infiltrates -- it ponds or runs off, matching the
    design doc's "wet-surface-over-frozen-substrate" state description."""
    total_input = rain_mm + melt_mm
    infiltration = np.where(frozen, 0.0, total_input)
    runoff = np.where(frozen, total_input, 0.0)
    return infiltration, runoff


def enhanced_temperature_index_melt(
    temp_k: np.ndarray,
    terrain_corrected_shortwave_w_m2: np.ndarray,
    swe_mm: np.ndarray,
    melt_temp_factor_mm_per_c_per_hr: float = 0.15,
    melt_radiation_factor_mm_per_w_m2_per_hr: float = 0.0006,
    base_temp_k: float = FREEZING_POINT_K,
) -> np.ndarray:
    """Enhanced temperature-index melt (temperature-index + a radiation
    term using the sky-view/aspect-corrected shortwave from
    physics/radiation.py) -- the design doc's S3: "aspect drives melt-out
    date differences of weeks at the same elevation."

    Coefficients are typical literature-range temperature-index melt
    factors (commonly 1-5 mm/degC/day => ~0.04-0.2 mm/degC/hr) and a small
    radiation-melt factor; tunable, not claimed as calibrated for Colorado
    without Rung 3 (snow-disappearance-date) validation.

    Melt is capped at the available SWE -- you cannot melt snow that
    isn't there.
    """
    temp_excess = np.maximum(temp_k - base_temp_k, 0.0)
    radiation_component = np.maximum(terrain_corrected_shortwave_w_m2, 0.0)

    potential_melt = (
        melt_temp_factor_mm_per_c_per_hr * temp_excess
        + melt_radiation_factor_mm_per_w_m2_per_hr * radiation_component
    )
    # No melt at or below freezing regardless of radiation input (radiation
    # alone doesn't melt subfreezing snowpack in this simplified scheme).
    potential_melt = np.where(temp_k > base_temp_k, potential_melt, 0.0)

    return np.minimum(potential_melt, swe_mm)
