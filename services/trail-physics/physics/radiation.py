"""Eq. 6 -- terrain-corrected shortwave radiation and the solar view factor.

Solar position (declination, hour angle, zenith/azimuth) follows the
standard solar-engineering formulas (Cooper 1969 declination approximation;
e.g. as presented in Duffie & Beckman, "Solar Engineering of Thermal
Processes" -- a textbook reference, not paywalled/proprietary). The
terrain-incidence and sky-view-factor combination follows the standard
direct+diffuse split used throughout terrain radiation-correction
literature (comparable to Dozier & Frew 1990's approach) -- this is
well-established methodology, independent of the paper this module's
`solar_view_factor()` function (added Session 3) directly implements.

This is what aspect-driven melt-out date differences (the design doc's S3,
"aspect drives melt-out date differences of weeks at the same elevation")
are actually computed from.

Session 3: added `solar_view_factor()`, a direct implementation of
Eylander et al. (2023) Eq. 6 (user-supplied copy; Session 2 had no access
to the paper for this equation and Eq. 6 was entirely unimplemented).
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


def solar_view_factor(
    lat_deg: float,
    lon_deg: float,
    day_of_year: int,
    slope_tan: np.ndarray,
    aspect_deg: np.ndarray,
    n_samples: int = 145,
) -> np.ndarray:
    """Eq. 6, transcribed from Eylander et al. (2023):

        iota = integral[alpha1 to alpha2] (n_sun . n_surf) d_alpha

    where alpha1/alpha2 are the sun's azimuth at sunrise/sunset and
    n_sun/n_surf are unit vectors toward the sun and normal to the terrain.
    n_sun . n_surf is exactly `local_incidence_cosine()`, evaluated along
    the sun's actual daily path (zenith and azimuth are linked through the
    day/latitude, not independent) rather than at a single instant.

    Implementation: numerically integrate over the sun's azimuth sweep
    (not time directly -- azimuth is the paper's stated integration
    variable) by densely sampling UTC hours across the day, keeping only
    above-horizon samples, and trapezoidally integrating the
    horizon-clipped incidence cosine against the azimuth increment between
    consecutive samples.

    **Normalization (a Session 3 interpretive choice, not stated explicitly
    in the paper):** the raw integral has units of "cosine x degrees" and
    is not obviously the dimensionless, roughly-[0,1]-ish quantity Eq. 5's
    `[Rd + (1-Rd)*iota]` prefactor would need to behave sensibly (at
    iota=1, that prefactor is exactly 1 = full potential evaporation).
    This function normalizes by the same integral computed for FLAT ground
    at the same location/day, so iota=1.0 exactly for flat ground, iota<1
    for self-shadowed/away-facing slopes, and iota can slightly exceed 1
    for a slope tilted optimally toward the sun's path. This is the
    standard normalization convention in solar-exposure literature for
    quantities named "view factor," and it is what makes the Eq. 5
    prefactor behave the way the equation's own structure implies it
    should -- but it is an inference, not a transcription, and should be
    revisited if the primary source's supplementary material or code
    (podpac-examples on GitHub, per the paper's Software and Data
    Availability section) turns out to specify a different convention.

    Assumes a single monotonic sunrise-to-sunset azimuth sweep (true for
    the CO/AU mid-latitudes this project targets); not valid as-is for
    polar-latitude multi-crossing sun paths.
    """
    hours = np.linspace(0.0, 24.0, n_samples, endpoint=False)
    zeniths = np.empty(n_samples)
    azimuths = np.empty(n_samples)
    for idx, h in enumerate(hours):
        z, a = solar_position(lat_deg, lon_deg, day_of_year, h)
        zeniths[idx] = z
        azimuths[idx] = a

    above_horizon = zeniths < 90.0
    if not np.any(above_horizon):
        # Polar night or a pathological input -- no daylight, no radiation,
        # iota is undefined; 0.0 is the safe/conservative value (matches
        # "no direct beam" elsewhere in this module).
        return np.zeros_like(np.asarray(slope_tan), dtype=np.float64)

    day_zeniths = zeniths[above_horizon]
    day_azimuths = azimuths[above_horizon]
    # Azimuth generally increases monotonically through the day (sunrise in
    # the east through solar noon to sunset in the west); sort defensively
    # in case wraparound near midnight put samples out of order.
    order = np.argsort(day_azimuths)
    day_zeniths = day_zeniths[order]
    day_azimuths = day_azimuths[order]
    dalpha = np.diff(day_azimuths)

    # Flat-ground reference: n_surf points straight up, so n_sun.n_surf = cos(zenith).
    flat_cos = np.clip(np.cos(np.radians(day_zeniths)), 0.0, None)
    flat_integral = np.sum(0.5 * (flat_cos[:-1] + flat_cos[1:]) * dalpha)
    if flat_integral <= 0.0:
        return np.zeros_like(np.asarray(slope_tan), dtype=np.float64)

    slope_tan_arr = np.asarray(slope_tan, dtype=np.float64)
    aspect_arr = np.asarray(aspect_deg, dtype=np.float64)
    iota = np.empty_like(slope_tan_arr)
    it = np.nditer(slope_tan_arr, flags=["multi_index"])
    for _ in it:
        idx = it.multi_index
        cos_local = local_incidence_cosine(
            day_zeniths, day_azimuths, slope_tan_arr[idx], aspect_arr[idx]
        )
        cos_local_clipped = np.clip(cos_local, 0.0, None)
        slope_integral = np.sum(
            0.5 * (cos_local_clipped[:-1] + cos_local_clipped[1:]) * dalpha
        )
        iota[idx] = slope_integral / flat_integral

    return iota
