"""Tests for derive_coarse_twi.py -- the real production equation's
lambda_bar (coarse HRRR-cell-mean TWI), per Session 8's discovery."""

import sys
from pathlib import Path

import numpy as np
import pytest
import rasterio

sys.path.insert(0, str(Path(__file__).parent.parent))
sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "services" / "trail-physics"))

from derive_coarse_twi import compute_coarse_twi_bar
from hrrr_grid import HrrrGrid


def _write_synthetic_twi(tmp_path, value_fn, nrows=20, ncols=20, cellsize=10.0):
    """A small synthetic TWI GeoTIFF, positioned over real Colorado
    coordinates (near Boulder) so HrrrGrid.geo_to_grid produces sane,
    real HRRR cell indices -- not an arbitrary/unphysical location."""
    from pyproj import Transformer

    # Boulder-ish location in EPSG:5070.
    t = Transformer.from_crs("EPSG:4326", "EPSG:5070", always_xy=True)
    cx, cy = t.transform(-105.3, 40.0)
    xmin = cx - (ncols * cellsize) / 2
    ymax = cy + (nrows * cellsize) / 2
    transform = rasterio.transform.from_origin(xmin, ymax, cellsize, cellsize)

    arr = np.fromfunction(lambda r, c: value_fn(r, c), (nrows, ncols), dtype=float).astype(np.float32)
    path = tmp_path / "synthetic_twi.tif"
    with rasterio.open(
        path, "w", driver="GTiff", height=nrows, width=ncols, count=1,
        dtype=np.float32, crs="EPSG:5070", transform=transform, nodata=np.nan,
    ) as dst:
        dst.write(arr, 1)
    return str(path)


class TestComputeCoarseTwiBar:
    def test_uniform_twi_gives_single_hrrr_cell_with_that_exact_value(self, tmp_path):
        """A small, spatially uniform TWI patch (much smaller than one 3km
        HRRR cell) must aggregate to exactly one HRRR cell with twi_bar
        equal to the uniform value -- the simplest possible correctness
        check."""
        path = _write_synthetic_twi(tmp_path, lambda r, c: np.full_like(r, 7.5), nrows=10, ncols=10, cellsize=10.0)
        rows, cols, twi_bar, counts = compute_coarse_twi_bar(path)
        assert len(rows) == 1
        assert twi_bar[0] == pytest.approx(7.5)
        assert counts[0] == 100

    def test_mean_is_correct_for_varying_values(self, tmp_path):
        """A small patch (all in one HRRR cell) with KNOWN varying values
        must produce the exact arithmetic mean, not an approximation."""
        path = _write_synthetic_twi(tmp_path, lambda r, c: r + c, nrows=10, ncols=10, cellsize=10.0)
        rows, cols, twi_bar, counts = compute_coarse_twi_bar(path)
        assert len(rows) == 1
        # Mean of r+c over a 10x10 grid of r,c in [0,9]: E[r]=E[c]=4.5 -> mean=9.0
        assert twi_bar[0] == pytest.approx(9.0)

    def test_nan_cells_excluded_from_mean(self, tmp_path):
        def value_fn(r, c):
            arr = np.full_like(r, 5.0)
            arr[0, 0] = np.nan
            return arr

        path = _write_synthetic_twi(tmp_path, value_fn, nrows=10, ncols=10, cellsize=10.0)
        rows, cols, twi_bar, counts = compute_coarse_twi_bar(path)
        assert counts[0] == 99  # one cell excluded
        assert twi_bar[0] == pytest.approx(5.0)

    def test_output_indices_are_valid_hrrr_grid_positions(self, tmp_path):
        path = _write_synthetic_twi(tmp_path, lambda r, c: np.full_like(r, 6.0), nrows=10, ncols=10, cellsize=10.0)
        rows, cols, twi_bar, counts = compute_coarse_twi_bar(path)
        hrrr = HrrrGrid.hrrr()
        assert np.all((rows >= 0) & (rows < hrrr.ny))
        assert np.all((cols >= 0) & (cols < hrrr.nx))


class TestRealPilotOutput:
    def test_real_data_regression(self):
        path = Path(__file__).parent.parent / "data" / "static" / "pilot_twi_bar.npz"
        if not path.exists():
            pytest.skip("real pilot_twi_bar.npz not built this session (see README.md)")
        data = np.load(path)
        assert len(data["hrrr_row"]) == len(data["hrrr_col"]) == len(data["twi_bar"]) == len(data["n_fine_cells"])
        # Real result from Session 11: ~186 distinct HRRR cells, twi_bar
        # values a strict subset of the fine TWI's own range (coarse means
        # must be less extreme than the fine field they average).
        assert 100 < len(data["hrrr_row"]) < 300
        assert data["twi_bar"].min() > 2.9  # fine TWI's own min was 2.924
        assert data["twi_bar"].max() < 14.98  # fine TWI's own max was 14.979
        assert np.all(data["n_fine_cells"] > 0)
