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


def derive_soil_params(sand_path: str, clay_path: str, output_dir: str) -> None:
    with rasterio.open(sand_path) as src:
        sand = src.read(1)
        profile = src.profile
    with rasterio.open(clay_path) as src:
        clay = src.read(1)

    valid = ~np.isnan(sand) & ~np.isnan(clay)

    print("Building USDA texture -> Noah SOILPARM.TBL lookup table (5,151 valid sand/clay pairs)...")
    theta_s_table, theta_wilt_table = build_lookup_tables()

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

    n_overshoot = int(overshoot[valid].sum())
    if n_overshoot:
        print(f"  NOTE: {n_overshoot} cell(s) had a sand%+clay% rounding overshoot >100 -- clay index clamped down")

    Path(output_dir).mkdir(parents=True, exist_ok=True)
    out_profile = dict(profile)
    out_profile.update(dtype="float32", nodata=np.nan, compress="deflate")
    for name, arr in [("theta_s", theta_s), ("theta_wilt", theta_wilt)]:
        out_path = Path(output_dir) / f"pilot_{name}.tif"
        with rasterio.open(out_path, "w", **out_profile) as dst:
            dst.write(arr, 1)
        v = arr[~np.isnan(arr)]
        print(f"  wrote {out_path}: mean={v.mean():.3f} min={v.min():.3f} max={v.max():.3f}")

    # Physical sanity, checked (not just asserted in a docstring): theta_s
    # (porosity) must exceed theta_wilt everywhere real soil exists.
    both_valid = ~np.isnan(theta_s) & ~np.isnan(theta_wilt)
    n_bad = int(np.sum(theta_s[both_valid] <= theta_wilt[both_valid]))
    if n_bad:
        raise ValueError(f"{n_bad} cell(s) have theta_s <= theta_wilt -- a real bug, not expected from a valid lookup table")
    print("  Sanity check passed: theta_s > theta_wilt at every valid cell.")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--sand-path", default="./data/static/pilot_sand_pct.tif")
    parser.add_argument("--clay-path", default="./data/static/pilot_clay_pct.tif")
    parser.add_argument("--output-dir", default="./data/static")
    args = parser.parse_args()
    derive_soil_params(args.sand_path, args.clay_path, args.output_dir)


if __name__ == "__main__":
    main()
