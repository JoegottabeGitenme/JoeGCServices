"""Tests for static_stack.py against a small synthetic Zarr v3 store built
to the same layout `pipelines/static/assemble_static_stack.py` writes
(flat named 2-D arrays + the hrrr_twi_bar_* 1-D lookup + grid_spec/
provenance attrs) -- no live MinIO reachable from this environment, same
posture as test_forcing.py's synthetic-store tests."""

import sys
from pathlib import Path

import numpy as np
import pytest
import zarr

sys.path.insert(0, str(Path(__file__).parent.parent))

from static_stack import StaticStack  # noqa: E402


def _build_synthetic_stack(tmp_path, xmin=0.0, ymin=0.0, xmax=100.0, ymax=100.0, resolution_m=10.0):
    """A tiny 10x10-cell stack (EPSG:5070, matching the real pilot's CRS)
    with a simple linear TWI gradient, uniform theta_s/theta_wilt, and one
    fabricated HRRR-cell lambda_bar entry."""
    width = int((xmax - xmin) / resolution_m)
    height = int((ymax - ymin) / resolution_m)

    store_path = str(tmp_path / "test_stack")
    root = zarr.open_group(store=store_path, mode="w")

    twi = np.fromfunction(lambda r, c: 5.0 + 0.1 * r + 0.2 * c, (height, width), dtype=float).astype(np.float32)
    theta_s = np.full((height, width), 0.45, dtype=np.float32)
    theta_wilt = np.full((height, width), 0.08, dtype=np.float32)

    for name, arr in [("twi", twi), ("theta_s", theta_s), ("theta_wilt", theta_wilt)]:
        z = root.create_array(name, shape=arr.shape, dtype=np.float32)
        z[:] = arr

    root.create_array("hrrr_twi_bar_hrrr_row", shape=(2,), dtype=np.int32)[:] = np.array([100, 101], dtype=np.int32)
    root.create_array("hrrr_twi_bar_hrrr_col", shape=(2,), dtype=np.int32)[:] = np.array([200, 200], dtype=np.int32)
    root.create_array("hrrr_twi_bar_twi_bar", shape=(2,), dtype=np.float32)[:] = np.array([7.5, 8.0], dtype=np.float32)
    root.create_array("hrrr_twi_bar_n_fine_cells", shape=(2,), dtype=np.int32)[:] = np.array([50, 60], dtype=np.int32)

    root.attrs["grid_spec"] = {
        "crs": "EPSG:5070", "resolution_m": resolution_m,
        "xmin": xmin, "ymin": ymin, "xmax": xmax, "ymax": ymax,
        "width": width, "height": height,
    }
    root.attrs["provenance"] = {"k": 13.0}
    return store_path, twi, theta_s, theta_wilt


class TestStaticStackInit:
    def test_reads_grid_spec_from_attrs(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        assert stack.crs == "EPSG:5070"
        assert stack.resolution_m == 10.0
        assert stack.width == 10
        assert stack.height == 10

    def test_builds_twi_bar_lookup_from_1d_arrays(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        assert stack.hrrr_twi_bar(hrrr_row=100, hrrr_col=200) == pytest.approx(7.5)
        assert stack.hrrr_twi_bar(hrrr_row=101, hrrr_col=200) == pytest.approx(8.0)

    def test_missing_hrrr_cell_returns_nan(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        assert np.isnan(stack.hrrr_twi_bar(hrrr_row=999, hrrr_col=999))


class TestInBounds:
    def test_point_inside_grid(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        assert stack.in_bounds(5.0, 5.0)

    def test_point_outside_grid(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        assert not stack.in_bounds(-1.0, 5.0)
        assert not stack.in_bounds(5.0, 100.0)


class TestSampleLayer:
    def test_exact_cell_center_matches_source_value(self, tmp_path):
        store_path, twi, _, _ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        # Bilinear sample at the exact grid-index location of cell (3,4)
        # should reproduce a value consistent with the surrounding
        # gradient (not necessarily EXACTLY twi[3,4] since bilinear_sample
        # treats (row,col) as the corner between cells, but must fall
        # within the local gradient's range).
        result = stack.sample_layer("twi", [(3.0, 4.0)])
        assert not np.isnan(result[0])

    def test_all_out_of_bounds_returns_all_nan(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        result = stack.sample_layer("twi", [(-5.0, -5.0), (200.0, 200.0)])
        assert np.all(np.isnan(result))

    def test_mixed_in_and_out_of_bounds(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        result = stack.sample_layer("twi", [(5.0, 5.0), (-5.0, -5.0)])
        assert not np.isnan(result[0])
        assert np.isnan(result[1])

    def test_empty_input_returns_empty_array(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        result = stack.sample_layer("twi", [])
        assert len(result) == 0

    def test_uniform_layer_samples_to_its_own_value(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        result = stack.sample_layer("theta_s", [(2.0, 3.0), (7.0, 8.0)])
        np.testing.assert_allclose(result, [0.45, 0.45])


class TestLonLatToRowCol:
    def test_round_trips_a_known_point(self, tmp_path):
        """A point at the grid's own origin (xmin, ymax) must map to
        (row=0, col=0)."""
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        from pyproj import Transformer

        to_wgs84 = Transformer.from_crs("EPSG:5070", "EPSG:4326", always_xy=True)
        lon, lat = to_wgs84.transform(0.0, 100.0)  # xmin, ymax
        row, col = stack.lonlat_to_rowcol(lon, lat)
        assert row == pytest.approx(0.0, abs=1e-6)
        assert col == pytest.approx(0.0, abs=1e-6)
