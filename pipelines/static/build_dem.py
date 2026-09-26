#!/usr/bin/env python3
"""Build the pilot region's DEM, reprojected onto the pinned grid
(`grid_spec.py`), from the real USGS 3DEP 1/3 arc-second source.

**Reads directly via `/vsicurl/` HTTP range requests, not a full-tile
download.** Confirmed live (Session 11): rasterio's bundled GDAL can open
`/vsicurl/https://prd-tnm.s3.amazonaws.com/.../USGS_13_n40w106.tif`
directly and read only the pilot bbox's window -- each source tile is
~413MB but the pilot only needs a fraction of one, so streaming the exact
window needed avoids downloading gigabytes of terrain outside the pilot
area. This is a real, live download of real data, just windowed rather
than whole-tile -- not a synthetic shortcut. `fetch_3dep.py`'s own
whole-tile download remains the right tool for an eventual full-state
build (where nearly the whole tile is needed anyway); this module is
specifically for the pilot's smaller footprint.

Pipeline: for each of the pilot's covering 3DEP tiles (`grid_spec.
PILOT_3DEP_TILES`) read the pilot bbox's window (in the tile's native
EPSG:4269 geographic CRS) -> merge (rasterio.merge, handles the
n40w106/n41w106 seam at 40N) -> reproject onto the pinned grid
(EPSG:5070, 10m, bilinear resampling -- elevation is a continuous field,
matching the resampling method already used for Shale Hills' own DEM
reprojection in Session 10).
"""

from __future__ import annotations

import argparse

import numpy as np
import rasterio
from rasterio.merge import merge
from rasterio.warp import Resampling, calculate_default_transform, reproject

from grid_spec import STATIC_STACK_CRS, GridSpec, pilot_grid_spec

BUCKET_URL = "https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/13/TIFF/current"


def tile_vsicurl_url(tile: str) -> str:
    return f"/vsicurl/{BUCKET_URL}/{tile}/USGS_13_{tile}.tif"


def read_windowed_tile(tile: str, bbox_wgs84: tuple[float, float, float, float]):
    """Open a 3DEP tile via vsicurl and read only the window overlapping
    bbox_wgs84 (the tile's own native CRS, EPSG:4269 -- confirmed live,
    see this module's docstring). Returns None if the tile doesn't
    actually overlap the bbox (can happen at the pilot's tile-boundary
    seam, where one of the two tiles might only marginally intersect)."""
    from rasterio.windows import from_bounds

    url = tile_vsicurl_url(tile)
    min_lon, min_lat, max_lon, max_lat = bbox_wgs84
    with rasterio.open(url) as src:
        window = from_bounds(min_lon, min_lat, max_lon, max_lat, transform=src.transform)
        window = window.intersection(rasterio.windows.Window(0, 0, src.width, src.height))
        if window.width <= 0 or window.height <= 0:
            return None
        data = src.read(1, window=window)
        window_transform = src.window_transform(window)
        return data, window_transform, src.crs, src.nodata


def build_pilot_dem(bbox_wgs84: tuple[float, float, float, float], tiles: list[str], output_path: str, grid: GridSpec) -> None:
    sources = []
    for tile in tiles:
        result = read_windowed_tile(tile, bbox_wgs84)
        if result is None:
            print(f"  {tile}: no overlap with the requested bbox, skipping")
            continue
        data, window_transform, src_crs, nodata = result
        sources.append((data, window_transform, src_crs, nodata))
        print(f"  {tile}: read {data.shape[1]}x{data.shape[0]} window")

    if not sources:
        raise RuntimeError(f"No 3DEP tile overlapped bbox {bbox_wgs84} -- check grid_spec.PILOT_3DEP_TILES")

    # rasterio.merge expects DatasetReader-like objects; build tiny
    # in-memory datasets for each window so merge() can handle the
    # 40N tile-boundary seam itself (its own, well-tested overlap logic),
    # rather than hand-rolling seam-stitching here.
    import rasterio.io

    memfiles = []
    datasets = []
    try:
        for data, window_transform, src_crs, nodata in sources:
            memfile = rasterio.io.MemoryFile()
            memfiles.append(memfile)
            with memfile.open(
                driver="GTiff", height=data.shape[0], width=data.shape[1], count=1,
                dtype=data.dtype, crs=src_crs, transform=window_transform, nodata=nodata,
            ) as ds:
                ds.write(data, 1)
            datasets.append(memfile.open())

        mosaic, mosaic_transform = merge(datasets)
        mosaic_crs = datasets[0].crs
        mosaic_nodata = datasets[0].nodata
    finally:
        for ds in datasets:
            ds.close()
        for mf in memfiles:
            mf.close()

    print(f"Mosaic: {mosaic.shape[2]}x{mosaic.shape[1]} in {mosaic_crs}")

    # Reproject the mosaic onto the pinned grid.
    dest = np.full((grid.height, grid.width), np.nan, dtype=np.float32)
    reproject(
        source=mosaic[0],
        destination=dest,
        src_transform=mosaic_transform,
        src_crs=mosaic_crs,
        src_nodata=mosaic_nodata,
        dst_transform=grid.transform,
        dst_crs=grid.crs,
        dst_nodata=np.nan,
        resampling=Resampling.bilinear,
    )

    with rasterio.open(
        output_path, "w", driver="GTiff", height=grid.height, width=grid.width, count=1,
        dtype=np.float32, crs=grid.crs, transform=grid.transform, nodata=np.nan, compress="deflate",
    ) as dst:
        dst.write(dest, 1)

    valid = dest[~np.isnan(dest)]
    print(f"Wrote {output_path}: {grid.width}x{grid.height}, elevation range {valid.min():.1f}-{valid.max():.1f}m")


def main():
    from grid_spec import PILOT_3DEP_TILES, PILOT_BBOX_WGS84

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", default="./data/static/pilot_dem.tif")
    args = parser.parse_args()

    grid = pilot_grid_spec()
    build_pilot_dem(PILOT_BBOX_WGS84, PILOT_3DEP_TILES, args.output, grid)


if __name__ == "__main__":
    main()
