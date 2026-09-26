"""The WS1 static-stack grid definition -- pinned once, here, and read from
every other module in this package (never re-derived independently). This
was explicitly flagged in prior sessions as undefined anywhere in the repo
(`build_corridor_mask.py`'s own docstring: "that grid's exact origin/extent
doesn't exist yet") -- this module is that definition.

**CRS choice: EPSG:5070 (NAD83 / Conus Albers Equal Area)**, not a UTM zone.
Colorado straddles UTM zones 12N and 13N (the zone boundary runs through
the state roughly along -108W) -- a UTM-zoned grid would need an arbitrary
zone choice and would distort cell areas away from that zone's central
meridian. CONUS Albers is the USGS's own standard equal-area CRS for
exactly this kind of statewide/multi-state raster work (3DEP's own gSSURGO
rasterized products, NLCD, and PRISM all ship in Albers variants) --
choosing it isn't a novel decision, it's following the established
convention of the very data sources this stack consumes.

**Resolution: 10m**, per every prior session's own stated target
(`pipelines/static/README.md`) and to match 3DEP's native 1/3-arcsecond
product (~9.26e-5 deg ~= 10.3m at Colorado's latitudes -- confirmed via a
live vsicurl read of a real 3DEP tile, Session 11) without wastefully
oversampling.

**Pilot region (Session 11)**: per the user's own decision to prove the
full fetch -> TWI -> Zarr -> main.py -> EDR chain on a smaller area before
committing to a statewide build, this module also defines
`PILOT_BBOX_WGS84` -- the Boulder-area foothills (Golden up through
Boulder/Lyons/Nederland), the highest-traffic trail corridor on the Front
Range and a genuinely varied testbed (steep granite/gneiss foothills,
mixed aspect, real elevation relief). Confirmed live (Session 11) to be
covered by exactly two adjacent 3DEP tiles (`n40w106`, `n41w106`), both
verified reachable via HTTP HEAD before this bbox was chosen -- not chosen
blind. Expanding to the full Front Range or statewide later requires no
code changes here beyond widening this one bbox constant.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

from pyproj import Transformer

STATIC_STACK_CRS = "EPSG:5070"  # NAD83 / Conus Albers Equal Area
STATIC_STACK_RESOLUTION_M = 10.0

# Boulder-area foothills pilot bbox, WGS84 lon/lat (min_lon, min_lat, max_lon, max_lat).
# Covers Golden, Boulder, Lyons, and Nederland's trail systems -- Colorado
# Front Range's highest-traffic corridor. Spans the n40w106/n41w106 3DEP
# tile boundary at 40N deliberately (a real test of the mosaic step, not
# an accidental edge case).
PILOT_BBOX_WGS84 = (-105.60, 39.85, -105.10, 40.15)

# The two 3DEP 1-degree tiles confirmed live (Session 11, HTTP HEAD) to
# cover PILOT_BBOX_WGS84. See fetch_3dep.py's own tiles_for_bbox() for the
# general-purpose version of this enumeration -- duplicated as an explicit
# constant here since the pilot bbox is small and fixed, and a live-checked
# literal is stronger evidence than trusting the enumeration function alone.
PILOT_3DEP_TILES = ["n40w106", "n41w106"]


@dataclass(frozen=True)
class GridSpec:
    """A pinned, reproducible raster grid definition. Every WS1 layer
    (elevation, TWI, slope, aspect, theta_s, theta_wilt) is resampled onto
    EXACTLY this grid before being written to the Zarr stack, so that
    per-vertex sampling in main.py never needs to reconcile mismatched
    per-layer grids."""

    crs: str
    resolution_m: float
    # Bounds in the grid's own CRS (EPSG:5070 meters): (xmin, ymin, xmax, ymax),
    # snapped to exact multiples of resolution_m so every layer's affine
    # transform is bit-for-bit identical regardless of which script wrote it.
    xmin: float
    ymin: float
    xmax: float
    ymax: float

    @property
    def width(self) -> int:
        return round((self.xmax - self.xmin) / self.resolution_m)

    @property
    def height(self) -> int:
        return round((self.ymax - self.ymin) / self.resolution_m)

    @property
    def transform(self):
        from rasterio.transform import from_origin

        return from_origin(self.xmin, self.ymax, self.resolution_m, self.resolution_m)

    def to_attrs_dict(self) -> dict:
        """Serializable form for Zarr group attrs -- so any consumer (a
        future rebuild, main.py's sampler) can read the grid definition
        from the data itself rather than re-importing this module and
        risking drift between what was written and what's assumed."""
        return {
            "crs": self.crs,
            "resolution_m": self.resolution_m,
            "xmin": self.xmin,
            "ymin": self.ymin,
            "xmax": self.xmax,
            "ymax": self.ymax,
            "width": self.width,
            "height": self.height,
        }


def bbox_wgs84_to_grid_spec(
    min_lon: float, min_lat: float, max_lon: float, max_lat: float, resolution_m: float = STATIC_STACK_RESOLUTION_M
) -> GridSpec:
    """Reproject a WGS84 bbox's corners to EPSG:5070 and snap outward to
    clean multiples of `resolution_m` -- so the resulting grid's origin is
    a round number in the target CRS (e.g. exactly on a 10m boundary),
    not an arbitrary reprojected coordinate. Snapping OUTWARD (floor the
    min, ceil the max) guarantees the requested bbox is fully contained,
    never clipped."""
    transformer = Transformer.from_crs("EPSG:4326", STATIC_STACK_CRS, always_xy=True)
    corners_lonlat = [(min_lon, min_lat), (max_lon, min_lat), (min_lon, max_lat), (max_lon, max_lat)]
    xs, ys = zip(*(transformer.transform(lon, lat) for lon, lat in corners_lonlat))

    xmin = _floor_to_multiple(min(xs), resolution_m)
    ymin = _floor_to_multiple(min(ys), resolution_m)
    xmax = _ceil_to_multiple(max(xs), resolution_m)
    ymax = _ceil_to_multiple(max(ys), resolution_m)

    return GridSpec(crs=STATIC_STACK_CRS, resolution_m=resolution_m, xmin=xmin, ymin=ymin, xmax=xmax, ymax=ymax)


def _floor_to_multiple(value: float, multiple: float) -> float:
    return math.floor(value / multiple) * multiple


def _ceil_to_multiple(value: float, multiple: float) -> float:
    return math.ceil(value / multiple) * multiple


def pilot_grid_spec() -> GridSpec:
    """The pinned pilot region's grid spec -- call this, not
    `bbox_wgs84_to_grid_spec` directly with a hand-typed bbox, everywhere
    else in this package, so there is exactly one source of truth for the
    pilot region's exact extent."""
    return bbox_wgs84_to_grid_spec(*PILOT_BBOX_WGS84)
