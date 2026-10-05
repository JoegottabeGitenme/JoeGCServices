"""Tests for derive_terrain_pydem.py. Full pyDEM runs on real data are
exercised by running the script directly (see README.md) -- these tests
cover masking logic with small synthetic arrays plus real-data regression
checks on the committed pilot outputs (skip gracefully if absent)."""

import sys
from pathlib import Path

import numpy as np
import pytest
import rasterio

sys.path.insert(0, str(Path(__file__).parent.parent))

from derive_terrain_pydem import (
    BLOCK_SIZE,
    DEFAULT_MARGIN_CELLS,
    DEFAULT_TILE_SIZE,
    TWI_APPLY_LIMITS,
    derive_terrain_layers,
    derive_tile,
    plan_tiles,
)


def test_frozen_twi_apply_limits_matches_validated_configuration():
    """This constant must remain True -- it's the exact configuration
    validated across all three sessions (Tarrawarra TDR/NMM, Shale Hills).
    A silent flip to False here would deploy an unvalidated TWI variant."""
    assert TWI_APPLY_LIMITS is True


class TestRealPilotOutputs:
    DATA_DIR = Path(__file__).parent.parent / "data" / "static"

    def _skip_if_absent(self, name):
        path = self.DATA_DIR / name
        if not path.exists():
            pytest.skip(f"real pilot output {path} not built this session (see README.md)")
        return path

    def test_twi_stats_are_plausible_and_reproducible(self):
        """Locked-in regression values from the real Session 11 run --
        confirms re-running produces the SAME numbers (pyDEM is
        deterministic), not just "some" plausible output."""
        path = self._skip_if_absent("pilot_twi.tif")
        with rasterio.open(path) as src:
            arr = src.read(1)
        valid = arr[~np.isnan(arr)]
        assert valid.mean() == pytest.approx(8.773, abs=0.01)
        assert valid.std() == pytest.approx(1.756, abs=0.01)

    def test_slope_aspect_nodata_is_superset_of_dem_nodata(self):
        """Real finding (Session 11): slope/aspect's NaN footprint is a
        SUPERSET of the DEM's, not identical to it -- Horn's method's 3x3
        kernel produces NaN at any cell whose neighborhood touches a real
        nodata cell (confirmed: 16,599 cells have a valid DEM elevation
        but an undefined slope, all adjacent to the DEM's own real nodata
        regions), not a different, drifted or buggy nodata footprint. The
        reverse (DEM is NaN but slope isn't) must never happen -- that
        WOULD indicate a masking bug (a fabricated slope at a location
        with no real elevation data)."""
        dem_path = self._skip_if_absent("pilot_dem.tif")
        slope_path = self._skip_if_absent("pilot_slope.tif")
        aspect_path = self._skip_if_absent("pilot_aspect.tif")
        with rasterio.open(dem_path) as src:
            dem = src.read(1)
        with rasterio.open(slope_path) as src:
            slope = src.read(1)
        with rasterio.open(aspect_path) as src:
            aspect = src.read(1)
        dem_nan = np.isnan(dem)
        assert np.sum(dem_nan & ~np.isnan(slope)) == 0
        assert np.sum(dem_nan & ~np.isnan(aspect)) == 0
        # The superset is real but small relative to the DEM's own nodata
        # count -- a loose upper bound, not a tight pin (exact count is
        # sensitive to nodata region shapes, which could shift slightly on
        # a re-run with different upstream tile mosaicking).
        extra_slope_nan = np.sum(~dem_nan & np.isnan(slope))
        assert 0 < extra_slope_nan < dem_nan.sum() * 0.05

    def test_aspect_is_a_valid_compass_bearing(self):
        path = self._skip_if_absent("pilot_aspect.tif")
        with rasterio.open(path) as src:
            arr = src.read(1)
        valid = arr[~np.isnan(arr)]
        assert valid.min() >= 0.0
        assert valid.max() <= 360.0

    def test_slope_is_nonnegative_tangent(self):
        path = self._skip_if_absent("pilot_slope.tif")
        with rasterio.open(path) as src:
            arr = src.read(1)
        valid = arr[~np.isnan(arr)]
        assert valid.min() >= 0.0


