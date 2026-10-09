"""Tests for check_lightning_api.py's pure checks. Run: pytest scripts/test_check_lightning_api.py

The smoke test is only worth running if it can FAIL. These feed it known-bad
responses -- above all a DEAD FEED that returns an empty (and so apparently
'quiet') answer, which is the failure that matters most."""

import copy
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from check_lightning_api import collection_problems  # noqa: E402

GOOD = {
    "type": "FeatureCollection",
    "numberReturned": 2,
    "timeStamp": "2026-10-07T21:00:00.000Z",
    "lastId": 12,
    "dataThrough": "2026-10-07T20:59:15.000Z",
    "dataAgeSeconds": 45.0,
    "features": [
        {"type": "Feature", "id": 10, "geometry": {"type": "Point", "coordinates": [-105.25, 40.01]},
         "properties": {"flash_time": "2026-10-07T20:59:30.000Z", "age_seconds": 30.0, "satellite": "goes-east", "energy_j": 4.7e-14, "quality": 0}},
        {"type": "Feature", "id": 12, "geometry": {"type": "Point", "coordinates": [-104.99, 39.74]},
         "properties": {"flash_time": "2026-10-07T20:59:40.000Z", "age_seconds": 20.0, "satellite": "goes-east", "energy_j": None, "quality": 3}},
    ],
}
EMPTY_FRESH = {**GOOD, "features": [], "numberReturned": 0, "lastId": None}


def mutate(base, **over):
    d = copy.deepcopy(base)
    d.update(over)
    return d


def test_healthy_response_has_no_problems():
    assert collection_problems(GOOD, 300) == []


def test_a_genuinely_quiet_sky_is_healthy():
    assert collection_problems(EMPTY_FRESH, 300) == []


def test_an_empty_answer_from_a_dead_feed_is_flagged_not_mistaken_for_a_quiet_sky():
    dead = mutate(EMPTY_FRESH, dataThrough="2026-10-07T20:20:00.000Z", dataAgeSeconds=2400.0)
    problems = collection_problems(dead, 300)
    assert any("STALE" in p and "quiet sky" in p for p in problems), problems


def test_unknown_freshness_is_flagged_never_treated_as_current():
    unknown = mutate(EMPTY_FRESH, dataThrough=None, dataAgeSeconds=None)
    assert any("freshness unknown" in p for p in collection_problems(unknown, 300))


def test_the_freshness_fields_are_required():
    for key in ("dataThrough", "dataAgeSeconds", "lastId", "timeStamp"):
        broken = {k: v for k, v in GOOD.items() if k != key}
        assert any(key in p for p in collection_problems(broken, 300)), key


def test_a_future_dated_data_through_is_flagged():
    assert any("future" in p for p in collection_problems(mutate(GOOD, dataAgeSeconds=-120.0), 300))


def test_the_stale_threshold_is_configurable():
    assert collection_problems(mutate(GOOD, dataAgeSeconds=200.0), 300) == []
    assert any("STALE" in p for p in collection_problems(mutate(GOOD, dataAgeSeconds=200.0), 120))


def test_cursor_invariants():
    assert any("lastId" in p for p in collection_problems(mutate(GOOD, lastId=99), 300))
    assert any("null when nothing" in p for p in collection_problems(mutate(EMPTY_FRESH, lastId=5), 300))
    assert any("numberReturned" in p for p in collection_problems(mutate(GOOD, numberReturned=7), 300))


def test_features_must_be_oldest_first():
    swapped = copy.deepcopy(GOOD)
    swapped["features"].reverse()
    assert any("oldest-first" in p for p in collection_problems(swapped, 300))


def test_feature_level_defects_are_caught():
    def with_feature(**props):
        d = copy.deepcopy(GOOD)
        d["features"][0]["properties"].update(props)
        return d

    assert any("future" in p for p in collection_problems(with_feature(flash_time="2026-10-07T21:05:00.000Z", age_seconds=-300.0), 300))
    assert any("disagrees" in p for p in collection_problems(with_feature(age_seconds=500.0), 300))
    assert any("satellite" in p for p in collection_problems(with_feature(satellite="G19"), 300))
    assert any("implausible" in p for p in collection_problems(with_feature(energy_j=4.7), 300))

    outside = copy.deepcopy(GOOD)
    outside["features"][0]["geometry"]["coordinates"] = [-20.0, 10.0]
    assert any("outside the CONUS" in p for p in collection_problems(outside, 300))

    not_point = copy.deepcopy(GOOD)
    not_point["features"][0]["geometry"]["type"] = "LineString"
    assert any("not a Point" in p for p in collection_problems(not_point, 300))
