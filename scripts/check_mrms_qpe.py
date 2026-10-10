#!/usr/bin/env python3
"""Contract smoke test for the MRMS QPE history (the hourly past-rain curve) -- run it
against a live deployment (default: production) after every deploy.

    python3 scripts/check_mrms_qpe.py
    python3 scripts/check_mrms_qpe.py --base https://folkweather.com --point -105.27,40.01

Asserts what `config/models/mrms-qpe.yaml` promises: 72 hours of HOURLY grids, current to
within a few hours, a one-request point series with no invented values, and the renamed WMS
layers. Standard library only. Exit 0 = all passed, 1 = a check failed. Checks that cannot
run are reported as SKIP, never silently passed.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
import urllib.error
import urllib.request

HOUR = dt.timedelta(hours=1)
# History promised by retention.hours: 72, with slack for the hour being fetched right now and
# for hours that exist in neither Pass1 nor Pass2.
MIN_SPAN = dt.timedelta(hours=68)
# Pass2 lands ~1 h after its nominal time and has been seen 5+ h late; Pass1 fills in after 90 min.
# The newest hour of 24H/72H (Pass2 only) is therefore normally 1-3 h behind "now".
MAX_LAG = dt.timedelta(hours=4)
# Pass1 is complete (24/24 most days) and fills Pass2's gaps after 90 minutes, so the hourly
# series should be nearly gap-free. Allow a few missing hours (neither pass published).
MAX_MISSING_HOURS = 4


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


def fetch(url: str, timeout: int = 120):
    """-> (status, parsed_json_or_None, raw_bytes). A real User-Agent is sent: the gateway 403s
    default scripting agents."""
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (mrms-qpe-contract-check)"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read()
            status = resp.status
    except urllib.error.HTTPError as e:
        body, status = e.read(), e.code
    try:
        return status, json.loads(body), body
    except ValueError:
        return status, None, body


def parse_time(s: str) -> dt.datetime:
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


# ---- pure checks (unit-tested in test_check_mrms_qpe.py) --------------------------------

def missing_hours(times: list[dt.datetime]) -> list[dt.datetime]:
    """Whole hours absent between the first and last of `times` (sorted ascending)."""
    if not times:
        return []
    have = set(times)
    out, t = [], times[0]
    while t <= times[-1]:
        if t not in have:
            out.append(t)
        t += HOUR
    return out


def history_problems(times: list[dt.datetime], now: dt.datetime) -> list[str]:
    """What is wrong with the list of QPE instance times (empty = healthy)."""
    if not times:
        return ["no QPE instances at all"]
    bad = []
    times = sorted(times)
    off_hour = [t for t in times if (t.minute, t.second) != (0, 0)]
    if off_hour:
        bad.append(f"{len(off_hour)} instance(s) not on the hour, e.g. {off_hour[0]:%d %H:%M}Z (QPE is hourly)")
    if now - times[-1] > MAX_LAG:
        bad.append(f"newest instance {times[-1]:%d %H:%MZ} is {now - times[-1]} old (limit {MAX_LAG})")
    if times[-1] - times[0] < MIN_SPAN:
        bad.append(f"history spans only {times[-1] - times[0]} (< {MIN_SPAN}); retention promises 72 h")
    gaps = missing_hours(times)
    if len(gaps) > MAX_MISSING_HOURS:
        bad.append(f"{len(gaps)} hours missing inside the history (limit {MAX_MISSING_HOURS}): {[f'{g:%d %H}Z' for g in gaps[:6]]}")
    return bad


def series_problems(times: list[str], values: list) -> list[str]:
    """What is wrong with one parameter's point series (empty = healthy)."""
    bad = []
    if len(times) != len(values):
        return [f"{len(times)} time steps but {len(values)} values"]
    parsed = [parse_time(t) for t in times]
    if parsed != sorted(parsed) or len(set(parsed)) != len(parsed):
        bad.append("time axis is not strictly ascending")
    if any((b - a) % HOUR != dt.timedelta(0) for a, b in zip(parsed, parsed[1:])) or any(t.minute or t.second for t in parsed):
        bad.append("time axis is not on whole hours (the series must be hourly)")
    # A step exists only where some QPE grid does, so an hour no product published shows up as a
    # 2-hour step. A few are expected (see MAX_MISSING_HOURS); many mean the history has holes.
    holes = sum(int((b - a) / HOUR) - 1 for a, b in zip(parsed, parsed[1:]) if b > a)
    if holes > MAX_MISSING_HOURS:
        bad.append(f"{holes} hours missing from the time axis (limit {MAX_MISSING_HOURS})")
    bogus = [v for v in values if v is not None and (not isinstance(v, (int, float)) or v < 0 or v > 500)]
    if bogus:
        bad.append(f"values outside 0-500 mm: {bogus[:3]}")
    return bad


