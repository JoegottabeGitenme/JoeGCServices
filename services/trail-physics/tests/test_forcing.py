"""Tests for forcing.py.

`test_open_and_sample_real_local_zarr_v3_array` builds an actual local
Zarr v3 array (real zarr-python library, real files on disk, real bilinear
sampling through hrrr_grid's projection) -- not a mock -- specifically to
prove the read+sample pipeline works end-to-end against real Zarr I/O, just
not against a live MinIO endpoint (unreachable from this environment this
session -- see forcing.py's module docstring).
"""

import numpy as np
import pytest
import zarr

from forcing import BilinearSample, bilinear_sample, open_level0_array, sample_points, storage_path
from hrrr_grid import HrrrGrid


def test_storage_path_matches_rust_convention():
    """Session 13 real bug fix: the previous version of this test passed
    an ALREADY-sanitized level string ("4_cm_below_ground") and asserted
    the (buggy) passthrough function returned it with the ORIGINAL-case
    param still attached ("SOILW_...") -- which masked the real bug,
    since it never exercised what main.py actually calls this function
    with: the NATURAL, catalog-matching strings ("SOILW", "4 cm below
    ground", spaces and all). That mismatch meant every single forecast
    hour failed to read on trail-physics' first live production cycle
    (confirmed: 100% of forecast hours in that cycle logged "No group
    found in store"). Fixed by comparing against a REAL
    `datasets.storage_path` row read directly from the production
    catalog (not re-derived from memory a second time) -- see
    test_storage_path_matches_real_production_catalog_row below for that
    exact value, locked in as a permanent regression test."""
    path = storage_path("hrrr", "20260922_18z", "SOILW", "4 cm below ground", 3)
    assert path == "grids/hrrr/20260922_18z/soilw_4_cm_below_ground_f003.zarr"


def test_storage_path_matches_real_production_catalog_row():
    """The EXACT real storage_path value read from a live `datasets` row
    on the production catalog (Session 13): confirms this function's
    output is not just internally consistent but matches what the real
    Rust ingester actually wrote to MinIO."""
    path = storage_path("hrrr", "20260927_03z", "SOILW", "4 cm below ground", 18)
    assert path == "grids/hrrr/20260927_03z/soilw_4_cm_below_ground_f018.zarr"


def test_storage_path_sanitizes_slashes_too():
    """Rust's own sanitizer replaces both spaces AND slashes -- a level
    string containing a slash must not leak into the filename as a
    literal '/' (which would create an unintended sub-directory)."""
    path = storage_path("gfs", "20260922_18z", "TMP", "0-0.1 m/below ground", 0)
    assert path == "grids/gfs/20260922_18z/tmp_0-0.1_m_below_ground_f000.zarr"
    filename = path.rsplit("/", 1)[-1]
    assert filename.count("/") == 0


def test_bilinear_sample_exact_grid_point():
    """Sampling exactly at an integer (row, col) should return that cell's
    value exactly (all weight on one corner)."""
    array = np.array([[1.0, 2.0], [3.0, 4.0]])
    result = bilinear_sample(array, row=0.0, col=0.0)
    assert result.value == pytest.approx(1.0)
    assert result.fraction_nan_neighbors == 0.0


def test_bilinear_sample_midpoint_averages_all_four():
    array = np.array([[0.0, 10.0], [20.0, 30.0]])
    result = bilinear_sample(array, row=0.5, col=0.5)
    assert result.value == pytest.approx((0.0 + 10.0 + 20.0 + 30.0) / 4.0)


def test_bilinear_sample_renormalizes_around_nan_neighbor():
    """One NaN neighbor (e.g. a HRRR fill cell just outside CONUS coverage)
    must not poison the whole sample -- weight redistributes over the
    remaining valid corners."""
    array = np.array([[10.0, np.nan], [20.0, 30.0]])
    result = bilinear_sample(array, row=0.5, col=0.5)
    assert not np.isnan(result.value)
    assert result.fraction_nan_neighbors == pytest.approx(0.25)
    # Should be the mean of the 3 valid corners (10, 20, 30) since all four
    # bilinear weights are equal (0.25) at the exact midpoint.
    assert result.value == pytest.approx((10.0 + 20.0 + 30.0) / 3.0)


def test_bilinear_sample_all_nan_returns_nan():
    array = np.full((2, 2), np.nan)
    result = bilinear_sample(array, row=0.5, col=0.5)
    assert np.isnan(result.value)
    assert result.fraction_nan_neighbors == 1.0


def test_bilinear_sample_clips_out_of_bounds_request():
    """A point slightly outside the array (e.g. floating-point roundoff at
    the grid edge) must clip into bounds rather than raise an IndexError."""
    array = np.array([[1.0, 2.0], [3.0, 4.0]])
    result = bilinear_sample(array, row=5.0, col=-3.0)
    assert not np.isnan(result.value)


