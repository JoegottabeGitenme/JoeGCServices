#!/usr/bin/env python3
"""Validate an assembled static stack, and cross-check it against another.

Two checks, both run against the REAL Zarr stacks (never mocks):

1. **`sanity_report`** -- per-layer valid fraction and value ranges, from a
   strided subsample (reading all 345.6M cells of a Front Range layer just to
   print a mean would need ~1.4 GB per layer for no analytical benefit).
   Catches the failure modes a build can have without raising: a layer that
   is mostly NaN (a tile that never got written), values in the wrong units,
   a layer on the wrong grid.

2. **`compare_stacks`** -- sample two stacks at the SAME lon/lat points and
   compare. The Front Range stack contains the already-live pilot region, so
   the pilot stack is an independent reference: both were derived from the
   same 3DEP DEM with the same validated method, but by different code
   paths (the pilot monolithically, the Front Range in tiles), on grids with
   different origins. Agreement over the shared region is direct evidence
   the tiled build didn't corrupt anything -- stronger than the synthetic
   tests, which can only show the tiling logic is self-consistent.

   What to expect is NOT identical values: the two grids are offset by a
   sub-cell shift, so each resamples the DEM slightly differently, and TWI
   (a log of upstream area over slope) is sensitive to that at the
   single-cell level. So the report gives correlations and differences, and
   the judgment of "close enough" is made against those numbers, with the
   reasoning recorded where the stack is deployed -- not hidden behind a
   threshold chosen before seeing any data.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).parent.parent.parent / "services" / "trail-physics"))

from static_stack import StaticStack  # noqa: E402

LAYERS = ("elevation", "twi", "slope", "aspect", "theta_s", "theta_wilt")

# Physically plausible bounds, used only to flag a layer as suspicious in the
# report -- generous, so they catch wrong units/garbage and not real variation.
PLAUSIBLE_RANGES = {
    "elevation": (0.0, 4500.0),       # m; Colorado's highest point is 4,401 m
    "twi": (0.0, 40.0),               # ln(uca/slope), pyDEM-limited
    "slope": (0.0, 20.0),             # tan(slope); >20 would be near-vertical beyond any DEM
    "aspect": (0.0, 360.0),
    "theta_s": (0.2, 0.7),            # saturated volumetric water content
    "theta_wilt": (0.0, 0.3),
}


def sanity_report(stack: StaticStack, stride: int = 8) -> dict:
    """Per-layer stats from a `stride`-subsampled read. Returns
    {layer: {"valid_fraction", "min", "max", "mean", "in_range"}}."""
    report = {}
    for name in LAYERS:
        arr = np.asarray(stack.root[name][::stride, ::stride], dtype=np.float64)
        valid = arr[~np.isnan(arr)]
        lo, hi = PLAUSIBLE_RANGES[name]
        if valid.size == 0:
            report[name] = {"valid_fraction": 0.0, "min": None, "max": None, "mean": None, "in_range": False}
            continue
        report[name] = {
            "valid_fraction": float(valid.size / arr.size),
            "min": float(valid.min()),
            "max": float(valid.max()),
            "mean": float(valid.mean()),
            "in_range": bool(valid.min() >= lo and valid.max() <= hi),
        }
    return report


def _shared_lonlat_bbox(a: StaticStack, b: StaticStack) -> tuple[float, float, float, float]:
    a0, a1, a2, a3 = a.wgs84_bbox()
    b0, b1, b2, b3 = b.wgs84_bbox()
    box = (max(a0, b0), max(a1, b1), min(a2, b2), min(a3, b3))
    if box[0] >= box[2] or box[1] >= box[3]:
        raise ValueError("the two stacks' extents do not overlap")
    return box


def compare_samples(a: np.ndarray, b: np.ndarray) -> dict:
    """Agreement stats between two sampled arrays over their mutually-valid
    entries. Pure function (the unit under test)."""
    both = ~np.isnan(a) & ~np.isnan(b)
    n = int(both.sum())
    if n < 2:
        return {"n": n, "pearson_r": None, "mean_abs_diff": None, "median_abs_diff": None, "p95_abs_diff": None,
                "mean_a": None, "mean_b": None}
    x, y = a[both], b[both]
    d = np.abs(x - y)
    std_x, std_y = x.std(), y.std()
    r = float(np.corrcoef(x, y)[0, 1]) if std_x > 0 and std_y > 0 else None
    return {
        "n": n,
        "pearson_r": r,
        "mean_abs_diff": float(d.mean()),
        "median_abs_diff": float(np.median(d)),
        "p95_abs_diff": float(np.percentile(d, 95)),
        "mean_a": float(x.mean()),
        "mean_b": float(y.mean()),
    }


def compare_stacks(a: StaticStack, b: StaticStack, n_points: int = 20_000, seed: int = 0) -> dict:
    """Sample both stacks at the same random lon/lat points inside their
    overlap. Returns {layer: compare_samples(...)} plus a `twi_bar` entry
    comparing the per-HRRR-cell lambda_bar lookups for cells both cover."""
    rng = np.random.default_rng(seed)
    min_lon, min_lat, max_lon, max_lat = _shared_lonlat_bbox(a, b)
    lons = rng.uniform(min_lon, max_lon, n_points)
    lats = rng.uniform(min_lat, max_lat, n_points)

    ra, ca = a.lonlat_to_rowcol_array(lons, lats)
    rb, cb = b.lonlat_to_rowcol_array(lons, lats)

    result = {}
    for name in LAYERS:
        result[name] = compare_samples(a.sample_layer_array(name, ra, ca), b.sample_layer_array(name, rb, cb))

    # lambda_bar: the coarse per-HRRR-cell mean TWI. A cell only PARTLY covered
    # by one stack gets a different mean there, so only compare cells where
    # both stacks saw a comparable number of fine cells.
    common = sorted(set(a._twi_bar_lookup) & set(b._twi_bar_lookup))
    if common:
        va = np.array([a._twi_bar_lookup[k] for k in common])
        vb = np.array([b._twi_bar_lookup[k] for k in common])
        result["twi_bar"] = compare_samples(va, vb)
        result["twi_bar"]["n_common_hrrr_cells"] = len(common)
    return result


def _fmt(x, digits=4):
    return "n/a" if x is None else f"{x:.{digits}f}"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("stack", help="path to the assembled .zarr stack to validate")
    parser.add_argument("--against", help="a second stack to cross-check against (e.g. the live pilot)")
    parser.add_argument("--points", type=int, default=20_000)
    args = parser.parse_args()

    stack = StaticStack(args.stack)
    print(f"== {args.stack}: {stack.width}x{stack.height} cells, {len(stack._twi_bar_lookup)} HRRR cells in lambda_bar")
    print(f"   WGS84 envelope: {tuple(round(v, 4) for v in stack.wgs84_bbox())}")
    print("-- sanity (stride-8 subsample)")
    bad = []
    for name, st in sanity_report(stack).items():
        flag = "" if st["in_range"] and st["valid_fraction"] > 0.5 else "   <-- CHECK"
        if flag:
            bad.append(name)
        print(f"   {name:11s} valid={_fmt(st['valid_fraction'], 3)} min={_fmt(st['min'], 3)} max={_fmt(st['max'], 3)} mean={_fmt(st['mean'], 3)}{flag}")

    if args.against:
        other = StaticStack(args.against)
        print(f"-- cross-check vs {args.against} ({args.points} shared random points)")
        for name, st in compare_stacks(stack, other, args.points).items():
            extra = f" cells={st['n_common_hrrr_cells']}" if "n_common_hrrr_cells" in st else ""
            print(f"   {name:11s} n={st['n']:6d} r={_fmt(st['pearson_r'])} mean|d|={_fmt(st['mean_abs_diff'])} "
                  f"median|d|={_fmt(st['median_abs_diff'])} p95|d|={_fmt(st['p95_abs_diff'])} "
                  f"means {_fmt(st['mean_a'], 3)} vs {_fmt(st['mean_b'], 3)}{extra}")

    if bad:
        print(f"\nSUSPICIOUS LAYERS: {bad}")
        sys.exit(1)


if __name__ == "__main__":
    main()
