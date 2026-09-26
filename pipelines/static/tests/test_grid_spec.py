"""Tests for grid_spec.py -- the pinned WS1 grid definition."""

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

from grid_spec import (
    PILOT_BBOX_WGS84,
    STATIC_STACK_CRS,
    STATIC_STACK_RESOLUTION_M,
    GridSpec,
    bbox_wgs84_to_grid_spec,
    pilot_grid_spec,
)


def test_grid_spec_width_height_are_whole_cell_counts():
    spec = GridSpec(crs="EPSG:5070", resolution_m=10.0, xmin=0.0, ymin=0.0, xmax=100.0, ymax=50.0)
    assert spec.width == 10
    assert spec.height == 5


def test_bbox_snapping_rounds_outward_not_inward():
    """A bbox whose reprojected corners don't land exactly on a 10m grid
    boundary must be snapped OUTWARD (floor the min, ceil the max) -- the
    resulting grid must fully contain the original bbox, never clip it."""
    spec = bbox_wgs84_to_grid_spec(-105.3, 39.99, -105.29, 40.0, resolution_m=10.0)
    assert spec.xmin % 10.0 == 0
    assert spec.ymin % 10.0 == 0
    assert spec.xmax % 10.0 == 0
    assert spec.ymax % 10.0 == 0


def test_bbox_to_grid_spec_produces_positive_extent():
    spec = bbox_wgs84_to_grid_spec(*PILOT_BBOX_WGS84)
    assert spec.xmax > spec.xmin
    assert spec.ymax > spec.ymin
    assert spec.crs == STATIC_STACK_CRS


def test_pilot_grid_spec_matches_direct_bbox_call():
    """pilot_grid_spec() must be the single source of truth -- confirms it
    isn't silently drifting from PILOT_BBOX_WGS84."""
    direct = bbox_wgs84_to_grid_spec(*PILOT_BBOX_WGS84)
    via_helper = pilot_grid_spec()
    assert direct == via_helper


def test_pilot_grid_spec_cell_count_is_plausible():
    """Sanity bound on the pilot's cell count -- catches a gross unit
    error (e.g. degrees vs meters) immediately rather than silently
    producing a grid a thousand times too large or too small."""
    spec = pilot_grid_spec()
    total_cells = spec.width * spec.height
    # ~42km x 33km at 10m -> tens of millions of cells, not billions or
    # thousands.
    assert 5_000_000 < total_cells < 50_000_000


def test_transform_places_origin_at_top_left():
    spec = GridSpec(crs="EPSG:5070", resolution_m=10.0, xmin=100.0, ymin=200.0, xmax=200.0, ymax=300.0)
    t = spec.transform
    # rasterio's Affine: c=x-origin (left), f=y-origin (top), e=-resolution
    # (north-up convention, row index increases southward).
    assert t.c == pytest.approx(100.0)
    assert t.f == pytest.approx(300.0)
    assert t.e == pytest.approx(-10.0)


def test_to_attrs_dict_is_json_serializable():
    import json

    spec = pilot_grid_spec()
    # Must not raise -- every value must be a plain JSON-compatible type
    # (Zarr attrs are JSON-serialized).
    json.dumps(spec.to_attrs_dict())


def test_resolution_matches_project_default():
    assert STATIC_STACK_RESOLUTION_M == 10.0
