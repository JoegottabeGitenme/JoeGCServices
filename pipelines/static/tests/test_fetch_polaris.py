"""Tests for fetch_polaris.py's region-generalization (Session 14). The
network fetch itself is exercised by running the script (see README.md)."""

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

from fetch_polaris import POLARIS_TILES, polaris_tiles_for_bbox
from grid_spec import region_bbox


def test_polaris_tiles_for_bbox_reproduces_pilot_list():
    """The pilot's hardcoded tile list was confirmed live (Session 11) --
    the enumeration rule must reproduce it exactly, so the naming scheme
    isn't an untested guess for new regions."""
    assert polaris_tiles_for_bbox(*region_bbox("pilot")) == POLARIS_TILES


def test_front_range_needs_six_tiles():
    tiles = polaris_tiles_for_bbox(*region_bbox("front-range"))
    assert tiles == [
        "lat3839_lon-106-105", "lat3839_lon-105-104",
        "lat3940_lon-106-105", "lat3940_lon-105-104",
        "lat4041_lon-106-105", "lat4041_lon-105-104",
    ]


def test_bbox_edge_exactly_on_an_integer_degree_excludes_the_next_tile():
    # max_lat == 40.0 must NOT pull in the 40-41 tile.
    assert polaris_tiles_for_bbox(-105.5, 39.2, -105.2, 40.0) == ["lat3940_lon-106-105"]


def test_single_cell_bbox_gives_one_tile():
    assert polaris_tiles_for_bbox(-105.3, 39.5, -105.2, 39.6) == ["lat3940_lon-106-105"]
