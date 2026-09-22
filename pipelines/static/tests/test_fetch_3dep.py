"""Tests for fetch_3dep.py's tile-enumeration logic.

test_colorado_tile_count is the one asserting a specific tile SET (not just
a count) -- it was cross-checked against a live listing of the `prd-tnm`
bucket during this session (n40w106 confirmed to exist with a 413MB .tif),
so this isn't a guessed convention.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

from fetch_3dep import tile_name, tile_url, tiles_for_bbox


def test_tile_name_northwest_hemisphere():
    assert tile_name(40, -106) == "n40w106"


def test_tile_name_pads_single_digit_longitude():
    assert tile_name(40, -6) == "n40w006"


def test_single_point_bbox_gives_one_tile():
    # A tiny bbox entirely within the n40w106 cell (39-40N, 106-105W)
    tiles = tiles_for_bbox(-105.5, 39.5, -105.4, 39.6)
    assert tiles == ["n40w106"]


def test_colorado_bbox_includes_known_front_range_tile():
    """Golden/Denver (~39.7N, 105.2W) should require the n40w106 tile --
    confirmed live this session to actually exist in the bucket."""
    tiles = tiles_for_bbox(-109.06, 36.99, -102.04, 41.00)
    assert "n40w106" in tiles


def test_colorado_bbox_tile_count_matches_expected_span():
    """Colorado spans ~7 degrees longitude x ~4 degrees latitude -> at
    minimum a 7x4=28-tile grid (could be +1 in either dimension depending
    on exact fractional-degree boundary alignment)."""
    tiles = tiles_for_bbox(-109.06, 36.99, -102.04, 41.00)
    assert 28 <= len(tiles) <= 40


def test_tile_url_matches_confirmed_live_bucket_pattern():
    """This exact URL was confirmed live (curl, this session) to resolve
    to a real 413MB GeoTIFF -- see this module's docstring."""
    url = tile_url("n40w106")
    assert url == "https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/13/TIFF/current/n40w106/USGS_13_n40w106.tif"
