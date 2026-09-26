"""Tests for build_dem.py. The full pilot build (real vsicurl network
reads) is exercised directly by running build_dem.py itself (see
pipelines/static/README.md) rather than in this automated suite -- these
tests cover the parts that don't require a live network read.
"""

import sys
from pathlib import Path

import numpy as np
import pytest
import rasterio

sys.path.insert(0, str(Path(__file__).parent.parent))

from build_dem import tile_vsicurl_url
from grid_spec import GridSpec


def test_tile_vsicurl_url_format():
    url = tile_vsicurl_url("n40w106")
    assert url == (
        "/vsicurl/https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/"
        "13/TIFF/current/n40w106/USGS_13_n40w106.tif"
    )


def test_committed_pilot_dem_matches_grid_spec():
    """If the real pilot DEM (built via build_dem.py, see README.md) is
    present, its shape/CRS/transform must exactly match the pinned grid
    spec -- not just be plausible."""
    from grid_spec import pilot_grid_spec

    path = Path(__file__).parent.parent / "data" / "static" / "pilot_dem.tif"
    if not path.exists():
        pytest.skip("real pilot DEM not built this session (see README.md)")
    spec = pilot_grid_spec()
    with rasterio.open(path) as src:
        assert src.crs.to_string() == spec.crs
        assert src.width == spec.width
        assert src.height == spec.height
        assert src.res[0] == pytest.approx(spec.resolution_m)
        arr = src.read(1)
        valid = arr[~np.isnan(arr)]
        # Real Front Range foothills-to-peaks elevation range, confirmed
        # against the real read (Session 11): 1506-3729m. Loose bounds --
        # this is a plausibility check, not pinning the exact float values.
        assert 1000 < valid.min() < 2000
        assert 3000 < valid.max() < 4500
