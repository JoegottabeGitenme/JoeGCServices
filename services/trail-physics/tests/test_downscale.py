"""Tests for downscale.py -- the per-vertex soil moisture downscaling that
combines a point's raw HRRR reading with the WS1 static stack (or falls
back to the raw value with reduced confidence where the stack doesn't
cover it)."""

import sys
from pathlib import Path
from unittest.mock import MagicMock

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

from downscale import downscale_soil_moisture  # noqa: E402
from hrrr_grid import HrrrGrid  # noqa: E402


def _fake_static_stack(twi_values, theta_s_values, theta_wilt_values, twi_bar_value, in_bounds_mask=None):
    """A MagicMock standing in for static_stack.StaticStack -- returns the
    given arrays from sample_layer regardless of the points passed
    (sufficient to test downscale.py's own combination logic in
    isolation; static_stack.py's own tests cover the real sampling math)."""
    stack = MagicMock()
    n = len(twi_values)
    if in_bounds_mask is None:
        in_bounds_mask = [True] * n
    stack.lonlat_to_rowcol.side_effect = lambda lon, lat: (0.0, 0.0)

    def sample_layer(name, points):
        values = {"twi": twi_values, "theta_s": theta_s_values, "theta_wilt": theta_wilt_values}[name]
        return np.array(values, dtype=float)

    stack.sample_layer.side_effect = sample_layer
    stack.hrrr_twi_bar.return_value = twi_bar_value
    return stack


class TestDownscaleSoilMoisture:
    def test_no_static_stack_falls_back_to_raw_everywhere(self):
        hrrr = HrrrGrid.hrrr()
        points = [(39.75, -105.2), (39.80, -105.3)]
        soilw = np.array([0.25, 0.30])
        result = downscale_soil_moisture(None, hrrr, points, soilw)
        np.testing.assert_allclose(result.predicted, soilw)
        assert result.confidence == 0.0

    def test_no_valid_hrrr_readings_gives_none_confidence(self):
        hrrr = HrrrGrid.hrrr()
        points = [(39.75, -105.2)]
        soilw = np.array([np.nan])
        result = downscale_soil_moisture(None, hrrr, points, soilw)
        assert result.confidence is None
        assert np.isnan(result.predicted[0])

    def test_fully_covered_points_get_downscaled_not_raw(self):
        hrrr = HrrrGrid.hrrr()
        points = [(39.75, -105.2), (39.80, -105.3)]
        soilw = np.array([0.25, 0.25])
        stack = _fake_static_stack(
            twi_values=[10.0, 6.0], theta_s_values=[0.45, 0.45], theta_wilt_values=[0.08, 0.08], twi_bar_value=8.0,
        )
        result = downscale_soil_moisture(stack, hrrr, points, soilw)
        assert result.confidence == pytest.approx(1.0)
        # Point 0 (twi=10 > twi_bar=8) must be WETTER than the raw coarse
        # value; point 1 (twi=6 < twi_bar=8) must be DRIER -- the real
        # equation's own sign convention (see physics/redistribution.py),
        # not just "some different number."
        assert result.predicted[0] > soilw[0]
        assert result.predicted[1] < soilw[1]

    def test_partial_coverage_gives_intermediate_confidence(self):
        hrrr = HrrrGrid.hrrr()
        points = [(39.75, -105.2), (39.80, -105.3)]
        soilw = np.array([0.25, 0.25])
        # Second point's theta_s is NaN -- simulates falling just outside
        # the static stack's real coverage while the first point is inside.
        stack = _fake_static_stack(
            twi_values=[10.0, 6.0], theta_s_values=[0.45, np.nan], theta_wilt_values=[0.08, 0.08], twi_bar_value=8.0,
        )
        result = downscale_soil_moisture(stack, hrrr, points, soilw)
        assert result.confidence == pytest.approx(0.5)
        assert result.predicted[0] != soilw[0]  # downscaled
        assert result.predicted[1] == pytest.approx(soilw[1])  # raw fallback, unchanged

    def test_uncovered_hrrr_cell_falls_back_for_that_point(self):
        """Even if twi/theta_s/theta_wilt are all available, a point whose
        HRRR cell isn't in the stack's lambda_bar lookup (NaN) must still
        fall back to raw -- the equation genuinely needs twi_bar, not just
        the fine-scale layers."""
        hrrr = HrrrGrid.hrrr()
        points = [(39.75, -105.2)]
        soilw = np.array([0.25])
        stack = _fake_static_stack(
            twi_values=[10.0], theta_s_values=[0.45], theta_wilt_values=[0.08], twi_bar_value=float("nan"),
        )
        result = downscale_soil_moisture(stack, hrrr, points, soilw)
        assert result.confidence == pytest.approx(0.0)
        assert result.predicted[0] == pytest.approx(soilw[0])

    def test_nan_soilw_point_excluded_from_confidence_denominator(self):
        """A point with no valid HRRR reading at all shouldn't count
        against coverage -- confidence is 'fraction of points WITH a
        reading that got real downscaling', not 'fraction of all points'."""
        hrrr = HrrrGrid.hrrr()
        points = [(39.75, -105.2), (39.80, -105.3)]
        soilw = np.array([0.25, np.nan])
        stack = _fake_static_stack(
            twi_values=[10.0, 6.0], theta_s_values=[0.45, 0.45], theta_wilt_values=[0.08, 0.08], twi_bar_value=8.0,
        )
        result = downscale_soil_moisture(stack, hrrr, points, soilw)
        assert result.confidence == pytest.approx(1.0)  # the 1 valid point WAS covered
        assert np.isnan(result.predicted[1])
