#!/usr/bin/env python3
"""
Generate a self-contained trends.html from load-test history files.

Charts key metrics across runs so system behavior can be observed over
days/weeks before making tuning decisions:
  - nightly: capacity breakpoints per workload, warm/soak p95s, fail rates
  - probe:   p95 per endpoint class, fail rate (gateway path health)

Stdlib only; no network. Runs on the NUC host.

Usage:
  generate_trends.py --results-dir /opt/weather-wms/loadtest/results \
      --out /opt/gateway/static/loadtest/trends.html
"""

import argparse
import json
import os

# Series to chart: (title, unit, history mode, list of (key, label))
CHARTS = [
    (
        "Capacity breakpoints (nightly)",
        "rps",
        "nightly",
        [
            ("03-ramp-wmts.breakpoint_rps", "WMTS tiles"),
            ("04-ramp-wms.breakpoint_rps", "WMS GetMap"),
            ("05-ramp-edr.breakpoint_rps", "EDR"),
        ],
    ),
    (
        "Steady-state p95 (nightly warm stage)",
        "ms",
        "nightly",
        [
            ("02-warm.wmts_tile_ms.p95", "WMTS tile"),
            ("02-warm.wms_getmap_ms.p95", "WMS GetMap"),
            ("02-warm.edr_position_ms.p95", "EDR position"),
            ("02-warm.edr_area_ms.p95", "EDR area"),
        ],
    ),
    (
        "Soak p95 vs warm p95 — drift means leaks (nightly)",
        "ms",
        "nightly",
        [
            ("07-soak.wmts_tile_ms.p95", "soak WMTS"),
            ("07-soak.wms_getmap_ms.p95", "soak GetMap"),
            ("07-soak.edr_position_ms.p95", "soak EDR position"),
        ],
    ),
    (
        "Cold-cache render p95 (nightly)",
        "ms",
        "nightly",
        [
            ("01-cold.wmts_tile_miss_ms.p95", "WMTS miss"),
            ("01-cold.wms_getmap_ms.p95", "GetMap"),
        ],
    ),
    (
        "Nightly fail rates",
        "%",
        "nightly",
        [
            ("02-warm.fail_rate", "warm"),
            ("07-soak.fail_rate", "soak"),
            ("06-spike.fail_rate", "spike"),
        ],
    ),
    (
        "Probe p95 (gateway path, daytime)",
        "ms",
        "probe",
        [
            ("probe.wmts_tile_ms.p95", "WMTS tile"),
            ("probe.wms_getmap_ms.p95", "WMS GetMap"),
            ("probe.edr_position_ms.p95", "EDR position"),
        ],
    ),
    (
        "Probe fail rate (gateway path)",
        "%",
        "probe",
        [("probe.fail_rate", "fail rate")],
    ),
]

