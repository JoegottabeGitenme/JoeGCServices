"""Tests for radiation.py's solar_view_factor (Eq. 6, transcribed from the
primary source in Session 3)."""

import numpy as np
import pytest

from physics.radiation import solar_view_factor

LAT_FRONT_RANGE = 39.75
LON_FRONT_RANGE = -105.2
SUMMER_SOLSTICE = 172
WINTER_SOLSTICE = 355

# Tarrawarra (Session 7, Stage 2 flux-correction work): 37deg39'S, 145deg26'E.
# Day 172 (northern summer solstice) is the AUSTRAL WINTER solstice and vice
# versa -- the labels below name the actual Tarrawarra season, not the day
# number's northern-hemisphere meaning, to avoid exactly the kind of mixup
# this test exists to catch.
LAT_TARRAWARRA = -37.65
LON_TARRAWARRA = 145.43
TARRAWARRA_AUSTRAL_WINTER = 172
TARRAWARRA_AUSTRAL_SUMMER = 355


def test_flat_ground_iota_is_exactly_one():
    """By construction: iota is normalized against the flat-ground integral
    for the same location/day, so flat ground must give exactly 1.0."""
    iota = solar_view_factor(
        LAT_FRONT_RANGE, LON_FRONT_RANGE, SUMMER_SOLSTICE, np.array([0.0]), np.array([0.0])
    )
    assert iota[0] == pytest.approx(1.0, abs=1e-9)


def test_north_facing_slope_gets_less_than_flat_in_northern_hemisphere():
    iota = solar_view_factor(
        LAT_FRONT_RANGE, LON_FRONT_RANGE, SUMMER_SOLSTICE, np.array([0.4]), np.array([0.0])
    )
    assert iota[0] < 1.0


def test_south_facing_slope_beats_north_facing_slope():
    common = dict(
        lat_deg=LAT_FRONT_RANGE, lon_deg=LON_FRONT_RANGE, day_of_year=SUMMER_SOLSTICE,
        slope_tan=np.array([0.4]),
    )
    iota_south = solar_view_factor(aspect_deg=np.array([180.0]), **common)
    iota_north = solar_view_factor(aspect_deg=np.array([0.0]), **common)
    assert iota_south[0] > iota_north[0]


def test_aspect_contrast_is_much_larger_in_winter_than_summer():
    """The core physical property this whole product depends on: aspect
    matters far more when the sun is low (winter) than when it's high
    (summer) -- this is *why* a north-facing winter slope stays frozen
    while the south-facing side of the same ridge is bare."""
    common = dict(lat_deg=LAT_FRONT_RANGE, lon_deg=LON_FRONT_RANGE, slope_tan=np.array([0.4]))

    summer_south = solar_view_factor(day_of_year=SUMMER_SOLSTICE, aspect_deg=np.array([180.0]), **common)[0]
    summer_north = solar_view_factor(day_of_year=SUMMER_SOLSTICE, aspect_deg=np.array([0.0]), **common)[0]
    winter_south = solar_view_factor(day_of_year=WINTER_SOLSTICE, aspect_deg=np.array([180.0]), **common)[0]
    winter_north = solar_view_factor(day_of_year=WINTER_SOLSTICE, aspect_deg=np.array([0.0]), **common)[0]

    summer_contrast = summer_south - summer_north
    winter_contrast = winter_south - winter_north
    assert winter_contrast > summer_contrast


def test_winter_north_facing_slope_is_heavily_shadowed():
    """A concrete, checkable version of "north-facing stays frozen": a
    moderately steep north-facing slope in winter should receive well
    under half the flat-ground insolation."""
    iota = solar_view_factor(
        LAT_FRONT_RANGE, LON_FRONT_RANGE, WINTER_SOLSTICE, np.array([0.4]), np.array([0.0])
    )
    assert iota[0] < 0.5


def test_handles_multiple_points_at_once():
    slopes = np.array([0.0, 0.2, 0.4])
    aspects = np.array([180.0, 180.0, 0.0])
    iota = solar_view_factor(LAT_FRONT_RANGE, LON_FRONT_RANGE, WINTER_SOLSTICE, slopes, aspects)
    assert iota.shape == (3,)
    assert iota[0] == pytest.approx(1.0, abs=1e-9)  # flat
    assert iota[1] > iota[2]  # south-facing beats north-facing


