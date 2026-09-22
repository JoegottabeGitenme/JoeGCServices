"""Unit tests for aggregate.py (S8 vertex-sampling aggregation)."""

from datetime import datetime, timezone

import numpy as np
import pytest

from aggregate import aggregate_frozen_fraction, aggregate_mean_ignoring_nan, build_segment_condition_row


def test_aggregate_mean_basic():
    assert aggregate_mean_ignoring_nan(np.array([0.1, 0.2, 0.3])) == pytest.approx(0.2)


def test_aggregate_mean_ignores_nan():
    result = aggregate_mean_ignoring_nan(np.array([0.1, np.nan, 0.3]))
    assert result == pytest.approx(0.2)


def test_aggregate_mean_all_nan_returns_none():
    result = aggregate_mean_ignoring_nan(np.array([np.nan, np.nan]))
    assert result is None


def test_aggregate_frozen_fraction_partial():
    """The design doc's explicit requirement: partial-frozen segments must
    be representable, not collapsed to a single boolean."""
    flags = np.array([True, True, False, False])
    assert aggregate_frozen_fraction(flags) == pytest.approx(0.5)


def test_aggregate_frozen_fraction_all_frozen():
    assert aggregate_frozen_fraction(np.array([True, True])) == pytest.approx(1.0)


def test_aggregate_frozen_fraction_empty_returns_none():
    assert aggregate_frozen_fraction(np.array([])) is None


def test_build_segment_condition_row_shape():
    row = build_segment_condition_row(
        feature_id=12345,
        run_time=datetime(2026, 9, 22, 18, tzinfo=timezone.utc),
        valid_time=datetime(2026, 9, 22, 21, tzinfo=timezone.utc),
        forecast_hour=3,
        soil_moisture_samples=np.array([0.15, 0.16, 0.14]),
        frozen_flags=np.array([False, False, True]),
        swe_samples=np.array([0.0, 0.0, 0.0]),
    )
    assert row["feature_id"] == 12345
    assert row["forecast_hour"] == 3
    assert row["soil_moisture"] == pytest.approx(0.15, abs=0.01)
    assert row["frozen_fraction"] == pytest.approx(1 / 3, abs=0.01)
    assert row["swe_mm"] == pytest.approx(0.0)
    assert row["softness_index"] is None
    assert row["confidence"] is None


def test_build_segment_condition_row_optional_fields_default_none():
    row = build_segment_condition_row(
        feature_id=1,
        run_time=datetime(2026, 9, 22, tzinfo=timezone.utc),
        valid_time=datetime(2026, 9, 22, tzinfo=timezone.utc),
        forecast_hour=0,
        soil_moisture_samples=np.array([0.2]),
    )
    assert row["frozen_fraction"] is None
    assert row["swe_mm"] is None
