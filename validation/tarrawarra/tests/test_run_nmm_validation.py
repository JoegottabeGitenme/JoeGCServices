"""Tests for run_nmm_validation.py -- the NMM holdout check for Session 8's
Rung 1 pass. Real-data tests skip gracefully if the data isn't present,
matching the pattern used throughout the rest of this test suite.
"""

import datetime
import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))
sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "services" / "trail-physics"))

from run_nmm_validation import (  # noqa: E402
    NUM_TUBES,
    load_all_nmm_readings,
    nmm_date_to_date,
    observed_value_for_profile,
)

DATA_DIR = Path(__file__).parent.parent / "data"


def test_nmm_date_to_date_parses_real_format():
    """'DD-Mon-YY', not the readme-documented 'dd/mm/yyyy' -- see
    parse_nmm_file's docstring for the same doc/reality mismatch."""
    assert nmm_date_to_date("20-Sep-95") == datetime.date(1995, 9, 20)
    assert nmm_date_to_date("7-Nov-96") == datetime.date(1996, 11, 7)


def test_observed_value_for_profile_averages_15_and_30cm():
    depths = np.array([15.0, 30.0, 45.0, 60.0, 90.0])
    moisture = np.array([36.4, 36.7, 41.7, 42.3, 33.5])
    result, num_excluded = observed_value_for_profile(depths, moisture)
    assert result == pytest.approx((36.4 + 36.7) / 2)
    assert num_excluded == 0


def test_observed_value_for_profile_falls_back_to_whichever_depth_present():
    """Real data has at least one profile missing the 15cm reading (see
    README.md) -- must fall back to 30cm alone, not fail or silently drop
    the whole profile."""
    depths = np.array([30.0, 45.0, 60.0, 90.0])
    moisture = np.array([40.0, 41.0, 42.0, 43.0])
    result, num_excluded = observed_value_for_profile(depths, moisture)
    assert result == pytest.approx(40.0)
    assert num_excluded == 0


def test_observed_value_for_profile_returns_none_if_neither_depth_present():
    depths = np.array([45.0, 60.0])
    moisture = np.array([41.0, 42.0])
    result, num_excluded = observed_value_for_profile(depths, moisture)
    assert result is None
    assert num_excluded == 0


def test_observed_value_for_profile_excludes_physically_impossible_negative_reading():
    """Real bug found in tube_16.dat, 20-Mar-97 (Session 9): a -8.3 %V/V
    reading at 30cm -- physically impossible, a real neutron-probe
    calibration artifact at an extreme dry-down, not a parsing bug. Must
    be excluded from the average (treated like a missing depth), not
    floored to 0 or silently averaged in."""
    depths = np.array([15.0, 30.0, 45.0])
    moisture = np.array([4.2, -8.3, 20.0])
    result, num_excluded = observed_value_for_profile(depths, moisture)
    assert result == pytest.approx(4.2)  # only the valid 15cm reading used
    assert num_excluded == 1


def test_observed_value_for_profile_returns_none_if_both_depths_negative():
    depths = np.array([15.0, 30.0])
    moisture = np.array([-1.0, -2.0])
    result, num_excluded = observed_value_for_profile(depths, moisture)
    assert result is None
    assert num_excluded == 2


class TestLoadAllNmmReadings:
    def test_real_data_59_dates(self):
        if not (DATA_DIR / "neutron.pos").exists():
            pytest.skip("real NMM data not present (see README.md)")
        for site in range(1, NUM_TUBES + 1):
            if not (DATA_DIR / "nmm" / f"tube_{site}.dat").exists():
                pytest.skip("real NMM data not present (see README.md)")
        readings = load_all_nmm_readings(DATA_DIR)
        # Per geowatch.pdf Section 4.2.1 and confirmed directly against the
        # real files (Session 9).
        assert len(readings) == 59

    def test_real_data_most_dates_have_all_20_tubes(self):
        if not (DATA_DIR / "neutron.pos").exists():
            pytest.skip("real NMM data not present (see README.md)")
        for site in range(1, NUM_TUBES + 1):
            if not (DATA_DIR / "nmm" / f"tube_{site}.dat").exists():
                pytest.skip("real NMM data not present (see README.md)")
        readings = load_all_nmm_readings(DATA_DIR)
        tube_counts = [len(sites) for sites in readings.values()]
        # Confirmed directly against real data (Session 9): 54 dates with
        # all 20 tubes, 5 dates with 19 (a missing tube -- real-world
        # variability matching Session 4/5's tube_20.dat finding, not a
        # parsing bug).
        assert sum(1 for c in tube_counts if c == 20) == 54
        assert sum(1 for c in tube_counts if c == 19) == 5

    def test_real_data_values_are_plausible_fractions(self):
        if not (DATA_DIR / "neutron.pos").exists():
            pytest.skip("real NMM data not present (see README.md)")
        for site in range(1, NUM_TUBES + 1):
            if not (DATA_DIR / "nmm" / f"tube_{site}.dat").exists():
                pytest.skip("real NMM data not present (see README.md)")
        readings = load_all_nmm_readings(DATA_DIR)
        for site_values in readings.values():
            for value in site_values.values():
                # Fractional (m3/m3, not %V/V) -- must be well under 1.0.
                assert 0.0 < value < 1.0
