# Point reads on projected grids landed in the wrong cell

## What was wrong

`ZarrGridProcessor::point_to_grid` turned a longitude/latitude into a grid cell by interpolating
linearly across the grid's longitude/latitude bounding box:

```
col = (lon - min_lon) / ((max_lon - min_lon) / width)
row = (lat - min_lat) / ((max_lat - min_lat) / height)
```

That is exact for a regular lat/lon grid (GFS, MRMS). It is wrong for HRRR, NDFD and NBM-CONUS
(Lambert conformal), NBM-Alaska (polar stereographic) and the small NBM regions (Mercator), whose rows
and columns follow projected metres. On HRRR the grid's four corners span lon -134.1 to -60.9 and lat
21.1 to 47.8, but its top-middle cell is at 52.6 N and its bottom-middle at 24.4 N; the corner
"envelope" is neither the grid's extent nor linearly related to it.

The region/area/tile paths already used the projection (`resample_lambert_to_geographic`); only the
point path did not.

## How big

Computed with pyproj on HRRR's projection, the cell the old code read versus the cell the point is in:

| Place | Error |
|---|---|
| Denver | 470 km |
| Seattle | 294 km |
| Miami | 536 km |
| Chicago | 469 km |
| New York | 333 km |
| centre of the grid | 478 km |

Measured on production before the fix (live EDR 2 m temperature vs `wgrib2 -lon` on the same HRRR
file, 2026-10-11 22Z): Denver -3.0 K, Seattle -0.6, Miami -2.7, Chicago +3.9, Dallas -3.7, Phoenix
-5.3. A correct read agrees to a fraction of a kelvin. It also rejected points that are inside the grid
(anything north of 47.8 N) and accepted points that are not (the Gulf of Mexico south of the grid's
curved bottom edge).

Affected: every EDR point-style query on `hrrr`, `ndfd` and `nbm-*` (position, locations, corridor,
radius point sampling, series). Not affected: GFS, MRMS, GLDAS and other geographic grids; area, cube,
tile and WMS rendering; trail conditions (`services/trail-physics` samples HRRR with its own transcription
of the Lambert transform, `hrrr_grid.py`, and never went through `point_to_grid`).

It went unnoticed because values stay plausible 300-500 km away, and the existing HRRR tests compared
smooth fields (pressure, precipitable water) with loose tolerances.

## The fix

`NativeGrid` (`crates/grid-processor/src/projection/native.rs`) selects the grid's own projection from
the stored model name and projection kind and returns fractional (column, row) with the projection
code that already existed in the `projection` crate. `point_to_grid` uses it when there is one and keeps
the linear map for geographic grids and for any projected model it does not list.

* A point inside the grid's footprint (cell centres +- half a cell) reads; the edge is clamped.
* Row index follows the stored `row_origin`.
* `hrrr-subh` shares HRRR's grid.

## How it was checked

* `projection::native::tests`: for HRRR, NBM-CONUS, NBM-Alaska, NBM-Hawaii, NBM-Puerto Rico and
  NBM-Guam, nine (five for the small ones) cells each, with the longitude/latitude **the data file
  itself assigns to the cell** (`wgrib2 -ijlat` on real NCEP messages), map back to that cell to within
  0.02 cells, in both 0-360 and -180..180 longitudes.
* `tests/projected_point_read.rs`: a real HRRR-sized Zarr array holding each cell's own index; reads at
  the file's coordinates return exactly that cell; fractional positions at six cities match pyproj
  (an independent implementation) within 0.01 cells; points outside the grid read as outside.
* `services/edr-api/tests/hrrr_subhourly.rs`: through the whole stack, REFC/PRATE/type values at two
  cells match `wgrib2`.
* Mutants (read ignoring the projection, flipped rows, strict bounds, missing clamp, wrong grid) all
  fail.

## Consequences

Values returned for these models change, to the correct ones. Anything caching them (EDR location
cache, Cloudflare) will serve the old values until it expires.

`GridMetadata::resolution()` (span / count rather than span / (count - 1)) is a separate, smaller
drift that still applies to the regular lat/lon grids; it is not touched here.
