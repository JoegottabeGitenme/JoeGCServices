"""Tests for check_trails_api.py's pure checks. Run with: pytest scripts/test_check_trails_api.py

The smoke test is only worth running if it can FAIL. These feed it known-bad
conditions blocks -- above all the Session 13 bug, a forecast hour returned as
`latest` -- and a healthy one."""

import datetime as dt
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from check_trails_api import conditions_problems  # noqa: E402

NOW = dt.datetime(2026, 10, 5, 22, 30, tzinfo=dt.timezone.utc)


def block(**over):
    c = {
        "run_time": "2026-10-05T20:00:00+00:00", "valid_time": "2026-10-05T22:00:00+00:00", "forecast_hour": 2,
        "model_version": "trail-physics-v1", "soil_moisture": 0.15, "saturation": 0.35, "frozen_fraction": 0.0, "confidence": 1.0,
    }
    c.update(over)
    return c


def test_healthy_block_has_no_problems():
    assert conditions_problems(block(), NOW) == []


def test_catches_the_session_13_bug_a_future_forecast_hour_served_as_latest():
    problems = conditions_problems(block(valid_time="2026-10-06T14:00:00+00:00", forecast_hour=18), NOW)
    assert any("FUTURE" in p for p in problems)


def test_catches_stale_data():
    assert any("stale" in p for p in conditions_problems(block(valid_time="2026-10-05T10:00:00+00:00"), NOW))


def test_catches_negative_soil_moisture():
    assert any("negative" in p for p in conditions_problems(block(soil_moisture=-0.04), NOW))


def test_catches_out_of_range_saturation_and_confidence():
    assert any("saturation" in p for p in conditions_problems(block(saturation=1.4), NOW))
    assert any("confidence" in p for p in conditions_problems(block(confidence=-0.1), NOW))


def test_catches_a_guessed_porosity_where_there_is_no_terrain_data():
    assert any("guessed" in p for p in conditions_problems(block(confidence=0, saturation=0.4), NOW))


def test_confidence_zero_with_null_saturation_is_fine():
    assert conditions_problems(block(confidence=0, saturation=None), NOW) == []


def test_missing_fields_are_reported_not_crashed_on():
    c = block()
    del c["saturation"]
    assert any("saturation" in p for p in conditions_problems(c, NOW))


def test_nulls_are_allowed_where_the_contract_allows_them():
    assert conditions_problems(block(soil_moisture=None, saturation=None, frozen_fraction=None, confidence=None), NOW) == []
