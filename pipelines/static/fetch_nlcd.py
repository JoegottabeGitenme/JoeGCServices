#!/usr/bin/env python3
"""Fetch NLCD land cover (confirmed live) and tree canopy density (source
identified, not fetched this session) for the Colorado domain.

**Land cover: fully live-verified this session, not just GetCapabilities.**
`https://www.mrlc.gov/geoserver/mrlc_download/wcs` (WCS 2.0.1) lists a real
current coverage (`mrlc_download__NLCD_2021_Land_Cover_L48`), and an actual
GetCoverage request against it (a live curl, this session, for a small
Front Range bbox) returned a real 159x199 RGB-palette GeoTIFF -- not just a
capabilities-document check. Getting there required debugging one real
error (`InvalidAxisLabel`, see `build_getcoverage_url`'s docstring) that a
naive "spec-compliant-looking" URL would have missed.

**Tree canopy density: NOT the same product line, not verified this
session.** This WCS's "Canopy" coverages are AK/HI/PR-only plus a change
index -- CONUS tree canopy cover is a separate NLCD-adjacent product
("NLCD TCC CONUS"), historically distributed by the USFS (Forest Service)
rather than through this MRLC WCS endpoint. Its current URL was not
confirmed live this session (ran out of session time chasing this one
down after the SSURGO distribution-mechanism surprise -- see
fetch_ssurgo.py). Fill in `TCC_SOURCE_URL` below once confirmed; canopy
density is used by physics/snow.py's canopy interception adjustment and by
Eq. 4's green-vegetation-fraction term.

Usage:
    python3 fetch_nlcd.py --bbox -109.06 36.99 -102.04 41.00 --output-dir ./data/nlcd
"""

from __future__ import annotations

import argparse
import urllib.request
import urllib.parse

WCS_BASE_URL = "https://www.mrlc.gov/geoserver/mrlc_download/wcs"
LAND_COVER_COVERAGE_ID = "mrlc_download__NLCD_2021_Land_Cover_L48"

# NOT VERIFIED THIS SESSION -- see module docstring.
TCC_SOURCE_URL = None  # e.g. "https://data.fs.usda.gov/geodata/rastergateway/treecanopycover/..."


def build_getcoverage_url(
    coverage_id: str, min_lon: float, min_lat: float, max_lon: float, max_lat: float
) -> str:
    """WCS 2.0.1 GetCoverage request, subsetted to a bbox in EPSG:4326.

    **Live-verified this session**, including debugging a real error: the
    coverage's native CRS is a projected CRS (DescribeCoverage reports
    native axisLabels="X Y", not Long/Lat) so a bare `Long(...)/Lat(...)`
    subset -- valid WCS 2.0 syntax in general, but wrong for THIS
    coverage's default subsetting CRS -- was rejected with
    `InvalidAxisLabel`. Fixed by adding the WCS CRS extension's
    `subsettingCrs` parameter to explicitly subset in EPSG:4326 regardless
    of the coverage's native CRS. Confirmed working: a live GetCoverage
    request with this exact URL shape returned a real 159x199 RGB-palette
    GeoTIFF (curl, this session).
    """
    params = {
        "service": "WCS",
        "version": "2.0.1",
        "request": "GetCoverage",
        "coverageId": coverage_id,
        "format": "image/geotiff",
        "subset": [f"Long({min_lon},{max_lon})", f"Lat({min_lat},{max_lat})"],
        "subsettingCrs": "http://www.opengis.net/def/crs/EPSG/0/4326",
        "outputCrs": "http://www.opengis.net/def/crs/EPSG/0/4326",
    }
    # subset appears twice -- urlencode with doseq handles repeated keys.
    query = urllib.parse.urlencode(params, doseq=True)
    return f"{WCS_BASE_URL}?{query}"


def fetch_land_cover(bbox: tuple[float, float, float, float], output_path: str) -> None:
    url = build_getcoverage_url(LAND_COVER_COVERAGE_ID, *bbox)
    print(f"Fetching NLCD 2021 land cover: {url}")
    urllib.request.urlretrieve(url, output_path)
    print(f"Wrote {output_path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bbox", nargs=4, type=float, metavar=("MIN_LON", "MIN_LAT", "MAX_LON", "MAX_LAT"),
                         default=[-109.06, 36.99, -102.04, 41.00])
    parser.add_argument("--output-dir", default="./data/nlcd")
    args = parser.parse_args()

    import os

    os.makedirs(args.output_dir, exist_ok=True)
    fetch_land_cover(tuple(args.bbox), os.path.join(args.output_dir, "nlcd_2021_land_cover_co.tif"))

    if TCC_SOURCE_URL is None:
        print(
            "\nWARNING: tree canopy density source not confirmed this session -- "
            "see this script's module docstring. Land cover fetched; canopy density was not."
        )


if __name__ == "__main__":
    main()
