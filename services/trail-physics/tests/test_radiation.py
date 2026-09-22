"""Unit tests for radiation.py (solar geometry + terrain-corrected shortwave)."""

import numpy as np
import pytest

from physics.radiation import (
    local_incidence_cosine,
    solar_declination_deg,
    solar_position,
    terrain_corrected_shortwave,
)


def test_declination_near_zero_at_equinox():
    """Around day 81 (March equinox) and day 264 (September equinox) the
    solar declination should be near zero -- the sun is over the equator."""
    assert solar_declination_deg(81) == pytest.approx(0.0, abs=1.5)


def test_declination_near_max_at_june_solstice():
    """Day ~172 (June 21) is the northern summer solstice: declination near
    +23.45 degrees (the Earth's axial tilt)."""
    assert solar_declination_deg(172) == pytest.approx(23.45, abs=0.5)


def test_declination_near_min_at_december_solstice():
    assert solar_declination_deg(355) == pytest.approx(-23.45, abs=0.5)


def test_solar_noon_zenith_lower_than_morning():
    """At a fixed northern-hemisphere location, the sun should be higher
    (lower zenith angle) at local solar noon than in the morning."""
    lat, lon, day = 39.7, -105.2, 172  # Front Range latitude, summer solstice
    zenith_morning, _ = solar_position(lat, lon, day, utc_hour=16.0)  # ~9am local (UTC-7)
    zenith_noon, _ = solar_position(lat, lon, day, utc_hour=19.0)  # ~noon local
    assert zenith_noon < zenith_morning


def test_south_facing_slope_gets_more_direct_beam_in_northern_hemisphere():
    """The core physical property the whole trail-conditions differentiator
    depends on: at northern-hemisphere latitudes, with the sun roughly to
    the south, a south-facing slope (aspect=180) should receive MORE direct
    beam radiation than a north-facing slope (aspect=0) at the same tilt."""
    zenith, azimuth = 45.0, 180.0  # sun due south, moderate elevation
    slope_tan = np.array([0.3])  # ~17 degree slope
    cos_south = local_incidence_cosine(zenith, azimuth, slope_tan, np.array([180.0]))
    cos_north = local_incidence_cosine(zenith, azimuth, slope_tan, np.array([0.0]))
    assert cos_south[0] > cos_north[0]


def test_flat_ground_incidence_equals_cosine_zenith():
    """On perfectly flat ground (slope=0), the local incidence angle must
    equal the solar zenith angle regardless of the (undefined) aspect."""
    zenith = 30.0
    cos_local = local_incidence_cosine(zenith, 180.0, np.array([0.0]), np.array([0.0]))
    assert cos_local[0] == pytest.approx(np.cos(np.radians(zenith)))


def test_terrain_corrected_shortwave_zero_when_sun_below_horizon():
    result = terrain_corrected_shortwave(
        dswrf_flat=np.array([0.0]),
        zenith_deg=95.0,  # below horizon
        azimuth_deg=270.0,
        slope_tan=np.array([0.2]),
        aspect_deg=np.array([180.0]),
        sky_view_factor=np.array([0.9]),
    )
    assert result[0] == pytest.approx(0.0)


def test_terrain_corrected_shortwave_reduced_by_low_sky_view_factor():
    """A deeply shadowed/enclosed point (low SVF, e.g. a narrow canyon)
    must receive less total shortwave than an open point, all else equal --
    this is the mechanism that makes canopy/topographic shading matter for
    melt timing (design doc S3)."""
    common = dict(
        dswrf_flat=np.array([500.0]),
        zenith_deg=20.0,
        azimuth_deg=180.0,
        slope_tan=np.array([0.0]),
        aspect_deg=np.array([0.0]),
    )
    open_result = terrain_corrected_shortwave(sky_view_factor=np.array([1.0]), **common)
    enclosed_result = terrain_corrected_shortwave(sky_view_factor=np.array([0.3]), **common)
    assert enclosed_result[0] < open_result[0]
