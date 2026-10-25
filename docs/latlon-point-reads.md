# Point reads on regular lat/lon grids

Point reads, map tiles and area queries on regular latitude/longitude grids landed up to a full
grid cell east and south of the requested location. The affected models were GFS, AIGFS, AIGEFS,
GFS-Wave, MRMS and MRMS-QPE (and GLDAS/NLDAS). Projected grids (HRRR, hrrr-subh, NBM, NDFD) were
not affected: they go through the native projection (see `projected-grid-point-reads.md`).

## What was wrong

A stored grid records its bbox and its shape, and readers turn a coordinate into a grid index
with `(lon - min_lon) / spacing`. Index 0 is the first grid point, sitting on `min_lon`.

- **Spacing divided by the wrong count.** Every GRIB2 and CF-NetCDF grid stores its bbox from the
  first grid point to the last: GFS `0..359.75 × 90..-90` over 1440 × 721, MRMS
  `-129.995..-60.005 × 54.995..20.005` over 7000 × 3500. So the spacing is the extent over
  `n - 1`. `GridMetadata::resolution()` divided by `n`, and `chunks_for_bbox`, the writer's
  `native_resolution` and the WMS GetFeatureInfo sampler's latitude step did the same. The error
  grows from zero at the west/north edge to a full cell at the far edge:

  | Grid | Error over CONUS |
  |---|---|
  | 0.25° global (GFS, AIGFS, AIGEFS, GFS-Wave) | 0.65–0.8 cell east and 0.2–0.4 cell south, about 15–20 km |
  | MRMS 0.01° | up to 0.9 cell (about 1 km) |

- **Grid corners truncated to the millidegree.** The GRIB2 parser stored grid corners with
  `micro / 1000`, an integer division. MRMS's last longitude, 299.994997, became 299.994: the
  extent came up 0.001° short, and reads were a tenth of a 0.01° cell off toward the east edge.
  The parser now rounds.

- **Two writers stored cell edges, not points.** GOES reprojection and the GeoTIFF reader stored
  pixel-edge bboxes. Every reader indexes from the first point, so those were half a pixel off.
  Both now store the span of their pixel centres, so there is one convention everywhere: **the
  stored bbox runs from the first grid point to the last.**

- **GeoTIFF ingest never read its georeferencing.** It looked tags up as `Tag::Unknown(33550)`.
  The `tiff` crate names that tag `ModelPixelScaleTag`, so the lookup never matched, and every
  GeoTIFF used the hard-coded VIIRS defaults. The same applied to the tiepoint and GDAL nodata
  tags. It now uses `Tag::from_u16_exhaustive`.

- **Region reads dropped the last column and row** when a request ended exactly on a grid point.
  The exclusive end was `ceil`; it is now "every point at or before the edge" (`end_index`).

## Measured before the fix

- **EDR, GFS-Wave significant wave height at (140W, 30N):** production returned 2.049 m. The file
  holds 2.11 m there and 2.04 m one cell east.
- **WMS GetMap, `gfswave_HTSGW` over 34–37N, 77–74W, 600 × 600 px** (50 px per grid cell). The
  rendered coastline was compared with the land/sea mask computed from the same GRIB file (a
  pixel is sea when all four surrounding grid points are defined, as bilinear interpolation
  requires). The best fit was **40 px west and 15 px north**, at 99.93% agreement, which is
  0.200° and 0.075°. The prediction from the formula was 0.197° and 0.077°.

## Tests

Each test fails without its fix.

- `services/edr-api/tests/latlon_point_reads.rs` (Docker) runs real GFS-Wave and MRMS-QPE files
  through the real ingester and the EDR position handler, at exact grid points whose neighbours
  differ. The values must equal `wgrib2 -lon`:
  - HTSGW 2.11 m and 2.81 m.
  - QPE 16.0 mm and 11.8 mm.

  Dividing by `n` gives 2.049 (the production value). Truncating the corners gives 16.18.
- `types::tests::grid_spacing_counts_gaps_between_points_not_points`.
- `sections::tests::grid_coordinates_round_to_the_nearest_millidegree`.
- `sampling::tests::a_global_grid_point_reads_its_own_cell` (WMS GetFeatureInfo).
- `reproject::tests::the_output_bbox_spans_the_sampled_pixel_centres` (GOES).
- `geotiff::tests::the_bbox_spans_the_pixel_centres_not_the_pixel_edges` writes a real GeoTIFF
  and reads its tags back.
- The synthetic Zarr fixtures (`testdata.rs`, `simple_10x10.zarr`, `zarr_roundtrip.rs`) used
  edge bboxes for point data; their bboxes now match where their points are.
  `test_simple_grid_point_read` now requires the exact value instead of ±2.

## Deploy

- Rebuild **ingester**, **edr-api** and **wms-api**. The read-side change corrects all stored
  GRIB2/CF data immediately; nothing needs re-ingesting.
- **Transitional:** GOES data written before the deploy still has edge bboxes. Until it rolls off
  (3 h retention), its reads are off by 0 to 1 pixel instead of a constant half pixel.
- MRMS data stored with a truncated corner (at most 0.1 cell) rolls off within its 2 h (MRMS) or
  72 h (MRMS-QPE) retention.
- Cached EDR responses (location cache, Cloudflare) and Redis WMTS tiles show the old positions
  until they expire.
