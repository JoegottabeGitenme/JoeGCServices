# Production probe, 2026-10-25

A read-only sweep of the NUC (containers, logs, the data pipeline, and about 250 live requests
against folkweather.com). Nothing was crash-looping or OOM-killed, every container was healthy,
none of the live requests returned a 5xx, and 16 of 19 models were current. Three things were broken
outright. All three are fixed below.

## Fixed

### GFS-Wave had never stored a dataset

The downloader had fetched 3,612 GFS-Wave files since Oct 18 and marked every one ingested. The
catalog held zero `gfswave` rows, the `gfswave_HTSGW` WMS layer failed every request, and the
`gfswave-surface` EDR collection was empty. There were two bugs, one behind the other:

1. **Model detection.** Files are saved as `gfswave_{date}_{cycle}z_f{hour}.grib2`.
   `extract_model_from_filename` had no wave branch, so `contains("gfs")` read them as `gfs`. The
   GFS parameter table has no oceanographic (discipline 10) entries, so nothing matched, and the
   ingest "succeeded" with zero datasets.
2. **Decoding.** GFS-Wave is JPEG2000-packed (data representation template 5.40). The `grib` crate
   was built with PNG unpacking only, so every wave message failed with `NotSupported(... 40)`.
   This bug stayed hidden because bug 1 kept the messages from ever reaching the decoder.

Fixes:

- A `gfswave` branch ahead of `gfs` (`crates/ingestion/src/metadata.rs`).
- The `jpeg2000-unpack-with-openjpeg` feature on `grib` (workspace `Cargo.toml`). openjpeg is
  compiled from bundled source and linked statically, so the runtime images need nothing new.
- A WARN, `GRIB2 file registered no datasets`, whenever a GRIB2 file produces nothing. Downloads
  are selective, so an empty result means the file was misunderstood, not that it was legitimately
  empty.

Tests:

- `metadata::tests::every_forecast_model_is_recognised_from_its_downloaded_file_name` builds the
  downloader's file name for every `forecast` model in `config/models` (including `file_types`
  variants) and requires it to resolve to that model. Before the fix it failed with
  `gfswave_… → gfs`.
- `crates/ingestion/tests/gfswave_ingest.rs` (Docker) ingests a real 4-message cut of
  `gfswave.t12z.global.0p25.f006.grib2` and requires HTSGW, PERPW, DIRPW and WVHGT under
  `gfswave`. It fails without the filename branch (`gfs`) and without JPEG2000 (no parameters).
- Decoded values match `wgrib2 -lon` exactly:
  - (220E, 30N): HTSGW 2.11 m, PERPW 12.6 s, DIRPW 292.32°.
  - (300E, 40N): 2.81 m, 10.4 s, 42.68°.
  - Land (255E, 40N): undefined in the file, NaN after decoding.

### Storm-events batches were refused with 413

The downloader posts 2,000 events per batch, each with both narratives and the raw CSV row, so a
batch runs to several MB. `/ingest/storm-events` had axum's 2 MB default body limit; only
`/ingest/lightning` had been raised. The result was 24 × `413 Payload Too Large` in the first
14 minutes after a restart, and the affected years never loaded. Failed years are never marked
complete, so the next cycle retries them.

- Fix: `DefaultBodyLimit::max(32 MiB)` on the route (`services/ingester/src/server.rs`).
- Test: `storm_events_http_tests::a_full_batch_of_storm_events_is_accepted_and_stored` (Docker)
  posts a 2,000-event batch of more than 4 MB and requires all 2,000 rows. Without the limit it
  gets 413.

### The nginx access log recorded nothing

The live gateway config (`/opt/gateway/nginx.conf` on the NUC) is maintained by hand, not
generated from `deploy/production/nginx/nginx.conf.template`. Its `log_format main` had lost every
`$variable`, so each line was `-[]rt=`. It was already like that in the May 28 backup. No other
directive was affected: proxy headers, the client-IP map and rate limiting were intact.

- Fix: the three `log_format` lines were restored from the template.
- Before writing the live file, the new config passed `nginx -t` inside the gateway container.
- The live file was overwritten in place: it is a single-file bind mount, so a new inode would
  not be seen by the container.
- nginx was then reloaded.
- Backup: `/opt/gateway/nginx.conf.bak.1792963751`.

### The hourly sync deleted freshly ingested data

Found while verifying the GFS-Wave fix. The DB/MinIO sync (`crates/retention`, run hourly in the
ingester) deletes catalog rows whose objects are missing from a MinIO listing, and gave rows a
15-minute grace measured back from the DB snapshot. But the listing itself took **18 min 19 s**
(953k objects). Rows written after the lister had passed their prefix were therefore missing
from the listing and older than the grace by the time of the snapshot. They were deleted with
their objects intact, and the next sync deleted the objects too, as unreferenced. Every model
was exposed, and the window grows with the bucket.

Evidence: the sync ran from 21:36:34 to 21:55:05. It deleted all 10 `gfswave` rows ingested at
21:37:33 as "orphan DB records", while `grids/gfswave/20261025_12z/*.zarr` still existed in MinIO.
The 1,545 "orphan" rows deleted at 20:52 were probably largely the same race.

- Fix: the grace is measured back from when the listing **started**
  (`find_orphans` in `crates/retention/src/lib.rs`, now a pure function). A row ingested after
  `listing_started - grace` is never an orphan, however long the listing takes. The DB snapshot
  is one query, `Catalog::get_storage_paths_with_ingested_at`, which returns each path with its
  newest `ingested_at`. The listing duration is logged (`listing_secs`).
- Tests:
  - `find_orphans_never_deletes_a_row_written_after_the_listing_started` replays the production
    timings. With the old rule it also flags the gfswave row and a row ingested just before the
    listing.
  - `storage_paths_come_with_their_newest_ingest_time` (Docker) checks the query. With `MIN` in
    place of `MAX` it fails.

## Found, not fixed

- **Host load.** Load average was 18-20 on 20 cores, with 8.8 GB of swap in use. Slow queries are
  chronic, mainly `MIN/MAX(valid_time)` and `DISTINCT valid_time` on `datasets` from edr-api
  (hundreds per hour) and the observation inserts. Autoheal restarted postgres (09:00Z), wms-api
  (10:50Z) and loki (06:35Z) after failed health checks.
- **Intermittent observation uploads.** NDBC failed about every 10 minutes, and DART, TAF and
  METAR occasionally. The failures cluster around the ingester's 10-minute observation-retention
  sweep. The next poll recovers.
- **DART retention churn.** Each sweep deletes thousands of DART rows against a 1,440 h cutoff,
  which suggests rows are re-ingested repeatedly. Not verified.
- **Disk.** 217 GB free (75%). Free space fell about 50 GB in the hour after the ingest-cancel fix,
  because runs now ingest completely; it looks to be levelling off. Also: 86 dangling images, and
  container logs are not rotated (no `daemon.json`, no compose `logging:`).
- **Cold first requests.** The first request to some endpoints takes 3-15 s (`populated/radius`
  14.8 s, `ndbc/radius` 8 s, GOES/AIGEFS positions 4-8 s); warm, 0.05-0.6 s.
  `hrrr-surface/radius` with no datetime returns 26 MB.
- **Small API defects.** `astro` lists parameters that `parameter-name=` rejects. Soil collections
  return nulls at a Colorado point.
- **Noise.** NLDAS still 404s on every file (about 64k requests a day), Postgres collation-version
  warnings in every service, Overpass 502s in the trail sync.
