#!/usr/bin/env python3
"""Contract smoke test for the `glm-lightning` EDR collection -- run it against a
live deployment (default: production) after every deploy.

It asserts the guarantees `docs/lightning-frontend.md` makes to a frontend team.
The one that matters most: **an empty answer must never be mistaken for a quiet
sky when the feed is actually dead.** Every response therefore has to carry
`dataThrough` / `dataAgeSeconds`, and this script fails if the data is stale.

    python3 scripts/check_lightning_api.py                       # production
    python3 scripts/check_lightning_api.py --base http://localhost:8083/edr
    python3 scripts/check_lightning_api.py --max-data-age 180    # stricter

Standard library only. Exit 0 = all checks passed, 1 = at least one failed.
Checks that cannot run (e.g. no flashes right now, so shape cannot be checked) are
reported as SKIP, never silently passed.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
import urllib.error
import urllib.request

# A box around Colorado + a wide CONUS one; flashes are sparse, so also test CONUS-wide.
BBOX_CO = "-109.1,36.9,-102.0,41.1"
SATELLITES = ("goes-east", "goes-west")


class Results:
    def __init__(self):
        self.failed = 0
        self.skipped = 0

    def check(self, cond, msg):
        print(f"  {'PASS' if cond else 'FAIL'}  {msg}")
        if not cond:
            self.failed += 1
        return bool(cond)

    def skip(self, msg):
        self.skipped += 1
        print(f"  SKIP  {msg}")


def fetch(url: str):
    """-> (status, headers, json_or_None). A real User-Agent: the gateway 403s default scripting agents."""
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (lightning-api-contract-check)"})
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return resp.status, resp.headers, _loads(resp.read())
    except urllib.error.HTTPError as e:
        return e.code, e.headers, _loads(e.read())


def _loads(body: bytes):
    try:
        return json.loads(body) if body else None
    except ValueError:
        return None


def parse_time(s: str) -> dt.datetime:
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


# ---------------------------------------------------------------------------
# Pure checks (unit tested in test_check_lightning_api.py)
# ---------------------------------------------------------------------------

def collection_problems(fc: dict, max_data_age: float) -> list[str]:
    """Everything wrong with one FeatureCollection response (empty list = healthy)."""
    bad = []
    for key in ("type", "features", "numberReturned", "timeStamp", "lastId", "dataThrough", "dataAgeSeconds"):
        if key not in fc:
            bad.append(f"missing top-level `{key}`")
    if bad:
        return bad
    if fc["type"] != "FeatureCollection":
        bad.append(f"type is {fc['type']!r}")
    feats = fc["features"]
    if fc["numberReturned"] != len(feats):
        bad.append(f"numberReturned {fc['numberReturned']} != len(features) {len(feats)}")
    ids = [f.get("id") for f in feats]
    if ids != sorted(ids):
        bad.append("features are not oldest-first (ids not ascending)")
    if feats and fc["lastId"] != max(ids):
        bad.append(f"lastId {fc['lastId']} != max feature id {max(ids)}")
    if not feats and fc["lastId"] is not None:
        bad.append("lastId must be null when nothing is returned")

    # -- freshness: the safety-critical part --
    if fc["dataThrough"] is None:
        bad.append("dataThrough is null: freshness unknown (nothing ingested, or a satellite never reported)")
    else:
        age = fc["dataAgeSeconds"]
        if age is None:
            bad.append("dataThrough present but dataAgeSeconds is null")
        elif age < -5:
            bad.append(f"dataAgeSeconds {age} is negative: dataThrough is in the future")
        elif age > max_data_age:
            bad.append(f"data is STALE: dataAgeSeconds {age} > {max_data_age} -- an empty result here would NOT mean a quiet sky")

    now = parse_time(fc["timeStamp"])
    for f in feats:
        p = f.get("properties", {})
        for key in ("flash_time", "age_seconds", "satellite", "quality", "energy_j"):
            if key not in p:
                bad.append(f"feature {f.get('id')}: missing property `{key}`")
        if f.get("geometry", {}).get("type") != "Point":
            bad.append(f"feature {f.get('id')}: geometry is not a Point")
            continue
        lon, lat = f["geometry"]["coordinates"]
        if not (-125.0 <= lon <= -66.0 and 24.0 <= lat <= 50.0):
            bad.append(f"feature {f['id']}: ({lon},{lat}) is outside the CONUS clip")
        if "flash_time" in p:
            ft = parse_time(p["flash_time"])
            if ft > now + dt.timedelta(seconds=2):
                bad.append(f"feature {f['id']}: flash_time {p['flash_time']} is in the future")
            if "age_seconds" in p and abs((now - ft).total_seconds() - p["age_seconds"]) > 0.5:
                bad.append(f"feature {f['id']}: age_seconds {p['age_seconds']} disagrees with flash_time")
        if p.get("satellite") not in SATELLITES:
            bad.append(f"feature {f['id']}: unknown satellite {p.get('satellite')!r}")
        if p.get("energy_j") is not None and not (1e-17 < p["energy_j"] < 1e-8):
            bad.append(f"feature {f['id']}: energy_j {p['energy_j']} is implausible")
    return bad


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", default="https://folkweather.com/edr")
    ap.add_argument("--max-data-age", type=float, default=300.0, help="fail if dataAgeSeconds exceeds this (default 300)")
    args = ap.parse_args()
    base = args.base.rstrip("/")
    col = f"{base}/collections/glm-lightning"
    r = Results()

    print(f"== collection metadata ({col})")
    status, _, meta = fetch(col)
    if r.check(status == 200, f"GET /collections/glm-lightning -> {status}"):
        r.check(meta.get("id") == "glm-lightning", "id is glm-lightning")
        bbox = (meta.get("extent", {}).get("spatial", {}).get("bbox") or [[]])[0]
        r.check(bbox == [-125.0, 24.0, -66.0, 50.0], f"spatial extent is the CONUS clip ({bbox})")
    status, _, lst = fetch(f"{base}/collections")
    r.check(status == 200 and any(c.get("id") == "glm-lightning" for c in (lst or {}).get("collections", [])),
            "listed in /collections (even when no flash is stored)")

    print("== default query (CONUS-wide, last 10 min, goes-east)")
    status, headers, fc = fetch(f"{col}/items")
    if r.check(status == 200 and fc is not None, f"GET /items -> {status}"):
        r.check("max-age=5" in headers.get("Cache-Control", ""), f"short cache ({headers.get('Cache-Control')})")
        r.check(headers.get("Content-Type", "").startswith("application/geo+json"), f"content-type ({headers.get('Content-Type')})")
        problems = collection_problems(fc, args.max_data_age)
        if r.check(not problems, f"response is well-formed and the data is fresh ({fc['numberReturned']} flashes, dataAge {fc.get('dataAgeSeconds')}s)"):
            pass
        else:
            for pr in problems[:10]:
                print(f"          {pr}")
        r.check(all(f["properties"]["satellite"] == "goes-east" for f in fc["features"]), "default satellite is goes-east only")

    print("== cursor")
    status, _, big = fetch(f"{col}/items?window=PT1H&satellite=both&limit=5000")
    if r.check(status == 200, f"window=PT1H&satellite=both -> {status}") and big["features"]:
        ids = [f["id"] for f in big["features"]]
        r.check(ids == sorted(ids) and len(set(ids)) == len(ids), f"{len(ids)} flashes, ids strictly ascending")
        mid = ids[len(ids) // 2]
        status, _, after = fetch(f"{col}/items?window=PT1H&satellite=both&limit=5000&after={mid}")
        got = [f["id"] for f in after["features"]]
        r.check(status == 200 and all(i > mid for i in got) and set(got) == {i for i in ids if i > mid} | (set(got) - set(ids)),
                f"after={mid} returns only newer ids ({len(got)})")
        status, _, none = fetch(f"{col}/items?satellite=both&after={big['lastId']}&window=PT1H")
        r.check(status == 200 and (none["numberReturned"] == 0 or all(f['id'] > big['lastId'] for f in none['features'])),
                "after=lastId returns nothing older (empty or only brand-new flashes)")
        if none["numberReturned"] == 0:
            r.check(none["lastId"] is None, "empty page has lastId null (keep the old cursor)")
        status, _, lim = fetch(f"{col}/items?window=PT1H&satellite=both&limit=3")
        r.check(status == 200 and lim["numberReturned"] == min(3, len(ids)), f"limit=3 honoured ({lim['numberReturned']})")
        if len(ids) > 3:
            r.check([f["id"] for f in lim["features"]] == ids[-3:], "without a cursor, limit keeps the NEWEST flashes (oldest-first)")
    else:
        r.skip("no flashes in the last hour; cursor/limit behaviour cannot be checked right now")

    print("== area & radius")
    status, _, a = fetch(f"{col}/area?coords={BBOX_CO}&window=PT1H&satellite=both")
    if r.check(status == 200, f"area -> {status}"):
        r.check(all(-109.1 <= f["geometry"]["coordinates"][0] <= -102.0 and 36.9 <= f["geometry"]["coordinates"][1] <= 41.1 for f in a["features"]),
                f"every area result is inside the box ({a['numberReturned']})")
        r.check(not collection_problems(a, args.max_data_age), "area response is well-formed and fresh")
    status, _, rad = fetch(f"{col}/radius?coords=POINT(-104.99%2039.74)&within=300&within-units=km&window=PT1H&satellite=both")
    r.check(status == 200 and "features" in rad, f"radius -> {status}")

    print("== validation")
    for qs, why in [("satellite=moon", "unknown satellite"), ("window=banana", "bad window"), ("window=PT25H", "window > retention"),
                    ("datetime=2026-10-07T20:00:00Z", "single instant"), ("after=-1", "negative cursor"),
                    ("window=PT5M&datetime=2026-10-07T20:00:00Z/..", "window + datetime"), ("bbox=-105,39,-106,40", "inverted bbox")]:
        s, _, _ = fetch(f"{col}/items?{qs}")
        r.check(s == 400, f"{why} -> 400 (got {s})")
    r.check(fetch(f"{base}/collections/nope/items")[0] == 404, "unknown collection -> 404")

    print()
    print(f"{'FAILED' if r.failed else 'ok'}: {r.failed} failed, {r.skipped} skipped")
    return 1 if r.failed else 0


if __name__ == "__main__":
    sys.exit(main())
