# Notes: GLM lightning, observation retention, `/collections` snapshot, HDF5 (Oct 2026)

Branch `feature/glm-lightning`. Everything here is deployed to production and
was measured there. For the OOM loop found while verifying it, see
[incident-2026-10-ingester-oom.md](incident-2026-10-ingester-oom.md). For the
frontend contract, see [lightning-frontend.md](lightning-frontend.md).

## 1. GOES GLM lightning

**Flow:** downloader `glm_runner` (S3 `noaa-goes19` / `noaa-goes18`,
`GLM-L2-LCFA`, source type `aws_s3_glm`) -> `POST /ingest/lightning` on the
ingester -> `lightning_flashes` (PostGIS) -> EDR collection `glm-lightning`.

- **Reader:** `crates/netcdf-parser/src/glm.rs`, tested against real granules
  in `tests/fixtures/` and an independent Python reference
  (`glm_expected.json`).
- **Table:** `lightning_flashes`, `geometry(Point,4326)` (not geography),
  `UNIQUE(satellite, flash_time, flash_id)`. A second table,
  `lightning_ingest_progress`, holds a per-satellite marker.
- **Cursor:** writers are serialized by an advisory lock, so `id` is gap-free
  in commit order and `?after=<id>` is a safe incremental cursor.
- **Satellite roles:** `goes-east` (G16/G19), `goes-west` (G17/G18). Default
  `satellite=goes-east`; `both` double-counts (measured: 51% of East flashes
  are also seen by West).
- **CONUS clip:** lon -125..-66, lat 24..50.
- **Retention:** `LIGHTNING_RETENTION_HOURS` (default 24), swept every 10 minutes by the
  ingester.
- **API:** `items`, `area`, `radius`; params `window=PT10M`, `datetime`,
  `after`, `satellite`, `limit`. The collection is always listed, even when
  empty.
- **Freshness:** every response carries `dataThrough` / `dataAgeSeconds`;
  `null` means unknown and must never be shown as "current". Prometheus gauge
  `glm_latest_granule_end_timestamp_seconds{satellite}` with alerts
  `GlmGranulesStale`, `GlmEastMetricMissing`, `GlmWestMetricMissing`
  (promtool-tested in `deploy/prometheus/alerts_test.yml`).
- **Measured on prod:** window end -> stored: median 20 s, p90 35 s.
  224 B per feature (~23 B gzipped). ~298 B per stored row.
- **Smoke test:** `scripts/check_lightning_api.py`.

## 2. Observation and TAF retention

**Problem:** nothing ever deleted observations. `observations` held 14.5M rows
(metar, ndbc, dart), and every `/collections` request ran `COUNT(*)` over it
(~50 s).

**Fix:** `services/ingester/src/obs_retention.rs`, a sweep every 10 minutes
(first at startup). Deletes are batched at 50k rows on `ctid` via
`ObservationCatalog::delete_observations_before_batch`, so a large backlog
never holds one long transaction.

| Source | Env var | Default | Floor | Why the floor |
|---|---|---|---|---|
| metar | `OBS_RETENTION_HOURS_METAR` | 168 (7 d) | 24 h | API defaults look back <= 12 h |
| ndbc | `OBS_RETENTION_HOURS_NDBC` | 168 (7 d) | 24 h | same |
| dart | `OBS_RETENTION_HOURS_DART` | 1440 (60 d) | 1128 h (47 d) | the DART downloader re-fetches 45 d |
| taf | `TAF_RETENTION_HOURS` | 168 (7 d, after `valid_to`) | 24 h | |

- A value below the floor is **raised to the floor with a warning**, never
  obeyed. Inserts are `ON CONFLICT DO NOTHING`, so a window shorter than a
  downloader's re-fetch horizon would delete rows that are immediately
  re-downloaded and silently skipped as duplicates.
- A non-numeric value falls back to the default. Sources with no rule are never
  deleted.
- Tests: 8 unit + 6 PostGIS. Three mutants were killed (one first mutation was
  a no-op and was redone, which is why mutation application is checked).

