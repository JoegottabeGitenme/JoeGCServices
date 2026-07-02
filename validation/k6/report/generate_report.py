#!/usr/bin/env python3
"""
weather-wms load test report generator.

Merges k6 stage summaries + docker-stats samples + Prometheus window queries
into one coherent markdown/HTML report with regression flags vs the previous
run of the same mode.

Stdlib only. Runs in a python:3.11-slim container attached to the compose
network (to reach prometheus:9090), with the loadtest dir mounted.

Usage:
  generate_report.py --run-dir results/nightly-20260702-030000 \
      [--mode nightly|probe] [--prometheus-url http://prometheus:9090] \
      [--history results/nightly-history.jsonl]
"""

import argparse
import glob
import json
import math
import os
import re
import sys
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone

# ---------------------------------------------------------------------------
# Stage summary parsing (k6 handleSummary JSON)
# ---------------------------------------------------------------------------

TREND_METRICS = [
    "http_req_duration",
    "wmts_tile_ms",
    "wmts_tile_l1_ms",
    "wmts_tile_l2_ms",
    "wmts_tile_miss_ms",
    "wms_getmap_ms",
    "wms_getmap_miss_ms",
    "edr_position_ms",
    "edr_radius_ms",
    "edr_area_ms",
    "edr_cube_ms",
    "edr_items_ms",
]


def parse_duration(s):
    """Parse k6 duration strings like '10m', '90s', '1h30m' to seconds."""
    total = 0
    for value, unit in re.findall(r"([\d.]+)\s*(h|m|s)", s):
        total += float(value) * {"h": 3600, "m": 60, "s": 1}[unit]
    return total or float(s)


def load_stage(path):
    with open(path) as f:
        data = json.load(f)
    meta = data.get("loadtest_meta", {})
    metrics = data.get("metrics", {})
    state = data.get("state", {})

    def mval(name, key, default=None):
        m = metrics.get(name)
        if not m:
            return default
        return m.get("values", {}).get(key, default)

    duration_ms = state.get("testRunDurationMs", 0)
    reqs = mval("http_reqs", "count", 0)

    stage = {
        "file": os.path.basename(path),
        "name": meta.get("scenario", os.path.basename(path).replace(".json", "")),
        "kind": meta.get("kind", "unknown"),
        "target": meta.get("target", "direct"),
        "finished_at": meta.get("finished_at"),
        "duration_s": round(duration_ms / 1000, 1),
        "requests": reqs,
        "rps": round(reqs / (duration_ms / 1000), 1) if duration_ms else 0,
        "fail_rate": mval("http_req_failed", "rate", 0.0),
        "dropped_iterations": mval("dropped_iterations", "count", 0),
        "checks_rate": mval("checks", "rate"),
        "trends": {},
        "cache_tiers": {},
        "thresholds_failed": [],
        "meta": meta,
    }

    for t in TREND_METRICS:
        m = metrics.get(t)
        if m and m.get("values", {}).get("count", m["values"].get("avg")) is not None:
            v = m["values"]
            stage["trends"][t] = {
                "p50": v.get("med"),
                "p95": v.get("p(95)"),
                "p99": v.get("p(99)"),
                "avg": v.get("avg"),
                "max": v.get("max"),
            }

    # threshold pass/fail
    for name, m in metrics.items():
        for th, res in (m.get("thresholds") or {}).items():
            if not res.get("ok", True):
                stage["thresholds_failed"].append(f"{name}: {th}")

    # cache tier counts via explicit Counter metrics
    for tier, metric in [
        ("L1-HIT", "cache_l1_total"),
        ("L2-HIT", "cache_l2_total"),
        ("MISS", "cache_miss_total"),
    ]:
        m = metrics.get(metric)
        if m:
            stage["cache_tiers"][tier] = int(m["values"].get("count", 0))

    # breakpoint estimation for ramps
    if stage["kind"] == "ramp":
        planned_s = parse_duration(meta.get("ramp_duration", "10m"))
        start = float(meta.get("start_rate", 10))
        max_rate = float(meta.get("max_rate", 100))
        actual_s = duration_ms / 1000
        # setup/teardown padding tolerance
        aborted = actual_s < planned_s * 0.97
        progress = min(1.0, actual_s / planned_s) if planned_s else 1.0
        scheduled_rps = start + (max_rate - start) * progress

        # Dropped iterations mean k6 fell behind the arrival schedule —
        # the system saturated even if latency thresholds never tripped.
        dropped = stage["dropped_iterations"]
        drop_ratio = dropped / (reqs + dropped) if (reqs + dropped) else 0
        saturated = drop_ratio > 0.05

        if aborted:
            achieved = scheduled_rps
            note = "aborted by threshold - achieved rate is the breakpoint (approx)"
        elif saturated:
            # For a linear ramp, avg rps ~= (start + end)/2 -> implied
            # sustained end rate; scheduled max was NOT actually reached.
            achieved = round(min(scheduled_rps, 2 * stage["rps"] - start), 1)
            note = (
                f"saturated: {drop_ratio:.0%} of iterations dropped "
                "(fell behind arrival schedule) - sustained rate estimated"
            )
        else:
            achieved = scheduled_rps
            note = "completed full ramp without breaching thresholds"

        stage["breakpoint"] = {
            "workload": meta.get("workload"),
            "aborted": aborted,
            "saturated": saturated,
            "achieved_rps": round(achieved, 1),
            "max_tested_rps": max_rate,
            "note": note,
        }

    return stage


