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


# =============================================================================
# Daily-timestep FAO-56 Penman-Monteith (Chapters 3-4). Session 7 addition,
# needed for the Tarrawarra validation harness (daily.met provides daily
# summaries, not hourly HRRR-style grids -- a genuinely different
# calculation path from `hourly_reference_et_mm` above, not a duplicate:
# different soil-heat-flux convention (G=0 for daily vs. threaded through
# for hourly), different numerator constant (900 vs. 37), and actual vapor
# pressure derived from wet-bulb depression or daily min/max, not specific
# humidity.
#
# Every equation number below (Eq. 6-40) refers to Allen et al. (1998),
# fetched live and cross-checked against two of the paper's OWN fully
# worked numerical examples (Example 4: wet-bulb psychrometric ea at
# 1200m elevation -> 1.91 kPa; Example 18: full daily ETo calculation for
# Brussels, 6 July -> 3.9 mm/day) -- see test_pet.py, both reproduced
# exactly.
# =============================================================================

STANDARD_PSYCHROMETER_COEFFICIENT = 0.000662  # /degC, ventilated (Assmann-type), ~5 m/s air movement -- FAO-56 Eq. 16


def atmospheric_pressure_kpa(elevation_m: float) -> float:
    """FAO-56 Eq. 7: standard atmospheric pressure from station elevation
    (assumes a standard atmosphere at 20degC -- FAO-56's own stated
    simplification, not an approximation introduced here)."""
    return 101.3 * ((293.0 - 0.0065 * elevation_m) / 293.0) ** 5.26


def actual_vapor_pressure_from_wetbulb_kpa(
    dry_bulb_c: np.ndarray,
    wet_bulb_c: np.ndarray,
    pressure_kpa: float,
    psychrometer_coefficient: float = STANDARD_PSYCHROMETER_COEFFICIENT,
) -> np.ndarray:
    """FAO-56 Eq. 15/16: actual vapor pressure from a wet/dry-bulb
    (psychrometer) pair, as Tarrawarra's daily.met provides (not relative
    humidity or dewpoint). `psychrometer_coefficient` defaults to FAO-56's
    "ventilated (Assmann type)" value (0.000662/degC) -- Readme.met doesn't
    state the instrument's exact ventilation, so this is a documented
    assumption for a standard automatic weather station, not a verified
    fact about Tarrawarra's specific psychrometer. The other two FAO-56
    options are 0.000800 (naturally ventilated) and 0.001200
    (non-ventilated, indoor) -- swap via the parameter if this assumption
    is ever revisited.
    """
    es_wet = saturation_vapor_pressure_kpa(wet_bulb_c + 273.15)
    gamma_psy = psychrometer_coefficient * pressure_kpa
    return es_wet - gamma_psy * (dry_bulb_c - wet_bulb_c)


def extraterrestrial_radiation_mj_m2_day(lat_deg: float, day_of_year: int) -> float:
    """FAO-56 Eq. 21/23/24/25: daily extraterrestrial radiation Ra. Positive
    latitude = northern hemisphere; Tarrawarra (37.65 S) must be passed as
    a negative value, per FAO-56's own explicit sign convention (Example 7
    in the primary source)."""
    lat_rad = np.pi / 180.0 * lat_deg
    j = day_of_year
    dr = 1.0 + 0.033 * np.cos(2 * np.pi * j / 365.0)
    solar_declination = 0.409 * np.sin(2 * np.pi * j / 365.0 - 1.39)
    # Sunset hour angle (Eq. 25). Clip the arccos argument to [-1, 1] --
    # near the poles in midsummer/midwinter it can drift fractionally
    # outside that range due to floating point, which would otherwise
    # raise instead of correctly saturating at a 24h or 0h day.
    cos_ws = -np.tan(lat_rad) * np.tan(solar_declination)
    ws = np.arccos(np.clip(cos_ws, -1.0, 1.0))
    gsc = 0.0820  # MJ/m^2/min, solar constant
    return (
        (24.0 * 60.0 / np.pi)
        * gsc
        * dr
        * (ws * np.sin(lat_rad) * np.sin(solar_declination) + np.cos(lat_rad) * np.cos(solar_declination) * np.sin(ws))
    )


def clear_sky_radiation_mj_m2_day(ra_mj_m2_day: float, elevation_m: float) -> float:
    """FAO-56 Eq. 37 (elevation-based form, used "when calibrated values
    for as and bs are not available" -- true here, no station-specific
    Angstrom calibration exists for Tarrawarra)."""
    return (0.75 + 2e-5 * elevation_m) * ra_mj_m2_day


