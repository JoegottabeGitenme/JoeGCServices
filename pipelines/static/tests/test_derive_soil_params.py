"""Tests for derive_soil_params.py -- the POLARIS texture -> Noah
SOILPARM.TBL pipeline, reused verbatim from every validation session."""

import sys
from pathlib import Path

import numpy as np
import pytest
import rasterio

sys.path.insert(0, str(Path(__file__).parent.parent))
sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "services" / "trail-physics"))

from derive_soil_params import build_lookup_tables, derive_soil_params
from physics.soil_texture import soil_hydraulic_properties


class TestBuildLookupTables:
    def test_lookup_table_matches_direct_call_at_every_valid_cell(self):
        """The whole point of the lookup-table optimization is that it
        must be EXACTLY equivalent to calling soil_hydraulic_properties
        directly -- spot-checked at several real points, not just trusted
        by construction."""
        theta_s_table, theta_wilt_table = build_lookup_tables()
        for sand, clay in [(0, 0), (50, 20), (30, 30), (10, 5), (100, 0), (0, 100)]:
            expected = soil_hydraulic_properties(float(sand), float(clay))
            assert theta_s_table[sand, clay] == pytest.approx(expected.maxsmc)
            assert theta_wilt_table[sand, clay] == pytest.approx(expected.wltsmc)

    def test_invalid_sand_clay_combos_are_nan(self):
        """sand=60, clay=60 (sums to 120%) is not a physically valid
        soil -- must be NaN in the table, not silently populated with a
        wrong nearby value."""
        theta_s_table, _ = build_lookup_tables()
        assert np.isnan(theta_s_table[60, 60])

    def test_theta_s_exceeds_theta_wilt_everywhere_valid(self):
        theta_s_table, theta_wilt_table = build_lookup_tables()
        valid = ~np.isnan(theta_s_table)
        assert np.all(theta_s_table[valid] > theta_wilt_table[valid])


class TestDeriveSoilParams:
    def test_synthetic_grid_end_to_end(self, tmp_path):
        """A small synthetic sand/clay grid with a known texture (pure
        sand, sand=90 clay=5) must produce the exact same theta_s/
        theta_wilt as a direct soil_hydraulic_properties call."""
        transform = rasterio.transform.from_origin(0, 10, 1.0, 1.0)
        sand = np.full((10, 10), 90.0, dtype=np.float32)
        clay = np.full((10, 10), 5.0, dtype=np.float32)
        sand_path = tmp_path / "sand.tif"
        clay_path = tmp_path / "clay.tif"
        for path, arr in [(sand_path, sand), (clay_path, clay)]:
            with rasterio.open(
                path, "w", driver="GTiff", height=10, width=10, count=1,
                dtype=np.float32, crs="EPSG:5070", transform=transform, nodata=np.nan,
            ) as dst:
                dst.write(arr, 1)

        derive_soil_params(str(sand_path), str(clay_path), str(tmp_path))

        expected = soil_hydraulic_properties(90.0, 5.0)
        with rasterio.open(tmp_path / "pilot_theta_s.tif") as src:
            theta_s = src.read(1)
        with rasterio.open(tmp_path / "pilot_theta_wilt.tif") as src:
            theta_wilt = src.read(1)
        assert np.allclose(theta_s, expected.maxsmc)
        assert np.allclose(theta_wilt, expected.wltsmc)

    def test_nan_input_propagates_to_nan_output_not_a_crash(self, tmp_path):
        transform = rasterio.transform.from_origin(0, 10, 1.0, 1.0)
        sand = np.full((10, 10), 50.0, dtype=np.float32)
        sand[0, 0] = np.nan
        clay = np.full((10, 10), 20.0, dtype=np.float32)
        sand_path = tmp_path / "sand.tif"
        clay_path = tmp_path / "clay.tif"
        for path, arr in [(sand_path, sand), (clay_path, clay)]:
            with rasterio.open(
                path, "w", driver="GTiff", height=10, width=10, count=1,
                dtype=np.float32, crs="EPSG:5070", transform=transform, nodata=np.nan,
            ) as dst:
                dst.write(arr, 1)

        derive_soil_params(str(sand_path), str(clay_path), str(tmp_path))
        with rasterio.open(tmp_path / "pilot_theta_s.tif") as src:
            theta_s = src.read(1)
        assert np.isnan(theta_s[0, 0])
        assert not np.isnan(theta_s[5, 5])


class TestRealPilotOutputs:
    DATA_DIR = Path(__file__).parent.parent / "data" / "static"

    def test_real_data_regression(self):
        theta_s_path = self.DATA_DIR / "pilot_theta_s.tif"
        theta_wilt_path = self.DATA_DIR / "pilot_theta_wilt.tif"
        if not theta_s_path.exists() or not theta_wilt_path.exists():
            pytest.skip("real pilot soil params not built this session (see README.md)")
        with rasterio.open(theta_s_path) as src:
            theta_s = src.read(1)
        with rasterio.open(theta_wilt_path) as src:
            theta_wilt = src.read(1)
        valid = ~np.isnan(theta_s) & ~np.isnan(theta_wilt)
        # Real Session 11 result: theta_s 0.404-0.476, theta_wilt 0.028-0.138
        # (silt-loam-to-sandy-loam range, matching Front Range decomposed
        # granite/sandy soils and valley-bottom finer soils).
        assert 0.35 < theta_s[valid].min()
        assert theta_s[valid].max() < 0.55
        assert theta_wilt[valid].min() < 0.05
        assert theta_wilt[valid].max() < 0.20
        assert np.all(theta_s[valid] > theta_wilt[valid])
