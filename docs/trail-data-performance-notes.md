# Notes: trail data performance, round 2 (Oct 2026)

Round 1 ([trail-conditions-performance-notes.md](trail-conditions-performance-notes.md)) fixed
`conditions=latest` and added the batch endpoint. This round covers what feeds and serves the data
around it: the QPE past-rain series and the downloader that brings QPE (and everything else) in.
Every number below was measured on production.

## 1. The QPE series: one catalog query, concurrent grid reads

`GET /edr/collections/mrms-qpe/position?...&datetime=start/end` asked the catalog for each hour in turn
(`find_by_time`: "the dataset nearest this instant") and read each grid in turn, every read blocking a
runtime worker for the S3 fetch and chunk decode. A 72-hour series was 72 sequential Postgres round trips
plus 72 sequential blocking reads.

| 72 h series, one point (QPE_01H) | Before | After |
|---|---|---|
| First request at a never-queried point | 4.2 s median (3.8-7.9 s) | **0.36 s** median (0.33-0.78 s) |
| Repeat | 0.41 s median | **0.20 s** median |
| Catalog lookups per request | 70 | **1** |

The "after" cold numbers were taken straight after an edr-api restart, i.e. with an empty chunk cache, so
they are conservative; the earlier worst case (19.8 s after a restart) was the same mechanism. The four
response bodies were compared with the pre-change responses and are **identical**.

How it works: `Catalog::find_datasets_in_valid_time_range` lists the candidate datasets once;
`GridDataService::read_point_series` matches each requested instant to the dataset valid at it (within 1 s,
`match_series_times`, a binary search) and reads each distinct grid on its own task, at most 8 at a time,
returning results in request order. An instant with no dataset is `NotFound`, which the handler already
turned into `null` (the per-step form returned the nearest dataset and relied on `value_at_instant` to
discard it, so results are unchanged).

Applies to observation collections (QPE, radar, satellite) on non-instance queries. Forecast models and
instance (`/instances/{id}/position`) queries keep the per-step path: their "which dataset" rule is not
simply "valid at this instant".

`GridDataService::catalog_lookups()` counts lookups so a test (or a metric) can see that a series is one.

## 2. The downloader

Read-only investigation first (Oct 10): the state DB held **694k rows, 693,678 of them `pending`**, 98% from
NLDAS (every NLDAS URL has been 404ing, a separate issue, not addressed here); the data volume held **1,314
files / 40 GB**, some from August; and a restart paused every feed for ~7 minutes while autoheal kept killing
the container. Four causes, all fixed:

| Problem | Cause | Fix |
|---|---|---|
| Restart blocks every feed; autoheal kill loop | `run_startup_cleanup` ran inline before `/health` and the scheduler, re-POSTing each unconfirmed file to the ingester one at a time (dozens of 420 MB GOES frames, ~1 min each) | In continuous mode the background cleanup task (its first tick is immediate) does it; `--once` still runs it inline. Healthy **6 s** after start. |
| 694k dead rows | `queue_download` is `INSERT OR IGNORE` and only `failed` rows were ever pruned | Periodic pruning of pending / in_progress / retrying rows untouched for `STALE_UNFINISHED_RETENTION_DAYS` (default 7, floored at 1), in 20k-row batches |
| `database is locked` | Rollback-journal mode (sqlx leaves the journal alone unless asked; `delete` verified on the prod file) and sqlx's 5 s busy timeout | WAL, `synchronous=NORMAL`, 30 s busy timeout |
| 40 GB of files nothing deletes | `cleanup_orphan_files` only recognises a file via a `completed_downloads` row with `ingested=1`; those rows are pruned after 7 days | New sweep: files **no row refers to**, older than `UNREFERENCED_FILE_MIN_AGE_SECS` (default 48 h, never below 6 h; unknown age counts as young), honours `--cleanup-dry-run` |

The periodic cleanup runs its cheap local steps first (partial files, ingested orphans, row pruning, the
unreferenced-file sweep: the sweep after the pruning, so a file whose only row was just pruned goes in the same
pass) and the slow ingest retries **last**, so a slow ingester can no longer postpone freeing disk. The first
production pass found this the hard way: 45 retries ahead of the cleanup would have taken about 45 minutes.

First production pass after deploy (about 25 seconds):

| | Before | After |
|---|---|---|
| `downloads` rows | ~694,400 | ~20,650 (NLDAS URLs re-queued by the next poll; they age out again after 7 days) |
| State DB file | 345 MB | 50 MB (+ a WAL that grew to 244 MB during the big delete) |
| Data volume | 1,324 files / 42.9 GB | 254 files / 17.1 GB (1,084 files / 27.9 GB deleted) |
| `database is locked` in 3 min | 16 per 10 min during a backfill | 0 |
| Autoheal actions on the downloader | 4 in the 4 h around the previous deploy | 0 in the following 30 min |
| `/status` | (not timed before; it loaded every unfinished row per call) | 0.49 s |

Not changed: the WAL file keeps its high-water size until it is truncated (`journal_size_limit` is not set),
so `downloads.db-wal` stays a few hundred MB for now. It is reused, not growing.

### Settings

| Env var | Default | Meaning |
|---|---|---|
| `STALE_UNFINISHED_RETENTION_DAYS` | 7 (min 1) | forget unfinished downloads untouched this long |
| `UNREFERENCED_FILE_MIN_AGE_SECS` | 172800 (min 21600) | delete unreferenced files this old |
| `CLEANUP_INTERVAL_SECS` | 3600 | how often the periodic cleanup runs (existing) |

## 3. Not done

- **NLDAS** still 404s (1,442 failures in 30 minutes, i.e. roughly 70k requests a day to NASA). Its rows now age out and no longer block anything,
  but the layers are a week stale. Needs its own investigation.
- **Edge caching** (Cloudflare Cache Rule) and **a QPE_01H gap or two** that depends on NOAA publishing.
