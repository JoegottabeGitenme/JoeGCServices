"""Per-vertex soil moisture downscaling: combines a point's own
bilinearly-sampled HRRR SOILW reading with the WS1 static terrain/soil
stack (`static_stack.py`), via the real Creare/GeoWATCH production
equation (`physics.redistribution.redistribute_podpac`, Session 8's
discovery) for points the stack covers -- or falls back to the raw HRRR
value, with correspondingly reduced confidence, for points it doesn't
(e.g. most of Colorado's trail network, before WS1 covers more than the
Boulder-area pilot; see `pipelines/static/README.md`).

**Per-point, not per-segment, coarse value and lambda_bar** -- a
deliberate design choice, not an oversight: the validation sessions
(Tarrawarra, NMM, Shale Hills) all used a single shared coarse value and a
single shared TWI mean for an entire site, because every one of those
sites is far smaller than a single HRRR cell (a 10.8ha catchment inside a
9 sq km grid cell). At Colorado scale a single trail segment can easily
span multiple HRRR cells, so each point uses ITS OWN bilinearly-sampled
HRRR reading as `theta_coarse` and ITS OWN covering HRRR cell's
`lambda_bar` -- the equation's literal per-pixel semantics
(`theta(x,y) = theta_coarse(x,y) + amplitude(x,y) * (twi(x,y) -
twi_bar(hrrr_cell_of(x,y)))`), not an approximation introduced for this
service. `physics.redistribution.redistribute_podpac` already supports
this without modification -- its arithmetic broadcasts arrays for
`theta_coarse`/`twi_mean` exactly the same way it broadcasts scalars
(confirmed by reading its implementation before relying on this, not
assumed).
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from hrrr_grid import HrrrGrid
from physics.redistribution import redistribute_podpac
from static_stack import StaticStack


@dataclass
class DownscaleResult:
    predicted: np.ndarray  # same shape as the input soilw_samples
    # Fraction of points with a valid HRRR reading that ALSO fell inside
    # the static stack's real coverage (i.e. got genuine terrain
    # downscaling, not just the raw HRRR fallback). None if there were no
    # valid HRRR readings at all (matches aggregate.py's own "no data"
    # convention -- None, not 0.0 or NaN).
    confidence: float | None


def downscale_soil_moisture(
    static_stack: StaticStack | None,
    hrrr_grid: HrrrGrid,
    points_lat_lon: list[tuple[float, float]],
    soilw_samples: np.ndarray,
) -> DownscaleResult:
    """Downscale one feature's per-vertex HRRR SOILW samples. `static_stack`
    may be None (stack not configured/reachable) -- in which case every
    point falls back to its own raw HRRR value, confidence 0.0 wherever a
    valid HRRR reading exists at all."""
    predicted = np.array(soilw_samples, dtype=float, copy=True)
    valid_soilw = ~np.isnan(soilw_samples)
    n_valid = int(valid_soilw.sum())

    if static_stack is None or n_valid == 0:
        confidence = 0.0 if n_valid > 0 else None
        return DownscaleResult(predicted=predicted, confidence=confidence)

    rows_cols = [static_stack.lonlat_to_rowcol(lon, lat) for lat, lon in points_lat_lon]
    twi = static_stack.sample_layer("twi", rows_cols)
    theta_s = static_stack.sample_layer("theta_s", rows_cols)
    theta_wilt = static_stack.sample_layer("theta_wilt", rows_cols)

    # HrrrGrid.geo_to_grid returns (i, j); row=j, col=i -- the same
    # convention forcing.sample_points already established (no flip
    # needed, south-origin grid).
    hrrr_ij = [hrrr_grid.geo_to_grid(lat, lon) for lat, lon in points_lat_lon]
    twi_bar = np.array([static_stack.hrrr_twi_bar(hrrr_row=j, hrrr_col=i) for i, j in hrrr_ij])

    covered = valid_soilw & ~np.isnan(twi) & ~np.isnan(theta_s) & ~np.isnan(theta_wilt) & ~np.isnan(twi_bar)

    if covered.any():
        downscaled = redistribute_podpac(
            theta_coarse=soilw_samples[covered],
            twi=twi[covered],
            theta_s=theta_s[covered],
            theta_wilt=theta_wilt[covered],
            twi_mean=twi_bar[covered],
        )
        predicted[covered] = downscaled

    confidence = float(covered.sum()) / n_valid
    return DownscaleResult(predicted=predicted, confidence=confidence)
