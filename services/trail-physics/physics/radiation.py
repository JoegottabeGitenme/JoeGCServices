"""Eq. 6 -- terrain-corrected shortwave radiation (solar/sky-view factor
correction).

Solar position (declination, hour angle, zenith/azimuth) follows the
standard solar-engineering formulas (Cooper 1969 declination approximation;
e.g. as presented in Duffie & Beckman, "Solar Engineering of Thermal
Processes" -- a textbook reference, not paywalled/proprietary). The
terrain-incidence and sky-view-factor combination follows the standard
direct+diffuse split used throughout terrain radiation-correction
literature (comparable to Dozier & Frew 1990's approach) -- this is
well-established methodology, distinct from the design doc's flagged-as-
unverified Eq. 2/7 (see relaxation.py).

This is what aspect-driven melt-out date differences (the design doc's S3,
"aspect drives melt-out date differences of weeks at the same elevation")
are actually computed from.
"""

from __future__ import annotations

import numpy as np


def solar_declination_deg(day_of_year: int) -> float:
    """Cooper (1969) approximation, degrees."""
    return 23.45 * np.sin(np.radians(360.0 * (284 + day_of_year) / 365.0))


def equation_of_time_minutes(day_of_year: int) -> float:
    """Spencer (1971) Fourier-series approximation, minutes."""
    b = np.radians(360.0 * (day_of_year - 81) / 364.0)
    return 9.87 * np.sin(2 * b) - 7.53 * np.cos(b) - 1.5 * np.sin(b)


def solar_position(
    lat_deg: float, lon_deg: float, day_of_year: int, utc_hour: float
) -> tuple[float, float]:
    """Solar zenith and azimuth angles (degrees) for a given location and UTC time.

    Azimuth follows the 0=north, clockwise convention (matching
    terrain.compute_aspect) so the two can be compared directly.
    """
    declination = np.radians(solar_declination_deg(day_of_year))
    eot = equation_of_time_minutes(day_of_year)

    solar_time = utc_hour + (lon_deg / 15.0) + (eot / 60.0)
    hour_angle = np.radians(15.0 * (solar_time - 12.0))

    lat_rad = np.radians(lat_deg)

    cos_zenith = np.sin(lat_rad) * np.sin(declination) + np.cos(lat_rad) * np.cos(
        declination
    ) * np.cos(hour_angle)
    cos_zenith = np.clip(cos_zenith, -1.0, 1.0)
    zenith = np.degrees(np.arccos(cos_zenith))

    sin_azimuth_num = -np.cos(declination) * np.sin(hour_angle)
    cos_azimuth_den = np.cos(np.radians(zenith))
    # Standard solar azimuth formula (0=north, clockwise); guards div-by-zero
    # at the poles of the local zenith angle (not a concern for CO/AU
    # latitudes but cheap to guard).
    sin_zenith = np.sin(np.radians(zenith))
    if abs(sin_zenith) < 1e-6:
        azimuth = 180.0
    else:
        cos_azimuth = (
            np.sin(declination) - np.sin(lat_rad) * cos_azimuth_den
        ) / (np.cos(lat_rad) * sin_zenith)
        cos_azimuth = np.clip(cos_azimuth, -1.0, 1.0)
        azimuth = np.degrees(np.arccos(cos_azimuth))
        if hour_angle > 0:
            azimuth = 360.0 - azimuth

    return zenith, azimuth


def local_incidence_cosine(
    zenith_deg: float, azimuth_deg: float, slope_tan: np.ndarray, aspect_deg: np.ndarray
) -> np.ndarray:
    """cos(theta_local), the cosine of the angle between the sun and the
    local surface normal -- the direct-beam terrain correction factor.
    Negative values (self-shadowed, sun below the local horizon) are
    clipped to zero by the caller, not here, so callers needing the raw
    signed value (e.g. for diagnostics) still can.
    """
    slope_rad = np.arctan(slope_tan)
    zenith_rad = np.radians(zenith_deg)
    aspect_rad = np.radians(aspect_deg)
    sun_azimuth_rad = np.radians(azimuth_deg)

    cos_theta = np.cos(slope_rad) * np.cos(zenith_rad) + np.sin(slope_rad) * np.sin(
        zenith_rad
    ) * np.cos(sun_azimuth_rad - aspect_rad)
    return cos_theta


def terrain_corrected_shortwave(
    dswrf_flat: np.ndarray,
    zenith_deg: float,
    azimuth_deg: float,
    slope_tan: np.ndarray,
    aspect_deg: np.ndarray,
    sky_view_factor: np.ndarray,
    diffuse_fraction: float = 0.2,
) -> np.ndarray:
    """Split HRRR's flat-terrain DSWRF into direct+diffuse components and
    recombine with terrain-specific direct-beam incidence and diffuse
    sky-view scaling.

    `diffuse_fraction=0.2` is a common clear-sky default (e.g. Iqbal 1983);
    a full implementation would derive this per-timestep from HRRR's
    cloud-cover fields rather than a constant -- noted as a simplification,
    not a physics claim.
    """
    direct_flat = dswrf_flat * (1.0 - diffuse_fraction)
    diffuse_flat = dswrf_flat * diffuse_fraction

    zenith_rad = np.radians(zenith_deg)
    cos_zenith = np.cos(zenith_rad)
    if cos_zenith <= 0.0:
        # Sun below the horizon: no direct beam anywhere, only (attenuated)
        # diffuse. Avoids dividing by ~0 in the direct-beam ratio below.
        return diffuse_flat * sky_view_factor

    cos_local = local_incidence_cosine(zenith_deg, azimuth_deg, slope_tan, aspect_deg)
    direct_ratio = np.clip(cos_local / cos_zenith, 0.0, None)  # self-shadowed -> 0

    direct_local = direct_flat * direct_ratio
    diffuse_local = diffuse_flat * sky_view_factor
    return direct_local + diffuse_local
