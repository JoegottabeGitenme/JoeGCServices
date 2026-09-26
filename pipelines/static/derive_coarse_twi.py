#!/usr/bin/env python3
"""Compute lambda_bar (the coarse HRRR-cell-mean TWI) -- the real Creare/
GeoWATCH production equation's actual `twi_bar` term, per Session 8's
discovery (`services/trail-physics/physics/redistribution.py`'s module
docstring): the PODPAC notebook reprojects the fine TWI layer onto the
SAME coarse grid as the coarse soil-moisture input (there, SMAP; here,
HRRR) and uses THAT cell's own mean, not one single domain-wide mean.

Every validation session (Tarrawarra, NMM, Shale Hills) used a single
domain mean because each site is smaller than one HRRR cell (a
10.8ha/8ha catchment is a tiny fraction of a 3km x 3km HRRR cell) --
`twi_mean` and "the one HRRR cell's own mean" were the same number there
by construction. At Colorado scale, the static stack spans MANY HRRR
cells, so this distinction becomes real for the first time: main.py must
look up the correction's lambda_bar from THIS cell's own mean, not a
single global pilot-wide average.

Output: a lookup table, NOT a full HRRR-grid-shaped raster (the full HRRR
grid is 1799x1059 -- storing it in full would mean shipping ~1.9M mostly-
empty cells for a stack that only covers ~150 of them). Written as a
small Zarr-friendly structure: parallel 1-D arrays of (hrrr_row, hrrr_col,
twi_bar, n_fine_cells).
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
import rasterio
from pyproj import Transformer

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from hrrr_grid import HrrrGrid  # noqa: E402


def compute_coarse_twi_bar(twi_path: str, hrrr_grid: HrrrGrid | None = None):
    """Returns (hrrr_rows, hrrr_cols, twi_bar_values, counts) -- one entry
    per DISTINCT HRRR cell that the fine TWI grid actually overlaps."""
    if hrrr_grid is None:
        hrrr_grid = HrrrGrid.hrrr()

    with rasterio.open(twi_path) as src:
        twi = src.read(1)
        crs = src.crs
        transform = src.transform

    nrows, ncols = twi.shape
    rows_idx, cols_idx = np.meshgrid(np.arange(nrows), np.arange(ncols), indexing="ij")
    # Cell-center coordinates in the fine grid's own CRS.
    xs, ys = rasterio.transform.xy(transform, rows_idx.ravel(), cols_idx.ravel())
    xs = np.asarray(xs)
    ys = np.asarray(ys)

    valid = ~np.isnan(twi.ravel())
    xs, ys = xs[valid], ys[valid]
    twi_valid = twi.ravel()[valid]

    to_wgs84 = Transformer.from_crs(crs, "EPSG:4326", always_xy=True)
    lons, lats = to_wgs84.transform(xs, ys)

    hrrr_rows = np.empty(len(lons), dtype=np.int32)
    hrrr_cols = np.empty(len(lons), dtype=np.int32)
    for idx, (lat, lon) in enumerate(zip(lats, lons)):
        i, j = hrrr_grid.geo_to_grid(lat, lon)
        hrrr_cols[idx] = round(i)
        hrrr_rows[idx] = round(j)

    # Aggregate: mean TWI per distinct (hrrr_row, hrrr_col) pair.
    keys = hrrr_rows.astype(np.int64) * hrrr_grid.nx + hrrr_cols.astype(np.int64)
    unique_keys, inverse, counts = np.unique(keys, return_inverse=True, return_counts=True)
    sums = np.zeros(len(unique_keys))
    np.add.at(sums, inverse, twi_valid)
    means = sums / counts

    out_rows = (unique_keys // hrrr_grid.nx).astype(np.int32)
    out_cols = (unique_keys % hrrr_grid.nx).astype(np.int32)
    return out_rows, out_cols, means.astype(np.float32), counts.astype(np.int32)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--twi-path", default="./data/static/pilot_twi.tif")
    parser.add_argument("--output", default="./data/static/pilot_twi_bar.npz")
    args = parser.parse_args()

    rows, cols, twi_bar, counts = compute_coarse_twi_bar(args.twi_path)
    np.savez(args.output, hrrr_row=rows, hrrr_col=cols, twi_bar=twi_bar, n_fine_cells=counts)
    print(
        f"Wrote {args.output}: {len(rows)} distinct HRRR cells covered by the pilot region "
        f"(twi_bar range {twi_bar.min():.3f}-{twi_bar.max():.3f}, "
        f"fine cells per HRRR cell: {counts.min()}-{counts.max()})"
    )


if __name__ == "__main__":
    main()
