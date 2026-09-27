"""Tests for downscale.py -- the per-vertex soil moisture downscaling that
combines a point's raw HRRR reading with the WS1 static stack (or falls
back to the raw value with reduced confidence where the stack doesn't
cover it).

Session 14: split into `sample_static_inputs` (the real I/O, batched
across however many points are passed) and `downscale_soil_moisture`
(pure combination logic over already-sampled `StaticSamples`, no I/O) --
see downscale.py's own module docstring for the live-profiled performance
bug this split fixes. Tested separately below."""

import sys
from pathlib import Path
from unittest.mock import MagicMock

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

from downscale import StaticSamples, downscale_soil_moisture, sample_static_inputs  # noqa: E402
from hrrr_grid import HrrrGrid  # noqa: E402


def _static_samples(twi_values, theta_s_values, theta_wilt_values, twi_bar_values) -> StaticSamples:
    return StaticSamples(
        twi=np.array(twi_values, dtype=float),
        theta_s=np.array(theta_s_values, dtype=float),
        theta_wilt=np.array(theta_wilt_values, dtype=float),
        twi_bar=np.array(twi_bar_values, dtype=float),
    )


class TestDownscaleSoilMoisture:
    def test_no_static_samples_falls_back_to_raw_everywhere(self):
        soilw = np.array([0.25, 0.30])
        result = downscale_soil_moisture(None, soilw)
        np.testing.assert_allclose(result.predicted, soilw)
        assert result.confidence == 0.0

    def test_no_valid_hrrr_readings_gives_none_confidence(self):
        soilw = np.array([np.nan])
        result = downscale_soil_moisture(None, soilw)
        assert result.confidence is None
        assert np.isnan(result.predicted[0])

    def test_fully_covered_points_get_downscaled_not_raw(self):
        soilw = np.array([0.25, 0.25])
        samples = _static_samples([10.0, 6.0], [0.45, 0.45], [0.08, 0.08], [8.0, 8.0])
        result = downscale_soil_moisture(samples, soilw)
        assert result.confidence == pytest.approx(1.0)
        # Point 0 (twi=10 > twi_bar=8) must be WETTER than the raw coarse
        # value; point 1 (twi=6 < twi_bar=8) must be DRIER -- the real
        # equation's own sign convention (see physics/redistribution.py),
        # not just "some different number."
        assert result.predicted[0] > soilw[0]
        assert result.predicted[1] < soilw[1]

    def test_partial_coverage_gives_intermediate_confidence(self):
        soilw = np.array([0.25, 0.25])
        # Second point's theta_s is NaN -- simulates falling just outside
        # the static stack's real coverage while the first point is inside.
        samples = _static_samples([10.0, 6.0], [0.45, np.nan], [0.08, 0.08], [8.0, 8.0])
        result = downscale_soil_moisture(samples, soilw)
        assert result.confidence == pytest.approx(0.5)
        assert result.predicted[0] != soilw[0]  # downscaled
        assert result.predicted[1] == pytest.approx(soilw[1])  # raw fallback, unchanged

    def test_uncovered_hrrr_cell_falls_back_for_that_point(self):
        """Even if twi/theta_s/theta_wilt are all available, a point whose
        HRRR cell isn't in the stack's lambda_bar lookup (NaN) must still
        fall back to raw -- the equation genuinely needs twi_bar, not just
        the fine-scale layers."""
        soilw = np.array([0.25])
        samples = _static_samples([10.0], [0.45], [0.08], [float("nan")])
        result = downscale_soil_moisture(samples, soilw)
        assert result.confidence == pytest.approx(0.0)
        assert result.predicted[0] == pytest.approx(soilw[0])

    def test_nan_soilw_point_excluded_from_confidence_denominator(self):
        """A point with no valid HRRR reading at all shouldn't count
        against coverage -- confidence is 'fraction of points WITH a
        reading that got real downscaling', not 'fraction of all points'."""
        soilw = np.array([0.25, np.nan])
        samples = _static_samples([10.0, 6.0], [0.45, 0.45], [0.08, 0.08], [8.0, 8.0])
        result = downscale_soil_moisture(samples, soilw)
        assert result.confidence == pytest.approx(1.0)  # the 1 valid point WAS covered
        assert np.isnan(result.predicted[1])


