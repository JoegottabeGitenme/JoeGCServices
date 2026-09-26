#!/usr/bin/env python3
"""Assemble every WS1 layer (elevation, TWI, slope, aspect, theta_s,
theta_wilt, plus the coarse HRRR-cell lambda_bar lookup) into a single
Zarr v3 group, with the grid spec and full data provenance written into
the group's own attrs -- so main.py's future sampler (Phase B) can read
the grid definition from the data itself, never re-deriving or assuming
it (the exact gap `build_corridor_mask.py` flagged: "that grid's exact
origin/extent doesn't exist yet").

Layout (`static/colorado-10m/pilot/`, a single Zarr v3 group):
    elevation, twi, slope, aspect, theta_s, theta_wilt   -- (height, width) 2D arrays
    hrrr_twi_bar_row, hrrr_twi_bar_col, hrrr_twi_bar_value, hrrr_twi_bar_n  -- 1D, one entry per covered HRRR cell

This intentionally does NOT follow `forcing.py::open_level0_array`'s
"level-0 pyramid" convention (`group["0"]`) -- that convention is for
multiscale HRRR/GFS grids written by the Rust ingester's `write_multiscale`;
this is a different kind of asset (one region's full named-layer stack, not
a resolution pyramid of one parameter), so a plain named-array layout is
the honest fit, not a forced reuse of an unrelated convention.
"""

from __future__ import annotations

import datetime
import json
from pathlib import Path

import numpy as np
import rasterio
import zarr

from grid_spec import pilot_grid_spec

LAYER_FILES = {
    "elevation": "pilot_dem.tif",
    "twi": "pilot_twi.tif",
    "slope": "pilot_slope.tif",
    "aspect": "pilot_aspect.tif",
    "theta_s": "pilot_theta_s.tif",
    "theta_wilt": "pilot_theta_wilt.tif",
}

CHUNK_SIZE = 512


def assemble(data_dir: str, output_path: str) -> None:
    grid = pilot_grid_spec()
    data_dir = Path(data_dir)

    root = zarr.open_group(store=output_path, mode="w")

    for name, filename in LAYER_FILES.items():
        path = data_dir / filename
        if not path.exists():
            raise FileNotFoundError(f"{path} not found -- run the corresponding derive/fetch script first")
        with rasterio.open(path) as src:
            arr = src.read(1)
            if arr.shape != (grid.height, grid.width):
                raise ValueError(
                    f"{filename}: shape {arr.shape} does not match the pinned grid spec "
                    f"({grid.height}, {grid.width}) -- every layer must be resampled onto "
                    f"EXACTLY the same grid before assembly."
                )
        chunks = (min(CHUNK_SIZE, grid.height), min(CHUNK_SIZE, grid.width))
        z_arr = root.create_array(name, shape=arr.shape, chunks=chunks, dtype=np.float32, overwrite=True)
        z_arr[:] = arr.astype(np.float32)
        valid = arr[~np.isnan(arr)]
        print(f"  {name}: {arr.shape}, {valid.size}/{arr.size} valid cells, mean={valid.mean():.3f}")

    twi_bar_path = data_dir / "pilot_twi_bar.npz"
    if not twi_bar_path.exists():
        raise FileNotFoundError(f"{twi_bar_path} not found -- run derive_coarse_twi.py first")
    npz = np.load(twi_bar_path)
    for field, dtype in [("hrrr_row", np.int32), ("hrrr_col", np.int32), ("twi_bar", np.float32), ("n_fine_cells", np.int32)]:
        z_arr = root.create_array(f"hrrr_twi_bar_{field}", shape=npz[field].shape, dtype=dtype, overwrite=True)
        z_arr[:] = npz[field].astype(dtype)
    print(f"  hrrr_twi_bar_*: {len(npz['hrrr_row'])} distinct HRRR cells")

    provenance = {
        "assembled_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "region": "colorado-10m-pilot-boulder-foothills",
        "sources": {
            "elevation": "USGS 3DEP 1/3 arc-second (tiles n40w106, n41w106), "
            "https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/13/TIFF/current/",
            "twi_slope_aspect": "Derived from elevation via pyDEM (Ueckermann et al. 2018) "
            "with apply_twi_limits=True (Horn's method for slope/aspect) -- the exact "
            "configuration validated in validation/tarrawarra (Sessions 8-9) and "
            "validation/shalehills (Session 10)",
            "theta_s_theta_wilt": "POLARIS (Chaney et al. 2019) sand%/clay%, 0-30cm "
            "thickness-weighted mean, classified via USDA texture triangle -> Noah "
            "SOILPARM.TBL STAS lookup (physics/soil_texture.py, the same pipeline used "
            "at every validation site)",
            "hrrr_twi_bar": "Block-mean TWI per overlapping HRRR grid cell -- the real "
            "Creare/GeoWATCH production equation's lambda_bar term, per Session 8's "
            "discovery (physics/redistribution.py's module docstring)",
        },
        "twi_configuration": {"engine": "pydem", "apply_twi_limits": True},
        "k": 13.0,
    }
    root.attrs["grid_spec"] = grid.to_attrs_dict()
    root.attrs["provenance"] = provenance
    print(f"Assembled Zarr stack at {output_path}")
    print(json.dumps(provenance, indent=2))


def main():
    import argparse

    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--data-dir", default="./data/static")
    parser.add_argument("--output", default="./data/static/colorado-10m-pilot.zarr")
    args = parser.parse_args()
    assemble(args.data_dir, args.output)


if __name__ == "__main__":
    main()
