# Ingest requests that time out no longer lose data

## What was wrong

The downloader asks the ingester to ingest each file with `POST /ingest`, and the ingester replies
only once every message in the file is written and registered. The downloader's HTTP client gave up
after **30 s**. The ingester did the work inside the request handler, and axum drops a handler when
its client goes away. So any ingest slower than 30 s stopped partway through the file. The messages
before the cut were registered, the rest were silently missing, and the tracker entry stayed
"processing" for good.

The cleanup task retried pending files with the same 30 s timeout. A slow file could be cut off on
every attempt, or before it got through its first message.

Measured on production (2026-10-25):

- **1,442** `Failed to trigger ingestion … operation timed out` in 24 h, across 15 models (hrrr
  329, goes18-fulldisk 166, aigefs 150, goes19-fulldisk 150, aigfs 145, nbm-conus 106, gfs 102,
  hrrr-subh 57, …).
- Hourly HRRR runs held 1,184–1,425 of ~1,425 datasets. The 16Z run was short by ~17%.
- hrrr-subh 16Z lacked PRATE/CRAIN/CSNOW/CICEP/CFRZR at +16 h. The downloader timed out at
  17:28:15. The ingester's last write for that file was REFC +16 h at 17:28:13, and its next message
  never arrived.
- `GET /status` on the ingester listed **24,700** "active" ingests, all left open by cancelled
  requests since the container started on Oct 12.
- Ingest durations: p50 ~6 s, and frequently up to and past 30 s when several models land at once.

## The fix

- **ingester** (`services/ingester/src/server.rs`): `ingest_handler` runs the ingest and its tracker
  bookkeeping in a `tokio::spawn`ed task and awaits it. A client hanging up drops only the wait;
  the file is still finished and the tracker entry is closed. A panic in the task now becomes a 500
  and a closed tracker entry instead of a reset connection.
- **downloader** (`INGEST_REQUEST_TIMEOUT` in `services/downloader/src/cleanup.rs`): the three
  `POST /ingest` calls (the model runner's two paths and the cleanup retry) wait up to 600 s. The
  shared client keeps its 30 s timeout for HEAD probes. A call that still times out loses nothing
  now; the file just stays pending and the retry ingests it again (an idempotent upsert).

Keeping the long timeout matters for load too. The downloader awaits each ingest, so its per-model
`max_concurrent` still bounds how many ingests run at once. With a short timeout plus detached
work, abandoned ingests would pile up behind new ones.

## Tests

- `server::cancel_tests::an_ingest_whose_client_hangs_up_still_registers_the_whole_file` (Docker,
  `cargo test -p ingester -- --ignored cancel_tests`). This test ingests the real 24-message
  `wrfsubhf01` fixture. It drops the request the moment the first dataset row appears, then
  requires all 24 rows and a closed tracker entry. With the work back inline in the handler it fails
  with "1 of 24 datasets", the same shape as production.
- `cleanup::tests::a_retry_waits_for_an_ingest_slower_than_thirty_seconds` (`--ignored`, 31 s).
  The mock ingester answers after 31 s, and the retry must succeed. Without the per-request timeout
  it fails.

## Deploy

Rebuild and recreate **ingester** and **downloader**; no config or schema change. Order doesn't
matter. Files still pending (inside `pending_ingestion_max_age_secs`) are re-ingested by the next
cleanup pass. Runs that were already cut short and whose files are gone stay incomplete until
retention rolls them off.