def net_radiation_daily_mj_m2_day(
    rs_mj_m2_day: np.ndarray,
    tmax_c: np.ndarray,
    tmin_c: np.ndarray,
    ea_kpa: np.ndarray,
    ra_mj_m2_day: float,
    elevation_m: float,
    albedo: float = 0.23,
) -> np.ndarray:
    """FAO-56 Eq. 38-40: net radiation estimated from measured solar
    (shortwave) radiation, used only when net radiation isn't measured
    directly (Tarrawarra's own net-radiation sensor has real gaps, e.g. a
    known outage in Feb 1996 -- see validation/tarrawarra/README.md).
    `albedo=0.23` is FAO-56's reference-grass value; Tarrawarra's actual
    surface (grazed pasture) is close enough that FAO-56's own standard
    value is used rather than inventing a site-specific override.
    """
    rns = (1.0 - albedo) * rs_mj_m2_day
    rso = clear_sky_radiation_mj_m2_day(ra_mj_m2_day, elevation_m)
    rs_rso = np.minimum(rs_mj_m2_day / rso, 1.0)  # FAO-56: "must be limited so that Rs/Rso <= 1.0"
    sigma_daily = 4.903e-9  # MJ/(K^4 m^2 day), FAO-56's daily Stefan-Boltzmann constant
    tmax_k = tmax_c + 273.16
    tmin_k = tmin_c + 273.16
    rnl = (
        sigma_daily
        * (tmax_k**4 + tmin_k**4)
        / 2.0
        * (0.34 - 0.14 * np.sqrt(np.maximum(ea_kpa, 0.0)))
        * (1.35 * rs_rso - 0.35)
    )
    return rns - rnl


def daily_reference_et_fao56(
    tmax_c: np.ndarray,
    tmin_c: np.ndarray,
    ea_kpa: np.ndarray,
    wind_2m_m_s: np.ndarray,
    net_radiation_mj_m2_day: np.ndarray,
    elevation_m: float,
) -> np.ndarray:
    """FAO-56 Eq. 6: daily reference evapotranspiration. Soil heat flux G
    is set to 0 per FAO-56's own explicit convention for 24-hour time
    steps ("the magnitude of daily soil heat flux ... is relatively small
    ... it may be ignored for 24-hour time steps" -- confirmed against
    FAO-56's own worked Example 18, where G=0 exactly).

    Args:
        tmax_c/tmin_c: daily max/min air temperature (Celsius).
        ea_kpa: actual vapor pressure (kPa) -- see
            `actual_vapor_pressure_from_wetbulb_kpa`.
        wind_2m_m_s: wind speed AT 2m height, in m/s. Tarrawarra's own
            anemometer is already at 2m (per Readme.met's instrument
            table) -- do NOT apply `wind_speed_2m()`'s 10m->2m log-profile
            adjustment to this data; that adjustment is for HRRR's native
            10m wind only.
        net_radiation_mj_m2_day: measured directly, or from
            `net_radiation_daily_mj_m2_day` when unmeasured.
        elevation_m: station elevation, for atmospheric pressure (Eq. 7).

    Returns:
        Reference ET in mm/day.
    """
    tmean_c = (tmax_c + tmin_c) / 2.0
    pressure_kpa = atmospheric_pressure_kpa(elevation_m)

    delta = vapor_pressure_slope_kpa_per_c(tmean_c + 273.15)
    gamma = psychrometric_constant_kpa_per_c(pressure_kpa * 1000.0)

    es = (saturation_vapor_pressure_kpa(tmax_c + 273.15) + saturation_vapor_pressure_kpa(tmin_c + 273.15)) / 2.0

    # FAO-56's own Eq. 6 literally uses "+273" (not +273.15) in this one
    # specific term -- matched exactly here, not a rounding shortcut, so
    # this function reproduces the primary source's own worked example
    # (Example 18) to the decimal place; see test_pet.py.
    numerator = 0.408 * delta * net_radiation_mj_m2_day + gamma * (900.0 / (tmean_c + 273.0)) * wind_2m_m_s * (
        es - ea_kpa
    )
    denominator = delta + gamma * (1.0 + 0.34 * wind_2m_m_s)

    et0 = numerator / denominator
    return np.maximum(et0, 0.0)