HTML = """<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Load Test Trends</title>
<style>
body {{ font-family: -apple-system, "Segoe UI", Roboto, sans-serif; max-width: 1100px;
       margin: 2rem auto; padding: 0 1rem; color: #1a202c; }}
h1 {{ border-bottom: 2px solid #2b6cb0; padding-bottom: 6px; }}
.chart {{ margin: 2rem 0; }}
.chart h2 {{ color: #2b6cb0; font-size: 1.1rem; margin-bottom: 0.3rem; }}
canvas {{ border: 1px solid #e2e8f0; border-radius: 4px; width: 100%; }}
.legend span {{ display: inline-block; margin-right: 1.2em; font-size: 0.85rem; }}
.legend i {{ display: inline-block; width: 10px; height: 10px; margin-right: 4px;
             border-radius: 2px; }}
.meta {{ color: #718096; font-size: 0.85rem; }}
</style></head><body>
<h1>Load Test Trends</h1>
<p class="meta">Generated {generated}. Nightly runs: {n_nightly}, probe runs: {n_probe}.
Links: <a href="index.html">latest nightly report</a> ·
<a href="probe.html">latest probe</a> · <a href="history/">nightly history</a></p>
<div id="charts"></div>
<script>
const DATA = {data};
const COLORS = ["#2b6cb0", "#c05621", "#2f855a", "#805ad5", "#c53030", "#718096"];

function drawChart(container, chart) {{
  const div = document.createElement("div");
  div.className = "chart";
  const h = document.createElement("h2");
  h.textContent = chart.title;
  div.appendChild(h);
  const legend = document.createElement("div");
  legend.className = "legend";
  chart.series.forEach((s, i) => {{
    const sp = document.createElement("span");
    sp.innerHTML = `<i style="background:${{COLORS[i % COLORS.length]}}"></i>${{s.label}}`;
    legend.appendChild(sp);
  }});
  div.appendChild(legend);
  const canvas = document.createElement("canvas");
  canvas.width = 1060; canvas.height = 260;
  div.appendChild(canvas);
  container.appendChild(div);

  const ctx = canvas.getContext("2d");
  const PAD = {{ l: 64, r: 12, t: 12, b: 42 }};
  const W = canvas.width - PAD.l - PAD.r;
  const H = canvas.height - PAD.t - PAD.b;

  const allVals = chart.series.flatMap(s => s.points.map(p => p[1])).filter(v => v != null);
  if (allVals.length === 0) {{
    ctx.fillStyle = "#718096";
    ctx.fillText("no data yet — runs will accumulate here", PAD.l, 60);
    return;
  }}
  const allX = chart.series.flatMap(s => s.points.map(p => p[0]));
  const xMin = Math.min(...allX), xMax = Math.max(...allX);
  const yMax = Math.max(...allVals) * 1.15 || 1;

  const xOf = t => PAD.l + (xMax === xMin ? W / 2 : (t - xMin) / (xMax - xMin) * W);
  const yOf = v => PAD.t + H - (v / yMax) * H;

  // gridlines + y labels
  ctx.strokeStyle = "#edf2f7"; ctx.fillStyle = "#718096";
  ctx.font = "11px sans-serif"; ctx.textAlign = "right";
  for (let g = 0; g <= 4; g++) {{
    const v = yMax * g / 4, y = yOf(v);
    ctx.beginPath(); ctx.moveTo(PAD.l, y); ctx.lineTo(PAD.l + W, y); ctx.stroke();
    ctx.fillText(v >= 100 ? v.toFixed(0) : v.toPrecision(3), PAD.l - 6, y + 4);
  }}
  ctx.fillText(chart.unit, PAD.l - 6, PAD.t - 2);

  // x labels (dates)
  ctx.textAlign = "center";
  const seen = new Set();
  chart.series[0] && chart.series[0].points.forEach(p => {{
    const d = new Date(p[0]);
    const label = (d.getUTCMonth() + 1) + "/" + d.getUTCDate();
    if (!seen.has(label)) {{
      seen.add(label);
      ctx.fillText(label, xOf(p[0]), PAD.t + H + 16);
    }}
  }});

  // series
  chart.series.forEach((s, i) => {{
    ctx.strokeStyle = ctx.fillStyle = COLORS[i % COLORS.length];
    ctx.lineWidth = 1.6;
    ctx.beginPath();
    let started = false;
    s.points.forEach(p => {{
      if (p[1] == null) return;
      const x = xOf(p[0]), y = yOf(p[1]);
      if (!started) {{ ctx.moveTo(x, y); started = true; }} else ctx.lineTo(x, y);
    }});
    ctx.stroke();
    s.points.forEach(p => {{
      if (p[1] == null) return;
      ctx.beginPath();
      ctx.arc(xOf(p[0]), yOf(p[1]), 2.6, 0, Math.PI * 2);
      ctx.fill();
    }});
  }});
}}

const container = document.getElementById("charts");
DATA.forEach(c => drawChart(container, c));
</script>
</body></html>
"""


def load_history(path):
    entries = []
    if not os.path.exists(path):
        return entries
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                e = json.loads(line)
            except json.JSONDecodeError:
                continue
            if e.get("finished") and e.get("keys"):
                entries.append(e)
    entries.sort(key=lambda e: e["finished"])
    return entries


def ts_ms(iso):
    from datetime import datetime

    return int(datetime.fromisoformat(iso.replace("Z", "+00:00")).timestamp() * 1000)


def build_series(entries, keys):
    series = []
    for key, label in keys:
        points = []
        for e in entries:
            v = e["keys"].get(key)
            if key.endswith(".fail_rate") and v is not None:
                v = round(v * 100, 3)  # to percent
            points.append([ts_ms(e["finished"]), v])
        series.append({"label": label, "points": points})
    return series


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--results-dir", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    hist = {
        "nightly": load_history(os.path.join(args.results_dir, "nightly-history.jsonl")),
        "probe": load_history(os.path.join(args.results_dir, "probe-history.jsonl")),
    }

    charts = []
    for title, unit, mode, keys in CHARTS:
        charts.append(
            {
                "title": title,
                "unit": unit,
                "series": build_series(hist[mode], keys),
            }
        )

    from datetime import datetime, timezone

    html = HTML.format(
        generated=datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
        n_nightly=len(hist["nightly"]),
        n_probe=len(hist["probe"]),
        data=json.dumps(charts),
    )
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w") as f:
        f.write(html)
    print(f"Trends written to {args.out} "
          f"({len(hist['nightly'])} nightly / {len(hist['probe'])} probe runs)")


if __name__ == "__main__":
    main()