# ---------------------------------------------------------------------------
# Prometheus enrichment
# ---------------------------------------------------------------------------


def prom_query(base_url, promql, at=None):
    params = {"query": promql}
    if at:
        params["time"] = at
    url = f"{base_url}/api/v1/query?{urllib.parse.urlencode(params)}"
    try:
        with urllib.request.urlopen(url, timeout=15) as r:
            body = json.load(r)
        if body.get("status") == "success":
            return body["data"]["result"]
    except Exception as e:
        print(f"  prometheus query failed ({promql}): {e}", file=sys.stderr)
    return []


def scalar(results, default=None):
    if results:
        try:
            return float(results[0]["value"][1])
        except (KeyError, ValueError, IndexError):
            pass
    return default


def prometheus_window(base_url, start_iso, end_iso):
    """Query server-side deltas/aggregates for the test window."""
    if not (start_iso and end_iso):
        return {}
    try:
        start = datetime.fromisoformat(start_iso.replace("Z", "+00:00"))
        end = datetime.fromisoformat(end_iso.replace("Z", "+00:00"))
    except ValueError:
        return {}
    window = max(60, int((end - start).total_seconds()))
    w = f"{window}s"
    at = end.timestamp()

    out = {"window_secs": window}
    out["wms_requests"] = scalar(
        prom_query(base_url, f"sum(increase(wms_requests_total[{w}]))", at)
    )
    out["wmts_requests"] = scalar(
        prom_query(base_url, f"sum(increase(wmts_requests_total[{w}]))", at)
    )
    out["edr_requests"] = scalar(
        prom_query(base_url, f"sum(increase(edr_requests_total[{w}]))", at)
    )
    out["edr_errors"] = scalar(
        prom_query(base_url, f"sum(increase(edr_errors_total[{w}]))", at)
    )
    out["edr_p95_max_s"] = scalar(
        prom_query(
            base_url,
            f'max(max_over_time(edr_request_duration_seconds{{quantile="0.95"}}[{w}]))',
            at,
        )
    )
    # l1/chunk caches are cumulative gauges -> delta()
    l1_hits = scalar(prom_query(base_url, f"sum(delta(l1_cache_hits[{w}]))", at))
    l1_miss = scalar(prom_query(base_url, f"sum(delta(l1_cache_misses[{w}]))", at))
    if l1_hits is not None and l1_miss is not None and (l1_hits + l1_miss) > 0:
        out["l1_hit_ratio"] = round(l1_hits / (l1_hits + l1_miss), 3)
    ch_hits = scalar(prom_query(base_url, f"sum(delta(chunk_cache_hits[{w}]))", at))
    ch_miss = scalar(prom_query(base_url, f"sum(delta(chunk_cache_misses[{w}]))", at))
    if ch_hits is not None and ch_miss is not None and (ch_hits + ch_miss) > 0:
        out["chunk_hit_ratio"] = round(ch_hits / (ch_hits + ch_miss), 3)
    out["max_process_mem_mb"] = scalar(
        prom_query(base_url, f"max(max_over_time(process_memory_bytes[{w}])) / 1048576", at)
    )
    return {k: v for k, v in out.items() if v is not None}


