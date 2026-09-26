"""Tests for assemble_static_stack.py."""

import sys
from pathlib import Path

import numpy as np
import pytest
import rasterio
import zarr

sys.path.insert(0, str(Path(__file__).parent.parent))

from assemble_static_stack import LAYER_FILES, assemble
from grid_spec import pilot_grid_spec


def _write_layer(path, grid, fill_value=1.0, nodata_corner=False):
    arr = np.full((grid.height, grid.width), fill_value, dtype=np.float32)
    if nodata_corner:
        arr[0, 0] = np.nan
    with rasterio.open(
        path, "w", driver="GTiff", height=grid.height, width=grid.width, count=1,
        dtype=np.float32, crs=grid.crs, transform=grid.transform, nodata=np.nan,
    ) as dst:
        dst.write(arr, 1)
    return arr


class TestAssemble:
    def test_shape_mismatch_raises(self, tmp_path, monkeypatch):
        """A layer written on the WRONG grid (different shape) must be
        rejected loudly, not silently assembled misaligned with the
        others -- this is exactly the kind of mistake that would silently
        corrupt every downstream sample."""
        grid = pilot_grid_spec()
        for name, filename in LAYER_FILES.items():
            if name == "elevation":
                # Deliberately wrong shape.
                arr = np.zeros((10, 10), dtype=np.float32)
                with rasterio.open(
                    tmp_path / filename, "w", driver="GTiff", height=10, width=10, count=1,
                    dtype=np.float32, crs=grid.crs, transform=grid.transform, nodata=np.nan,
                ) as dst:
                    dst.write(arr, 1)
            else:
                _write_layer(tmp_path / filename, grid)
        # No twi_bar file needed -- should fail on the shape check first.
        with pytest.raises(ValueError, match="does not match the pinned grid spec"):
            assemble(str(tmp_path), str(tmp_path / "out.zarr"))

    def test_missing_layer_file_raises(self, tmp_path):
        with pytest.raises(FileNotFoundError):
            assemble(str(tmp_path), str(tmp_path / "out.zarr"))

    def test_full_synthetic_assembly_round_trips(self, tmp_path):
        grid = pilot_grid_spec()
        # A small grid overrides the pilot grid spec's actual size to keep
        # this test fast -- monkeypatch pilot_grid_spec isn't needed since
        # we write the layers at the REAL pilot grid's shape instead (small
        # relative to memory, this is metadata/shape only, no huge array
        # values need to be meaningful for this test).
        for name, filename in LAYER_FILES.items():
            _write_layer(tmp_path / filename, grid, fill_value=5.0 if name != "theta_s" else 0.4)
        np.savez(
            tmp_path / "pilot_twi_bar.npz",
            hrrr_row=np.array([100], dtype=np.int32),
            hrrr_col=np.array([200], dtype=np.int32),
            twi_bar=np.array([7.0], dtype=np.float32),
            n_fine_cells=np.array([1000], dtype=np.int32),
        )
        out_path = tmp_path / "out.zarr"
        assemble(str(tmp_path), str(out_path))

        root = zarr.open_group(store=str(out_path), mode="r")
        assert set(LAYER_FILES.keys()).issubset(set(root.array_keys()))
        assert root.attrs["grid_spec"]["crs"] == "EPSG:5070"
        assert root.attrs["provenance"]["k"] == 13.0
        assert root["hrrr_twi_bar_twi_bar"][:][0] == pytest.approx(7.0)


class TestRealAssembledStack:
    def test_real_data_regression(self):
        path = Path(__file__).parent.parent / "data" / "static" / "colorado-10m-pilot.zarr"
        if not path.exists():
            pytest.skip("real assembled stack not built this session (see README.md)")
        root = zarr.open_group(store=str(path), mode="r")
        for name in LAYER_FILES:
            assert name in root.array_keys()
        assert "hrrr_twi_bar_row" not in root.array_keys()  # real prefix is hrrr_twi_bar_hrrr_row
        assert "hrrr_twi_bar_hrrr_row" in root.array_keys()
        spec = root.attrs["grid_spec"]
        assert spec["crs"] == "EPSG:5070"
        assert spec["resolution_m"] == 10.0
        twi = root["twi"][:]
        valid = ~np.isnan(twi)
        assert twi[valid].mean() == pytest.approx(8.773, abs=0.01)
