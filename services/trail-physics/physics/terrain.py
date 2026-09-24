"""Terrain derivatives from a DEM: slope, D8 flow accumulation, topographic
wetness index (TWI).

Method: single-flow-direction D8 (Beven & Kirkby 1979 lineage; the classic
TOPMODEL choice, and the simplest defensible method for a ~4,000-cell
catchment like Tarrawarra). D-infinity or multiple-flow-direction methods
give smoother TWI fields on larger/gentler terrain but are not needed to
reproduce a fully-specified, small-catchment validation target -- D8 is a
standard, well-documented choice, not a shortcut invented for this project.

All functions operate on 2-D numpy arrays with north-up row-major layout
(row 0 = north edge, consistent with the DEM file convention documented in
the Tarrawarra Readme.topo: "elevations ... starting in the North western
corner of the dem and proceeding row by row").
"""

from __future__ import annotations

import warnings

import numpy as np

# D8 neighbor offsets (row_delta, col_delta) and their real-world distance
# multiplier relative to cellsize (1.0 for cardinal, sqrt(2) for diagonal).
_D8_OFFSETS = [
    (-1, 0, 1.0),  # N
    (-1, 1, 2**0.5),  # NE
    (0, 1, 1.0),  # E
    (1, 1, 2**0.5),  # SE
    (1, 0, 1.0),  # S
    (1, -1, 2**0.5),  # SW
    (0, -1, 1.0),  # W
    (-1, -1, 2**0.5),  # NW
]


def compute_slope(dem: np.ndarray, cellsize: float) -> np.ndarray:
    """Slope (tan(beta), dimensionless rise/run) via Horn's method (a 3x3
    finite-difference kernel; the standard method used by most GIS slope
    tools, e.g. ArcGIS/GDAL). Edge cells use a smaller effective kernel
    (numpy edge-replicate padding) rather than being dropped, since a small
    catchment like Tarrawarra can't afford to lose a border of cells.

    Returns tan(beta), not degrees or radians -- Eq. 1's TWI (lambda =
    ln(a / tan(beta))) wants the raw ratio.
    """
    z = np.pad(dem, 1, mode="edge")

    # Horn's method: weighted central difference over the 3x3 window.
    dz_dx = (
        (z[0:-2, 2:] + 2 * z[1:-1, 2:] + z[2:, 2:])
        - (z[0:-2, 0:-2] + 2 * z[1:-1, 0:-2] + z[2:, 0:-2])
    ) / (8 * cellsize)
    dz_dy = (
        (z[2:, 0:-2] + 2 * z[2:, 1:-1] + z[2:, 2:])
        - (z[0:-2, 0:-2] + 2 * z[0:-2, 1:-1] + z[0:-2, 2:])
    ) / (8 * cellsize)

    tan_beta = np.sqrt(dz_dx**2 + dz_dy**2)
    return tan_beta


def compute_aspect(dem: np.ndarray, cellsize: float) -> np.ndarray:
    """Aspect in degrees, 0=north, 90=east, clockwise (standard GIS
    convention) -- i.e. the compass bearing of the downhill (steepest
    descent) direction.

    Derivation, since this array's row index increases SOUTHWARD (row 0 =
    north edge, matching the DEM file convention documented in the
    Tarrawarra Readme.topo and this module's header): let Gx = dz/d(col)
    (eastward gradient) and Gr = dz/d(row) (southward gradient, i.e. the
    quantity computed below as `dz_dr`). The downhill direction vector is
    -gradient; converting its south-component to a north-component flips
    its sign twice (north_component = -south_component = -(-Gr) = Gr), so
    the downhill vector's (east, north) components are (-Gx, Gr). Compass
    bearing clockwise from north is then atan2(east, north) = atan2(-Gx,
    Gr). Verified against all four cardinal cases (a DEM tilted purely
    N/S/E/W each recovers exactly 0/180/90/270) -- see test_terrain.py.
    """
    z = np.pad(dem, 1, mode="edge")
    dz_dx = (
        (z[0:-2, 2:] + 2 * z[1:-1, 2:] + z[2:, 2:])
        - (z[0:-2, 0:-2] + 2 * z[1:-1, 0:-2] + z[2:, 0:-2])
    ) / (8 * cellsize)
    dz_dr = (
        (z[2:, 0:-2] + 2 * z[2:, 1:-1] + z[2:, 2:])
        - (z[0:-2, 0:-2] + 2 * z[0:-2, 1:-1] + z[0:-2, 2:])
    ) / (8 * cellsize)
    return np.degrees(np.arctan2(-dz_dx, dz_dr)) % 360.0