# ---------------------------------------------------------------------------
# docker stats samples (collected by the orchestration script)
# ---------------------------------------------------------------------------


def load_docker_stats(run_dir):
    path = os.path.join(run_dir, "docker-stats.jsonl")
    if not os.path.exists(path):
        return {}
    per = {}
    with open(path) as f:
        for line in f:
            try:
                s = json.loads(line)
            except json.JSONDecodeError:
                continue
            name = s.get("Name", "?")
            cpu = float(str(s.get("CPUPerc", "0%")).rstrip("%") or 0)
            mem = str(s.get("MemUsage", "0MiB / 0MiB")).split("/")[0].strip()
            mem_mb = _to_mb(mem)
            d = per.setdefault(name, {"cpu": [], "mem_mb": []})
            d["cpu"].append(cpu)
            d["mem_mb"].append(mem_mb)
    out = {}
    for name, d in per.items():
        if not d["cpu"]:
            continue
        out[name] = {
            "cpu_avg": round(sum(d["cpu"]) / len(d["cpu"]), 1),
            "cpu_max": round(max(d["cpu"]), 1),
            "mem_max_mb": round(max(d["mem_mb"]), 0),
            "samples": len(d["cpu"]),
        }
    return out


def _to_mb(s):
    m = re.match(r"([\d.]+)\s*([KMGT]i?B)", s)
    if not m:
        return 0.0
    v = float(m.group(1))
    unit = m.group(2)[0]
    return v * {"K": 1 / 1024, "M": 1, "G": 1024, "T": 1024 * 1024}[unit]


# ---------------------------------------------------------------------------
# Regression comparison
# ---------------------------------------------------------------------------

LATENCY_REGRESSION_PCT = 20
BREAKPOINT_REGRESSION_PCT = 15


def key_numbers(stages):
    """Extract comparable key metrics from a run for history/regression."""
    keys = {}
    for st in stages:
        prefix = st["name"]
        for trend, v in st["trends"].items():
            if v.get("p95") is not None:
                keys[f"{prefix}.{trend}.p95"] = round(v["p95"], 1)
        keys[f"{prefix}.fail_rate"] = round(st["fail_rate"], 4)
        if "breakpoint" in st:
            keys[f"{prefix}.breakpoint_rps"] = st["breakpoint"]["achieved_rps"]
    return keys


def compare(current, previous):
    flags = []
    if not previous:
        return flags
    for k, cur in current.items():
        prev = previous.get(k)
        if prev is None or prev == 0:
            continue
        change_pct = (cur - prev) / prev * 100
        if k.endswith(".p95") and change_pct > LATENCY_REGRESSION_PCT:
            flags.append(f"REGRESSION {k}: {prev} -> {cur} ms (+{change_pct:.0f}%)")
        elif k.endswith(".breakpoint_rps") and change_pct < -BREAKPOINT_REGRESSION_PCT:
            flags.append(f"REGRESSION {k}: {prev} -> {cur} rps ({change_pct:.0f}%)")
        elif k.endswith(".fail_rate") and cur > 0.02 and cur > prev * 2:
            flags.append(f"REGRESSION {k}: {prev:.3f} -> {cur:.3f}")
    return flags


