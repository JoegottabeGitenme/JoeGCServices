# Notes: trail-conditions performance, retention and the batch endpoint (Oct 2026)

Frontend report: `/collections/trails/items?...&conditions=latest` was 10-100x slower
than the same request without conditions (1.7-3.3 s typical, 8.6 s / 14.7 s / 34 s
spikes, against 0.1-0.3 s), roughly flat in the number of trails. For the contract
see [trail-conditions-frontend.md](trail-conditions-frontend.md).

## Root cause (measured on production, not inferred)

Two compounding problems in `segment_conditions`:

1. **No retention.** Nothing ever deleted a row. 11.4M rows / 4.5 GB, growing about
   1.6M rows a day, while the API serves only the last 6 hours plus the forecast
   horizon.
2. **The wrong query shape.** `get_latest_for_features` was
   `SELECT DISTINCT ON (feature_id) ... ORDER BY feature_id, valid_time DESC, run_time DESC`.
   That reads and sorts *every* historical row of every requested trail to keep one
   each. For the 711-trail Boulder viewport: 233,550 rows sorted (spilling to temp
   files), ~63,000 buffers read, to return 711 rows. The cost followed the table's
   history, which is why it was flat in feature count.

The 8-34 s spikes were the hourly trail-physics pass. After each HRRR run it upserts
66,496 rows per forecast hour (up to 11 hours pending, ~70 s per hour, 731k rows per
pass) into the same bloated table, and requests that arrive during it queue behind that
I/O. Slow statements from unrelated queries (a plain `COUNT(*)` taking 14 s) showed the
whole database was busy at those moments.

## What changed

| | Before | After |
|---|---|---|
| `conditions=latest` query | sort of all history per trail | one index probe per trail (`LATERAL ... LIMIT 1`), ~4,200 buffers for the same 711 rows |
| `segment_conditions` rows | 11.4M | 3.3M (12 h of past, plus the forecast) |
| Dead tuples | 1.77M, growing | 0 after the first vacuum |

No new index was needed: the existing `(feature_id, valid_time)` serves the probe.

## Measured results (production, trail-physics idle)

| Request | Before | After |
|---|---|---|
| 1,494-trail viewport, `conditions=latest` (smoke script, median of 3) | 9.1 s (measured here) | 0.24 s (geometry-only 0.23 s) |
| Boulder 711 trails | 2.5 s typical, 8.6-34 s spikes (frontend's measurements) | 0.21-0.27 s warm, 0.9 s first request |
| Castle Rock 474 trails | 3.3 s cold, 0.38 s repeat (frontend's measurements) | 0.4 s first request, 0.18-0.21 s repeat |
| First request for a never-requested region (Evergreen, Longmont, Loveland) | n/a | 0.17-0.30 s |
| 40-id batch vs one single-trail call | (40 requests) | 0.16 s vs 0.09 s |
| 500-id batch | (500 requests) | 0.37 s |

`scripts/check_trails_api.py` now guards both: `conditions=latest` must stay within
max(2x, +0.75 s) of geometry-only, and a 40-id batch within max(4x, +1 s) of one single
call. Run against the pre-change production it reported 12 failures.

## Runbook

- **Retention** (`services/ingester/src/conditions_retention.rs`): every 10 minutes and
  at startup, rows with `valid_time` older than `TRAIL_CONDITIONS_RETENTION_HOURS` are
  deleted in 50k-row batches. Default 12 h; floor 8 h (the served history of 6 h plus 2),
  enforced at compile time against `TIMESERIES_HISTORY_HOURS`. Below the floor is raised
  with a warning; a non-integer falls back to the default. Future forecast hours are never
  touched. `trail_physics_progress` is a separate table, so deleting old rows does not make
  the worker reprocess old hours. The first sweep after the 9 Oct deploy removed 8.1M rows
  in about two minutes.
- **After a large delete, vacuum by hand:**
  `VACUUM (ANALYZE, PARALLEL 0) segment_conditions;` (about 3 minutes). `PARALLEL 0` because
  the Postgres container's 64 MB `/dev/shm` fails parallel vacuum with "No space left on
  device".
- **Disk is not reclaimed.** The heap is 1.6 GB for 3.3M rows, but the five indexes are
  still about 2.9 GB because vacuum frees pages for reuse without shrinking the files.
  `REINDEX INDEX CONCURRENTLY` on each would shrink them (it needs temporary extra space and
  can leave an invalid index if it fails). Not done: everything fits in the page cache, so
  the gain is marginal; revisit if the table's disk footprint matters.
- **If the worker stops for more than the window** the affected trails have no `latest` row
  and render uncolored. That is intended: it is better than presenting hours-old values as
  current. `latest_valid_time` / `last_worker_progress` show whether the worker is behind.
- **A tie-break was added to every read query** (`model_version DESC`).
  `UNIQUE(feature_id, valid_time, model_version)` allows two versions on one hour, and when
  they also share a `run_time` the old `ORDER BY` left the winner to the planner, so the
  single-trail and batch endpoints could have disagreed.

## Not done / queued

- **Edge caching.** Responses already send `Cache-Control: max-age=300`, but Cloudflare reports
  `cf-cache-status: DYNAMIC` because it does not cache JSON from an API path unless a Cache
  Rule tells it to. That is a Cloudflare dashboard setting (honour origin TTL on
  `/edr/collections/trails*`, query string in the cache key), not something in this repo.
  Deferred by request. An alternative is a micro-cache in the gateway nginx.
- **B4: MRMS QPE history.** `config/models/mrms.yaml` has one `retention.hours: 2` for the whole
  model, so QPE keeps only ~2 h. Fixing it means giving QPE its own retention (and a disk
  estimate per QPE grid first), because the same setting governs REFL/PrecipRate grids that
  arrive every ~2 minutes. Queued.
- The frontend's note that `radius` ignores `limit`/`class` was already fixed in `1c96b99` and is
  live (verified: `limit=3` returns 3, `class=mtb_trail` returns only that class).
