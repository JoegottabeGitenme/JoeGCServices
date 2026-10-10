"""Tests for check_mrms_qpe.py's pure checks. Run with: pytest scripts/test_check_mrms_qpe.py

A smoke test is only worth running if it can FAIL: these feed it the failure modes this feature
actually had or could have (a short history, a stale feed, holes, a snapped neighbouring grid)."""

import datetime as dt
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from check_mrms_qpe import history_problems, missing_hours, repeated_nonzero, series_problems  # noqa: E402

UTC = dt.timezone.utc
NOW = dt.datetime(2026, 10, 10, 2, 30, tzinfo=UTC)


def hours(first: dt.datetime, n: int):
    return [first + dt.timedelta(hours=i) for i in range(n)]


def healthy():
    # 72 h ending at 00:00Z, i.e. 2.5 h old
    return hours(dt.datetime(2026, 10, 7, 1, 0, tzinfo=UTC), 72)


def iso(ts):
    return [t.strftime("%Y-%m-%dT%H:%M:%SZ") for t in ts]


# ---- history ----------------------------------------------------------------

def test_healthy_history_has_no_problems():
    assert history_problems(healthy(), NOW) == []


def test_catches_the_original_two_hour_history():
    two_hours = hours(dt.datetime(2026, 10, 9, 23, 0, tzinfo=UTC), 2)
    assert any("spans only" in p for p in history_problems(two_hours, NOW))


def test_catches_a_stalled_feed():
    old = hours(dt.datetime(2026, 10, 6, 0, 0, tzinfo=UTC), 72)  # newest 2026-10-08 23:00Z
    assert any("old" in p for p in history_problems(old, NOW))


def test_catches_missing_hours_but_tolerates_a_few():
    ts = healthy()
    few = [t for i, t in enumerate(ts) if i not in (10, 30, 31)]
    assert history_problems(few, NOW) == []  # 3 holes: neither pass published them
    many = [t for i, t in enumerate(ts) if i % 7 != 3]
    assert any("missing" in p for p in history_problems(many, NOW))


def test_catches_instances_that_are_not_on_the_hour():
    ts = healthy()
    ts[5] = ts[5].replace(minute=2)  # a 2-minute radar timestamp leaking into the hourly model
    assert any("not on the hour" in p for p in history_problems(ts, NOW))


def test_empty_history_is_a_problem():
    assert history_problems([], NOW)


def test_missing_hours_lists_exactly_the_holes():
    ts = hours(dt.datetime(2026, 10, 9, 0, 0, tzinfo=UTC), 6)
    del ts[2], ts[3]  # remove 02:00 and (after shift) 04:00
    assert [f"{m:%H}" for m in missing_hours(ts)] == ["02", "04"]
    assert missing_hours([]) == []


# ---- series -----------------------------------------------------------------

def test_healthy_series_has_no_problems():
    ts = hours(dt.datetime(2026, 10, 9, 0, 0, tzinfo=UTC), 6)
    assert series_problems(iso(ts), [0, 0.47, 2.05, None, 1.4, 0]) == []


def test_a_few_missing_hours_are_tolerated_many_are_not():
    ts = hours(dt.datetime(2026, 10, 9, 0, 0, tzinfo=UTC), 12)
    few = [t for i, t in enumerate(ts) if i not in (3, 7)]  # 2 holes
    assert series_problems(iso(few), [0] * len(few)) == []
    many = [t for i, t in enumerate(ts) if i % 2 == 0]  # 5 holes
    assert any("missing from the time axis" in p for p in series_problems(iso(many), [0] * len(many)))


def test_catches_an_axis_that_is_not_on_whole_hours_or_not_ordered():
    ts = hours(dt.datetime(2026, 10, 9, 0, 0, tzinfo=UTC), 4)
    radar_like = iso([ts[0], ts[0] + dt.timedelta(minutes=2), ts[0] + dt.timedelta(minutes=4)])
    assert any("whole hours" in p for p in series_problems(radar_like, [0, 0, 0]))
    assert any("ascending" in p for p in series_problems(iso(ts[::-1]), [0, 0, 0, 0]))


def test_catches_implausible_values_and_length_mismatch():
    ts = iso(hours(dt.datetime(2026, 10, 9, 0, 0, tzinfo=UTC), 3))
    assert any("outside" in p for p in series_problems(ts, [0, -5, 1]))
    assert any("outside" in p for p in series_problems(ts, [0, 9999, 1]))
    assert series_problems(ts, [0, 1])  # length mismatch


# ---- snapped neighbour ------------------------------------------------------

def test_a_repeated_nonzero_total_is_flagged_as_a_snapped_grid():
    # The bug this feature shipped a fix for: 00:00Z repeating 23:00Z's rain exactly.
    assert repeated_nonzero([0, 0, 1.487, 1.487]) == [3]


def test_a_rolling_total_may_legitimately_repeat_so_only_the_hourly_product_is_checked():
    # 24 h totals stay identical when the hour entering and the hour leaving the window were
    # both dry. The script applies repeated_nonzero to QPE_01H only (documented on the function).
    from check_mrms_qpe import repeated_nonzero as rn
    assert rn([3.2, 3.2, 3.2]) == [1, 2]  # the function itself would flag it...
    import inspect, check_mrms_qpe
    src = inspect.getsource(check_mrms_qpe.main)
    assert 'if param == "QPE_01H":' in src  # ...which is why main() restricts it to the hourly product


def test_dry_spells_and_nulls_are_not_flagged():
    assert repeated_nonzero([0, 0, 0, 0]) == []
    assert repeated_nonzero([1.2, None, 1.2]) == []
    assert repeated_nonzero([0.4, 0.41, 0.4]) == []
