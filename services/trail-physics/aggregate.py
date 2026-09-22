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
) -> dict:
    """Build one row dict ready for db.upsert_segment_conditions.

    `softness_index` and `confidence` are left as None (schema columns
    exist for S6/S1-triple-collocation, neither implemented this session --
    see SEGMENT_CONDITIONS_SCHEMA_SQL's doc comment in catalog.rs).
    """
    return {
        "feature_id": feature_id,
        "run_time": run_time,
        "valid_time": valid_time,
        "forecast_hour": forecast_hour,
        "soil_moisture": aggregate_mean_ignoring_nan(soil_moisture_samples),
        "frozen_fraction": (
            aggregate_frozen_fraction(frozen_flags) if frozen_flags is not None else None
        ),
        "frost_depth_m": None,  # not implemented this session
        "swe_mm": (
            aggregate_mean_ignoring_nan(swe_samples) if swe_samples is not None else None
        ),
        "softness_index": None,  # S6, blocked on c1/c2 (see design doc)
        "confidence": None,  # S1 triple-collocation, not implemented this session
    }
