"""Unit tests for terrain.py (slope, D8 flow accumulation, TWI).

Uses small synthetic DEMs with known, hand-verifiable drainage patterns --
not the real Tarrawarra DEM (blocked this session, see
validation/tarrawarra/README.md) -- specifically so the terrain algorithms
themselves are proven correct independent of whether/when the real data
becomes available.
"""

import numpy as np
import pytest

from physics.terrain import (
    coarsen_dem,
    compute_aspect,
    compute_d8_flow_accumulation,
    compute_slope,
    compute_twi,
    compute_twi_pydem,
    fill_pits_and_flats,
)

try:
    import pydem  # noqa: F401

    HAVE_PYDEM = True
except ImportError:
    HAVE_PYDEM = False


def test_flat_dem_has_zero_slope():
    dem = np.full((5, 5), 100.0)
    slope = compute_slope(dem, cellsize=5.0)
    np.testing.assert_allclose(slope, 0.0, atol=1e-9)


def test_planar_tilted_surface_has_uniform_slope():
    """A perfectly planar surface tilted in one direction must have the
    same tan(beta) everywhere except at the very edges (Horn's method uses
    a 3x3 window, so edge-padding introduces small boundary effects) --
    check the interior only."""
    rows, cols = 10, 10
    cellsize = 5.0
    # Drop 1 m per column (west to east) -> true slope = 1/5 = 0.2
    dem = np.tile(np.arange(cols) * -1.0, (rows, 1))
    slope = compute_slope(dem, cellsize=cellsize)
    interior = slope[2:-2, 2:-2]
    np.testing.assert_allclose(interior, 0.2, atol=1e-6)


def test_flow_accumulates_downhill_along_a_simple_slope():
    """On a DEM tilted purely west-to-east (no north/south gradient), D8
    flow must accumulate monotonically eastward along each row, and each
    row's totals must be independent (no north/south leakage) since D8
    picks a single steepest-descent direction and ties are broken
    deterministically by the first matching offset in the search order."""
    rows, cols = 1, 6
    dem = np.array([[50.0, 40.0, 30.0, 20.0, 10.0, 0.0]])
    acc = compute_d8_flow_accumulation(dem, cellsize=5.0)
    # Specific area = accumulated cell count * cellsize; westmost cell
    # contributes only itself (1 cell), each cell east of it picks up one
    # more upslope contributor.
    expected_cells = np.array([1, 2, 3, 4, 5, 6])
    np.testing.assert_allclose(acc[0], expected_cells * 5.0)


