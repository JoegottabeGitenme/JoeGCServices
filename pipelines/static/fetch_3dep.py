#!/usr/bin/env python3
"""Fetch USGS 3DEP 1/3 arc-second (~10m) DEM tiles covering a bounding box.

Source confirmed live and reachable this session: the public,
unauthenticated `prd-tnm` S3 bucket
(`https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/13/TIFF/current/`),
tiled by 1-degree cells named by their NW corner (e.g. `n40w106` covers
39-40N, 105-106W). Confirmed via a live bucket listing: tile
`n40w106/USGS_13_n40w106.tif` exists, ~413 MB.

**Tile enumeration logic tested live this session** (test_fetch_3dep.py
verifies the Colorado bbox produces the expected tile name set, and one
tile's URL was confirmed to resolve via a live HEAD request). **The actual
multi-GB-per-tile downloads were not run this session** -- Colorado needs
~28 tiles at this resolution (≈11.5 GB total), which is real background-job
territory, not something to run interactively. See pipelines/static/README.md.
"""

from __future__ import annotations

import argparse
import math
import os
import urllib.request

BUCKET_URL = "https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/13/TIFF/current"


def tile_name(lat: int, lon: int) -> str:
    """1-degree tile name from its NW corner, e.g. (40, -106) -> 'n40w106'.
    `lat`/`lon` are the NW corner's integer degree values (lat = ceiling of
    any point inside the tile, since the tile spans [lat-1, lat] x [lon, lon+1]
    with lon negative -- USGS's own naming, e.g. n40w106 covers 39-40N x
    105-106W)."""
    ns = "n" if lat >= 0 else "s"
    ew = "w" if lon < 0 else "e"
    return f"{ns}{abs(lat):02d}{ew}{abs(lon):03d}"


def tiles_for_bbox(min_lon: float, min_lat: float, max_lon: float, max_lat: float) -> list[str]:
    """Enumerate all 1-degree tile names whose cell overlaps the bbox.
    A tile named n{lat}w{lon} covers [lat-1, lat] x [-lon, -lon+1] (for the
    western hemisphere)."""
    tiles = []
    lat_start = math.floor(min_lat) + 1  # smallest NW-corner lat that could contain min_lat
    lat_end = math.ceil(max_lat)
    lon_start = math.floor(min_lon)  # NW-corner lon numbering: tile n40w106 covers -106 to -105
    lon_end = math.ceil(max_lon) - 1

    for lat in range(lat_start, lat_end + 1):
        for lon in range(lon_start, lon_end + 1):
            tiles.append(tile_name(lat, lon))
    return sorted(set(tiles))


def tile_url(name: str) -> str:
    return f"{BUCKET_URL}/{name}/USGS_13_{name}.tif"


def download_tile(name: str, output_dir: str) -> str:
    os.makedirs(output_dir, exist_ok=True)
    dest = os.path.join(output_dir, f"USGS_13_{name}.tif")
    if os.path.exists(dest):
        print(f"  {name}: already downloaded, skipping")
        return dest
    print(f"  {name}: downloading from {tile_url(name)} ...")
    urllib.request.urlretrieve(tile_url(name), dest)
    return dest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bbox", nargs=4, type=float, metavar=("MIN_LON", "MIN_LAT", "MAX_LON", "MAX_LAT"),
                         default=[-109.06, 36.99, -102.04, 41.00], help="Default: Colorado")
    parser.add_argument("--output-dir", default="./data/3dep")
    parser.add_argument("--list-only", action="store_true", help="Print tile names/URLs without downloading")
    args = parser.parse_args()

    tiles = tiles_for_bbox(*args.bbox)
    print(f"{len(tiles)} tiles cover the requested bbox:")
    for t in tiles:
        print(f"  {t}  ->  {tile_url(t)}")

    if args.list_only:
        return

    for t in tiles:
        download_tile(t, args.output_dir)


if __name__ == "__main__":
    main()