# ---------------------------------------------------------------------------
# Report rendering
# ---------------------------------------------------------------------------


def fmt_ms(v):
    if v is None:
        return "-"
    return f"{v:.0f}" if v >= 10 else f"{v:.1f}"


def render_markdown(mode, run_dir, stages, prom, dstats, flags, prev_ts):
    lines = []
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    lines.append(f"# Load Test Report — {mode}")
    lines.append("")
    lines.append(f"- **Run**: `{os.path.basename(run_dir)}`  ")
    lines.append(f"- **Generated**: {now}  ")
    total_reqs = sum(s["requests"] for s in stages)
    lines.append(f"- **Total requests**: {total_reqs:,}  ")
    failed_stages = [s["name"] for s in stages if s["thresholds_failed"]]
    verdict = "PASS" if not failed_stages and not flags else "ATTENTION"
    lines.append(f"- **Verdict**: **{verdict}**")
    lines.append("")

    if flags:
        lines.append("## Regressions vs previous run" + (f" ({prev_ts})" if prev_ts else ""))
        lines.append("")
        for f in flags:
            lines.append(f"- {f}")
        lines.append("")

    if failed_stages:
        lines.append("## Threshold failures")
        lines.append("")
        for s in stages:
            for t in s["thresholds_failed"]:
                lines.append(f"- `{s['name']}`: {t}")
        lines.append("")

    # Breakpoints (the right-sizing numbers)
    ramps = [s for s in stages if "breakpoint" in s]
    if ramps:
        lines.append("## Capacity breakpoints")
        lines.append("")
        lines.append("| Workload | Breakpoint (rps) | Max tested | Outcome |")
        lines.append("|---|---|---|---|")
        for s in ramps:
            bp = s["breakpoint"]
            lines.append(
                f"| {bp['workload']} | **{bp['achieved_rps']}** | {bp['max_tested_rps']} | {bp['note']} |"
            )
        lines.append("")

    # Per-stage table
    lines.append("## Stages")
    lines.append("")
    lines.append(
        "| Stage | Target | Duration | Requests | RPS | Fail % | Dropped | p95 (ms) | p99 (ms) |"
    )
    lines.append("|---|---|---|---|---|---|---|---|---|")
    for s in stages:
        t = s["trends"].get("http_req_duration", {})
        lines.append(
            f"| {s['name']} | {s['target']} | {s['duration_s']}s | {s['requests']:,} "
            f"| {s['rps']} | {s['fail_rate'] * 100:.2f} | {s['dropped_iterations']} "
            f"| {fmt_ms(t.get('p95'))} | {fmt_ms(t.get('p99'))} |"
        )
    lines.append("")

    # Endpoint-class latencies
    lines.append("## Endpoint-class latency (per stage)")
    lines.append("")
    lines.append("| Stage | Metric | p50 | p95 | p99 | max |")
    lines.append("|---|---|---|---|---|---|")
    for s in stages:
        for name, v in s["trends"].items():
            if name == "http_req_duration":
                continue
            lines.append(
                f"| {s['name']} | {name} | {fmt_ms(v.get('p50'))} | {fmt_ms(v.get('p95'))} "
                f"| {fmt_ms(v.get('p99'))} | {fmt_ms(v.get('max'))} |"
            )
    lines.append("")

    # Cache tiers
    tiered = [s for s in stages if s["cache_tiers"]]
    if tiered:
        lines.append("## WMTS cache-tier distribution")
        lines.append("")
        lines.append("| Stage | L1 hits | L2 hits | Misses | L1 p95 | L2 p95 | Miss p95 |")
        lines.append("|---|---|---|---|---|---|---|")
        for s in tiered:
            ct = s["cache_tiers"]
            tr = s["trends"]
            lines.append(
                f"| {s['name']} | {ct.get('L1-HIT', 0)} | {ct.get('L2-HIT', 0)} | {ct.get('MISS', 0)} "
                f"| {fmt_ms(tr.get('wmts_tile_l1_ms', {}).get('p95'))} "
                f"| {fmt_ms(tr.get('wmts_tile_l2_ms', {}).get('p95'))} "
                f"| {fmt_ms(tr.get('wmts_tile_miss_ms', {}).get('p95'))} |"
            )
        lines.append("")

    # Server-side (Prometheus)
    if prom:
        lines.append("## Server-side metrics (Prometheus, test window)")
        lines.append("")
        label_map = {
            "wms_requests": ("WMS requests served", "{:,.0f}"),
            "wmts_requests": ("WMTS requests served", "{:,.0f}"),
            "edr_requests": ("EDR requests served", "{:,.0f}"),
            "edr_errors": ("EDR errors", "{:,.0f}"),
            "edr_p95_max_s": ("EDR server-side p95 (worst)", "{:.2f} s"),
            "l1_hit_ratio": ("L1 tile-cache hit ratio", "{:.1%}"),
            "chunk_hit_ratio": ("Chunk-cache hit ratio", "{:.1%}"),
            "max_process_mem_mb": ("Peak process memory", "{:,.0f} MB"),
        }
        for k, (label, fmt) in label_map.items():
            if k in prom:
                lines.append(f"- **{label}**: {fmt.format(prom[k])}")
        lines.append("")

    # docker stats
    if dstats:
        lines.append("## Container resources during run (docker stats)")
        lines.append("")
        lines.append("| Container | CPU avg % | CPU max % | Mem max (MB) |")
        lines.append("|---|---|---|---|")
        for name in sorted(dstats):
            d = dstats[name]
            lines.append(
                f"| {name} | {d['cpu_avg']} | {d['cpu_max']} | {d['mem_max_mb']:.0f} |"
            )
        lines.append("")

    lines.append("---")
    lines.append(
        "_Note: load generator runs on the same host; its CPU usage is included "
        "in host totals. Ingest pipelines (MRMS/GOES/etc.) run concurrently — "
        "this reflects realistic production conditions._"
    )
    return "\n".join(lines) + "\n"


