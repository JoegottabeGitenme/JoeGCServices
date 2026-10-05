#!/usr/bin/env python3
"""Derive theta_s (saturated soil moisture) and theta_wilt (wilting point)
grids from the pilot region's POLARIS sand%/clay% (`fetch_polaris.py`),
via the SAME USDA-texture-triangle -> Noah SOILPARM.TBL pipeline used at
every validation site (`physics/soil_texture.py`) -- not a different
soil-parameter methodology, so this remains a genuine deployment of the
validated approach rather than something that merely also produces
plausible-looking output.

**Why a precomputed lookup table, not a per-cell loop**: `classify_usda_
texture`/`soil_hydraulic_properties` are pure functions of (sand%, clay%)
alone with only 12 possible discrete outputs (the USDA texture classes).
Looping them directly over ~14M valid pixels in pure Python would be slow
for no benefit -- instead, every INTEGER (sand%, clay%) combination
(5,151 valid pairs, since sand+clay<=100) is classified ONCE (11ms,
confirmed live this session), building a 101x101 lookup table; the full
grid is then rounded to the nearest integer percent and the table applied
via vectorized numpy fancy-indexing. This is exact, not an approximation
beyond 1-percentage-point rounding of sand%/clay% themselves -- negligible
given POLARIS's own values are machine-learning PREDICTIONS with
uncertainty far exceeding 1 percentage point.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
import rasterio
from rasterio.windows import Window

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from physics.soil_texture import soil_hydraulic_properties  # noqa: E402


def build_lookup_tables() -> tuple[np.ndarray, np.ndarray]:
    """Returns (theta_s_table, theta_wilt_table), each a (101, 101) array
    indexed [sand_pct_int, clay_pct_int]. Entries where sand+clay > 100
    (not a physically valid soil) are NaN."""
    theta_s_table = np.full((101, 101), np.nan, dtype=np.float32)
    theta_wilt_table = np.full((101, 101), np.nan, dtype=np.float32)
    for sand in range(101):
        for clay in range(101 - sand):  # clay in [0, 100-sand], so sand+clay <= 100 always
            params = soil_hydraulic_properties(float(sand), float(clay))
            theta_s_table[sand, clay] = params.maxsmc
            theta_wilt_table[sand, clay] = params.wltsmc
    return theta_s_table, theta_wilt_table


def classify_block(
    sand: np.ndarray, clay: np.ndarray, theta_s_table: np.ndarray, theta_wilt_table: np.ndarray
) -> tuple[np.ndarray, np.ndarray, int]:
    """(theta_s, theta_wilt, n_overshoot) for one block of sand%/clay%.
    Pure function of its inputs -- the per-cell math is exactly what the
    whole-raster version did (Session 11); the banded driver below just
    applies it to row bands."""
    valid = ~np.isnan(sand) & ~np.isnan(clay)

    # Fill NaN with a safe placeholder (0) before rounding/casting to int --
    # casting NaN directly to int64 is undefined behavior (produces an
    # out-of-range sentinel that then crashes the lookup-table indexing
    # below). The `valid` mask discards these placeholder results anyway;
    # this just prevents the intermediate indexing step from ever seeing
    # an out-of-bounds index.
    sand_safe = np.where(valid, sand, 0.0)
    clay_safe = np.where(valid, clay, 0.0)
    sand_idx = np.clip(np.round(sand_safe), 0, 100).astype(np.int64)
    clay_idx = np.clip(np.round(clay_safe), 0, 100).astype(np.int64)
    # Clamp any rounding overshoot where sand_idx + clay_idx > 100 (can
    # happen right at the boundary, e.g. sand=60.4 clay=39.8 rounds to
    # 60+40=100, fine, but sand=60.6 clay=39.6 rounds to 61+40=101) by
    # shrinking clay's index -- a tiny, documented rounding-edge fix, not
    # a silent NaN-producing gap in otherwise-valid data.
    overshoot = (sand_idx + clay_idx) > 100
    clay_idx = np.where(overshoot, 100 - sand_idx, clay_idx)

    theta_s = np.where(valid, theta_s_table[sand_idx, clay_idx], np.nan).astype(np.float32)
    theta_wilt = np.where(valid, theta_wilt_table[sand_idx, clay_idx], np.nan).astype(np.float32)
    return theta_s, theta_wilt, int(overshoot[valid].sum())


DEFAULT_BAND_ROWS = 512  # matches the output GeoTIFFs' 512-row internal blocks


class _RunningStats:
    """mean/min/max over values streamed in bands -- the whole-raster
    version printed these from one full array; at 345.6M cells that array
    (plus its int64 index temporaries) is what made this step memory-bound."""

    def __init__(self):
        self.n, self.total, self.lo, self.hi = 0, 0.0, np.inf, -np.inf

    def update(self, arr: np.ndarray) -> None:
        v = arr[~np.isnan(arr)]
        if v.size:
            self.n += int(v.size)
            self.total += float(v.sum(dtype=np.float64))
            self.lo = min(self.lo, float(v.min()))
            self.hi = max(self.hi, float(v.max()))

    def line(self) -> str:
        if not self.n:
            return "NO VALID CELLS"
        return f"mean={self.total / self.n:.3f} min={self.lo:.3f} max={self.hi:.3f}"


def derive_soil_params(
    sand_path: str, clay_path: str, output_dir: str, prefix: str = "pilot", band_rows: int = DEFAULT_BAND_ROWS
) -> None:
    """Processed in row bands (Session 14): read a band of sand/clay, classify
    it, write the band -- peak memory is a few bands, not several full-grid
    int64/float arrays (~10+ GB at the Front Range's 345.6M cells)."""
    print("Building USDA texture -> Noah SOILPARM.TBL lookup table (5,151 valid sand/clay pairs)...")
    theta_s_table, theta_wilt_table = build_lookup_tables()

    with rasterio.open(sand_path) as sand_src, rasterio.open(clay_path) as clay_src:
        if (sand_src.height, sand_src.width) != (clay_src.height, clay_src.width):
            raise ValueError(f"sand {sand_src.shape} and clay {clay_src.shape} grids differ")
        height, width = sand_src.height, sand_src.width
        out_profile = dict(sand_src.profile)
        out_profile.update(
            dtype="float32", nodata=np.nan, compress="deflate", tiled=True, blockxsize=512, blockysize=512, BIGTIFF="YES"
        )

        Path(output_dir).mkdir(parents=True, exist_ok=True)
        names = ("theta_s", "theta_wilt")
        out_paths = {n: Path(output_dir) / f"{prefix}_{n}.tif" for n in names}
        stats = {n: _RunningStats() for n in names}
        n_overshoot_total = 0
        n_bad_total = 0

        outs = {n: rasterio.open(out_paths[n], "w", **out_profile) for n in names}
        try:
            for r0 in range(0, height, band_rows):
                win = Window(0, r0, width, min(band_rows, height - r0))
                theta_s, theta_wilt, n_overshoot = classify_block(
                    sand_src.read(1, window=win), clay_src.read(1, window=win), theta_s_table, theta_wilt_table
                )
                n_overshoot_total += n_overshoot

                # Physical sanity, checked (not just asserted in a docstring):
                # theta_s (porosity) must exceed theta_wilt everywhere real
                # soil exists.
                both_valid = ~np.isnan(theta_s) & ~np.isnan(theta_wilt)
                n_bad_total += int(np.sum(theta_s[both_valid] <= theta_wilt[both_valid]))

                outs["theta_s"].write(theta_s, 1, window=win)
                outs["theta_wilt"].write(theta_wilt, 1, window=win)
                stats["theta_s"].update(theta_s)
                stats["theta_wilt"].update(theta_wilt)
        finally:
            for ds in outs.values():
                ds.close()

    if n_overshoot_total:
        print(f"  NOTE: {n_overshoot_total} cell(s) had a sand%+clay% rounding overshoot >100 -- clay index clamped down")
    for n in names:
        print(f"  wrote {out_paths[n]}: {stats[n].line()}")
    if n_bad_total:
        raise ValueError(f"{n_bad_total} cell(s) have theta_s <= theta_wilt -- a real bug, not expected from a valid lookup table")
    print("  Sanity check passed: theta_s > theta_wilt at every valid cell.")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--region", default="pilot")
    parser.add_argument("--sand-path", default=None, help="default: ./data/static/<region>_sand_pct.tif")
    parser.add_argument("--clay-path", default=None, help="default: ./data/static/<region>_clay_pct.tif")
    parser.add_argument("--output-dir", default="./data/static")
    args = parser.parse_args()
    derive_soil_params(
        args.sand_path or f"./data/static/{args.region}_sand_pct.tif",
        args.clay_path or f"./data/static/{args.region}_clay_pct.tif",
        args.output_dir,
        prefix=args.region,
    )


if __name__ == "__main__":
    main()