# =============================================================================
# Tiling (Session 14) -- a monolithic pyDEM run needs ~198 bytes/cell, so the
# Front Range's 345.6M cells (~68 GB) cannot be done in one call.
# =============================================================================


def _synthetic_dem(ny, nx, seed=1):
    """Smooth ridges/valleys on a tilted plane plus faint noise -- real
    drainage structure for pyDEM's flow router (pure random noise, tried
    first in Session 14, makes pyDEM's pit-draining pathologically slow and
    isn't terrain-like at all)."""
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:ny, 0:nx].astype(float)
    return 2500 + 0.15 * y + 18 * np.sin(x / 55.0) + 12 * np.cos(y / 40.0 + x / 90.0) + rng.normal(0, 0.15, (ny, nx))


def test_tile_defaults_are_block_aligned_and_margin_matches_measurement():
    assert DEFAULT_TILE_SIZE % BLOCK_SIZE == 0
    # Session 14 measurement: converged by 100 cells on 3 real pilot
    # locations; 250 is the deliberate 2.5x headroom (see module docstring).
    assert DEFAULT_MARGIN_CELLS >= 100


class TestPlanTiles:
    @pytest.mark.parametrize("height,width,tile,margin", [(100, 100, 40, 10), (97, 211, 64, 16), (50, 50, 64, 8), (512, 512, 512, 0)])
    def test_cores_partition_the_grid_exactly(self, height, width, tile, margin):
        covered = np.zeros((height, width), dtype=int)
        for t in plan_tiles(height, width, tile, margin):
            r, c = int(t.core.row_off), int(t.core.col_off)
            covered[r:r + int(t.core.height), c:c + int(t.core.width)] += 1
        assert (covered == 1).all(), "every cell must belong to exactly one core tile"

    def test_padded_window_contains_core_and_stays_inside_grid(self):
        for t in plan_tiles(97, 211, 64, 16):
            assert t.padded.row_off <= t.core.row_off and t.padded.col_off <= t.core.col_off
            assert t.padded.row_off + t.padded.height >= t.core.row_off + t.core.height
            assert t.padded.col_off + t.padded.width >= t.core.col_off + t.core.width
            assert t.padded.row_off >= 0 and t.padded.col_off >= 0
            assert t.padded.row_off + t.padded.height <= 97 and t.padded.col_off + t.padded.width <= 211

    def test_front_range_tile_count_is_what_the_runtime_estimate_assumes(self):
        # 13770 x 25096 at 4096 -> 4 x 7 = 28 tiles (some all-nodata corners,
        # which are skipped cheaply) -- the basis of the ~1h build estimate.
        assert len(plan_tiles(25096, 13770, DEFAULT_TILE_SIZE, DEFAULT_MARGIN_CELLS)) == 28


class TestDeriveTileEquivalence:
    CORE = (100, 150, 300, 300)  # row_off, col_off, h, w within a 700x800 DEM

    @pytest.fixture(scope="class")
    def dem_and_full(self):
        dem = _synthetic_dem(700, 800)
        return dem, derive_tile(dem, 10.0, 0, 0, 700, 800)

    def _tiled(self, dem, margin):
        r, c, h, w = self.CORE
        r0, r1, c0, c1 = max(0, r - margin), min(700, r + h + margin), max(0, c - margin), min(800, c + w + margin)
        return derive_tile(dem[r0:r1, c0:c1], 10.0, r - r0, c - c0, h, w)

    def test_tiled_matches_monolithic(self, dem_and_full):
        dem, full = dem_and_full
        r, c, h, w = self.CORE
        twi, slope, aspect = self._tiled(dem, margin=100)
        diff = np.abs(twi - full[0][r:r + h, c:c + w])
        assert np.nanmean(diff) < 1e-3
        assert np.nanmean(diff > 0.1) < 1e-3
        # Horn's method needs only a 1-cell halo -- exact, not approximate.
        np.testing.assert_array_equal(slope, full[1][r:r + h, c:c + w])
        np.testing.assert_array_equal(aspect, full[2][r:r + h, c:c + w])

    def test_zero_margin_is_measurably_worse__negative_control(self, dem_and_full):
        """Proves the test above can fail: with no margin the tile edges
        really are wrong. Without this, a pass above could just mean the
        synthetic DEM is too easy to distinguish anything."""
        dem, full = dem_and_full
        r, c, h, w = self.CORE
        twi0, slope0, _ = self._tiled(dem, margin=0)
        twi100, _, _ = self._tiled(dem, margin=100)
        err0 = np.nanmean(np.abs(twi0 - full[0][r:r + h, c:c + w]))
        err100 = np.nanmean(np.abs(twi100 - full[0][r:r + h, c:c + w]))
        assert err0 > 20 * max(err100, 1e-6)
        assert np.nanmax(np.abs(slope0 - full[1][r:r + h, c:c + w])) > 0  # edge kernel differs too

    def test_slope_aspect_nan_wherever_dem_is_nan(self):
        dem = _synthetic_dem(120, 120)
        dem[40:60, 40:60] = np.nan
        _, slope, aspect = derive_tile(dem, 10.0, 0, 0, 120, 120)
        assert np.isnan(slope[40:60, 40:60]).all()
        assert np.isnan(aspect[40:60, 40:60]).all()


