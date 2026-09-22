#!/usr/bin/env python3
"""Derive slope, aspect, TWI, sky-view factor, and 16-azimuth Winstral Sx
from a mosaicked 3DEP DEM, at Front-Range/Colorado scale (~300M cells).

**Not executed this session** -- WhiteboxTools isn't installed in this
environment, and running it needs the actual mosaicked Colorado DEM (which
itself needs fetch_3dep.py's ~11.5GB of tile downloads, also not run this
session -- see pipelines/static/README.md).

Why WhiteboxTools specifically, not this project's own
`services/trail-physics/physics/terrain.py`: that module's D8 accumulation
and iterative pit-fill are explicitly documented there as appropriate for
small catchments (~4,000 cells, i.e. Tarrawarra) and NOT for a
Front-Range-scale DEM -- the O(n log n) sort-based accumulation and
iterative pit-fill both become impractical at ~300M cells. WhiteboxTools
(`pip install whitebox`, wraps a compiled Rust/whitebox_tools binary) is
the standard tool for exactly this scale, with a proper priority-flood
depression-filling algorithm and D8/D-infinity flow accumulation
implementations used throughout the hydrology community.

WhiteboxTools' Python API (the `whitebox` PyPI package) is a thin wrapper
that shells out to the compiled binary per-tool-call, each taking an input
raster path and writing an output raster path -- the tool names and
argument shapes below (`fill_depressions`, `d8_pointer`,
`d8_flow_accumulation`, `slope`, `aspect`, `wetness_index`,
`sky_view_factor`) are WhiteboxTools' own documented, stable tool names
(this project has no ability to independently verify them run correctly
without the binary installed -- treat this as the intended API shape, and
smoke-test tool-by-tool against a small DEM tile before trusting the full
pipeline).
"""

from __future__ import annotations

import argparse
import os


def derive_terrain_stack(dem_path: str, output_dir: str) -> None:
    """Run the full terrain-derivative chain via WhiteboxTools."""
    import whitebox

    wbt = whitebox.WhiteboxTools()
    wbt.verbose = True
    os.makedirs(output_dir, exist_ok=True)

    def out(name: str) -> str:
        return os.path.join(output_dir, name)

    # 1. Priority-flood depression filling -- the Front-Range-scale
    #    equivalent of physics/terrain.py's fill_pits_and_flats, but a real
    #    algorithm at this scale (Wang & Liu 2006 / Planchon & Darboux 2001
    #    lineage; WhiteboxTools' own implementation, not this project's).
    filled_dem = out("dem_filled.tif")
    wbt.fill_depressions(dem_path, filled_dem)

    # 2. D8 flow direction + accumulation (specific catchment area).
    d8_pointer = out("d8_pointer.tif")
    wbt.d8_pointer(filled_dem, d8_pointer)
    flow_accum = out("flow_accumulation.tif")
    wbt.d8_flow_accumulation(filled_dem, flow_accum, out_type="specific contributing area")

    # 3. Slope + aspect (WhiteboxTools' own implementations -- not
    #    physics/terrain.py's, which is the small-catchment/Tarrawarra path).
    slope = out("slope.tif")
    wbt.slope(filled_dem, slope)
    aspect = out("aspect.tif")
    wbt.aspect(filled_dem, aspect)

    # 4. TWI directly -- WhiteboxTools has a built-in wetness_index tool
    #    (lambda = ln(specific_catchment_area / tan(slope)), matching Eq. 1's
    #    input exactly).
    twi = out("twi.tif")
    wbt.wetness_index(flow_accum, slope, twi)

    # 5. Sky-view factor (Eq. 6's diffuse-radiation term).
    svf = out("sky_view_factor.tif")
    wbt.sky_view_factor(filled_dem, svf)

    # 6. Winstral Sx, all 16 azimuths (per the design doc's Q3 resolution:
    #    computed now as a diagnostic even though storm-weighted wind
    #    redistribution is deferred behind Rung 3 residual analysis).
    #    WhiteboxTools doesn't have a single "Sx" tool; horizon_angle at each
    #    of 16 azimuths (every 360/16 = 22.5 degrees) is the standard way to
    #    build it. Using float azimuths (not truncated to int) so this is
    #    actually 16 evenly-spaced directions, not 17 unevenly-spaced ones.
    azimuths = [i * 360.0 / 16 for i in range(16)]
    for azimuth in azimuths:
        horizon_out = out(f"horizon_angle_{azimuth:05.1f}.tif")
        wbt.horizon_angle(filled_dem, horizon_out, azimuth=azimuth, max_dist=300.0)

    print(f"Terrain stack written to {output_dir}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dem-path", required=True, help="Mosaicked DEM (from fetch_3dep.py's tiles)")
    parser.add_argument("--output-dir", default="./data/terrain")
    args = parser.parse_args()
    derive_terrain_stack(args.dem_path, args.output_dir)


if __name__ == "__main__":
    main()
