# k6 Load Testing — WMS / WMTS / EDR

Automated load testing for the three public services, with two tiers:

- **Daytime probes** — light mixed traffic (~15 rps, 3 min) through the
  gateway, every ~3 hours with random jitter. Pass/fail via k6 thresholds.
- **Nightly stress suite** (03:00, ~85 min) — full intensity, direct to
  services: cold-cache, steady-state, per-service breakpoint ramps, spike,
  soak, and a gateway-overhead comparison stage.

Each run produces a consolidated report (markdown + HTML + JSON) merging:
client-side k6 metrics (per endpoint class, cache-tier segmented via the
`X-Cache` header), server-side Prometheus counters/quantiles for the exact
test window, per-container CPU/mem from a `docker stats` sampler, and
regression flags vs the previous run.

## Layout

```
lib/        common.js (targets, metrics, geo), wms.js, edr.js
scenarios/  probe.js, mixed.js, ramp.js, spike.js
report/     generate_report.py (stdlib-only; runs in python:3.11-slim)
scripts/    lib.sh, run-probe.sh, run-nightly.sh   (run on the NUC)
systemd/    loadtest-{probe,nightly}.{service,timer}
```

Deployed to the NUC at `/opt/weather-wms/loadtest/`.

## Traffic realism

- WMTS/WMS layer pools mirror real layers (`gfs_TMP`, `mrms_REFL`,
  `goes19_CMI_C13`, ...). Most requests omit TIME (server returns latest,
  like real map clients); `TIME_VARIETY` adds a fraction with explicit
  recent times for temporal cache-miss coverage.
- EDR collections/parameters/extents are **discovered at runtime** from
  `/edr/collections`, so requests are always currently valid. Storm-event
  queries use closed year ranges (server requirement).
- Query mixes weight cheap point queries over expensive cubes, like real
  client behavior. Cube 4xx responses (size limits) are not counted as
  failures.

## Manual usage (on the NUC)

```bash
cd /opt/weather-wms/loadtest

# probe (3 min, through gateway)
./scripts/run-probe.sh

# full nightly suite (~85 min)
./scripts/run-nightly.sh

# quick shakeout of the nightly (~15 min)
SCALE=0.15 ./scripts/run-nightly.sh

# single ad-hoc stage
docker run --rm -u "$(id -u):$(id -g)" --network shared-services \
  -v /opt/weather-wms/loadtest:/lt \
  -e SCENARIO_NAME=adhoc -e SUMMARY_PATH=/lt/results/adhoc.json \
  -e TARGET=direct -e RATE=50 -e DURATION=2m \
  grafana/k6:latest run /lt/scenarios/mixed.js
```

## Reports

- Files: `/opt/weather-wms/loadtest/results/<run>/report.{md,html,json}`
- Published (admin basic auth): `https://folkweather.com/loadtest-reports/`
  (latest nightly = `index.html`, latest probe = `probe.html`,
  past nightlies under `history/`). Note: `/loadtest` (no suffix) is the
  older Rust load-test dashboard proxied to wms-api.
- History/regression data: `results/{nightly,probe}-history.jsonl`

## Schedule control

```bash
systemctl list-timers 'loadtest-*'          # next run times
sudo systemctl stop loadtest-probe.timer    # pause probes
sudo systemctl start loadtest-nightly.service  # trigger nightly now
journalctl -u loadtest-nightly -n 100       # logs
```

## Interpreting breakpoints

Ramp stages increase arrival rate linearly until error rate (>5%) or p95
latency breaches limits, then abort — the report interpolates the achieved
RPS at abort. `dropped_iterations` > 0 also indicates saturation (the
system fell behind the arrival schedule). Ramps that complete without
breach mean capacity exceeds `MAX_RATE` — raise it next run.

## Notes / caveats

- The generator shares the NUC with the services; its CPU is visible in the
  docker-stats table (k6 itself is cheap, but not free).
- Ingest pipelines (MRMS every 2 min, GOES, etc.) run concurrently — this
  is deliberate: results reflect realistic production conditions.
- Never point these at `folkweather.com` (Cloudflare) — targets are LAN-only.
- wms-api exposes only cache/process metrics on `/metrics`; render
  histograms are JSON-only (`/api/metrics`) and not yet in Prometheus —
  candidate future improvement.
