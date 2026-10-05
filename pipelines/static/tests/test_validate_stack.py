"""Tests for validate_stack.py -- the stack sanity/cross-check tooling used to
judge the Front Range build against the already-live pilot."""

import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

from validate_stack import LAYERS, compare_samples, compare_stacks, sanity_report

PILOT = Path(__file__).parent.parent / "data" / "static" / "colorado-10m-pilot.zarr"


class TestCompareSamples:
    def test_identical_arrays_agree_perfectly(self):
        a = np.array([1.0, 2.0, 3.0, 4.0])
        r = compare_samples(a, a.copy())
        assert r["pearson_r"] == pytest.approx(1.0)
        assert r["mean_abs_diff"] == 0.0 and r["p95_abs_diff"] == 0.0
        assert r["n"] == 4

    def test_perfectly_anticorrelated(self):
        a = np.array([1.0, 2.0, 3.0, 4.0])
        assert compare_samples(a, -a)["pearson_r"] == pytest.approx(-1.0)

    def test_nan_entries_are_excluded_from_both_sides(self):
        a = np.array([1.0, np.nan, 3.0, 4.0])
        b = np.array([1.0, 2.0, np.nan, 4.0])
        r = compare_samples(a, b)
        assert r["n"] == 2  # only indices 0 and 3 are valid in both
        assert r["mean_abs_diff"] == 0.0

    def test_constant_input_gives_no_correlation_not_a_nan_or_crash(self):
        r = compare_samples(np.array([5.0, 5.0, 5.0]), np.array([1.0, 2.0, 3.0]))
        assert r["pearson_r"] is None
        assert r["n"] == 3

    def test_too_few_overlapping_points_returns_none_stats(self):
        r = compare_samples(np.array([1.0, np.nan]), np.array([np.nan, 2.0]))
        assert r["n"] == 0 and r["pearson_r"] is None

    def test_reports_a_systematic_offset(self):
        a = np.linspace(0, 10, 50)
        r = compare_samples(a, a + 0.5)
        assert r["pearson_r"] == pytest.approx(1.0)  # correlation ignores a constant offset...
        assert r["mean_abs_diff"] == pytest.approx(0.5)  # ...but the difference stats must not


@pytest.mark.skipif(not PILOT.exists(), reason="real pilot stack not built locally (see README.md)")
class TestAgainstTheRealPilot:
    @pytest.fixture(scope="class")
    def pilot(self):
        from static_stack import StaticStack

        return StaticStack(str(PILOT))

    def test_pilot_passes_its_own_sanity_report(self, pilot):
        report = sanity_report(pilot)
        assert set(report) == set(LAYERS)
        for name, st in report.items():
            assert st["in_range"], f"{name} outside plausible range: {st}"
            assert st["valid_fraction"] > 0.5, f"{name} mostly NaN: {st}"

    def test_pilot_compared_with_itself_agrees_perfectly(self, pilot):
        """Harness check: before trusting compare_stacks to judge a NEW
        stack, it must report perfect agreement for identical inputs."""
        result = compare_stacks(pilot, pilot, n_points=3000)
        for name in LAYERS:
            assert result[name]["pearson_r"] == pytest.approx(1.0), name
            assert result[name]["mean_abs_diff"] == pytest.approx(0.0, abs=1e-9), name
        assert result["twi_bar"]["pearson_r"] == pytest.approx(1.0)

    def test_nonoverlapping_stacks_are_rejected(self, pilot):
        from validate_stack import _shared_lonlat_bbox

        class _Far:
            def wgs84_bbox(self):
                return (-80.0, 35.0, -79.0, 36.0)

        with pytest.raises(ValueError, match="do not overlap"):
            _shared_lonlat_bbox(pilot, _Far())