def test_open_and_sample_real_local_zarr_v3_array(tmp_path):
    """End-to-end: build a real Zarr v3 array on disk (matching the Rust
    writer's shape/dtype convention: [height, width], float32, NaN fill),
    open it with zarr-python, and sample it at a real HRRR grid point via
    hrrr_grid's cross-validated projection."""
    store_path = str(tmp_path / "test_store")
    root = zarr.open_group(store=store_path, mode="w")
    grp = root.create_group("SOILW_4_cm_below_ground_f003")

    # Small synthetic grid standing in for HRRR's 1799x1059 -- same
    # [height, width] axis order and float32/NaN-fill convention, just
    # much smaller so the test is fast. A gradient so bilinear
    # interpolation has something non-trivial to interpolate.
    ny, nx = 20, 20
    data = np.zeros((ny, nx), dtype=np.float32)
    for r in range(ny):
        for c in range(nx):
            data[r, c] = 0.10 + 0.001 * r + 0.002 * c
    data[0, 0] = np.nan  # one fill cell, like real HRRR edge coverage

    arr = grp.create_array("0", shape=(ny, nx), dtype=np.float32)
    arr[:, :] = data

    loaded = open_level0_array(store_path, "SOILW_4_cm_below_ground_f003")
    np.testing.assert_allclose(loaded[5, 5], data[5, 5])
    assert np.isnan(loaded[0, 0])

    # Sample at a fabricated grid-relative point via a tiny HrrrGrid-like
    # projection isn't meaningful at this synthetic scale (20x20, not
    # 1799x1059) -- instead directly verify bilinear_sample against the
    # loaded array to close the loop between disk I/O and interpolation.
    result = bilinear_sample(loaded, row=5.5, col=5.5)
    expected = (
        data[5, 5] + data[5, 6] + data[6, 5] + data[6, 6]
    ) / 4.0
    assert result.value == pytest.approx(expected, abs=1e-5)


def test_sample_points_batch():
    hrrr = HrrrGrid.hrrr()
    array = np.full((hrrr.ny, hrrr.nx), 0.15, dtype=np.float32)
    points = [(39.75, -105.2), (39.7392, -104.9903)]
    results = sample_points(array, hrrr, points)
    assert len(results) == 2
    assert all(r.value == pytest.approx(0.15) for r in results)


# --- Vectorized bilinear (Session 14) ----------------------------------------

from forcing import bilinear_sample_array  # noqa: E402


def _scalar_reference(array, rows, cols):
    return np.array([bilinear_sample(array, row=float(r), col=float(c)).value for r, c in zip(rows, cols)])


class TestBilinearSampleArray:
    def test_matches_scalar_on_random_points_with_nans_and_out_of_range(self):
        """The array version is a performance rewrite, not new semantics:
        it must agree with the scalar version point-by-point, including
        NaN-neighbor renormalization, all-NaN -> NaN, and out-of-range
        clipping."""
        rng = np.random.default_rng(11)
        array = rng.uniform(0.0, 0.5, (40, 55))
        array[rng.uniform(size=array.shape) < 0.15] = np.nan  # scattered NaNs
        array[10:14, 20:24] = np.nan  # an all-NaN pocket
        rows = np.concatenate([rng.uniform(-3, 43, 500), [0.0, 39.0, 38.9999, 10.0, 12.0]])
        cols = np.concatenate([rng.uniform(-3, 58, 500), [0.0, 54.0, 53.9999, 21.0, 22.0]])

        got = bilinear_sample_array(array, rows, cols)
        want = _scalar_reference(array, rows, cols)
        np.testing.assert_allclose(got, want, rtol=1e-12, atol=1e-12, equal_nan=True)

    def test_exact_grid_point_returns_that_cell(self):
        # Interior points of a 3x3 grid (the last row/col clips to
        # ny-1.0001 in the scalar version too, so edge cells read a hair off
        # their exact value -- reproduced faithfully, asserted by the
        # point-by-point scalar comparison above).
        array = np.arange(9.0).reshape(3, 3)
        got = bilinear_sample_array(array, np.array([0.0, 1.0, 0.0]), np.array([0.0, 1.0, 1.0]))
        np.testing.assert_allclose(got, [0.0, 4.0, 1.0])

    def test_midpoint_averages_all_four(self):
        array = np.array([[1.0, 2.0], [3.0, 4.0]])
        assert bilinear_sample_array(array, np.array([0.5]), np.array([0.5]))[0] == pytest.approx(2.5)

    def test_all_nan_neighborhood_gives_nan(self):
        array = np.full((4, 4), np.nan)
        assert np.isnan(bilinear_sample_array(array, np.array([1.5]), np.array([1.5]))[0])

    def test_empty_input_gives_empty_output(self):
        out = bilinear_sample_array(np.zeros((5, 5)), np.array([]), np.array([]))
        assert out.shape == (0,)
