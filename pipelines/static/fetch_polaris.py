#!/usr/bin/env python3
"""Fetch POLARIS soil property tiles (sand%/clay%, 0-30cm thickness-
weighted) for the pilot region, reprojected onto the pinned grid.

**Why POLARIS, not gSSURGO** (per the user's own Session-9-era decision):
gSSURGO's distribution moved to a JS-rendered Box folder with no stable
download URL (confirmed dead this session and prior --
`fetch_ssurgo.py`'s own docstring). POLARIS (Chaney et al. 2019) is a
public, direct-HTTP, 30m probabilistic SSURGO-derived product with sand%/
clay% at multiple depths -- fully automatable, and the SAME underlying
source data (SSURGO) machine-learning-downscaled to 30m, not a different
soil database. Confirmed live this session:
`http://hydrology.cee.duke.edu/POLARIS/PROPERTIES/v1.0/{property}/mean/
{depth}/lat{Y}{Y+1}_lon{X}{X+1}.tif`, EPSG:4326, 3600x3600 per 1-degree
tile (~30m), values already in percent (0-100) -- confirmed via a live
windowed read (sand% 26.5-83.6 over a Front-Range test window, physically
plausible for this region's decomposed-granite/sandy soils).

**Only sand% and clay% are fetched** -- Session 8's discovery
(`physics/redistribution.py`'s module docstring) means the real production
equation needs theta_s/theta_wilt, not measured Ks, and this project's own
established pipeline (`physics/soil_texture.py`, used at every validation
site) derives BOTH from a USDA texture classification of sand%/clay%
alone. POLARIS's own theta_s/theta_r layers are deliberately NOT used
directly here (theta_r is not the same quantity as wilting point, and
bypassing the texture-triangle/Noah-lookup step would be a different,
unvalidated soil-parameter methodology -- see derive_soil_params.py).

**0-30cm thickness-weighted mean, matching the "top 30cm" convention used
at every validation site** (Tarrawarra's particle.dat shallowest layer,
Shale Hills' SSURGO dominant-component shallowest horizon): POLARIS's own
depth bins are 0-5cm, 5-15cm, 15-30cm -- weighted by each bin's thickness
(5, 10, 15cm respectively, summing to the full 30cm) rather than a plain
unweighted average across bins of different thickness.

Tile coverage for the pilot bbox confirmed live before writing this
module: the pilot's longitude range (-105.60 to -105.10) falls entirely
within POLARIS's single 1-degree longitude bin [-106, -105]; the latitude
range (39.85-40.15) spans two bins ([39,40] and [40,41]) -- 2 tiles
needed per property per depth, matching the same 40N tile-seam pattern
`build_dem.py` already handles for 3DEP.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
import rasterio
from rasterio.merge import merge
from rasterio.warp import Resampling, reproject
from rasterio.windows import from_bounds

sys.path.insert(0, str(Path(__file__).parent))

from grid_spec import GridSpec, pilot_grid_spec

POLARIS_ROOT = "http://hydrology.cee.duke.edu/POLARIS/PROPERTIES/v1.0"
POLARIS_DEPTHS_CM = [("0_5", 5.0), ("5_15", 10.0), ("15_30", 15.0)]  # (depth label, thickness cm)
POLARIS_TILES = ["lat3940_lon-106-105", "lat4041_lon-106-105"]  # confirmed live, see module docstring


def polaris_tile_url(soil_property: str, depth_label: str, tile: str) -> str:
    return f"/vsicurl/{POLARIS_ROOT}/{soil_property}/mean/{depth_label}/{tile}.tif"


def _read_windowed(url: str, bbox_wgs84: tuple[float, float, float, float]):
    min_lon, min_lat, max_lon, max_lat = bbox_wgs84
    with rasterio.open(url) as src:
        window = from_bounds(min_lon, min_lat, max_lon, max_lat, transform=src.transform)
        window = window.intersection(rasterio.windows.Window(0, 0, src.width, src.height))
        if window.width <= 0 or window.height <= 0:
            return None
        data = src.read(1, window=window)
        return data, src.window_transform(window), src.crs, src.nodata


def _mosaic_windows(windows: list) -> tuple:
    import rasterio.io

    memfiles, datasets = [], []
    try:
        for data, transform, crs, nodata in windows:
            mf = rasterio.io.MemoryFile()
            memfiles.append(mf)
            with mf.open(
                driver="GTiff", height=data.shape[0], width=data.shape[1], count=1,
                dtype=data.dtype, crs=crs, transform=transform, nodata=nodata,
            ) as ds:
                ds.write(data, 1)
            datasets.append(mf.open())
        mosaic, mosaic_transform = merge(datasets)
        return mosaic[0], mosaic_transform, datasets[0].crs, datasets[0].nodata
    finally:
        for ds in datasets:
            ds.close()
        for mf in memfiles:
            mf.close()


def fetch_property_0_30cm(
    soil_property: str, bbox_wgs84: tuple[float, float, float, float], tiles: list[str], grid: GridSpec
) -> np.ndarray:
    """Returns the thickness-weighted 0-30cm mean of `soil_property`
    ('sand' or 'clay'), reprojected onto `grid`."""
    weighted_sum = None
    for depth_label, thickness_cm in POLARIS_DEPTHS_CM:
        windows = []
        for tile in tiles:
            url = polaris_tile_url(soil_property, depth_label, tile)
            result = _read_windowed(url, bbox_wgs84)
            if result is not None:
                windows.append(result)
        if not windows:
            raise RuntimeError(f"No POLARIS tile overlapped bbox {bbox_wgs84} for {soil_property}/{depth_label}")
        mosaic, mosaic_transform, mosaic_crs, mosaic_nodata = _mosaic_windows(windows)

        dest = np.full((grid.height, grid.width), np.nan, dtype=np.float32)
        reproject(
            source=mosaic, destination=dest, src_transform=mosaic_transform, src_crs=mosaic_crs,
            src_nodata=mosaic_nodata, dst_transform=grid.transform, dst_crs=grid.crs, dst_nodata=np.nan,
            resampling=Resampling.bilinear,
        )
        contribution = dest * thickness_cm
        weighted_sum = contribution if weighted_sum is None else weighted_sum + contribution
        print(f"  {soil_property}/{depth_label}: mosaic {mosaic.shape}, reprojected, thickness={thickness_cm}cm")

    total_thickness = sum(t for _, t in POLARIS_DEPTHS_CM)
    return weighted_sum / total_thickness


def main():
    from grid_spec import PILOT_BBOX_WGS84

    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output-dir", default="./data/static")
    args = parser.parse_args()

    grid = pilot_grid_spec()
    Path(args.output_dir).mkdir(parents=True, exist_ok=True)

    for soil_property in ["sand", "clay"]:
        print(f"Fetching {soil_property}% (0-30cm thickness-weighted)...")
        arr = fetch_property_0_30cm(soil_property, PILOT_BBOX_WGS84, POLARIS_TILES, grid)
        out_path = Path(args.output_dir) / f"pilot_{soil_property}_pct.tif"
        with rasterio.open(
            out_path, "w", driver="GTiff", height=grid.height, width=grid.width, count=1,
            dtype=np.float32, crs=grid.crs, transform=grid.transform, nodata=np.nan, compress="deflate",
        ) as dst:
            dst.write(arr.astype(np.float32), 1)
        valid = arr[~np.isnan(arr)]
        print(f"  wrote {out_path}: mean={valid.mean():.1f}% min={valid.min():.1f}% max={valid.max():.1f}%")


if __name__ == "__main__":
    main()
