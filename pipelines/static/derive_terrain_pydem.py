#!/usr/bin/env python3
"""Derive TWI, slope, and aspect from a region's DEM (`build_dem.py`'s
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

**Tiled (Session 14), because monolithic does not fit.** The Session 11
pilot (17.1M cells) ran in one pyDEM call in ~2 minutes. Session 14
measured peak memory directly: **3.39 GB for the 17.1M-cell pilot, ~198
bytes/cell** (pyDEM holds many full-size working arrays). The Front
Range grid is 345.6M cells (`grid_spec.FRONT_RANGE_BBOX_WGS84`) -> ~68 GB
monolithic, on a 31 GB machine; a 154M-cell trial run died mid-flight for
exactly that reason. Runtime scales linearly (116.6s at 17M cells, 455s at
68M: 146-150K cells/s), so time was never the problem -- memory is.

So each core tile is processed with a margin of real DEM around it,
then cropped back to the core. The margin matters because TWI depends on
*upstream* contributing area, which a tile boundary would truncate.
**How big a margin is enough was measured, not assumed**
(`margin_test`-style experiment, Session 14): recomputing a 1000x1000 core
inside the real pilot DEM from a crop padded by M cells, and comparing
against the pilot's own monolithic TWI:

    margin   mean|diff|   frac of cells differing > 0.1
        0     0.0117          1.47%
       25     0.0003          0.08%
       50     0.0001          0.01%
      100     0.0000          0.00%   (0.07% at the worst of 3 locations)

Convergence is fast because `apply_twi_limits` saturates upstream area
(`uca_saturation_limit`), so distant drainage stops mattering. We use
`DEFAULT_MARGIN_CELLS = 250` (2.5 km) -- 2.5x the margin at which the
three tested locations had already converged, as headroom for terrain
character the pilot doesn't have (the Front Range grid includes flat
plains-edge terrain where flow paths run long). Slope/aspect (Horn's
method, 3x3 kernel) need only 1 cell of margin and get exact equality.

Real caveat, stated plainly: that margin study used foothills terrain
(the pilot). `test_tiled_matches_monolithic` re-checks the property on a
synthetic DEM in CI, but the production Front Range run should still be
spot-checked at tile seams (see `seam_report`).

Real invalid-cell accounting (Session 11): the source DEM has ~17% nodata
cells (reprojecting geographic 3DEP tiles onto a rotated Albers rectangle
leaves real corner gaps -- an expected reprojection artifact, not a bug).
**Slope/aspect's nodata footprint is a SUPERSET of the DEM's own, not
identical to it**: Horn's method's 3x3 kernel produces NaN at any cell
whose neighborhood touches a real nodata cell. The masking step only
guards the other direction (never fabricate a slope/aspect value at a
cell with no real elevation) -- it does not try to "fill in" this natural
kernel-contamination edge effect.

**Resumable**: a 20x-pilot run takes about an hour. Completed tiles are
recorded in `<output>/<prefix>_terrain_progress.json`; re-running skips
them and reopens the outputs in r+ mode, so a crash at tile 19 of 22 costs
one tile, not the run.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import rasterio
from rasterio.windows import Window

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from physics.terrain import compute_aspect, compute_slope, compute_twi_pydem  # noqa: E402

# Session 8's validated configuration -- see module docstring. Not
# re-tuned here, no CLI flag to change it, same discipline as the
# validation harnesses' FROZEN_CONFIG.
TWI_APPLY_LIMITS = True

# 8 x 512: a multiple of the output GeoTIFFs' 512-cell internal blocks, so
# every windowed write lands on whole blocks (no read-modify-write of
# compressed blocks, which is slow and bloats deflate output).
DEFAULT_TILE_SIZE = 4096
DEFAULT_MARGIN_CELLS = 250  # see module docstring for the measurement behind this
BLOCK_SIZE = 512
LAYER_NAMES = ("twi", "slope", "aspect")


@dataclass(frozen=True)
class TileSpec:
    index: int
    core: Window  # the cells this tile is responsible for writing
    padded: Window  # core + margin, clipped to the grid -- what pyDEM sees


def plan_tiles(height: int, width: int, tile_size: int, margin: int) -> list[TileSpec]:
    """Partition the grid into core tiles of `tile_size` (edge tiles are
    smaller), each with a `margin`-padded read window clipped to the grid.
    Cores partition the grid exactly: every cell belongs to exactly one."""
    tiles = []
    index = 0
    for row_off in range(0, height, tile_size):
        for col_off in range(0, width, tile_size):
            core_h = min(tile_size, height - row_off)
            core_w = min(tile_size, width - col_off)
            pr0 = max(0, row_off - margin)
            pc0 = max(0, col_off - margin)
            pr1 = min(height, row_off + core_h + margin)
            pc1 = min(width, col_off + core_w + margin)
            tiles.append(
                TileSpec(
                    index=index,
                    core=Window(col_off, row_off, core_w, core_h),
                    padded=Window(pc0, pr0, pc1 - pc0, pr1 - pr0),
                )
            )
            index += 1
    return tiles


def derive_tile(dem_padded: np.ndarray, cellsize: float, core_row_off: int, core_col_off: int, core_h: int, core_w: int):
    """Compute (twi, slope, aspect) for one padded DEM window and return
    just the core region of each. Pure function of its inputs -- the unit
    under test for tiled-vs-monolithic equivalence.

    `core_row_off`/`core_col_off` locate the core within the padded array.
    """
    twi = compute_twi_pydem(dem_padded, cellsize, apply_twi_limits=TWI_APPLY_LIMITS)
    slope = compute_slope(dem_padded, cellsize)
    aspect = compute_aspect(dem_padded, cellsize)

    # Slope/aspect are undefined wherever the source DEM is nodata --
    # compute_slope/compute_aspect don't NaN-propagate through the edge-pad
    # step on their own (see physics/terrain.py), so mask explicitly rather
    # than silently shipping a fabricated value at every real nodata cell.
    dem_nodata_mask = np.isnan(dem_padded)
    slope = np.where(dem_nodata_mask, np.nan, slope)
    aspect = np.where(dem_nodata_mask, np.nan, aspect)

    rs = slice(core_row_off, core_row_off + core_h)
    cs = slice(core_col_off, core_col_off + core_w)
    return twi[rs, cs], slope[rs, cs], aspect[rs, cs]


def derive_terrain_layers(
    dem_path: str,
    output_dir: str,
    prefix: str = "pilot",
    tile_size: int = DEFAULT_TILE_SIZE,
    margin: int = DEFAULT_MARGIN_CELLS,
) -> None:
    out_dir = Path(output_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    out_paths = {name: out_dir / f"{prefix}_{name}.tif" for name in LAYER_NAMES}
    progress_path = out_dir / f"{prefix}_terrain_progress.json"

    with rasterio.open(dem_path) as src:
        height, width = src.height, src.width
        cellsize = src.res[0]
        profile = dict(src.profile)

    tiles = plan_tiles(height, width, tile_size, margin)

    done: set[int] = set()
    resuming = progress_path.exists() and all(p.exists() for p in out_paths.values())
    if resuming:
        saved = json.loads(progress_path.read_text())
        if saved.get("tile_size") == tile_size and saved.get("margin") == margin and saved.get("shape") == [height, width]:
            done = set(saved["done"])
            print(f"Resuming: {len(done)}/{len(tiles)} tiles already done")
        else:
            print("Existing progress file has different tiling parameters -- starting over")
            resuming = False

    out_profile = dict(profile)
    out_profile.update(
        dtype="float32", nodata=np.nan, compress="deflate", tiled=True, blockxsize=BLOCK_SIZE, blockysize=BLOCK_SIZE,
        BIGTIFF="YES",  # a 345M-cell float32 layer is 1.4 GB uncompressed; don't depend on compression to stay under 4 GB
    )
    mode = "r+" if resuming else "w"
    outputs = {name: rasterio.open(path, mode, **({} if resuming else out_profile)) for name, path in out_paths.items()}

    print(f"Deriving terrain on {height}x{width} ({height * width:,} cells) in {len(tiles)} tiles "
          f"(tile={tile_size}, margin={margin}, pyDEM apply_twi_limits={TWI_APPLY_LIMITS})")
    t_start = time.time()
    computed_this_run = 0
    try:
        with rasterio.open(dem_path) as src:
            for tile in tiles:
                if tile.index in done:
                    continue
                t0 = time.time()
                dem_padded = src.read(1, window=tile.padded).astype(np.float64)

                core_r = int(tile.core.row_off - tile.padded.row_off)
                core_c = int(tile.core.col_off - tile.padded.col_off)
                core_h, core_w = int(tile.core.height), int(tile.core.width)

                core_dem = dem_padded[core_r:core_r + core_h, core_c:core_c + core_w]
                if np.isnan(core_dem).all():
                    # Nothing real to derive (e.g. an Albers-rotation corner
                    # of the grid). Write explicit NaN rather than leave the
                    # blocks unwritten (unwritten compressed blocks read back
                    # as 0, not nodata, without SPARSE_OK).
                    layers = [np.full((core_h, core_w), np.nan, dtype=np.float32)] * 3
                    note = "all-nodata core, wrote NaN"
                else:
                    layers = derive_tile(dem_padded, cellsize, core_r, core_c, core_h, core_w)
                    note = f"{dem_padded.size / 1e6:.1f}M cells computed"

                for name, arr in zip(LAYER_NAMES, layers):
                    outputs[name].write(np.asarray(arr, dtype=np.float32), 1, window=tile.core)

                done.add(tile.index)
                computed_this_run += 1
                progress_path.write_text(json.dumps(
                    {"tile_size": tile_size, "margin": margin, "shape": [height, width], "done": sorted(done)}
                ))
                elapsed = time.time() - t_start
                remaining = len(tiles) - len(done)
                eta = (elapsed / computed_this_run) * remaining
                print(f"  tile {tile.index + 1}/{len(tiles)} ({note}) {time.time() - t0:.0f}s; "
                      f"{remaining} left, ETA {eta / 60:.0f} min", flush=True)
                del dem_padded, layers
    finally:
        for ds in outputs.values():
            ds.close()

    print("Terrain derivation complete; summarizing outputs...")
    for name, path in out_paths.items():
        _print_layer_stats(name, path)


def _print_layer_stats(name: str, path: Path, stride: int = 8) -> None:
    """Summary stats on a strided subsample (a full read of a 345M-cell
    layer just to print mean/std would need ~1.4 GB for no analytical
    benefit)."""
    with rasterio.open(path) as src:
        arr = src.read(1, out_shape=(src.height // stride, src.width // stride))
    valid = arr[~np.isnan(arr)]
    if valid.size == 0:
        print(f"  {path.name}: NO VALID CELLS")
        return
    print(f"  {path.name}: (stride-{stride} subsample) mean={valid.mean():.3f} std={valid.std():.3f} "
          f"min={valid.min():.3f} max={valid.max():.3f}")


def seam_stats(twi_path: str, tile_size: int = DEFAULT_TILE_SIZE, band: int = 3) -> list[dict]:
    """Seam spot-check of a finished TWI layer, for BOTH tile-boundary
    directions. At each interior tile boundary compare the mean absolute
    difference between adjacent cells ACROSS the boundary against the same
    statistic one cell over (ordinary roughness right next to it). A seam
    artifact (margin too small) shows up as a boundary jump well above the
    background; a ratio near 1.0 means the seam is invisible. Returns one
    dict per seam: {"axis", "index", "across", "background", "ratio"}.

    (Session 14 first version checked only row seams -- the horizontal
    boundaries -- and so said nothing about the vertical ones; both are tile
    boundaries.)"""
    out = []
    with rasterio.open(twi_path) as src:
        height, width = src.height, src.width
        for r in range(tile_size, height, tile_size):
            strip = src.read(1, window=Window(0, r - band, width, 2 * band))
            out.append(_seam_row("row", r, np.abs(strip[band] - strip[band - 1]), np.abs(strip[1] - strip[0])))
        for c in range(tile_size, width, tile_size):
            strip = src.read(1, window=Window(c - band, 0, 2 * band, height))
            out.append(_seam_row("col", c, np.abs(strip[:, band] - strip[:, band - 1]), np.abs(strip[:, 1] - strip[:, 0])))
    return out


def _seam_row(axis: str, index: int, across: np.ndarray, background: np.ndarray) -> dict:
    a, b = float(np.nanmean(across)), float(np.nanmean(background))
    return {"axis": axis, "index": index, "across": a, "background": b, "ratio": a / b if b else float("nan")}


def seam_report(twi_path: str, tile_size: int = DEFAULT_TILE_SIZE, band: int = 3) -> None:
    for st in seam_stats(twi_path, tile_size, band):
        print(f"  {st['axis']} seam {st['index']}: across={st['across']:.4f} background={st['background']:.4f} ratio={st['ratio']:.2f}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--region", default="pilot")
    parser.add_argument("--dem-path", default=None, help="default: ./data/static/<region>_dem.tif")
    parser.add_argument("--output-dir", default="./data/static")
    parser.add_argument("--tile-size", type=int, default=DEFAULT_TILE_SIZE)
    parser.add_argument("--margin", type=int, default=DEFAULT_MARGIN_CELLS)
    parser.add_argument("--seam-report", action="store_true", help="after deriving, print a seam spot-check of the TWI layer")
    args = parser.parse_args()
    dem_path = args.dem_path or f"./data/static/{args.region}_dem.tif"
    derive_terrain_layers(dem_path, args.output_dir, prefix=args.region, tile_size=args.tile_size, margin=args.margin)
    if args.seam_report:
        seam_report(str(Path(args.output_dir) / f"{args.region}_twi.tif"), args.tile_size)


if __name__ == "__main__":
    main()
