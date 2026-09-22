"""Reads HRRR forcing grids (Zarr v3, written by the Rust ingester --
crates/grid-processor/src/writer/zarr_writer.rs) and bilinearly samples them
at arbitrary (lat, lon) points -- the corridor cell centers, once WS2's
corridor mask exists.

Storage-layer facts this module depends on (verified against the actual
Rust writer during the trail-conditions ingredients session, not assumed):
- Zarr v3, one group per parameter/level/forecast-hour, pyramid level "0"
  is native resolution (`write_multiscale`, zarr_writer.rs).
- Array shape is **[height, width]** = **[ny, nx]** (row-major; confirmed
  from `ArrayBuilder::new(vec![height as u64, width as u64], ...)`,
  zarr_writer.rs `write_level_sharded`), float32, NaN fill value.
- For Lambert Conformal grids (HRRR), `row_origin: "south"` means **row 0 is
  the grid's southernmost row** -- confirmed from
  crates/grid-processor/src/processor/zarr.rs's own doc comment
  ("RowOrigin::South: Row 0 is at min_lat (bottom) - Lambert Conformal
  grids"). This means `hrrr_grid.HrrrGrid.geo_to_grid()`'s `j` index maps
  **directly** to the array's row index with no flip -- j=0 is also the
  grid's first (southernmost) point by construction. Confirm this remains
  true if this module is ever pointed at a non-Lambert grid.
- Object key convention: `grids/{model}/{YYYYMMDD_HHz}/{param}_{level}_f{FFF}.zarr`
  (`build_storage_path`, crates/ingestion/src/grib2.rs).

**Not tested against live production MinIO data this session** -- the S3
API port (9000) isn't publicly exposed (only the admin console on 9001 is
proxied through the gateway), so there was no reachable endpoint to test
against from this environment. Tested instead against a local Zarr v3 array
built to the same shape/dtype/fill-value convention (see
tests/test_forcing.py), which proves the bilinear-sampling logic
correctly, just not a live end-to-end read. s3fs + zarr-python 3 is a
well-established, standard combination for this (not a novel integration
risk) -- treat "does it actually reach MinIO" as a one-time smoke test to
run once this service is deployed, not an open design question.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np
import zarr

from hrrr_grid import HrrrGrid


def storage_path(model: str, reference_time_str: str, param: str, level: str, forecast_hour: int) -> str:
    """Mirrors `build_storage_path` (crates/ingestion/src/grib2.rs):
    grids/{model}/{YYYYMMDD_HHz}/{param}_{level}_f{FFF}.zarr

    `reference_time_str` must already be in `YYYYMMDD_HHz` form (e.g.
    "20260922_18z") -- this module doesn't own datetime formatting, the
    catalog-polling caller does.
    """
    return f"grids/{model}/{reference_time_str}/{param}_{level}_f{forecast_hour:03d}.zarr"


@dataclass
class BilinearSample:
    value: float
    fraction_nan_neighbors: float  # 0.0 = all 4 neighbors valid, 1.0 = all NaN


def bilinear_sample(array: np.ndarray, row: float, col: float) -> BilinearSample:
    """Bilinear interpolation at fractional (row, col) into a 2-D array.

    NaN-aware: if some (not all) of the 4 surrounding cells are NaN
    (outside the grid's valid data mask, e.g. a corridor cell right at the
    grid edge), renormalizes weights over only the valid neighbors rather
    than propagating NaN. If all 4 are NaN, returns NaN with
    fraction_nan_neighbors=1.0 so the caller can flag/skip that point.
    """
    ny, nx = array.shape
    row = np.clip(row, 0.0, ny - 1.0001)
    col = np.clip(col, 0.0, nx - 1.0001)

    r0, c0 = int(math.floor(row)), int(math.floor(col))
    r1, c1 = r0 + 1, c0 + 1
    fr, fc = row - r0, col - c0

    corners = [
        (r0, c0, (1 - fr) * (1 - fc)),
        (r0, c1, (1 - fr) * fc),
        (r1, c0, fr * (1 - fc)),
        (r1, c1, fr * fc),
    ]

    total_weight = 0.0
    weighted_sum = 0.0
    nan_count = 0
    for r, c, w in corners:
        v = array[r, c]
        if np.isnan(v):
            nan_count += 1
            continue
        weighted_sum += w * v
        total_weight += w

    if total_weight <= 0.0:
        return BilinearSample(value=float("nan"), fraction_nan_neighbors=1.0)

    return BilinearSample(
        value=weighted_sum / total_weight,
        fraction_nan_neighbors=nan_count / 4.0,
    )


def open_level0_array(store, group_path: str) -> np.ndarray:
    """Open a Zarr v3 group written by write_multiscale and return pyramid
    level 0 (native resolution) as a plain in-memory numpy array.

    `store` is whatever zarr.open accepts: a local path string, an
    `fsspec`/`s3fs` mapper for `s3://bucket/...`, or an already-constructed
    zarr store object -- kept generic so tests can pass a local directory
    and production can pass an s3fs-backed store without this function
    changing.
    """
    group = zarr.open_group(store=store, path=group_path, mode="r")
    return np.asarray(group["0"][:])


def sample_points(
    array: np.ndarray, hrrr_grid: HrrrGrid, points_lat_lon: list[tuple[float, float]]
) -> list[BilinearSample]:
    """Sample a level-0 HRRR grid at a batch of (lat, lon) points."""
    results = []
    for lat, lon in points_lat_lon:
        i, j = hrrr_grid.geo_to_grid(lat, lon)
        # row = j (south-origin, confirmed no flip needed -- see module docstring),
        # col = i.
        results.append(bilinear_sample(array, row=j, col=i))
    return results
