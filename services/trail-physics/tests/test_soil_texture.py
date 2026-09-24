"""Unit tests for soil_texture.py (USDA texture triangle classification +
Noah SOILPARM.TBL lookup).
"""

import pytest

from physics.soil_texture import (
    NOAH_SOILPARM_STAS,
    USDA_TEXTURE_TRIANGLE,
    _point_in_polygon,
    classify_usda_texture,
    soil_hydraulic_properties,
)


def test_pure_sand_corner():
    assert classify_usda_texture(100, 0) == "SAND"


def test_pure_clay_corner():
    assert classify_usda_texture(0, 100) == "CLAY"


def test_pure_silt_corner():
    assert classify_usda_texture(0, 0) == "SILT"


def test_classic_clay_loam_region():
    """Roughly equal sand/silt/clay thirds is the textbook clay loam
    example in every soil-texture-triangle teaching diagram."""
    assert classify_usda_texture(33, 33) == "CLAY LOAM"


def test_all_twelve_classes_have_a_lookup_entry():
    """Every polygon name must have a corresponding Noah parameter entry --
    a mismatch here would mean classify_usda_texture could return a
    texture soil_hydraulic_properties can't look up."""
    assert set(USDA_TEXTURE_TRIANGLE.keys()) == set(NOAH_SOILPARM_STAS.keys())


def test_triangle_has_no_coverage_gaps():
    """Every valid (sand, clay) point (sand + clay <= 100) must fall
    inside at least one polygon -- a real bug in the transcribed vertex
    data would show up as an unclassified region. Uses a moderately fine
    grid (2% steps) covering the full valid triangle."""
    gaps = []
    for sand in range(0, 101, 2):
        for clay in range(0, 101 - sand, 2):
            matches = [
                name for name, verts in USDA_TEXTURE_TRIANGLE.items() if _point_in_polygon(sand, clay, verts)
            ]
            if not matches:
                gaps.append((sand, clay))
    assert gaps == [], f"unclassified points found (transcription gap): {gaps[:10]}"


def test_classify_rejects_negative_inputs():
    with pytest.raises(ValueError, match="non-negative"):
        classify_usda_texture(-5, 10)


def test_classify_rejects_sand_plus_clay_far_over_100():
    with pytest.raises(ValueError, match="exceeds 100%"):
        classify_usda_texture(80, 80)


def test_classify_tolerates_small_rounding_overshoot():
    """Real lab measurements occasionally sum to just over 100% due to
    rounding -- a small overshoot (e.g. sand=90, clay=12, implying
    silt=-2) must fall back to a nearest-polygon match, not raise."""
    result = classify_usda_texture(90, 12)  # sums to 102, small overshoot
    assert result in USDA_TEXTURE_TRIANGLE


def test_noah_soilparm_sand_values_match_fetched_table():
    """Spot-check against the live-fetched wrf-model/WRF SOILPARM.TBL
    (STAS table) values, transcribed in Session 6 -- catches a
    transcription slip in the most commonly-hit class."""
    sand = NOAH_SOILPARM_STAS["SAND"]
    assert sand.satdk_m_per_s == pytest.approx(4.66e-5)
    assert sand.maxsmc == pytest.approx(0.339)
    assert sand.refsmc == pytest.approx(0.192)
    assert sand.wltsmc == pytest.approx(0.010)


def test_noah_soilparm_clay_values_match_fetched_table():
    clay = NOAH_SOILPARM_STAS["CLAY"]
    assert clay.satdk_m_per_s == pytest.approx(9.74e-7)
    assert clay.maxsmc == pytest.approx(0.468)
    assert clay.refsmc == pytest.approx(0.412)
    assert clay.wltsmc == pytest.approx(0.138)


def test_soil_hydraulic_properties_end_to_end():
    """Classify then look up in one call -- pure sand should return SAND's
    Noah parameters."""
    params = soil_hydraulic_properties(100, 0)
    assert params == NOAH_SOILPARM_STAS["SAND"]


def test_saturated_conductivity_decreases_with_clay_content():
    """Physical sanity check: finer-textured (higher clay) soils drain
    more slowly -- SATDK should be monotonically related to texture, at
    least for the clearest end-member comparison (pure sand vs pure
    clay)."""
    sand_ks = NOAH_SOILPARM_STAS["SAND"].satdk_m_per_s
    clay_ks = NOAH_SOILPARM_STAS["CLAY"].satdk_m_per_s
    assert sand_ks > clay_ks
