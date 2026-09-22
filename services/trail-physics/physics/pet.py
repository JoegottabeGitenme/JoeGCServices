"""Potential evapotranspiration via FAO-56 Penman-Monteith (Allen et al.
1998, "Crop evapotranspiration - Guidelines for computing crop water
requirements", FAO Irrigation and Drainage Paper 56 -- freely published by
the FAO, not paywalled, and the de facto standard reference-ET equation).

Needed because HRRR does not export a usable PEVPR field (confirmed absent
from wrfsfc/wrfprs/wrfnat during the ingredients-fetch session -- see
docs/trail-conditions-design.md's amendment table) but does export every
input Penman-Monteith needs: 2 m T/Q, 10 m wind, DSWRF, DLWRF, surface
pressure.

Uses the direct energy-balance form of net radiation (Rn = net shortwave +
net longwave, both computed straight from HRRR's DSWRF/DLWRF) rather than
FAO-56's daily clear-sky-radiation estimate, since we have observed downward
longwave directly -- a more direct measurement than FAO-56's own fallback
for stations that lack it.
"""

from __future__ import annotations

import numpy as np

STEFAN_BOLTZMANN = 5.670374e-8  # W/(m^2 K^4)


def saturation_vapor_pressure_kpa(temp_k: np.ndarray) -> np.ndarray:
    """Saturation vapor pressure (kPa) from temperature (Kelvin), Tetens'
    formula as used in FAO-56 Eq. 11."""
    temp_c = temp_k - 273.15
    return 0.6108 * np.exp((17.27 * temp_c) / (temp_c + 237.3))


def vapor_pressure_slope_kpa_per_c(temp_k: np.ndarray) -> np.ndarray:
    """Slope of the saturation vapor pressure curve (Delta, kPa/degC),
    FAO-56 Eq. 13."""
    temp_c = temp_k - 273.15
    es = saturation_vapor_pressure_kpa(temp_k)
    return (4098.0 * es) / ((temp_c + 237.3) ** 2)


def psychrometric_constant_kpa_per_c(pressure_pa: np.ndarray) -> np.ndarray:
    """Psychrometric constant (gamma, kPa/degC), FAO-56 Eq. 8."""
    pressure_kpa = pressure_pa / 1000.0
    return 0.000665 * pressure_kpa


def wind_speed_2m(wind_speed_10m: np.ndarray) -> np.ndarray:
    """Log-wind-profile adjustment from 10 m (HRRR's native height) to 2 m
    (the FAO-56 reference height), FAO-56 Eq. 47 generalized:
    u_z = u_10 * 4.87 / ln(67.8*10 - 5.42)."""
    return wind_speed_10m * 4.87 / np.log(67.8 * 10 - 5.42)


def net_radiation_w_m2(
    dswrf: np.ndarray,
    dlwrf: np.ndarray,
    surface_temp_k: np.ndarray,
    albedo: float = 0.23,
    emissivity: float = 0.98,
) -> np.ndarray:
    """Net radiation (W/m^2) from HRRR's directly-observed downward
    shortwave and longwave fluxes, rather than FAO-56's clear-sky estimate.

    albedo=0.23 is FAO-56's standard reference-grass value; emissivity=0.98
    is a typical value for vegetated/soil surfaces.
    """
    net_shortwave = (1.0 - albedo) * dswrf
    outgoing_longwave = emissivity * STEFAN_BOLTZMANN * surface_temp_k**4
    net_longwave = emissivity * dlwrf - outgoing_longwave
    return net_shortwave + net_longwave


def hourly_reference_et_mm(
    temp_k: np.ndarray,
    spfh: np.ndarray,
    pressure_pa: np.ndarray,
    wind_speed_10m: np.ndarray,
    net_radiation_w_m2_: np.ndarray,
    soil_heat_flux_w_m2: np.ndarray | float = 0.0,
) -> np.ndarray:
    """Hourly reference evapotranspiration (mm/hour), FAO-56's hourly
    Penman-Monteith form (FAO-56 Ch. 4, "Calculation procedures ... hourly
    or shorter periods").

    Args:
        temp_k: 2 m air temperature (Kelvin, HRRR TMP).
        spfh: 2 m specific humidity (kg/kg, HRRR SPFH) -- converted to
            actual vapor pressure via `pressure_pa`.
        pressure_pa: surface pressure (Pa, HRRR PRES).
        wind_speed_10m: 10 m wind speed magnitude (m/s, from HRRR UGRD/VGRD).
        net_radiation_w_m2_: from `net_radiation_w_m2`.
        soil_heat_flux_w_m2: FAO-56 recommends G ~= 0.1*Rn during daytime,
            ~= 0.5*Rn at night for hourly steps; pass 0.0 for a conservative
            simplification (small term relative to Rn) unless a day/night
            split is threaded through by the caller.

    Returns:
        Reference ET in mm for the hour (a rate, not an accumulation --
        multiply by hours to accumulate).
    """
    delta = vapor_pressure_slope_kpa_per_c(temp_k)
    gamma = psychrometric_constant_kpa_per_c(pressure_pa)
    u2 = wind_speed_2m(wind_speed_10m)

    es = saturation_vapor_pressure_kpa(temp_k)
    # Actual vapor pressure from specific humidity: e = q*p / (0.622 + 0.378*q)
    ea = (spfh * (pressure_pa / 1000.0)) / (0.622 + 0.378 * spfh)

    temp_c = temp_k - 273.15
    # Net radiation and soil heat flux in MJ/(m^2 hr): 1 W/m^2 = 0.0036 MJ/(m^2 hr)
    rn_mj = net_radiation_w_m2_ * 0.0036
    g_mj = np.asarray(soil_heat_flux_w_m2) * 0.0036

    numerator = 0.408 * delta * (rn_mj - g_mj) + gamma * (37.0 / (temp_c + 273.0)) * u2 * (
        es - ea
    )
    denominator = delta + gamma * (1.0 + 0.34 * u2)

    et0 = numerator / denominator
    return np.maximum(et0, 0.0)  # ET can't be negative (condensation is a separate term)
