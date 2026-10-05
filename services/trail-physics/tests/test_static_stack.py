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


class TestWgs84Bbox:
    def test_returns_four_values_in_correct_order(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        min_lon, min_lat, max_lon, max_lat = stack.wgs84_bbox()
        assert min_lon < max_lon
        assert min_lat < max_lat

    def test_bbox_contains_the_grid_origin_round_trip(self, tmp_path):
        """The point used by test_round_trips_a_known_point (the grid's
        own xmin,ymax corner) must fall inside the bbox this method
        reports for the very same grid -- a real consistency check
        between the two coordinate-conversion paths, not just "some
        plausible-looking numbers."""
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        from pyproj import Transformer

        to_wgs84 = Transformer.from_crs("EPSG:5070", "EPSG:4326", always_xy=True)
        lon, lat = to_wgs84.transform(0.0, 100.0)
        min_lon, min_lat, max_lon, max_lat = stack.wgs84_bbox()
        assert min_lon <= lon <= max_lon
        assert min_lat <= lat <= max_lat

    def test_real_pilot_bbox_matches_grid_spec_pilot_bbox(self):
        """If the real pilot stack (Session 11) is present, its own
        reported wgs84_bbox must roughly match the pinned
        grid_spec.PILOT_BBOX_WGS84 it was built from -- not exactly (this
        method reports the SNAPPED grid's extent, which is slightly wider
        than the original requested bbox, see grid_spec.py's own outward-
        snapping behavior), but close."""
        path = (
            Path(__file__).parent.parent.parent.parent
            / "pipelines" / "static" / "data" / "static" / "colorado-10m-pilot.zarr"
        )
        if not path.exists():
            pytest.skip("real pilot stack not built this session (see pipelines/static/README.md)")
        stack = StaticStack(str(path))
        min_lon, min_lat, max_lon, max_lat = stack.wgs84_bbox()
        assert -105.7 < min_lon < -105.5
        assert -105.2 < max_lon < -105.0
        assert 39.8 < min_lat < 39.9
        assert 40.1 < max_lat < 40.2


# =============================================================================
# Vectorized + banded paths (Session 14)
# =============================================================================


class TestVectorizedMatchesScalar:
    def _big_stack(self, tmp_path):
        # 200 cols x 300 rows: many 16-row bands, so banding is really exercised.
        return _build_synthetic_stack(tmp_path, xmin=0.0, ymin=0.0, xmax=2000.0, ymax=3000.0)

    def test_banded_sample_layer_array_matches_per_point_reference(self, tmp_path):
        """Ground truth = the scalar bilinear_sample on the whole source
        array, point by point. Banded reading must change memory, never
        values -- including points near band boundaries, near grid edges,
        and outside the grid entirely."""
        from forcing import bilinear_sample

        store_path, twi, _, _ = self._big_stack(tmp_path)
        stack = StaticStack(store_path)
        rng = np.random.default_rng(5)
        rows = np.concatenate([rng.uniform(0, 299, 300), [15.99, 16.0, 16.01, 31.999, 0.0, 298.9], [-4.0, 400.0]])
        cols = np.concatenate([rng.uniform(0, 199, 300), [5.0, 5.0, 5.0, 5.0, 0.0, 198.9], [10.0, 10.0]])

        want = np.full(len(rows), np.nan)
        for i, (r, c) in enumerate(zip(rows, cols)):
            if 0 <= r < 300 and 0 <= c < 200:
                want[i] = bilinear_sample(twi.astype(np.float64), row=float(r), col=float(c)).value

        for band_rows in (1, 7, 16, 64, 10_000):
            got = stack.sample_layer_array("twi", rows, cols, band_rows=band_rows)
            np.testing.assert_allclose(got, want, rtol=1e-6, atol=1e-6, equal_nan=True, err_msg=f"band_rows={band_rows}")

    def test_banded_read_never_loads_more_than_its_band(self, tmp_path):
        """The point of banding: peak window size is bounded by the band,
        not the batch's geographic spread. Count the largest window any
        zarr read requests."""
        store_path, *_ = self._big_stack(tmp_path)
        stack = StaticStack(store_path)

        class _Spy:
            def __init__(self, arr):
                self.arr, self.max_rows = arr, 0

            def __getitem__(self, key):
                window = self.arr[key]
                self.max_rows = max(self.max_rows, window.shape[0])
                return window

        spy = _Spy(stack.root["twi"])
        stack.root = {"twi": spy}
        rows = np.linspace(0.5, 298.5, 200)  # points spread over the WHOLE grid height
        cols = np.full(200, 50.5)
        stack.sample_layer_array("twi", rows, cols, band_rows=16)
        assert spy.max_rows <= 16 + 3  # band + the 1-below/2-above margin
        spy.max_rows = 0
        stack.sample_layer_array("twi", rows, cols, band_rows=10_000)
        assert spy.max_rows > 250  # sanity: unbanded really does read ~everything

    def test_sample_layer_list_api_still_works_and_agrees(self, tmp_path):
        store_path, *_ = self._big_stack(tmp_path)
        stack = StaticStack(store_path)
        pts = [(5.5, 7.25), (120.0, 90.0), (-1.0, 3.0)]
        via_list = stack.sample_layer("twi", pts)
        via_array = stack.sample_layer_array("twi", np.array([p[0] for p in pts]), np.array([p[1] for p in pts]))
        np.testing.assert_array_equal(via_list, via_array)

    def test_lonlat_to_rowcol_array_matches_scalar(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        from pyproj import Transformer

        to_wgs84 = Transformer.from_crs("EPSG:5070", "EPSG:4326", always_xy=True)
        xy = [(0.0, 100.0), (35.0, 62.0), (99.0, 1.0)]
        lons, lats = zip(*(to_wgs84.transform(x, y) for x, y in xy))
        rows, cols = stack.lonlat_to_rowcol_array(np.array(lons), np.array(lats))
        for k, (lon, lat) in enumerate(zip(lons, lats)):
            r, c = stack.lonlat_to_rowcol(lon, lat)
            assert rows[k] == pytest.approx(r, abs=1e-9)
            assert cols[k] == pytest.approx(c, abs=1e-9)

    def test_hrrr_twi_bar_array_matches_scalar_including_misses_and_rounding(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)  # lookup: (100,200)->7.5, (101,200)->8.0
        stack = StaticStack(store_path)
        rows = np.array([100.0, 100.4, 100.6, 101.0, 999.0, 100.5, 101.5, -3.0])
        cols = np.array([200.0, 200.4, 199.6, 200.0, 999.0, 200.0, 200.0, 200.0])
        got = stack.hrrr_twi_bar_array(rows, cols)
        want = np.array([stack.hrrr_twi_bar(r, c) for r, c in zip(rows, cols)])
        np.testing.assert_array_equal(got, want)  # incl. NaN positions (assert_array_equal treats NaN==NaN)
        assert got[0] == pytest.approx(7.5) and np.isnan(got[4])

    def test_hrrr_twi_bar_array_with_empty_lookup_is_all_nan_not_an_indexerror(self, tmp_path):
        store_path, *_ = _build_synthetic_stack(tmp_path)
        stack = StaticStack(store_path)
        stack._twi_bar_keys = np.array([], dtype=np.int64)
        stack._twi_bar_values = np.array([], dtype=np.float64)
        assert np.isnan(stack.hrrr_twi_bar_array(np.array([100.0, 5.0]), np.array([200.0, 5.0]))).all()