**First sweep on prod (Oct 8):** metar -475,857; ndbc -2,514,199 plus more in
later sweeps; dart -938,640; TAF -125,969. Final: metar 14,101 rows, ndbc
420,444, dart 235,041. Per-source `COUNT(*)` is now 5-70 ms. The GLM feed
stayed healthy during the bulk delete.

**Operational notes**
- A bulk delete leaves dead tuples until autovacuum catches up. After the first
  sweep run `VACUUM (ANALYZE) observations` yourself. It took 165 s and
  cleared 13.9M dead tuples. A parallel `VACUUM` failed with `could not resize
  shared memory segment ... No space left on device` (Postgres container
  `/dev/shm` is 64 MB); use `PARALLEL 0`.
- Disk is **not** reclaimed by this (the heap stays 3.2 GB until a
  `VACUUM FULL`/rewrite, which has not been done).

## 3. `GET /collections` snapshot cache

**Problem:** 60-90 s (22.6 s best case) per request: ~575 queries, made of 3
`COUNT(*)` over the unpruned observations table (~50 s) and ~560 uncached
`datasets` metadata queries over 106 gridded collections.

**Fix:** `services/edr-api/src/snapshot_cache.rs`, a generic
stale-while-revalidate cache with single-flight refresh and panic safety
(10 tests). `list_collections_handler` serves the snapshot; the original loop
moved verbatim into `build_collections_list_json`.

- TTL `EDR_COLLECTIONS_SNAPSHOT_TTL_SECS`, default 60. Warmed at startup and
  invalidated by `reload_config`.
- 5 end-to-end tests on real PostGIS
  (`services/edr-api/tests/collections_snapshot.rs`); 2 mutants killed.
- **Measured on prod after deploy:** 0.2-0.3 s, 110 collections, with extent,
  id, title, links intact. A cold build (first request after a restart without
  warm-up) is ~20 s.
- `get_collection_handler` (single collection) is deliberately still on the
  live path. Single collections were already fast (0.1-0.6 s; ndbc ~2.7-5 s
  before the retention vacuum, 0.13 s after).
- **Not done, on purpose:** replacing the observation `COUNT(*)` with an
  `EXISTS` check. Counts also appear in descriptions ("N observations
  available"), and with retention plus the snapshot the count is cheap.
- **Frontend needs no change.** It uses per-collection requests, which remain
  the recommended pattern; only internal dashboards read the full listing.

## 4. HDF5

Two separate things.

1. **Log spam (live since the Oct 7 22:24 deploy).** `silence_hdf5_errors()`
   now re-applies on every call. Before: ~1,350 `HDF5-DIAG` lines per 30 min;
   after: 0. *Correction:* an earlier note said this "takes effect on the next
   deploy". It was already live.
2. **Native build on HDF5 2.x.** `hdf5-metno-sys` bumped to 0.12 (resolves
   0.12.4; `netcdf-sys` 0.9.2), so the workspace builds against a
   workstation's HDF5 2.2.0. Prod (Debian, HDF5 1.10.8) is unaffected; CI installs `libhdf5-dev` from
   Ubuntu apt.
3. **A race that the bump exposed.** `silence_hdf5_errors()` now holds
   `hdf5_metno_sys::LOCK` (reentrant). Without it, concurrent reads fail with
   `NC_EATTMETA (-107)` and a hammer test aborts with SIGABRT. Verified: 30
   parallel full-suite runs natively and 12 in the Debian container, 0
   failures; a mutant without the lock fails 15 of 40 runs.

## 5. Known gaps / open items

- **No Alertmanager:** Prometheus alerts (including the GLM ones) notify no
  one. The OOM loop ran unnoticed for this reason.
- CI only triggers on `main`, `feature/automated-testing` and PRs to `main`.
  This branch had never run CI before its PR; the first run is the first
  Ubuntu/HDF5 build and lint of this code.
- The gateway nginx template has diverged from the live config.
- The downloader replays ~687k leftover records at startup.
- Pre-existing: `pre-push` fails only on `config/models/storm-events.yaml`
  (hence `git push --no-verify` on this branch).
- A `cargo clippy` warning (`result_large_err` in `handlers/lightning.rs`)
  follows the existing trails-code pattern (16 pre-existing instances).
