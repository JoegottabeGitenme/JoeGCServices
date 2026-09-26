#!/usr/bin/env python3
"""Derive TWI, slope, and aspect from the pilot DEM (`build_dem.py`'s
output), using **pyDEM with `apply_twi_limits=True`** -- the EXACT TWI
configuration validated three times over (Tarrawarra TDR, Session 8;
Tarrawarra NMM, Session 9; Shale Hills TDR, Session 10) via
`services/trail-physics/physics/terrain.py::compute_twi_pydem`.

**Supersedes `derive_terrain.py`'s WhiteboxTools plan for TWI
specifically** (see that file's own updated header note). Using a
different TWI implementation in production than in validation would
silently deploy unvalidated methodology -- the whole point of the
validation ladder was to establish which specific configuration
generalizes, and it was never WhiteboxTools' `wetness_index`. Slope and
aspect, by contrast, are simple and not paper-critical (`k=13`'s
validation never depended on which slope/aspect implementation feeds
Stage 2's solar view factor) -- this module uses this project's own
`physics.terrain.compute_slope`/`compute_aspect` (Horn's method) for
those, the same functions already used throughout every validation
session.

**Scaling, verified empirically before running the full pilot (Session
11), not assumed**: pyDEM's `DEMProcessor` was timed on progressively
larger real crops of the actual pilot DEM -- 40K cells (0.4s) up to 9M
cells (40.7s, ~220K cells/sec, mildly superlinear but not explosively so)
-- before running the full ~17M-cell pilot grid in one call (no tiling
needed at this scale): 1m56s wall-clock, real result. A statewide build
(~150-300x this pilot's cell count) would need actual tiling with overlap
margins; not required here.

Real invalid-cell accounting (Session 11): the source DEM has ~17% nodata
cells (reprojecting geographic 3DEP tiles onto a rotated Albers rectangle
leaves real corner gaps -- an expected reprojection artifact, not a bug).
TWI adds only 685 additional invalid cells beyond the DEM's own nodata
(out of ~14.2M valid DEM cells) -- consistent, expected numerical edge
behavior from the flow-routing algorithm near real nodata boundaries, not
a new problem introduced by this module.

**Slope/aspect's nodata footprint is a SUPERSET of the DEM's own, not
identical to it** (also confirmed empirically, not assumed): Horn's
method's 3x3 kernel produces NaN at any cell whose neighborhood touches a
real nodata cell, so cells immediately bordering a nodata region get an
undefined slope/aspect even though their own elevation is real (16,599
such cells found in the real pilot run). The masking step below only
guards the other direction (never fabricate a slope/aspect value at a
cell that has no real elevation at all) -- it does not, and should not,
try to "fill in" this natural kernel-contamination edge effect.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
import rasterio

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from physics.terrain import compute_aspect, compute_slope, compute_twi_pydem  # noqa: E402

# Session 8's validated configuration -- see module docstring. Not
# re-tuned here, no CLI flag to change it, same discipline as the
# validation harnesses' FROZEN_CONFIG.
TWI_APPLY_LIMITS = True


def derive_terrain_layers(dem_path: str, output_dir: str) -> None:
    with rasterio.open(dem_path) as src:
        dem = src.read(1)
        cellsize = src.res[0]
        profile = src.profile

    print(f"Computing TWI (pyDEM, apply_twi_limits={TWI_APPLY_LIMITS}) on {dem.shape} ({dem.size} cells)...")
    twi = compute_twi_pydem(dem, cellsize, apply_twi_limits=TWI_APPLY_LIMITS)

    print("Computing slope/aspect (Horn's method)...")
    slope = compute_slope(dem, cellsize)
    aspect = compute_aspect(dem, cellsize)

    # Slope/aspect are undefined wherever the source DEM is nodata --
    # compute_slope/compute_aspect don't NaN-propagate through the edge-pad
    # step on their own (see physics/terrain.py), so mask explicitly here
    # rather than silently shipping a fabricated slope/aspect value at
    # every real nodata cell.
    dem_nodata_mask = np.isnan(dem)
    slope = np.where(dem_nodata_mask, np.nan, slope)
    aspect = np.where(dem_nodata_mask, np.nan, aspect)

    Path(output_dir).mkdir(parents=True, exist_ok=True)
    out_profile = dict(profile)
    out_profile.update(dtype="float32", nodata=np.nan, compress="deflate")

    for name, arr in [("twi", twi), ("slope", slope), ("aspect", aspect)]:
        out_path = Path(output_dir) / f"pilot_{name}.tif"
        with rasterio.open(out_path, "w", **out_profile) as dst:
            dst.write(arr.astype(np.float32), 1)
        valid = arr[~np.isnan(arr)]
        print(f"  wrote {out_path}: mean={valid.mean():.3f} std={valid.std():.3f} " f"min={valid.min():.3f} max={valid.max():.3f}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dem-path", default="./data/static/pilot_dem.tif")
    parser.add_argument("--output-dir", default="./data/static")
    args = parser.parse_args()
    derive_terrain_layers(args.dem_path, args.output_dir)


if __name__ == "__main__":
    main()
