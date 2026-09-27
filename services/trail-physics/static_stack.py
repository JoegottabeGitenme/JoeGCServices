"""Reads the WS1 static terrain/soil stack (Zarr v3, assembled by
`pipelines/static/assemble_static_stack.py`) and samples it at arbitrary
(lat, lon) points -- the real per-vertex TWI/theta_s/theta_wilt inputs
Session 8's corrected Creare/GeoWATCH equation
(`physics.redistribution.redistribute_podpac`) needs, replacing
`main.py`'s previous uniform-zero placeholder (see that module's git
history for the NOTE this supersedes).

**Grid spec is read from the Zarr group's own attrs, never re-derived or
assumed** -- exactly the discipline `pipelines/static/grid_spec.py`'s own
docstring calls for ("a future consumer reads the grid definition from
the data itself"). This module is that consumer.

**Lazy, windowed reads, not whole-array loads** -- deliberately different
from `forcing.open_level0_array`'s "load the whole level-0 array into
memory" approach, which is fine for HRRR (~1900x1059 cells) but would be
wasteful (and eventually, at a full-Colorado-scale stack, infeasible) for
a 10m static layer. `sample_layer` reads only the bounding-box window
covering the batch of points being sampled.

**Coverage is a fact, not an assumption**: the pilot stack covers a small
Boulder-area region (see `pipelines/static/README.md`) -- most of
Colorado's trail network falls entirely outside it for now. Points
outside the stack's grid extent, or landing on a real nodata cell within
it (both real, expected situations, not error conditions), simply
sample as NaN -- the caller (`downscale.py`) is responsible for falling
back to the raw HRRR value and reporting reduced confidence, not this
module silently fabricating a value.
"""

from __future__ import annotations

import numpy as np
import zarr
from pyproj import Transformer

from forcing import bilinear_sample


class StaticStack:
    def __init__(self, store):
        self.root = zarr.open_group(store=store, mode="r")
        grid_spec = self.root.attrs["grid_spec"]
        self.crs: str = grid_spec["crs"]
        self.resolution_m: float = grid_spec["resolution_m"]
        self.xmin: float = grid_spec["xmin"]
        self.ymin: float = grid_spec["ymin"]
        self.xmax: float = grid_spec["xmax"]
        self.ymax: float = grid_spec["ymax"]
        self.width: int = grid_spec["width"]
        self.height: int = grid_spec["height"]
        self._to_grid_crs = Transformer.from_crs("EPSG:4326", self.crs, always_xy=True)
        self._to_wgs84 = Transformer.from_crs(self.crs, "EPSG:4326", always_xy=True)

        # The coarse HRRR-cell lambda_bar lookup is tiny (186 entries for
        # the pilot -- one per HRRR cell the stack overlaps, not a full
        # 1799x1059 HRRR-grid-shaped array) -- loading it fully into memory
        # as a plain dict is the right call at any realistic stack size,
        # unlike the 2-D layers above.
        rows = np.asarray(self.root["hrrr_twi_bar_hrrr_row"][:])
        cols = np.asarray(self.root["hrrr_twi_bar_hrrr_col"][:])
        values = np.asarray(self.root["hrrr_twi_bar_twi_bar"][:])
        self._twi_bar_lookup: dict[tuple[int, int], float] = {
            (int(r), int(c)): float(v) for r, c, v in zip(rows, cols, values)
        }

    def lonlat_to_rowcol(self, lon: float, lat: float) -> tuple[float, float]:
        """WGS84 (lon, lat) -> fractional (row, col) in this stack's own
        grid. Row 0 = north edge (ymax), matching every other grid
        convention in this codebase (Tarrawarra/Shale Hills DEMs,
        pipelines/static/grid_spec.py's own GridSpec.transform)."""
        x, y = self._to_grid_crs.transform(lon, lat)
        col = (x - self.xmin) / self.resolution_m
        row = (self.ymax - y) / self.resolution_m
        return row, col

    def in_bounds(self, row: float, col: float) -> bool:
        return 0.0 <= row < self.height and 0.0 <= col < self.width

    def wgs84_bbox(self) -> tuple[float, float, float, float]:
        """This stack's own extent reprojected to WGS84 (min_lon, min_lat,
        max_lon, max_lat) -- used to pre-filter which trail segments are
        even worth processing (Session 13: without this, main.py would
        attempt every active trail statewide for every forecast hour, far
        more work than can complete within one poll interval; see
        db.py::get_active_feature_ids_in_bbox). An approximate rectangle,
        not the stack's exact (possibly non-rectangular, real-nodata-
        pocketed) coverage -- deliberately a cheap pre-filter, not the
        final word on coverage; `sample_layer`'s own per-point NaN handling
        remains the authority on whether a specific point is truly
        covered."""
        corners_x = [self.xmin, self.xmax, self.xmin, self.xmax]
        corners_y = [self.ymin, self.ymin, self.ymax, self.ymax]
        lons, lats = self._to_wgs84.transform(corners_x, corners_y)
        return min(lons), min(lats), max(lons), max(lats)

    def sample_layer(self, layer_name: str, points_row_col: list[tuple[float, float]]) -> np.ndarray:
        """Bilinear-sample one named 2-D layer at a batch of fractional
        (row, col) static-grid coordinates. Points outside the grid
        entirely are NaN in the output without being read at all (a
        single windowed zarr read covers the batch's bounding box, clipped
        to the grid's own extent -- never a full-array load)."""
        n = len(points_row_col)
        results = np.full(n, np.nan)
        if n == 0:
            return results

        rows = np.array([p[0] for p in points_row_col])
        cols = np.array([p[1] for p in points_row_col])
        in_bounds_mask = (rows >= 0.0) & (rows < self.height) & (cols >= 0.0) & (cols < self.width)
        if not in_bounds_mask.any():
            return results

        r0 = max(0, int(np.floor(rows[in_bounds_mask].min())) - 1)
        r1 = min(self.height, int(np.ceil(rows[in_bounds_mask].max())) + 2)
        c0 = max(0, int(np.floor(cols[in_bounds_mask].min())) - 1)
        c1 = min(self.width, int(np.ceil(cols[in_bounds_mask].max())) + 2)

        window = np.asarray(self.root[layer_name][r0:r1, c0:c1])

        for idx in range(n):
            if not in_bounds_mask[idx]:
                continue
            sample = bilinear_sample(window, row=rows[idx] - r0, col=cols[idx] - c0)
            results[idx] = sample.value
        return results

    def hrrr_twi_bar(self, hrrr_row: float, hrrr_col: float) -> float:
        """The real production equation's lambda_bar for the HRRR cell
        containing (hrrr_row, hrrr_col) (fractional HRRR grid indices,
        e.g. from `hrrr_grid.HrrrGrid.geo_to_grid`) -- NaN if this stack
        doesn't cover that HRRR cell at all (a real, expected situation
        for any point outside the pilot region, not an error)."""
        key = (int(round(hrrr_row)), int(round(hrrr_col)))
        return self._twi_bar_lookup.get(key, float("nan"))
