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


class TestSaturation:
    """Session 14: degree of saturation = downscaled soil moisture /
    theta_s, clipped to [0, 1], NaN wherever there's no real theta_s."""

    def test_saturation_is_downscaled_moisture_over_theta_s(self):
        soilw = np.array([0.25, 0.25])
        samples = _static_samples([10.0, 6.0], [0.45, 0.50], [0.08, 0.08], [8.0, 8.0])
        result = downscale_soil_moisture(samples, soilw)
        np.testing.assert_allclose(result.saturation, result.predicted / np.array([0.45, 0.50]))

    def test_wetter_terrain_is_more_saturated_than_drier_at_same_coarse_value(self):
        """Same coarse HRRR value, same soil: the high-TWI point must read
        MORE saturated than the low-TWI one -- the whole point of the
        topographic downscaling, surfaced in the user-facing number."""
        soilw = np.array([0.25, 0.25])
        samples = _static_samples([10.0, 6.0], [0.45, 0.45], [0.08, 0.08], [8.0, 8.0])
        result = downscale_soil_moisture(samples, soilw)
        assert result.saturation[0] > result.saturation[1]

    def test_clipped_to_unit_interval(self):
        # theta_s tiny relative to the predicted value forces an overshoot;
        # theta_s huge forces ~0.
        soilw = np.array([0.40, 0.001])
        samples = _static_samples([14.0, 2.0], [0.10, 5.0], [0.02, 0.02], [8.0, 8.0])
        result = downscale_soil_moisture(samples, soilw)
        assert (result.saturation >= 0.0).all() and (result.saturation <= 1.0).all()
        assert result.saturation[0] == 1.0

    def test_uncovered_points_have_nan_saturation_not_a_fabricated_value(self):
        soilw = np.array([0.25, 0.25])
        samples = _static_samples([10.0, 6.0], [0.45, np.nan], [0.08, 0.08], [8.0, 8.0])  # 2nd point: no theta_s
        result = downscale_soil_moisture(samples, soilw)
        assert not np.isnan(result.saturation[0])
        assert np.isnan(result.saturation[1])

    def test_no_static_samples_gives_all_nan_saturation(self):
        result = downscale_soil_moisture(None, np.array([0.25, 0.30]))
        assert np.isnan(result.saturation).all()
        assert result.saturation.shape == (2,)

    def test_nan_hrrr_reading_gives_nan_saturation(self):
        samples = _static_samples([10.0], [0.45], [0.08], [8.0])
        result = downscale_soil_moisture(samples, np.array([np.nan]))
        assert np.isnan(result.saturation).all()


class TestSampleStaticInputs:
    """The real batched I/O -- exactly 3 `sample_layer_array` calls (twi,
    theta_s, theta_wilt) no matter how many points are in the batch.
    Session 14: called ONCE PER CYCLE (not per segment, not per forecast
    hour) -- the static stack and the trail geometry are time-invariant.
    History: per-segment calls measured ~11ms x 3 layers x 9,029 segments
    =~ 5 min/forecast-hour in production (slower than HRRR's own ~24
    forecast-hours/hour ingest rate); then per-hour repetition was removed
    for the 20x-larger Front Range stack."""

    @staticmethod
    def _fake_stack(twi_values, theta_s_values, theta_wilt_values, twi_bar_values):
        stack = MagicMock()
        stack.lonlat_to_rowcol_array.side_effect = lambda lons, lats: (np.zeros(len(lons)), np.zeros(len(lons)))

        def sample_layer_array(name, rows, cols):
            values = {"twi": twi_values, "theta_s": theta_s_values, "theta_wilt": theta_wilt_values}[name]
            return np.array(values, dtype=float)

        stack.sample_layer_array.side_effect = sample_layer_array
        stack.hrrr_twi_bar_array.return_value = np.array(twi_bar_values, dtype=float)
        return stack

    def test_calls_sample_layer_array_exactly_once_per_named_layer_regardless_of_batch_size(self):
        n = 50
        stack = self._fake_stack([10.0] * n, [0.45] * n, [0.08] * n, [8.0] * n)
        zeros = np.zeros(n)
        sample_static_inputs(stack, lats=zeros + 39.75, lons=zeros - 105.2, hrrr_rows=zeros, hrrr_cols=zeros)
        assert stack.sample_layer_array.call_count == 3  # NOT 3 x n

    def test_returns_arrays_matching_point_count(self):
        stack = self._fake_stack([10.0, 6.0], [0.45, 0.45], [0.08, 0.08], [8.0, 8.0])
        z = np.zeros(2)
        result = sample_static_inputs(stack, lats=z + 39.75, lons=z - 105.2, hrrr_rows=z, hrrr_cols=z)
        assert len(result.twi) == len(result.theta_s) == len(result.theta_wilt) == len(result.twi_bar) == 2

    def test_passes_the_precomputed_hrrr_indices_to_the_lambda_bar_lookup(self):
        """main.py already computed each vertex's HRRR position for forcing
        sampling; it must be reused for lambda_bar, not recomputed (and
        rows/cols must not be swapped -- hrrr_twi_bar_array(rows, cols))."""
        stack = self._fake_stack([1.0], [0.4], [0.1], [7.0])
        sample_static_inputs(
            stack, lats=np.array([39.75]), lons=np.array([-105.2]), hrrr_rows=np.array([123.0]), hrrr_cols=np.array([456.0])
        )
        rows_arg, cols_arg = stack.hrrr_twi_bar_array.call_args.args
        assert rows_arg[0] == 123.0 and cols_arg[0] == 456.0

    def test_empty_batch_returns_empty_arrays(self):
        stack = self._fake_stack([], [], [], [])
        e = np.empty(0)
        assert len(sample_static_inputs(stack, lats=e, lons=e, hrrr_rows=e, hrrr_cols=e)) == 0


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
