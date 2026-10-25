"""Tests for check_trails_api.py's pure checks. Run with: pytest scripts/test_check_trails_api.py

The smoke test is only worth running if it can FAIL. These feed it known-bad
conditions blocks -- above all the Session 13 bug, a forecast hour returned as
`latest` -- and a healthy one."""

import datetime as dt
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from check_trails_api import batch_latency_ok, batch_problems, conditions_problems, latest_latency_ok  # noqa: E402

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


def test_future_valid_time_is_flagged_even_when_other_fields_are_missing():
    """Regression for a flaw in this very checker: it used to return early on a
    missing field and so never reached the valid_time check. The pre-deploy
    production response (Session 13 build) had no `saturation` AND a future
    valid_time -- both must be reported."""
    c = block(valid_time="2026-10-06T14:00:00+00:00")
    del c["saturation"]
    problems = conditions_problems(c, NOW)
    assert any("FUTURE" in p for p in problems)
    assert any("saturation" in p for p in problems)


# ---- latency guards ---------------------------------------------------------

def test_latest_latency_catches_the_reported_regression():
    # frontend measured 2.5-3.3 s for conditions=latest against 0.22-0.28 s geometry-only
    assert not latest_latency_ok(0.22, 2.5)
    assert not latest_latency_ok(0.28, 3.3)
    assert not latest_latency_ok(0.12, 1.7)


def test_latest_latency_accepts_the_target():
    assert latest_latency_ok(0.28, 0.40)   # a bit slower
    assert latest_latency_ok(0.10, 0.19)   # just under 2x
    assert latest_latency_ok(0.10, 0.80)   # jitter on a very fast base: +0.75 s allowance
    assert not latest_latency_ok(0.10, 0.90)


def test_batch_latency_catches_a_fan_out_and_accepts_one_call():
    # 40 sequential single calls ~ 40 x 0.1 s
    assert not batch_latency_ok(0.10, 4.0)
    assert batch_latency_ok(0.10, 0.25)
    assert batch_latency_ok(0.05, 0.9)     # +1 s allowance for a very fast single call
    assert not batch_latency_ok(0.05, 1.2)


# ---- batch contract ---------------------------------------------------------

def entry(fid, n=2):
    return {"feature_id": fid, "name": f"t{fid}", "run_time": "2026-10-05T20:00:00+00:00",
            "model_version": "v", "conditions": [{"valid_time": f"2026-10-05T{h:02d}:00:00+00:00"} for h in range(n)]}


def test_healthy_batch_has_no_problems():
    batch = {"series": [entry(3), entry(1)], "unknown_ids": [2]}
    assert batch_problems(batch, [3, 2, 1], {3: entry(3), 1: entry(1)}) == []


def test_batch_with_duplicates_in_the_request_is_fine_when_deduped():
    batch = {"series": [entry(5)], "unknown_ids": []}
    assert batch_problems(batch, [5, 5], {}) == []


def test_catches_a_missing_top_level_key():
    assert batch_problems({"series": []}, [1], {})
    assert batch_problems([], [1], {})


def test_catches_an_id_silently_dropped():
    batch = {"series": [entry(1)], "unknown_ids": []}
    assert any("!= requested" in p for p in batch_problems(batch, [1, 2], {}))


def test_catches_an_id_in_both_lists_and_duplicates():
    both = {"series": [entry(1)], "unknown_ids": [1]}
    assert any("both" in p for p in batch_problems(both, [1], {}))
    dup = {"series": [entry(1), entry(1)], "unknown_ids": []}
    assert any("duplicate" in p for p in batch_problems(dup, [1], {}))


def test_catches_wrong_order():
    batch = {"series": [entry(1), entry(3)], "unknown_ids": []}
    assert any("request order" in p for p in batch_problems(batch, [3, 1], {}))
    unk = {"series": [], "unknown_ids": [9, 8]}
    assert any("unknown_ids" in p and "order" in p for p in batch_problems(unk, [8, 9], {}))


def test_catches_a_batch_entry_that_differs_from_the_single_trail_body():
    batch = {"series": [entry(1, n=2)], "unknown_ids": []}
    problems = batch_problems(batch, [1], {1: entry(1, n=3)})
    assert any("differs from /items/1/conditions" in p for p in problems)