def _write_dem(path, dem):
    from rasterio.transform import from_origin

    with rasterio.open(
        path, "w", driver="GTiff", height=dem.shape[0], width=dem.shape[1], count=1, dtype="float32",
        crs="EPSG:5070", transform=from_origin(-1_000_000.0, 2_000_000.0, 10.0, 10.0), nodata=np.nan,
    ) as dst:
        dst.write(dem.astype(np.float32), 1)


class TestDeriveTerrainLayersEndToEnd:
    def test_tiled_run_matches_monolithic_and_writes_every_layer(self, tmp_path):
        dem = _synthetic_dem(700, 800).astype(np.float32)
        dem_path = tmp_path / "t_dem.tif"
        _write_dem(dem_path, dem)

        derive_terrain_layers(str(dem_path), str(tmp_path), prefix="t", tile_size=512, margin=100)

        full_twi, full_slope, full_aspect = derive_tile(dem.astype(np.float64), 10.0, 0, 0, 700, 800)
        for name, full in [("twi", full_twi), ("slope", full_slope), ("aspect", full_aspect)]:
            with rasterio.open(tmp_path / f"t_{name}.tif") as src:
                got = src.read(1)
            assert got.shape == (700, 800)
            # pyDEM itself leaves a handful of NaN TWI cells on a fully
            # valid DEM (7 of 560,000 monolithically here; 685 of 14.2M in
            # the Session 11 pilot) -- so the check is "no more NaN than
            # that edge behavior", not "zero NaN". A truly unwritten tile
            # would be thousands of NaN/zero cells.
            assert np.isnan(got).mean() < 1e-4, f"{name}: far more NaN than pyDEM's own numerical edge cases"
            both = ~np.isnan(got) & ~np.isnan(full)
            if name == "twi":
                assert np.mean(np.abs(got[both] - full[both])) < 1e-3
            else:
                np.testing.assert_allclose(got[both], full[both].astype(np.float32), atol=1e-4)

    def test_all_nodata_core_tile_is_written_as_nan_not_zero(self, tmp_path):
        """Unwritten compressed GeoTIFF blocks read back as 0, which would
        be a silently-plausible TWI/slope -- the exact corner case the
        Albers-rotation nodata corners of the real grid hit."""
        dem = _synthetic_dem(700, 800).astype(np.float32)
        dem[0:512, 512:800] = np.nan  # the whole top-right core tile
        dem_path = tmp_path / "n_dem.tif"
        _write_dem(dem_path, dem)

        derive_terrain_layers(str(dem_path), str(tmp_path), prefix="n", tile_size=512, margin=100)

        for name in ("twi", "slope", "aspect"):
            with rasterio.open(tmp_path / f"n_{name}.tif") as src:
                got = src.read(1)
            assert np.isnan(got[0:512, 512:800]).all(), f"{name}: all-nodata tile must be NaN, not 0"

    def test_resume_skips_done_tiles_and_reproduces_identical_output(self, tmp_path):
        import json

        dem = _synthetic_dem(700, 800).astype(np.float32)
        dem_path = tmp_path / "r_dem.tif"
        _write_dem(dem_path, dem)
        derive_terrain_layers(str(dem_path), str(tmp_path), prefix="r", tile_size=512, margin=100)
        with rasterio.open(tmp_path / "r_twi.tif") as src:
            first = src.read(1)

        # Simulate a crash after the first tile: progress says 1 done.
        prog = tmp_path / "r_terrain_progress.json"
        saved = json.loads(prog.read_text())
        assert len(saved["done"]) == 4
        saved["done"] = saved["done"][:1]
        prog.write_text(json.dumps(saved))

        derive_terrain_layers(str(dem_path), str(tmp_path), prefix="r", tile_size=512, margin=100)
        with rasterio.open(tmp_path / "r_twi.tif") as src:
            second = src.read(1)
        np.testing.assert_array_equal(first, second)
        assert len(json.loads(prog.read_text())["done"]) == 4

    def test_changed_tiling_parameters_invalidate_stale_progress(self, tmp_path):
        """A progress file from a different tile_size/margin describes a
        different partition -- resuming from it would leave holes."""
        import json

        dem = _synthetic_dem(300, 300).astype(np.float32)
        dem_path = tmp_path / "s_dem.tif"
        _write_dem(dem_path, dem)
        derive_terrain_layers(str(dem_path), str(tmp_path), prefix="s", tile_size=512, margin=50)
        prog = tmp_path / "s_terrain_progress.json"
        saved = json.loads(prog.read_text())
        saved["margin"] = 999
        prog.write_text(json.dumps(saved))

        derive_terrain_layers(str(dem_path), str(tmp_path), prefix="s", tile_size=512, margin=50)
        assert json.loads(prog.read_text())["margin"] == 50