HTML_TEMPLATE = """<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{title}</title>
<style>
body {{ font-family: -apple-system, "Segoe UI", Roboto, sans-serif; max-width: 1000px;
       margin: 2rem auto; padding: 0 1rem; color: #1a202c; }}
table {{ border-collapse: collapse; margin: 1rem 0; font-size: 0.9rem; }}
th, td {{ border: 1px solid #cbd5e0; padding: 4px 10px; text-align: left; }}
th {{ background: #edf2f7; }}
code {{ background: #edf2f7; padding: 1px 5px; border-radius: 3px; }}
h1 {{ border-bottom: 2px solid #2b6cb0; padding-bottom: 6px; }}
h2 {{ color: #2b6cb0; margin-top: 2rem; }}
.attention {{ color: #c53030; font-weight: bold; }}
</style></head><body>
{body}
</body></html>
"""


def md_to_html(md):
    """Minimal markdown -> HTML (headings, tables, lists, bold, code)."""
    html_lines = []
    in_table = False
    in_list = False
    for line in md.split("\n"):
        if line.startswith("|"):
            cells = [c.strip() for c in line.strip("|").split("|")]
            if all(re.fullmatch(r"-{3,}", c) for c in cells):
                continue  # separator row
            tag = "th" if not in_table else "td"
            if not in_table:
                html_lines.append("<table>")
                in_table = True
            row = "".join(f"<{tag}>{inline(c)}</{tag}>" for c in cells)
            html_lines.append(f"<tr>{row}</tr>")
            continue
        if in_table:
            html_lines.append("</table>")
            in_table = False
        if line.startswith("- "):
            if not in_list:
                html_lines.append("<ul>")
                in_list = True
            html_lines.append(f"<li>{inline(line[2:])}</li>")
            continue
        if in_list:
            html_lines.append("</ul>")
            in_list = False
        if line.startswith("# "):
            html_lines.append(f"<h1>{inline(line[2:])}</h1>")
        elif line.startswith("## "):
            html_lines.append(f"<h2>{inline(line[3:])}</h2>")
        elif line.startswith("---"):
            html_lines.append("<hr>")
        elif line.strip():
            html_lines.append(f"<p>{inline(line)}</p>")
    if in_table:
        html_lines.append("</table>")
    if in_list:
        html_lines.append("</ul>")
    return "\n".join(html_lines)