def compute_d8_flow_accumulation(dem: np.ndarray, cellsize: float) -> np.ndarray:
    """Specific catchment area `a` (m^2 upslope contributing area per unit
    contour width, approximated here as accumulated-cell-count * cellsize,
    the standard D8 approximation) via single-flow-direction D8 routing.

    Algorithm: process cells in descending elevation order (a topological
    sort valid for D8 on a DEM with no flat/pit cells -- see
    ``fill_pits_and_flats`` for why that precondition matters), routing each
    cell's accumulated flow (starting at 1 cell) to its single steepest
    downslope neighbor.
    """
    rows, cols = dem.shape
    flow_to = np.full((rows, cols, 2), -1, dtype=np.int32)

    padded = np.pad(dem, 1, mode="edge")
    for r in range(rows):
        for c in range(cols):
            z0 = dem[r, c]
            best_slope = 0.0
            best_target = None
            for dr, dc, dist in _D8_OFFSETS:
                pr, pc = r + 1 + dr, c + 1 + dc
                z1 = padded[pr, pc]
                drop = z0 - z1
                if drop <= 0:
                    continue
                s = drop / (dist * cellsize)
                if s > best_slope:
                    best_slope = s
                    best_target = (r + dr, c + dc)
            if best_target is not None:
                tr, tc = best_target
                if 0 <= tr < rows and 0 <= tc < cols:
                    flow_to[r, c] = (tr, tc)
                # else: flows off the DEM edge -- terminal, same as a sink

    # Process cells highest-to-lowest so a cell's accumulation is finalized
    # (all upslope contributors already added) before it passes flow onward.
    order = np.argsort(-dem.ravel())
    accumulation = np.ones((rows, cols), dtype=np.float64)  # each cell contributes itself
    flat_flow_to = flow_to.reshape(rows * cols, 2)
    flat_acc = accumulation.ravel()
    for idx in order:
        tr, tc = flat_flow_to[idx]
        if tr >= 0:
            flat_acc[tr * cols + tc] += flat_acc[idx]

    specific_area = accumulation * cellsize
    return specific_area


def fill_pits_and_flats(dem: np.ndarray, epsilon: float = 1e-4) -> np.ndarray:
    """Minimally raise interior pits/flats so every non-edge cell has at
    least one downslope neighbor -- a precondition for the D8 accumulation's
    descending-elevation topological sort to terminate correctly (a true
    pit has nowhere to route to, which would silently drop its upslope
    contribution). Iterative epsilon-fill (Planchon & Darboux 2001 lineage,
    simplified): repeatedly raise any interior cell that is <= all its
    neighbors, by epsilon above the lowest neighbor, until none remain.

    Small catchments like Tarrawarra converge in a handful of passes; this
    is not appropriate for a full Front-Range-scale DEM (use a real
    priority-flood implementation, e.g. WhiteboxTools, for WS1's statewide
    static stack -- see pipelines/static/derive_terrain.py).
    """
    filled = dem.copy().astype(np.float64)
    rows, cols = filled.shape
    for _ in range(10_000):
        padded = np.pad(filled, 1, mode="edge")
        neighbor_min = np.full_like(filled, np.inf)
        for dr, dc, _dist in _D8_OFFSETS:
            window = padded[1 + dr : 1 + dr + rows, 1 + dc : 1 + dc + cols]
            neighbor_min = np.minimum(neighbor_min, window)
        is_pit = filled <= neighbor_min
        # Never touch the DEM boundary -- those cells are legitimate outlets.
        is_pit[0, :] = is_pit[-1, :] = is_pit[:, 0] = is_pit[:, -1] = False
        if not is_pit.any():
            break
        filled[is_pit] = neighbor_min[is_pit] + epsilon
    return filled