class TestSeamStats:
    """seam_stats checks BOTH tile-boundary directions of a finished TWI
    layer (the first version only checked row seams)."""

    def test_reports_every_interior_seam_in_both_directions(self, tmp_path):
        from derive_terrain_pydem import seam_stats

        dem = _synthetic_dem(700, 800).astype(np.float32)
        dem_path = tmp_path / "s_dem.tif"
        _write_dem(dem_path, dem)
        derive_terrain_layers(str(dem_path), str(tmp_path), prefix="s", tile_size=512, margin=100)

        stats = seam_stats(str(tmp_path / "s_twi.tif"), tile_size=512)
        # 700 rows -> one interior row seam (512); 800 cols -> one interior col seam (512).
        assert [(s["axis"], s["index"]) for s in stats] == [("row", 512), ("col", 512)]
        for s in stats:
            assert np.isfinite(s["ratio"]) and s["background"] > 0

    def test_a_properly_margined_tiling_has_no_visible_seam(self, tmp_path):
        from derive_terrain_pydem import seam_stats

        dem = _synthetic_dem(700, 800).astype(np.float32)
        dem_path = tmp_path / "m_dem.tif"
        _write_dem(dem_path, dem)
        derive_terrain_layers(str(dem_path), str(tmp_path), prefix="m", tile_size=512, margin=100)
        for s in seam_stats(str(tmp_path / "m_twi.tif"), tile_size=512):
            assert 0.8 < s["ratio"] < 1.25, s

    def test_a_planted_seam_discontinuity_is_detected(self, tmp_path):
        """Negative control: the detector must actually flag a bad seam. Plant
        a +5 offset on one side of a row boundary in a smooth TWI raster."""
        from derive_terrain_pydem import seam_stats
        from rasterio.transform import from_origin

        twi = np.tile(np.linspace(5, 9, 600, dtype=np.float32), (700, 1))
        twi[512:, :] += 5.0  # a hard step exactly at the tile boundary
        path = tmp_path / "planted_twi.tif"
        with rasterio.open(path, "w", driver="GTiff", height=700, width=600, count=1, dtype="float32",
                           crs="EPSG:5070", transform=from_origin(0, 7000, 10, 10), nodata=np.nan) as dst:
            dst.write(twi, 1)
        row_seam = next(s for s in seam_stats(str(path), tile_size=512) if s["axis"] == "row")
        assert row_seam["ratio"] > 5 or not np.isfinite(row_seam["ratio"]), row_seam
