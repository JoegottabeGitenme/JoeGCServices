"""S8 -- aggregate per-point physics samples along a trail segment into one
condition row per (feature_id, valid_time).

**Scope note**: the design doc's S8 is zonal statistics over a buffered
(~15m) corridor mask at 10m resolution -- that needs WS1 (static stack) and
WS2 (corridor mask), neither built yet. This module implements a
vertex-sampling approximation instead: aggregate whatever per-vertex
samples the caller already took (e.g. via forcing.sample_points at each of
a way's own OSM vertices, the same pattern documented for app-side use in
docs/trail-conditions-frontend.md) into one row. This is a legitimate,
useful v1 -- and specifically an *upgrade path*, not a dead end: the
`segment_conditions` schema doesn't care whether the samples behind a row
came from 5 vertices or 500 corridor cells, so swapping in true zonal
stats later requires no migration, just a better sample-collection step
feeding the same aggregation functions.
"""

from __future__ import annotations

from datetime import datetime

import numpy as np


def aggregate_mean_ignoring_nan(samples: np.ndarray) -> float | None:
    """Mean of per-point samples, ignoring NaN (e.g. points that fell
    outside valid grid coverage). Returns None (not NaN) if every sample
    was NaN -- a clean "no data" signal for the caller/DB column."""
    valid = samples[~np.isnan(samples)]
    if len(valid) == 0:
        return None
    return float(np.mean(valid))


def aggregate_frozen_fraction(frozen_flags: np.ndarray) -> float | None:
    """Fraction of sampled points currently on frozen ground (S4's
    TSOIL<=273.15K proxy). This is deliberately NOT collapsed into a single
    boolean -- the design doc explicitly warns against merging distinct
    states: 'frozen-firm and good are distinct states with the same
    verdict but different stability -- one becomes soft-stay-off by
    afternoon. Do not collapse them.' A fraction lets a downstream classifier
    (S7, not built yet) represent a *partially* frozen segment, matching
    the same "partial trail" philosophy as the `active` fraction concept
    for feature availability."""
    if len(frozen_flags) == 0:
        return None
    return float(np.mean(frozen_flags.astype(np.float64)))


def build_segment_condition_row(
    feature_id: int,
    run_time: datetime,
    valid_time: datetime,
    forecast_hour: int,
    soil_moisture_samples: np.ndarray,
    frozen_flags: np.ndarray | None = None,
    swe_samples: np.ndarray | None = None,
    confidence: float | None = None,
    model_version: str | None = None,
    saturation_samples: np.ndarray | None = None,
) -> dict:
    """Build one row dict ready for db.upsert_segment_conditions.

    `confidence` (Session 12): the fraction of this segment's sampled
    vertices that received genuine WS1 terrain/soil downscaling, as
    opposed to falling back to the raw (undownscaled) HRRR value because
    they fell outside the static stack's current coverage (see
    `downscale.py::DownscaleResult.confidence`) -- 1.0 means every vertex
    was topographically downscaled, 0.0 means none were (raw HRRR
    everywhere), None means there was no valid HRRR reading at all for
    this segment/hour. This is NOT the S1 triple-collocation confidence
    the schema comment in catalog.rs originally envisioned (that remains
    unimplemented) -- reusing the same nullable column for a real,
    simpler, and immediately useful notion of confidence rather than
    leaving it unpopulated until triple-collocation exists.

    `saturation_samples` (Session 14): per-vertex degree of saturation
    (soil_moisture / theta_s, see `downscale.DownscaleResult.saturation`);
    aggregated as a NaN-ignoring mean, None when no vertex had real
    terrain/soil data (i.e. outside static-stack coverage).

    `softness_index` is left as None (S6, blocked on restricted Army FASST
    coefficients -- see design doc).
    """
    return {
        "feature_id": feature_id,
        "run_time": run_time,
        "valid_time": valid_time,
        "forecast_hour": forecast_hour,
        "soil_moisture": aggregate_mean_ignoring_nan(soil_moisture_samples),
        "saturation": (
            aggregate_mean_ignoring_nan(saturation_samples) if saturation_samples is not None else None
        ),
        "frozen_fraction": (
            aggregate_frozen_fraction(frozen_flags) if frozen_flags is not None else None
        ),
        "frost_depth_m": None,  # not implemented this session
        "swe_mm": (
            aggregate_mean_ignoring_nan(swe_samples) if swe_samples is not None else None
        ),
        "softness_index": None,  # S6, blocked on c1/c2 (see design doc)
        "confidence": confidence,
        # Only included if the caller passed one -- db.upsert_segment_conditions
        # falls back to its own default ("trail-physics-v0") if this key is
        # absent from the row dict; main.py always passes config.model_version
        # explicitly (Session 12) so that fallback is a last-resort safety
        # net, not the actual mechanism in normal operation.
        **({"model_version": model_version} if model_version is not None else {}),
    }