class TestSampleStaticInputs:
    """The real batched I/O -- exactly 3 `sample_layer` calls total (twi,
    theta_s, theta_wilt) no matter how many points are in the batch. This
    is the whole point of the Session 14 fix: call this ONCE per forecast
    hour across every point from every processed segment, not once per
    segment (measured live: ~11ms/call x 3 layers x 9,029 segments =~ 5
    minutes/forecast-hour with the old per-segment pattern, slower than
    HRRR's own ~24 forecast-hours/hour ingest rate)."""

    @staticmethod
    def _fake_stack(twi_values, theta_s_values, theta_wilt_values, twi_bar_value):
        stack = MagicMock()
        stack.lonlat_to_rowcol.side_effect = lambda lon, lat: (0.0, 0.0)

        def sample_layer(name, points):
            values = {"twi": twi_values, "theta_s": theta_s_values, "theta_wilt": theta_wilt_values}[name]
            return np.array(values, dtype=float)

        stack.sample_layer.side_effect = sample_layer
        stack.hrrr_twi_bar.return_value = twi_bar_value
        return stack

    def test_calls_sample_layer_exactly_once_per_named_layer_regardless_of_batch_size(self):
        hrrr = HrrrGrid.hrrr()
        points = [(39.75, -105.2), (39.80, -105.3), (39.70, -105.1)]
        stack = self._fake_stack([10.0, 6.0, 5.0], [0.45, 0.45, 0.45], [0.08, 0.08, 0.08], 8.0)
        sample_static_inputs(stack, hrrr, points)
        assert stack.sample_layer.call_count == 3  # NOT 3 x len(points)

    def test_returns_arrays_matching_point_count(self):
        hrrr = HrrrGrid.hrrr()
        points = [(39.75, -105.2), (39.80, -105.3)]
        stack = self._fake_stack([10.0, 6.0], [0.45, 0.45], [0.08, 0.08], 8.0)
        result = sample_static_inputs(stack, hrrr, points)
        assert len(result.twi) == 2
        assert len(result.theta_s) == 2
        assert len(result.theta_wilt) == 2
        assert len(result.twi_bar) == 2

    def test_empty_batch_returns_empty_arrays(self):
        hrrr = HrrrGrid.hrrr()
        stack = self._fake_stack([], [], [], float("nan"))
        result = sample_static_inputs(stack, hrrr, [])
        assert len(result) == 0


class TestStaticSamplesSlice:
    """The zero-I/O per-segment slicing that lets one batched
    sample_static_inputs call serve every segment's own combination
    step."""

    def test_slice_returns_matching_subrange(self):
        samples = StaticSamples(
            twi=np.array([1.0, 2.0, 3.0]),
            theta_s=np.array([0.4, 0.5, 0.6]),
            theta_wilt=np.array([0.1, 0.2, 0.3]),
            twi_bar=np.array([7.0, 8.0, 9.0]),
        )
        sliced = samples.slice(1, 3)
        assert len(sliced) == 2
        np.testing.assert_allclose(sliced.twi, [2.0, 3.0])
        np.testing.assert_allclose(sliced.theta_s, [0.5, 0.6])
        np.testing.assert_allclose(sliced.theta_wilt, [0.2, 0.3])
        np.testing.assert_allclose(sliced.twi_bar, [8.0, 9.0])

    def test_slice_then_downscale_matches_direct_computation(self):
        """The actual end-to-end guarantee this refactor depends on:
        slicing a batched StaticSamples and feeding it to
        downscale_soil_moisture must give bit-identical results to
        computing that segment in isolation -- batching must not change
        the physics, only how many network reads it costs."""
        batch = StaticSamples(
            twi=np.array([10.0, 6.0, 12.0, 4.0]),
            theta_s=np.array([0.45, 0.45, 0.45, 0.45]),
            theta_wilt=np.array([0.08, 0.08, 0.08, 0.08]),
            twi_bar=np.array([8.0, 8.0, 8.0, 8.0]),
        )
        soilw_all = np.array([0.25, 0.25, 0.30, 0.20])

        # Segment A = points [0:2], segment B = points [2:4].
        result_a_batched = downscale_soil_moisture(batch.slice(0, 2), soilw_all[0:2])
        result_b_batched = downscale_soil_moisture(batch.slice(2, 4), soilw_all[2:4])

        result_a_direct = downscale_soil_moisture(
            _static_samples([10.0, 6.0], [0.45, 0.45], [0.08, 0.08], [8.0, 8.0]), soilw_all[0:2]
        )
        result_b_direct = downscale_soil_moisture(
            _static_samples([12.0, 4.0], [0.45, 0.45], [0.08, 0.08], [8.0, 8.0]), soilw_all[2:4]
        )

        np.testing.assert_allclose(result_a_batched.predicted, result_a_direct.predicted)
        np.testing.assert_allclose(result_b_batched.predicted, result_b_direct.predicted)
        assert result_a_batched.confidence == result_a_direct.confidence
        assert result_b_batched.confidence == result_b_direct.confidence
