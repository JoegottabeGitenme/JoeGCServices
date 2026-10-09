# Incident: ingester OOM-killed ~2x/hour, silently (Oct 2026)

**Impact:** the ingester restarted every 14-45 minutes and dropped every
in-flight ingest each time. Some HRRR forecast hours never landed (the
trail-conditions series had gaps 28-36 h ahead). Nothing was user-visible as an
outage, and nothing alerted.
**Duration:** unknown start (the "~2 restarts/hour" pattern was already known);
found and fixed 2026-10-08.
**Data loss:** forecast hours / parameters whose ingest was in flight at a kill
(e.g. HRRR 12z run: hours 19, 21, 22, 38, 39, 42, 44 absent or partial).
**Lost hours are not re-ingested** (the downloader considers them done); the
trails series (newest run per valid time) only recovers because newer runs
cover the same valid times, so the gap at +28-36 h persists until the 18z
extended run reaches those hours. Runs ingested before the fix keep their
holes permanently.

---

## Symptom

`weather-wms-ingester-1` showed `RestartCount` climbing, health briefly
unhealthy, and the previous run's logs ending abruptly with no panic and no
shutdown message.

## Why it was hard to see

Every normal indicator said "not OOM":

| Indicator | Reading | Why it misleads |
|---|---|---|
| `docker inspect ... .State.OOMKilled` | `false` | The restart policy brings the container back; the flag reflects the container's final state, not each kill. |
| `docker inspect ... .State.ExitCode` | `0` | Not the kernel's SIGKILL status. |
| cgroup `memory.events` `oom_kill` | `0` | Read on the *restarted* cgroup instance. (`max` was non-zero: 235 hits.) |
| `docker events` | no `die`/`start`/`oom` events | Lifecycle events were not delivered on this host. |
| autoheal logs | silent about the ingester | It was not the one restarting it; `restart: unless-stopped` was. |

**The authoritative source is the kernel log**, readable from a privileged
container if the host log is not directly accessible:

```bash
docker run --rm --privileged --pid=host debian:bookworm-slim \
  sh -c 'dmesg -T | grep -i "killed process"'
```

On Oct 8 it showed kills at 15:45, 16:29, 16:43, 17:45, 17:59 and 18:30 UTC,
each `Memory cgroup out of memory: Killed process ... (ingester)` with
`anon-rss` ~8.2 GB, i.e. the 8 GiB container limit.

An earlier note in this investigation concluded "no OOM" from the first three
rows above. That was wrong.

## Root cause

A full-disk GOES band-2 file (`goes18/19-fulldisk`, CMI_C02, 250-430 MB on
disk) is 21696 x 21696 = 470M cells = **1.9 GB per f32 copy**.
`crates/ingestion/src/netcdf.rs::ingest_netcdf` held:

1. the file bytes (up to 430 MB),
2. `raw_data` (the decoded source grid, 1.9 GB),
3. `reprojected_data` (the same size again), and
4. `filtered_data` (a third copy made by a `.map().collect()`),

and nothing limited how many ingests ran concurrently. The downloader sends
several at once, especially when replaying a backlog after a restart. Measured
with a 2-second sampler of the container's `memory.current`:

| Phase of one full-disk band-2 ingest, running alone | Container memory |
|---|---|
| idle | 0.5-0.7 GB |
| parse | 4.5 GB |
| after reprojection (third copy) | **8.0 GB** (pinned at the limit) |

One ingest alone just fits; two overlapping ones are fatal. A restart then
re-triggers the backlog replay, which made the loop self-sustaining.

CONUS-sized GOES files and GRIB ingests are small by comparison (all 16 bands
x 2 satellites concurrently peaked at 2.6 GB).

## Fix (commit `a7a76ea`)

- At most **one file >= 100 MB** is ingested at a time (a static
  `tokio::sync::Semaphore`; smaller files never wait).
- File bytes are dropped after parsing and the raw grid after reprojection;
  `valid_range` masking is done in place. Two fewer full-size copies.
  `mask_out_of_range` is tested bit-for-bit against the previous clone+map
  implementation.
- Prod ingester memory limit 8 GiB -> 12 GiB
  (`deploy/production/docker-compose.prod.yml`) for headroom. Host: 31 GiB
  total, ~23 GiB available.

## Verification

Deployed 19:01 UTC Oct 8. Over the next ~1h48m: `RestartCount` stayed 0, the
last kernel-log kill remained the 18:30 one, 47 full-disk band-2 ingests ran,
and container memory peaked at 6.8 GB within the 12 GiB limit (it was pinned
at 8.0 GB of 8 GiB before).

Tests: the serialization test fails if the gate is disabled; the equivalence
test fails if the mask condition is changed (both mutations were applied and
confirmed to fail, then reverted).

## Follow-ups

- **Nothing alerted.** There is no Alertmanager, so Prometheus alert rules
  notify no one (see `deploy/prometheus/alerts.yml`). A container-restart or
  memory-pressure alert would have caught this on day one, but needs a
  notification path first (and a metric source for container restarts, which
  this stack does not currently export).
- The remaining headroom is not unlimited: a full-disk band-2 ingest still
  peaks near 5-7 GB. If more full-disk bands or a higher-resolution product
  are added, re-measure.
- The downloader replays ~687k leftover records at startup; autoheal kills it
  once while it does so. Separate, pre-existing.
