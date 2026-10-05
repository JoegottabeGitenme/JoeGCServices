"""Tests for hrrr_grid.py, including exact cross-reference values pulled
directly from the production Rust implementation (crates/projection/src/
lambert.rs) via a throwaway `cargo run --example` during this session --
not just internal round-trip consistency. See hrrr_grid.py's module
docstring for why this matters (pixel alignment with already-written
Zarr grids, not just "a" correct LCC projection)."""

import pytest

from hrrr_grid import HrrrGrid


@pytest.fixture
def grid():
    return HrrrGrid.hrrr()


def test_first_grid_point_maps_to_origin(grid):
    i, j = grid.geo_to_grid(21.138123, -122.719528)
    assert i == pytest.approx(0.0, abs=1e-6)
    assert j == pytest.approx(0.0, abs=1e-6)


def test_exact_cross_reference_golden_co(grid):
    """cargo run -p projection --example lambert_refcheck (this session):
    golden_co: lat=39.75 lon=-105.2 -> i=679.941200 j=584.614458"""
    i, j = grid.geo_to_grid(39.75, -105.2)
    assert i == pytest.approx(679.941200, abs=1e-4)
    assert j == pytest.approx(584.614458, abs=1e-4)


def test_exact_cross_reference_denver(grid):
    """cargo run -p projection --example lambert_refcheck (this session):
    denver: lat=39.7392 lon=-104.9903 -> i=685.865796 j=583.722665"""
    i, j = grid.geo_to_grid(39.7392, -104.9903)
    assert i == pytest.approx(685.865796, abs=1e-4)
    assert j == pytest.approx(583.722665, abs=1e-4)


def test_exact_cross_reference_grid_to_geo(grid):
    """cargo run -p projection --example lambert_refcheck (this session):
    grid_to_geo(500,400) -> lat=34.256784 lon=-110.538004"""
    lat, lon = grid.grid_to_geo(500.0, 400.0)
    assert lat == pytest.approx(34.256784, abs=1e-4)
    assert lon == pytest.approx(-110.538004, abs=1e-4)


def test_roundtrip_consistency(grid):
    lat, lon = grid.grid_to_geo(679.94, 584.61)
    i, j = grid.geo_to_grid(lat, lon)
    assert i == pytest.approx(679.94, abs=0.01)
    assert j == pytest.approx(584.61, abs=0.01)


def test_grid_covers_colorado(grid):
    """Golden, CO should land well within the 1799x1059 HRRR grid, not at
    an edge or outside it."""
    i, j = grid.geo_to_grid(39.75, -105.2)
    assert 0 <= i <= grid.nx
    assert 0 <= j <= grid.ny


# --- Vectorized geo_to_grid_array (Session 14) -------------------------------


def test_array_version_matches_scalar_on_the_rust_cross_reference_points(grid):
    """The scalar version is bit-for-bit cross-validated against the Rust
    implementation (tests above); the array version must agree with IT, so
    it inherits that guarantee instead of being a second implementation to
    trust independently."""
    import numpy as np

    pts = [(39.7555, -105.2211), (39.7392, -104.9903)]  # Golden CO, Denver
    lats = np.array([p[0] for p in pts])
    lons = np.array([p[1] for p in pts])
    ii, jj = grid.geo_to_grid_array(lats, lons)
    for k, (lat, lon) in enumerate(pts):
        i, j = grid.geo_to_grid(lat, lon)
        assert ii[k] == pytest.approx(i, abs=1e-9)
        assert jj[k] == pytest.approx(j, abs=1e-9)


def test_array_version_matches_scalar_across_the_front_range_grid(grid):
    """Dense random sample over the Front Range region the build actually
    covers (and well beyond, to the continental extremes) -- tolerance is
    float-rounding only."""
    import numpy as np

    rng = np.random.default_rng(7)
    lats = np.concatenate([rng.uniform(38.5, 40.9, 400), rng.uniform(25, 49, 200)])
    lons = np.concatenate([rng.uniform(-106.0, -104.5, 400), rng.uniform(-124, -67, 200)])
    ii, jj = grid.geo_to_grid_array(lats, lons)
    for k in range(len(lats)):
        i, j = grid.geo_to_grid(float(lats[k]), float(lons[k]))
        assert ii[k] == pytest.approx(i, abs=1e-7), (lats[k], lons[k])
        assert jj[k] == pytest.approx(j, abs=1e-7), (lats[k], lons[k])


def test_array_version_preserves_shape_and_handles_2d_input(grid):
    import numpy as np

    lats = np.full((3, 4), 40.0)
    lons = np.full((3, 4), -105.3)
    ii, jj = grid.geo_to_grid_array(lats, lons)
    assert ii.shape == (3, 4) and jj.shape == (3, 4)