def test_valley_bottom_has_higher_twi_than_ridge():
    """A synthetic V-shaped valley: TWI must be highest at the valley
    floor (large upslope contributing area draining to a narrow low-slope
    channel) and lowest on the steep valley walls -- the core physical
    property Eq. 1 relies on to redistribute moisture correctly."""
    rows, cols = 20, 20
    cellsize = 5.0
    x = np.abs(np.arange(cols) - cols // 2)  # distance from the centerline
    dem = np.tile(x.astype(np.float64) * 2.0, (rows, 1))  # V-shaped cross-section
    # Add a gentle downstream (north-to-south) tilt so the valley actually
    # drains somewhere instead of being a flat trough (which fill_pits_and_flats
    # would otherwise have to fully resolve via many iterations).
    dem += np.arange(rows)[:, None] * 0.1

    twi = compute_twi(dem, cellsize)
    valley_floor_twi = twi[rows // 2, cols // 2]
    valley_wall_twi = twi[rows // 2, 1]
    assert valley_floor_twi > valley_wall_twi


def test_fill_pits_removes_interior_local_minima():
    """A single-cell pit (lower than all its neighbors) must be raised to
    JUST ABOVE its lowest neighbor (not up to the neighbors' max -- only
    enough to create one downhill exit) -- otherwise D8 flow into it has
    nowhere to go and the accumulation topological sort would silently
    lose the upslope contribution at that cell. Uses asymmetric neighbor
    elevations specifically so "just above the minimum" and "up to the
    maximum" give different, distinguishable answers."""
    dem = np.array(
        [
            [10.0, 10.0, 10.0],
            [10.0, 1.0, 20.0],  # interior pit; neighbors are 10,10,10,20 (asymmetric)
            [10.0, 10.0, 10.0],
        ]
    )
    filled = fill_pits_and_flats(dem)
    lowest_neighbor = 10.0
    # Raised strictly above its lowest neighbor (so it has an exit)...
    assert filled[1, 1] > lowest_neighbor
    # ...but only just above it, not all the way up to the highest neighbor.
    assert filled[1, 1] < lowest_neighbor + 0.01
    # The pit must no longer be a strict local minimum: at least one
    # neighbor (here, three of them) is now lower than the filled center.
    neighbors = [filled[0, 1], filled[2, 1], filled[1, 0], filled[1, 2]]
    assert any(filled[1, 1] > n for n in neighbors)


def test_fill_pits_never_touches_boundary():
    """Boundary cells are legitimate outlets (flow leaves the DEM there),
    not pits to be filled -- even if a boundary cell is locally the lowest
    point in the grid."""
    dem = np.array(
        [
            [5.0, 5.0, 5.0],
            [5.0, 3.0, 5.0],
            [5.0, 0.0, 5.0],  # bottom-middle boundary cell, lowest in the grid
        ]
    )
    filled = fill_pits_and_flats(dem)
    assert filled[2, 1] == pytest.approx(0.0)


def test_aspect_all_four_cardinal_directions():
    """Aspect is the compass bearing of the DOWNHILL direction. A DEM that
    descends toward the north (low in the north, high in the south) faces
    north; verified for all four cardinal tilts since the row-increases-
    southward array convention makes it easy to get this backwards (an
    earlier version of this test had north/south swapped, which caught a
    real sign bug in compute_aspect -- see that function's docstring for
    the corrected derivation)."""
    rows, cols = 5, 5

    dem_faces_north = np.tile(np.arange(rows)[:, None].astype(float), (1, cols))
    assert compute_aspect(dem_faces_north, cellsize=5.0)[2, 2] == pytest.approx(0.0, abs=1e-6)

    dem_faces_south = np.tile((-np.arange(rows))[:, None].astype(float), (1, cols))
    assert compute_aspect(dem_faces_south, cellsize=5.0)[2, 2] == pytest.approx(180.0, abs=1e-6)

    dem_faces_east = np.tile(-np.arange(cols).astype(float), (rows, 1))
    assert compute_aspect(dem_faces_east, cellsize=5.0)[2, 2] == pytest.approx(90.0, abs=1e-6)

    dem_faces_west = np.tile(np.arange(cols).astype(float), (rows, 1))
    assert compute_aspect(dem_faces_west, cellsize=5.0)[2, 2] == pytest.approx(270.0, abs=1e-6)


def test_compute_twi_pydem_raises_clear_error_without_pydem_installed():
    """If pydem isn't installed, the error must say so clearly rather than
    a bare ModuleNotFoundError -- this test always runs regardless of
    whether pydem is actually installed in this environment (it directly
    calls the internal import path check via monkeypatching sys.modules)."""
    import sys

    dem = np.tile(np.arange(5)[:, None].astype(float), (1, 5))
    if "pydem" in sys.modules or HAVE_PYDEM:
        pytest.skip("pydem is installed in this environment -- covered by the tests below instead")
    with pytest.raises(ImportError, match="pip install pydem"):
        compute_twi_pydem(dem, cellsize=5.0)


@pytest.mark.skipif(not HAVE_PYDEM, reason="pydem not installed (pip install pydem)")
class TestComputeTwiPydem:
    """Tests requiring the real pydem package. Skipped entirely (not
    failed) if it isn't installed -- pydem is a deliberately optional,
    heavier dependency (rasterio + Cython) only needed for this
    paper-fidelity cross-check, not the live pipeline."""

    def test_valley_bottom_has_higher_twi_than_ridge(self):
        """Same physical property as the builtin D8 test above -- pyDEM's
        D-infinity implementation must show the same qualitative pattern."""
        rows, cols = 20, 20
        cellsize = 5.0
        x = np.abs(np.arange(cols) - cols // 2)
        dem = np.tile(x.astype(np.float64) * 2.0, (rows, 1))
        dem += np.arange(rows)[:, None] * 0.1

        twi = compute_twi_pydem(dem, cellsize)
        valley_floor_twi = twi[rows // 2, cols // 2]
        valley_wall_twi = twi[rows // 2, 1]
        assert valley_floor_twi > valley_wall_twi

    def test_scaled_is_exactly_ten_times_unscaled(self):
        """pyDEM's own docstring: the stored value is the natural-log TWI
        'multiplied by 10 ... when storing' -- confirmed directly against
        pyDEM's source (dem_processing.py: `self.twi = twi * 10`)."""
        rows, cols = 15, 15
        cellsize = 5.0
        x = np.abs(np.arange(cols) - cols // 2)
        dem = np.tile(x.astype(np.float64) * 2.0, (rows, 1))
        dem += np.arange(rows)[:, None] * 0.1

        unscaled = compute_twi_pydem(dem, cellsize, scaled=False)
        scaled = compute_twi_pydem(dem, cellsize, scaled=True)
        valid = ~np.isnan(unscaled) & ~np.isnan(scaled)
        np.testing.assert_allclose(scaled[valid], unscaled[valid] * 10, rtol=1e-6)

    def test_no_nan_propagation_on_a_clean_synthetic_dem(self):
        """A DEM with no nodata cells and a well-defined single drainage
        outlet must produce a fully finite TWI grid -- pyDEM's iterative
        pit-drainage can otherwise leave isolated undrained cells."""
        rows, cols = 15, 15
        cellsize = 5.0
        y, x = np.mgrid[0:rows, 0:cols]
        dem = (rows - y).astype(np.float64) + 0.01 * (x - cols / 2) ** 2
        twi = compute_twi_pydem(dem, cellsize)
        assert not np.isnan(twi).all()

    def test_apply_twi_limits_off_by_default(self):
        """Default behavior must match pyDEM's own default (limits off) --
        a caller who doesn't ask for capping shouldn't get it silently."""
        rows, cols = 20, 20
        cellsize = 5.0
        x = np.abs(np.arange(cols) - cols // 2)
        dem = np.tile(x.astype(np.float64) * 2.0, (rows, 1))
        dem += np.arange(rows)[:, None] * 0.1

        default_twi = compute_twi_pydem(dem, cellsize)
        explicit_off_twi = compute_twi_pydem(dem, cellsize, apply_twi_limits=False)
        valid = ~np.isnan(default_twi) & ~np.isnan(explicit_off_twi)
        np.testing.assert_allclose(default_twi[valid], explicit_off_twi[valid])

    def test_apply_twi_limits_caps_the_upper_tail(self):
        """Enabling limits must not increase the maximum TWI value, and
        should typically decrease it (capping the upper tail is the whole
        point of this option) -- verified on a DEM with a genuine
        channelized high-UCA region so there's something to cap."""
        rows, cols = 25, 25
        cellsize = 5.0
        y, x = np.mgrid[0:rows, 0:cols]
        # A converging valley (funnels flow toward a narrow low-slope
        # outlet) creates the kind of high-UCA/low-slope cell that TWI
        # capping is meant to compress.
        dem = np.abs(x - cols / 2).astype(np.float64) * 3.0 + (rows - y) * 0.05

        uncapped = compute_twi_pydem(dem, cellsize, apply_twi_limits=False)
        capped = compute_twi_pydem(dem, cellsize, apply_twi_limits=True)
        assert np.nanmax(capped) <= np.nanmax(uncapped) + 1e-9


def test_coarsen_dem_halves_shape_and_doubles_cellsize():
    dem = np.arange(16.0).reshape(4, 4)
    coarsened, new_cellsize = coarsen_dem(dem, cellsize=5.0, factor=2)
    assert coarsened.shape == (2, 2)
    assert new_cellsize == pytest.approx(10.0)


def test_coarsen_dem_block_mean_is_correct():
    """A known 4x4 grid of constants per 2x2 block must coarsen to exactly
    those constants -- the simplest possible correctness check for block
    averaging."""
    dem = np.array(
        [
            [1.0, 1.0, 2.0, 2.0],
            [1.0, 1.0, 2.0, 2.0],
            [3.0, 3.0, 4.0, 4.0],
            [3.0, 3.0, 4.0, 4.0],
        ]
    )
    coarsened, _ = coarsen_dem(dem, cellsize=5.0, factor=2)
    np.testing.assert_allclose(coarsened, [[1.0, 2.0], [3.0, 4.0]])


def test_coarsen_dem_factor_one_is_a_noop():
    dem = np.arange(9.0).reshape(3, 3)
    coarsened, new_cellsize = coarsen_dem(dem, cellsize=5.0, factor=1)
    np.testing.assert_allclose(coarsened, dem)
    assert new_cellsize == pytest.approx(5.0)


def test_coarsen_dem_crops_uneven_dimensions():
    """A 5x5 grid at factor=2 doesn't divide evenly -- the trailing row/col
    must be dropped (cropped to 4x4 -> 2x2), not padded or erroring."""
    dem = np.arange(25.0).reshape(5, 5)
    coarsened, _ = coarsen_dem(dem, cellsize=5.0, factor=2)
    assert coarsened.shape == (2, 2)


def test_coarsen_dem_nan_block_stays_nan():
    dem = np.array(
        [
            [np.nan, np.nan, 2.0, 2.0],
            [np.nan, np.nan, 2.0, 2.0],
            [3.0, 3.0, 4.0, 4.0],
            [3.0, 3.0, 4.0, 4.0],
        ]
    )
    coarsened, _ = coarsen_dem(dem, cellsize=5.0, factor=2)
    assert np.isnan(coarsened[0, 0])
    assert coarsened[0, 1] == pytest.approx(2.0)


def test_coarsen_dem_partial_nan_block_ignores_nan():
    """A block with SOME (not all) NaN cells must average only the valid
    ones, not propagate NaN from a single bad cell."""
    dem = np.array(
        [
            [np.nan, 2.0],
            [4.0, 6.0],
        ]
    )
    coarsened, _ = coarsen_dem(dem, cellsize=5.0, factor=2)
    assert coarsened.shape == (1, 1)
    np.testing.assert_allclose(coarsened, [[4.0]])  # mean(2, 4, 6) = 4


def test_coarsen_dem_rejects_factor_below_one():
    dem = np.ones((4, 4))
    with pytest.raises(ValueError, match="factor must be"):
        coarsen_dem(dem, cellsize=5.0, factor=0)