def test_non_negative():
    rng = np.random.default_rng(3)
    slopes = rng.uniform(0, 1.5, 20)
    aspects = rng.uniform(0, 360, 20)
    iota = solar_view_factor(LAT_FRONT_RANGE, LON_FRONT_RANGE, WINTER_SOLSTICE, slopes, aspects)
    assert np.all(iota >= 0.0)


# =============================================================================
# Southern hemisphere (Session 7): every test above uses a positive
# (northern-hemisphere) latitude. solar_position/local_incidence_cosine are
# pure trigonometry with no hemisphere-specific branch, so there's no a
# priori reason to expect a sign bug here (and Session 3's aspect-sign bug
# in terrain.compute_aspect, caught by exactly this kind of directional
# test, is the reason to check rather than assume). Confirmed empirically
# before writing these assertions: at Tarrawarra's real coordinates, a
# north-facing slope gets iota=2.03 vs. a south-facing slope's iota=0.14 in
# austral winter -- the mirror image of the northern-hemisphere case, as
# physically required.
# =============================================================================


def test_southern_hemisphere_north_facing_beats_south_facing_in_winter():
    """The mirror image of test_south_facing_slope_beats_north_facing_slope
    above: south of the equator, the sun's midday path is to the north, so
    NORTH-facing slopes get more sun in winter -- not south-facing."""
    common = dict(
        lat_deg=LAT_TARRAWARRA, lon_deg=LON_TARRAWARRA, day_of_year=TARRAWARRA_AUSTRAL_WINTER,
        slope_tan=np.array([0.4]),
    )
    iota_south = solar_view_factor(aspect_deg=np.array([180.0]), **common)[0]
    iota_north = solar_view_factor(aspect_deg=np.array([0.0]), **common)[0]
    assert iota_north > iota_south


def test_southern_hemisphere_flat_ground_iota_is_exactly_one():
    iota = solar_view_factor(
        LAT_TARRAWARRA, LON_TARRAWARRA, TARRAWARRA_AUSTRAL_SUMMER, np.array([0.0]), np.array([0.0])
    )
    assert iota[0] == pytest.approx(1.0, abs=1e-9)


def test_southern_hemisphere_aspect_contrast_larger_in_winter_than_summer():
    """Same physical property as the northern-hemisphere test above, mirrored:
    aspect matters far more when the sun is low (austral winter) than high
    (austral summer)."""
    common = dict(lat_deg=LAT_TARRAWARRA, lon_deg=LON_TARRAWARRA, slope_tan=np.array([0.4]))

    summer_north = solar_view_factor(day_of_year=TARRAWARRA_AUSTRAL_SUMMER, aspect_deg=np.array([0.0]), **common)[0]
    summer_south = solar_view_factor(day_of_year=TARRAWARRA_AUSTRAL_SUMMER, aspect_deg=np.array([180.0]), **common)[0]
    winter_north = solar_view_factor(day_of_year=TARRAWARRA_AUSTRAL_WINTER, aspect_deg=np.array([0.0]), **common)[0]
    winter_south = solar_view_factor(day_of_year=TARRAWARRA_AUSTRAL_WINTER, aspect_deg=np.array([180.0]), **common)[0]

    summer_contrast = summer_north - summer_south
    winter_contrast = winter_north - winter_south
    assert winter_contrast > summer_contrast


def test_southern_hemisphere_winter_south_facing_slope_is_heavily_shadowed():
    """The mirror image of test_winter_north_facing_slope_is_heavily_shadowed:
    in the southern hemisphere, it's the SOUTH-facing slope that's
    heavily shadowed in winter."""
    iota = solar_view_factor(
        LAT_TARRAWARRA, LON_TARRAWARRA, TARRAWARRA_AUSTRAL_WINTER, np.array([0.4]), np.array([180.0])
    )
    assert iota[0] < 0.5