def repeated_nonzero(values: list) -> list[int]:
    """Indices where a non-zero value equals the previous step's exactly.

    For the 1-hour product only: an hour's rainfall repeating to full float precision is
    essentially impossible, while a snapped neighbouring grid does exactly that. NOT valid
    for the 24 h / 72 h products: a rolling total legitimately repeats whenever the hour
    entering and the hour leaving the window were both dry."""
    return [i for i in range(1, len(values))
            if values[i] is not None and values[i] != 0 and values[i] == values[i - 1]]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", default="https://folkweather.com")
    ap.add_argument("--point", default="-122.3,47.6", help="lon,lat to take the series at (default Seattle, usually has some rain)")
    args = ap.parse_args()
    base = args.base.rstrip("/")
    edr = f"{base}/edr/collections"
    lon, lat = args.point.split(",")
    r = Results()
    now = dt.datetime.now(dt.timezone.utc)

    print("== collections")
    for cid in ("mrms-qpe", "mrms-qpe-latest"):
        status, meta, _ = fetch(f"{edr}/{cid}")
        if r.check(status == 200 and meta is not None, f"GET /collections/{cid} -> {status}"):
            names = sorted((meta.get("parameter_names") or {}).keys())
            r.check(names == ["QPE_01H", "QPE_24H", "QPE_72H"], f"{cid} parameters: {names}")
    status, meta, _ = fetch(f"{edr}/mrms-single-level")
    if status == 200 and meta:
        names = sorted((meta.get("parameter_names") or {}).keys())
        r.check(
            not any(n.startswith("QPE") for n in names) and {"PRECIP_RATE", "REFL"} <= set(names),
            f"radar collection no longer lists QPE (and keeps REFL/PRECIP_RATE): {names}",
        )

    print("== 72 hours of hourly history")
    status, inst, _ = fetch(f"{edr}/mrms-qpe/instances")
    times = []
    if r.check(status == 200 and inst is not None, f"GET /collections/mrms-qpe/instances -> {status}"):
        times = sorted(parse_time(i["id"]) for i in inst.get("instances", []))
        problems = history_problems(times, now)
        if r.check(not problems, f"{len(times)} hourly instances, {times[0]:%d %H:%MZ} .. {times[-1]:%d %H:%MZ}" if times else "no instances"):
            pass
        else:
            for p in problems:
                print(f"          {p}")

    print(f"== hourly point series at ({lon}, {lat}), one request")
    if len(times) < 2:
        r.skip("no instances, cannot build a series query")
    else:
        q = (f"{edr}/mrms-qpe/position?coords=POINT({lon}%20{lat})&parameter-name=QPE_01H,QPE_24H"
             f"&datetime={times[0]:%Y-%m-%dT%H:%M:%SZ}/{times[-1]:%Y-%m-%dT%H:%M:%SZ}")
        t0 = dt.datetime.now()
        status, cov, _ = fetch(q)
        took = (dt.datetime.now() - t0).total_seconds()
        if r.check(status == 200 and cov is not None and "ranges" in cov, f"series query -> {status} in {took:.1f}s"):
            axis = cov["domain"]["axes"]["t"]["values"]
            r.check(len(axis) >= len(times) - MAX_MISSING_HOURS, f"{len(axis)} time steps for {len(times)} instances")
            for param in ("QPE_01H", "QPE_24H"):
                vals = cov["ranges"][param]["values"]
                problems = series_problems(axis, vals)
                if r.check(not problems, f"{param}: whole hours, ascending, plausible values ({sum(v is None for v in vals)} null of {len(vals)})"):
                    pass
                else:
                    for p in problems:
                        print(f"          {p}")
                if param == "QPE_01H":
                    rep = repeated_nonzero(vals)
                    r.check(not rep, f"{param}: no non-zero hourly total repeats its predecessor (a snapped neighbouring grid would)"
                            + (f" -- steps {rep[:5]}" if rep else ""))
            nulls_01h = sum(v is None for v in cov["ranges"]["QPE_01H"]["values"])
            r.check(nulls_01h <= MAX_MISSING_HOURS, f"QPE_01H nulls {nulls_01h} <= {MAX_MISSING_HOURS} (Pass1 fills Pass2's gaps)")

    print("== WMS layers renamed with the model")
    status, _, body = fetch(f"{base}/wms?REQUEST=GetCapabilities&SERVICE=WMS&VERSION=1.3.0&layer=mrms-qpe_QPE_01H,mrms-qpe_QPE_24H")
    text = body.decode("utf-8", "replace")
    r.check(status == 200 and "mrms-qpe_QPE_01H" in text and "mrms-qpe_QPE_24H" in text, f"mrms-qpe_QPE_01H / _24H advertised ({status})")
    status, _, _ = fetch(f"{base}/wms?REQUEST=GetCapabilities&SERVICE=WMS&VERSION=1.3.0&layer=mrms_QPE_01H")
    r.check(status == 400, f"old name mrms_QPE_01H is rejected cleanly ({status})")
    status, _, png = fetch(
        f"{base}/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=mrms-qpe_QPE_01H&STYLES=&CRS=EPSG:3857"
        "&BBOX=-13200000,3500000,-9000000,6500000&WIDTH=256&HEIGHT=192&FORMAT=image/png"
    )
    r.check(status == 200 and png[:8] == b"\x89PNG\r\n\x1a\n", f"GetMap mrms-qpe_QPE_01H renders a PNG ({status}, {len(png)} B)")

    print()
    print(f"{'FAILED' if r.failed else 'ok'}: {r.failed} failed, {r.skipped} skipped")
    return 1 if r.failed else 0


if __name__ == "__main__":
    sys.exit(main())
