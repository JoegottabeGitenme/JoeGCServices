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

**Session 14: static-stack sampling split from combination, and batched
across an entire forecast hour, not called per segment.** Live profiling
of the first production backlog (Session 13) found the real bottleneck:
each per-segment call to the old combined `downscale_soil_moisture` did 3
independent `static_stack.sample_layer` windowed S3 reads (twi, theta_s,
theta_wilt), each measured at ~11ms live against production MinIO. At
9,029 segments/forecast-hour that's ~27,000 reads x ~11ms =~ 5 minutes --
*entirely* request-count-bound (the actual HRRR forcing-grid read this
was originally suspected to be the culprit for takes ~25ms total,
confirmed live; it was never the bottleneck). Slower than HRRR's own
~24 forecast-hours/hour ingest rate, so the backlog could mathematically
never clear. `sample_static_inputs` now does exactly 3 reads *per
forecast hour* (covering every point from every processed segment in one
batch each), and `downscale_soil_moisture` is now pure combination logic
over already-sampled arrays -- no I/O, callable per-segment (by slicing
the batch) at effectively zero marginal cost.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from physics.redistribution import redistribute_podpac
from static_stack import StaticStack


@dataclass
class StaticSamples:
    """Pre-sampled static-stack values for a batch of points -- the output
    of the one real I/O pass (`sample_static_inputs`), sliced per-segment
    afterward with `.slice()` at zero additional I/O cost."""

    twi: np.ndarray
    theta_s: np.ndarray
    theta_wilt: np.ndarray
    twi_bar: np.ndarray

    def __len__(self) -> int:
        return len(self.twi)

    def slice(self, start: int, end: int) -> "StaticSamples":
        return StaticSamples(
            twi=self.twi[start:end],
            theta_s=self.theta_s[start:end],
            theta_wilt=self.theta_wilt[start:end],
            twi_bar=self.twi_bar[start:end],
        )


@dataclass
class DownscaleResult:
    predicted: np.ndarray  # same shape as the input soilw_samples
    # Fraction of points with a valid HRRR reading that ALSO fell inside
    # the static stack's real coverage (i.e. got genuine terrain
    # downscaling, not just the raw HRRR fallback). None if there were no
    # valid HRRR readings at all (matches aggregate.py's own "no data"
    # convention -- None, not 0.0 or NaN).
    confidence: float | None
    # Degree of saturation per point: downscaled soil moisture / theta_s
    # (the fraction of the soil's pore space holding water), clipped to
    # [0, 1]. NaN wherever the point didn't get real terrain/soil data --
    # there is no theta_s outside the static stack's coverage, and
    # inventing one (e.g. a generic porosity) would be a fabricated
    # number presented as a measurement. Same convention as `confidence`.
    saturation: np.ndarray


def sample_static_inputs(
    static_stack: StaticStack,
    lats: np.ndarray,
    lons: np.ndarray,
    hrrr_rows: np.ndarray,
    hrrr_cols: np.ndarray,
) -> StaticSamples:
    """The real I/O: exactly 3 (banded) static-stack layer reads (twi,
    theta_s, theta_wilt) covering ALL given points, plus the in-memory
    lambda_bar lookup.

    **Time-invariant -- call once per cycle, not once per forecast hour**
    (Session 14). The static stack and the trail geometry don't change
    between forecast hours, so these values don't either; `main.py`
    computes them once per cycle in `build_segment_batch` and reuses them
    for every pending hour. (Session 14 first fixed the per-SEGMENT call
    pattern -- thousands of tiny reads -- then, for the 20x-larger Front
    Range stack, the per-HOUR repetition of this whole pass.)

    `hrrr_rows`/`hrrr_cols` are the points' fractional HRRR grid indices
    (j and i from `HrrrGrid.geo_to_grid_array`), which `main.py` has
    already computed for forcing sampling -- passed in, not recomputed."""
    rows, cols = static_stack.lonlat_to_rowcol_array(lons, lats)
    twi = static_stack.sample_layer_array("twi", rows, cols)
    theta_s = static_stack.sample_layer_array("theta_s", rows, cols)
    theta_wilt = static_stack.sample_layer_array("theta_wilt", rows, cols)
    twi_bar = static_stack.hrrr_twi_bar_array(hrrr_rows, hrrr_cols)
    return StaticSamples(twi=twi, theta_s=theta_s, theta_wilt=theta_wilt, twi_bar=twi_bar)


def downscale_soil_moisture(
    static_samples: StaticSamples | None,
    soilw_samples: np.ndarray,
) -> DownscaleResult:
    """Pure combination logic, no I/O -- combines a batch's own HRRR
    readings with its ALREADY-sampled static values (see
    `sample_static_inputs`). `static_samples` may be None (stack not
    configured/reachable, or this segment had no points in the sampled
    batch) -- in which case every point falls back to its own raw HRRR
    value, confidence 0.0 wherever a valid HRRR reading exists at all."""
    predicted = np.array(soilw_samples, dtype=float, copy=True)
    valid_soilw = ~np.isnan(soilw_samples)
    n_valid = int(valid_soilw.sum())

    no_saturation = np.full(predicted.shape, np.nan)
    if static_samples is None or n_valid == 0:
        confidence = 0.0 if n_valid > 0 else None
        return DownscaleResult(predicted=predicted, confidence=confidence, saturation=no_saturation)

    covered = (
        valid_soilw
        & ~np.isnan(static_samples.twi)
        & ~np.isnan(static_samples.theta_s)
        & ~np.isnan(static_samples.theta_wilt)
        & ~np.isnan(static_samples.twi_bar)
    )

    if covered.any():
        downscaled = redistribute_podpac(
            theta_coarse=soilw_samples[covered],
            twi=static_samples.twi[covered],
            theta_s=static_samples.theta_s[covered],
            theta_wilt=static_samples.theta_wilt[covered],
            twi_mean=static_samples.twi_bar[covered],
        )
        predicted[covered] = downscaled

    # Clipped: the redistribution equation can push a point slightly above
    # theta_s (or below 0) at extreme TWI; a consumer needs a bounded
    # 0-1 quantity, and "fully saturated" is the honest reading of an
    # overshoot.
    saturation = no_saturation
    if covered.any():
        saturation = no_saturation.copy()
        saturation[covered] = np.clip(predicted[covered] / static_samples.theta_s[covered], 0.0, 1.0)

    confidence = float(covered.sum()) / n_valid
    return DownscaleResult(predicted=predicted, confidence=confidence, saturation=saturation)
