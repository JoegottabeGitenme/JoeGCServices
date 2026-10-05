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
from rasterio.windows import Window

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from hrrr_grid import HrrrGrid  # noqa: E402


DEFAULT_BAND_ROWS = 512


def compute_coarse_twi_bar(twi_path: str, hrrr_grid: HrrrGrid | None = None, band_rows: int = DEFAULT_BAND_ROWS):
    """Returns (hrrr_rows, hrrr_cols, twi_bar_values, counts) -- one entry
    per DISTINCT HRRR cell that the fine TWI grid actually overlaps.

    **Processed in row bands (Session 14)**, accumulating per-HRRR-cell
    sums/counts across bands, because the original whole-raster version
    built full-grid int64 `meshgrid` index arrays plus float64 coordinate
    arrays -- tens of GB at the Front Range's 345.6M cells (it was fine at
    the pilot's 17M). It also looped over every fine cell in Python calling
    the scalar `geo_to_grid` (~287M calls for the Front Range); this uses
    `HrrrGrid.geo_to_grid_array` (vectorized, tested against the scalar
    version) instead. The number of distinct HRRR cells is small (186 for
    the pilot, a few thousand for the Front Range), so the cross-band
    accumulator is a plain dict.

    Results match the unbanded computation up to float summation order
    (see `test_banded_matches_unbanded`)."""
    if hrrr_grid is None:
        hrrr_grid = HrrrGrid.hrrr()

    sums: dict[int, float] = {}
    counts: dict[int, int] = {}

    with rasterio.open(twi_path) as src:
        crs = src.crs
        transform = src.transform
        nrows, ncols = src.height, src.width
        if transform.b != 0 or transform.d != 0:
            raise ValueError("compute_coarse_twi_bar assumes a north-up, unrotated grid transform")
        to_wgs84 = Transformer.from_crs(crs, "EPSG:4326", always_xy=True)

        # Cell-center x never changes between bands.
        col_x = transform.c + (np.arange(ncols) + 0.5) * transform.a

        for r0 in range(0, nrows, band_rows):
            nr = min(band_rows, nrows - r0)
            band = src.read(1, window=Window(0, r0, ncols, nr))
            valid = ~np.isnan(band)
            if not valid.any():
                continue

            row_y = transform.f + (np.arange(r0, r0 + nr) + 0.5) * transform.e
            xs = np.broadcast_to(col_x, (nr, ncols))[valid]
            ys = np.broadcast_to(row_y[:, None], (nr, ncols))[valid]
            twi_valid = band[valid].astype(np.float64)

            lons, lats = to_wgs84.transform(xs, ys)
            ii, jj = hrrr_grid.geo_to_grid_array(lats, lons)
            # np.round and Python's round() both round half to even, so this
            # matches the original scalar version's `round(i)` exactly.
            keys = np.round(jj).astype(np.int64) * hrrr_grid.nx + np.round(ii).astype(np.int64)

            unique_keys, inverse, key_counts = np.unique(keys, return_inverse=True, return_counts=True)
            key_sums = np.bincount(inverse, weights=twi_valid, minlength=len(unique_keys))
            for k, sm, ct in zip(unique_keys.tolist(), key_sums.tolist(), key_counts.tolist()):
                sums[k] = sums.get(k, 0.0) + sm
                counts[k] = counts.get(k, 0) + ct

    ordered = sorted(sums)
    out_rows = np.array([k // hrrr_grid.nx for k in ordered], dtype=np.int32)
    out_cols = np.array([k % hrrr_grid.nx for k in ordered], dtype=np.int32)
    means = np.array([sums[k] / counts[k] for k in ordered], dtype=np.float32)
    out_counts = np.array([counts[k] for k in ordered], dtype=np.int32)
    return out_rows, out_cols, means, out_counts


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--region", default="pilot")
    parser.add_argument("--twi-path", default=None, help="default: ./data/static/<region>_twi.tif")
    parser.add_argument("--output", default=None, help="default: ./data/static/<region>_twi_bar.npz")
    args = parser.parse_args()
    args.twi_path = args.twi_path or f"./data/static/{args.region}_twi.tif"
    args.output = args.output or f"./data/static/{args.region}_twi_bar.npz"

    rows, cols, twi_bar, counts = compute_coarse_twi_bar(args.twi_path)
    np.savez(args.output, hrrr_row=rows, hrrr_col=cols, twi_bar=twi_bar, n_fine_cells=counts)
    print(
        f"Wrote {args.output}: {len(rows)} distinct HRRR cells covered by the {args.region} region "
        f"(twi_bar range {twi_bar.min():.3f}-{twi_bar.max():.3f}, "
        f"fine cells per HRRR cell: {counts.min()}-{counts.max()})"
    )


if __name__ == "__main__":
    main()