def coarsen_dem(dem: np.ndarray, cellsize: float, factor: int) -> tuple[np.ndarray, float]:
    """Block-average elevation to a coarser grid, e.g. 5m -> 30m at
    factor=6. Recomputing slope/flow-accumulation/TWI on genuinely
    coarsened elevation (this function) is the physically correct way to
    test a resolution-scale hypothesis -- it is NOT the same as computing
    TWI at fine resolution and then averaging the resulting TWI values
    post-hoc, which would not reproduce the smoothing effect a coarser
    input DEM has on slope and specific catchment area.

    Session 6 motivation: GeoWATCH's paper (Section 2.4) states its global
    elevation composite is 30m resolution and "sets the finest scale at
    which downscaled soil moisture products can be computed" -- yet
    Session 4/5's Tarrawarra validation used the site's native 5m DEM
    throughout. If k=13 was calibrated against 30m-resolution TWI (coarser
    grids smooth slope and aggregate flow paths, generally SHRINKING TWI's
    spread), that is a plausible, testable explanation for the ~5-6x
    magnitude gap documented in validation/tarrawarra/README.md.

    NaN-aware (block mean ignores nodata cells within a block; a block
    that is entirely NaN stays NaN). If the grid dimensions aren't evenly
    divisible by `factor`, trailing rows/columns are dropped (cropped to
    the largest exact multiple) rather than padded -- simpler and honest
    about a small amount of boundary data loss, appropriate for this
    diagnostic use, not for a production resampling pipeline.

    Args:
        dem: elevation grid, NaN for nodata.
        cellsize: current cell size in meters (assumed square cells).
        factor: integer number of cells to average per side (e.g. 6 to go
            from 5m to 30m). Must be >= 1.

    Returns:
        (coarsened_elevation, new_cellsize)
    """
    if factor < 1:
        raise ValueError(f"factor must be >= 1, got {factor}")
    if factor == 1:
        return dem.copy(), float(cellsize)

    rows, cols = dem.shape
    cropped_rows = (rows // factor) * factor
    cropped_cols = (cols // factor) * factor
    cropped = dem[:cropped_rows, :cropped_cols]

    reshaped = cropped.reshape(cropped_rows // factor, factor, cropped_cols // factor, factor)
    with warnings.catch_warnings():
        # A block that is entirely NaN legitimately triggers "Mean of empty
        # slice" and correctly produces NaN -- expected, not an error.
        warnings.simplefilter("ignore", category=RuntimeWarning)
        coarsened = np.nanmean(reshaped, axis=(1, 3))
    return coarsened, float(cellsize) * factor


def compute_twi_pydem(
    dem: np.ndarray,
    cellsize: float,
    scaled: bool = False,
    apply_twi_limits: bool = False,
    uca_saturation_limit: float = 32.0,
) -> np.ndarray:
    """Topographic wetness index computed via pyDEM (Ueckermann et al. 2018,
    github.com/creare-com/pydem) -- the specific tool the GeoWATCH paper
    (Eylander et al. 2023, Section 2.2) states it used to compute TWI, as
    opposed to `compute_twi` above (this module's own from-scratch D8
    implementation, never independently verified against pyDEM's numbers).

    **Session 5 finding**: pyDEM uses D-infinity flow routing (Tarboton
    1997) rather than D8, and its formula differs slightly (`ln(uca / (mag
    + twi_min_slope))` with an additive slope floor, vs. this module's
    `ln(a / max(tan_beta, floor))` with a clamping floor). Cross-checked
    against the real Tarrawarra DEM: pyDEM's TWI has *similar* (not
    dramatically different) standard deviation to `compute_twi`'s own
    output (1.43 vs. 1.28), and per-date correlation with real observed
    soil moisture anomalies is modestly BETTER using pyDEM (see
    validation/tarrawarra/README.md) -- but this alone does *not* close the
    ~5x gap between the design doc's k=13 and the locally best-fit
    coefficient. Kept here because it's still the more faithful
    reproduction of the paper's own method, and because ruling out "which
    flow-routing algorithm" was a necessary step regardless of whether it
    alone explains the discrepancy.

    Args:
        dem: elevation grid, NaN for nodata (same convention as
            `compute_twi`).
        cellsize: grid cell size in meters (assumed square cells).
        scaled: if True, return pyDEM's own stored `self.twi` value, which
            is the natural-log TWI multiplied by 10 ("for better integer
            resolution when storing" -- pyDEM's own module docstring). If
            GeoWATCH's k=13 was calibrated against pyDEM's saved GeoTIFF
            rasters directly (as opposed to calling `calc_twi()` and using
            its return value), this is the version that would match. False
            (default) returns the plain, unscaled ln(uca/slope) value.
        apply_twi_limits: pyDEM option, off by default in pyDEM itself and
            in this function (matching pyDEM's own default). When True,
            caps TWI (and the underlying UCA) at `uca_saturation_limit *
            min_area`, compressing the upper tail of the distribution.
            **Session 6 finding**: on the real Tarrawarra DEM, this shrinks
            TWI's standard deviation by about 30% (1.43 -> 1.01) -- tested
            through the full Rung 1 harness via
            `run_validation.py --twi-engine pydem --twi-apply-limits`, see
            validation/tarrawarra/README.md for the result.
        uca_saturation_limit: only used when apply_twi_limits=True. pyDEM's
            own default (32, "units of area" per its own source) is kept
            as the default here too -- not tuned for this project.

    Raises:
        ImportError: if the `pydem` package isn't installed. This is a
            deliberately optional, lazily-imported dependency -- heavier
            (rasterio, Cython) than anything else in this package, and
            only needed for this specific cross-check / validation path,
            not for the live HRRR-forced pipeline.
    """
    try:
        from pydem.dem_processing import DEMProcessor
    except ImportError as e:
        raise ImportError(
            "pyDEM is required for compute_twi_pydem but is not installed. "
            "Install with `pip install pydem` (requires a working C/Cython "
            "build toolchain for its upstream-contributing-area extension)."
        ) from e

    dp = DEMProcessor(
        elev=dem.copy(),
        dX=float(cellsize),
        dY=float(cellsize),
        apply_twi_limits=apply_twi_limits,
        apply_twi_limits_on_uca=apply_twi_limits,
        uca_saturation_limit=uca_saturation_limit,
    )
    dp.calc_slopes_directions()
    dp.calc_uca()
    unscaled = dp.calc_twi()
    return dp.twi.copy() if scaled else unscaled.copy()


def compute_twi(dem: np.ndarray, cellsize: float) -> np.ndarray:
    """Topographic wetness index: lambda = ln(a / tan(beta)).

    `a` is specific catchment area (compute_d8_flow_accumulation), tan(beta)
    is local slope (compute_slope). A small tan(beta) floor prevents
    division blow-up on genuinely flat cells (a standard TWI implementation
    detail, not a physics assumption).

    **Known gap (Session 4, real-data Rung 1 run)**: this D8-based
    implementation is structurally correct -- against real Tarrawarra data,
    its per-date correlation with observed moisture anomalies is strongly
    positive (0.07-0.62), and weakest on exactly the driest dates, matching
    the GeoWATCH paper's own described physical behavior -- but it is NOT
    numerically compatible with GeoWATCH's own TWI computation. The paper
    (Eylander et al. 2023, Section 2.2) computed TWI using pyDEM
    (Ueckermann et al. 2018, github.com/creare-com/pydem), a specific tool
    this implementation was never cross-checked against. A least-squares
    fit against real Tarrawarra data implies this function's TWI deviations
    are roughly 5-6x larger than whatever k=13 (redistribution.py) was
    actually calibrated against -- see validation/tarrawarra/README.md for
    the full diagnosis. Do not "fix" this by tuning k; the concrete next
    step is adopting pyDEM itself.
    """
    filled = fill_pits_and_flats(dem)
    a = compute_d8_flow_accumulation(filled, cellsize)
    tan_beta = compute_slope(dem, cellsize)
    tan_beta_floored = np.maximum(tan_beta, 0.001)
    return np.log(a / tan_beta_floored)
