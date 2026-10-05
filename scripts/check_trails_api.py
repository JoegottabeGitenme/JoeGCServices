#!/usr/bin/env python3
"""Contract smoke test for the trails conditions API -- run it against a live
deployment (default: production) after every deploy.

It asserts the guarantees `docs/trail-conditions-frontend.md` makes to a
frontend team, so a regression in any of them is caught by a script and not by
someone noticing odd colors on a map. In particular it would have caught the
bug where `?conditions=latest` returned a FUTURE forecast hour (it was live
from Session 13 until Session 14 and nothing noticed).

    python3 scripts/check_trails_api.py                      # production
    python3 scripts/check_trails_api.py --base http://localhost:8083/edr

Standard library only. Exit status 0 = all checks passed, 1 = at least one failed.
Checks that cannot run (e.g. no covered trail found) are reported as SKIP, never
silently passed.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
import urllib.error
import urllib.request

# A viewport inside the covered region with plenty of trails (Boulder foothills).
DEFAULT_BBOX = "-105.30,39.95,-105.20,40.05"
# Slack for "the current hour": HRRR lags, the worker polls every minute.
MAX_LATEST_AGE = dt.timedelta(hours=3)


class Results:
    def __init__(self):
        self.failed = 0
        self.skipped = 0

    def ok(self, msg):
        print(f"  PASS  {msg}")

    def fail(self, msg):
        self.failed += 1
        print(f"  FAIL  {msg}")

    def skip(self, msg):
        self.skipped += 1
        print(f"  SKIP  {msg}")

    def check(self, cond, msg):
        (self.ok if cond else self.fail)(msg)
        return bool(cond)


def _loads(body: bytes):
    """Parsed JSON, or None if the body is empty / not JSON (e.g. the YAML
    OpenAPI document)."""
    try:
        return json.loads(body) if body else None
    except ValueError:
        return None


def fetch_text(url: str) -> tuple[int, str]:
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (trails-api-contract-check)"})
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return resp.status, resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, ""


def fetch(url: str):
    """-> (status, headers, parsed_json_or_None). A real User-Agent is sent: the
    gateway 403s default scripting agents."""
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (trails-api-contract-check)"})
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            body = resp.read()
            return resp.status, resp.headers, _loads(body)
    except urllib.error.HTTPError as e:
        return e.code, e.headers, _loads(e.read())


def parse_time(s: str) -> dt.datetime:
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def conditions_problems(c: dict, now: dt.datetime) -> list[str]:
    """Everything wrong with one `conditions` block, as human-readable strings
    (empty list = healthy). A pure function so it can be tested against known-
    bad blocks -- including the exact Session 13 bug (a future `valid_time`)."""
    bad = []
    for key in ("run_time", "valid_time", "forecast_hour", "model_version", "saturation", "soil_moisture", "frozen_fraction", "confidence"):
        if key not in c:
            bad.append(f"missing `{key}`")
    if bad:
        return bad
    vt = parse_time(c["valid_time"])
    if vt > now:
        bad.append(f"valid_time {c['valid_time']} is in the FUTURE (a forecast hour served as 'latest')")
    elif now - vt > MAX_LATEST_AGE:
        bad.append(f"valid_time is stale: {now - vt} old (limit {MAX_LATEST_AGE})")
    if parse_time(c["run_time"]) > vt:
        bad.append("run_time is after valid_time")
    for key, lo in (("saturation", 0.0), ("frozen_fraction", 0.0), ("confidence", 0.0)):
        v = c.get(key)
        if v is not None and not (lo <= v <= 1.0):
            bad.append(f"{key} {v} outside [0, 1]")
    if c.get("soil_moisture") is not None and c["soil_moisture"] < 0:
        bad.append(f"soil_moisture {c['soil_moisture']} is negative (physically impossible)")
    if c.get("confidence") == 0 and c.get("saturation") is not None:
        bad.append("confidence 0 but saturation is not null (a porosity was guessed)")
    return bad


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", default="https://folkweather.com/edr")
    ap.add_argument("--bbox", default=DEFAULT_BBOX, help="a viewport inside the covered region")
    args = ap.parse_args()
    base = args.base.rstrip("/")
    trails = f"{base}/collections/trails"
    r = Results()
    now = dt.datetime.now(dt.timezone.utc)

    print(f"== collection metadata ({trails})")
    status, _, meta = fetch(trails)
    if r.check(status == 200, f"GET /collections/trails -> {status}"):
        cov = (meta or {}).get("conditions_coverage")
        if r.check(cov is not None, "collection exposes `conditions_coverage`"):
            bbox = cov["bbox"]
            r.check(len(bbox) == 4 and bbox[0] < bbox[2] and bbox[1] < bbox[3], f"coverage bbox is well-formed {bbox}")

    print("== items with conditions=latest")
    status, headers, fc = fetch(f"{trails}/items?bbox={args.bbox}&conditions=latest&limit=200")
    covered = None
    if r.check(status == 200 and fc and fc.get("type") == "FeatureCollection", f"GET items?conditions=latest -> {status}"):
        feats = fc["features"]
        r.check(len(feats) > 0, f"{len(feats)} trails returned for the viewport")
        r.check("max-age=300" in headers.get("Cache-Control", ""), f"conditions response is cached 5 min (Cache-Control: {headers.get('Cache-Control')})")
        with_c = [f for f in feats if "conditions" in f["properties"]]
        r.check(all(f["properties"].get("conditions") is not None for f in feats), "no feature has `conditions: null` (absent, never null)")
        r.check(len(with_c) > 0, f"{len(with_c)}/{len(feats)} trails carry conditions")
        problems = [(f["id"], pr) for f in with_c for pr in conditions_problems(f["properties"]["conditions"], now)]
        if r.check(not problems, f"all {len(with_c)} conditions blocks are well-formed, fresh, in range, and never in the future"):
            pass
        else:
            for fid, pr in problems[:10]:
                print(f"          trail {fid}: {pr}")
        covered = next((f for f in with_c if f["properties"]["conditions"].get("confidence") == 1), with_c[0] if with_c else None)
        r.check(all(f["geometry"]["type"] == "LineString" for f in feats), "every feature is a LineString")

    print("== geometry-only response keeps the long cache")
    status, headers, _ = fetch(f"{trails}/items?bbox={args.bbox}&limit=5")
    r.check(status == 200 and "max-age=3600" in headers.get("Cache-Control", ""), f"geometry-only cached 1 h (Cache-Control: {headers.get('Cache-Control')})")

    print("== per-trail series")
    if covered is None:
        r.skip("no covered trail found in the viewport; cannot test the series")
    else:
        fid = covered["id"]
        status, _, ts = fetch(f"{trails}/items/{fid}/conditions")
        if r.check(status == 200 and ts is not None, f"GET /items/{fid}/conditions -> {status}"):
            pts = ts["conditions"]
            if r.check(len(pts) > 0, f"{len(pts)} points"):
                hrs = [parse_time(p["valid_time"]) for p in pts]
                r.check(all(b - a == dt.timedelta(hours=1) for a, b in zip(hrs, hrs[1:])), "strictly hourly, ascending, no gaps, one point per hour")
                r.check(hrs[0] >= now - dt.timedelta(hours=7), f"starts at most ~6 h ago (first {hrs[0]:%d %H:%M}Z)")
                r.check(hrs[-1] > now + dt.timedelta(hours=6), f"reaches well into the future (horizon +{(hrs[-1] - now).total_seconds() / 3600:.1f} h)")
                r.check(all("run_time" in p and "saturation" in p for p in pts), "every point carries its own run_time and a saturation key")
                r.check(ts["run_time"] == max(p["run_time"] for p in pts), "top-level run_time is the newest contributing run")
                for p in pts:
                    if p["saturation"] is not None and not (0 <= p["saturation"] <= 1):
                        r.fail(f"series saturation out of range at {p['valid_time']}")
                        break
                else:
                    r.ok("series saturation within [0, 1] throughout")
                    if all(p["soil_moisture"] is None or p["soil_moisture"] >= 0 for p in pts):
                        r.ok("series soil_moisture never negative")
                    else:
                        r.fail("series has negative soil_moisture")

    print("== error handling")
    r.check(fetch(f"{trails}/items/1/conditions")[0] == 404, "unknown trail id -> 404")
    r.check(fetch(f"{base}/collections/nope/items/1/conditions")[0] == 404, "unknown collection -> 404")
    r.check(fetch(f"{base}/collections/hrrr-soil/items/1/conditions")[0] == 400, "non-trail collection -> 400")

    print("== the served OpenAPI document describes this contract")
    status, spec = fetch_text(f"{base}/api")
    if r.check(status == 200, f"GET /api -> {status}"):
        for needle in ("/collections/{collectionId}/items/{featureId}/conditions", "TrailConditions:", "saturation:", "trailConditions:"):
            r.check(needle in spec, f"OpenAPI mentions `{needle}`")

    print()
    verdict = "FAILED" if r.failed else "ok"
    print(f"{verdict}: {r.failed} failed, {r.skipped} skipped")
    return 1 if r.failed else 0


if __name__ == "__main__":
    sys.exit(main())