def inline(s):
    s = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", s)
    s = re.sub(r"`(.+?)`", r"<code>\1</code>", s)
    s = s.replace("ATTENTION", '<span class="attention">ATTENTION</span>')
    s = s.replace("REGRESSION", '<span class="attention">REGRESSION</span>')
    return s


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--run-dir", required=True)
    ap.add_argument("--mode", default="nightly", choices=["nightly", "probe"])
    ap.add_argument("--prometheus-url", default="http://prometheus:9090")
    ap.add_argument("--history", default=None)
    args = ap.parse_args()

    run_dir = args.run_dir
    stage_files = sorted(glob.glob(os.path.join(run_dir, "*.json")))
    stage_files = [
        f
        for f in stage_files
        if os.path.basename(f) not in ("report.json", "prometheus.json")
    ]
    if not stage_files:
        print(f"No stage summaries found in {run_dir}", file=sys.stderr)
        sys.exit(1)

    stages = []
    for f in stage_files:
        try:
            stages.append(load_stage(f))
        except Exception as e:
            print(f"  skipping {f}: {e}", file=sys.stderr)
    # order by finish time
    stages.sort(key=lambda s: s.get("finished_at") or "")

    # test window
    starts, ends = [], []
    for s in stages:
        if s["finished_at"]:
            end = datetime.fromisoformat(s["finished_at"].replace("Z", "+00:00"))
            ends.append(end)
            starts.append(end - timedelta(seconds=s["duration_s"]))
    start_iso = min(starts).isoformat() if starts else None
    end_iso = max(ends).isoformat() if ends else None

    print("Querying Prometheus for the test window...")
    prom = prometheus_window(args.prometheus_url, start_iso, end_iso)
    dstats = load_docker_stats(run_dir)

    # history + regression
    history_path = args.history or os.path.join(
        os.path.dirname(run_dir.rstrip("/")), f"{args.mode}-history.jsonl"
    )
    previous, prev_ts = None, None
    if os.path.exists(history_path):
        with open(history_path) as f:
            entries = [json.loads(l) for l in f if l.strip()]
        if entries:
            previous = entries[-1].get("keys")
            prev_ts = entries[-1].get("run")
    keys = key_numbers(stages)
    flags = compare(keys, previous)

    md = render_markdown(args.mode, run_dir, stages, prom, dstats, flags, prev_ts)
    with open(os.path.join(run_dir, "report.md"), "w") as f:
        f.write(md)
    html = HTML_TEMPLATE.format(
        title=f"Load Test — {os.path.basename(run_dir)}", body=md_to_html(md)
    )
    with open(os.path.join(run_dir, "report.html"), "w") as f:
        f.write(html)
    report_json = {
        "run": os.path.basename(run_dir),
        "mode": args.mode,
        "window": {"start": start_iso, "end": end_iso},
        "stages": stages,
        "prometheus": prom,
        "docker_stats": dstats,
        "regressions": flags,
        "keys": keys,
    }
    with open(os.path.join(run_dir, "report.json"), "w") as f:
        json.dump(report_json, f, indent=2, default=str)

    with open(history_path, "a") as f:
        f.write(
            json.dumps(
                {
                    "run": os.path.basename(run_dir),
                    "finished": end_iso,
                    "keys": keys,
                    "regressions": len(flags),
                }
            )
            + "\n"
        )

    print(f"Report written to {run_dir}/report.{{md,html,json}}")
    if flags:
        print("\nREGRESSIONS DETECTED:")
        for fl in flags:
            print(f"  - {fl}")


if __name__ == "__main__":
    main()
