# MRMS QPE: 72 hours of hourly precipitation (the past-rain curve)

Frontend request "B4": an hourly past-rain curve at a point (e.g. a trail). The EDR
`position` endpoint already returned point series over a datetime range; what was missing was
the history, and the QPE hours themselves.

## For the frontend

One request returns the whole curve:

```
GET https://folkweather.com/edr/collections/mrms-qpe/position
      ?coords=POINT(-122.3 47.6)
      &parameter-name=QPE_01H
      &datetime=2026-10-07T00:00:00Z/2026-10-10T02:00:00Z
```

Response: CoverageJSON `PointSeries`. `domain.axes.t.values` are the hours (ISO 8601, on the hour),
`ranges.QPE_01H.values` are millimetres of rain **in that hour**, same order. Add
`QPE_24H` / `QPE_72H` (comma-separated `parameter-name`) for the rolling totals.

- **History:** 72 hours of hourly grids, refreshed every 10 minutes. The newest hour is normally
  1-3 hours behind "now" (NOAA publishes each hour about an hour after it ends).
- **`null` means "no data for that hour"**, not zero and not a repeat of a neighbouring hour.
  A real zero is `0.0`. Expect a few nulls at most: see "Where the data comes from".
- **Hours with no data for any QPE product are simply absent from the time axis** (NOAA published
  nothing for 2026-10-07 15:00Z, for example). Plot by timestamp, not by index.
- **`QPE_24H` / `QPE_72H` are rolling totals** ending at that hour, so they legitimately stay
  identical from one hour to the next when neither the hour entering nor the hour leaving the window
  had rain. Only `QPE_01H` is a per-hour amount.
- **Cost, measured on production:** the full 72 h series for one point is 0.4-0.6 s warm (4-7 KB),
  1-10 s when the underlying grid chunks are cold; ~26 hours is 0.2 s.
- **Instances:** `GET /collections/mrms-qpe/instances` lists the available hours (about 70 of 72 in
  steady state).
- `mrms-qpe-latest` is the same data for "most recent value" queries, unchanged.

### WMS / WMTS layer names changed

QPE is its own model now, and layer names are `{model}_{parameter}`:

| Before | Now |
|---|---|
| `mrms_QPE_01H` | `mrms-qpe_QPE_01H` |
| `mrms_QPE_24H` | `mrms-qpe_QPE_24H` |

The old names return a clean `LayerNotDefined`. Both layers have a `TIME` dimension with the hourly
timestamps (72 h). The radar layers (`mrms_REFL`, `mrms_PRECIP_RATE`) are unchanged.
The EDR collection ids (`mrms-qpe`, `mrms-qpe-latest`) are unchanged, and
`mrms-single-level(-latest)` no longer list QPE parameters, because their model (`mrms`) no longer
ingests them (asking for them would only ever have returned nothing).

## Where the data comes from

| Product | Upstream files per day | Arrives |
|---|---|---|
| QPE_24H, QPE_72H (Pass2) | 24 | ~1 h after the hour |
| QPE_01H Pass2 (gauge-corrected) | 18-24 (drops 2-3 hours a day; was 5+ hours late on 9 Oct) | ~1 h when present |
| QPE_01H Pass1 | 23-24 | ~20 min after the hour |

`QPE_01H` prefers Pass2. For an hour whose Pass2 file is still missing **90 minutes** after its nominal
time, the Pass1 file is used instead (Pass1 is nearly complete but uses fewer gauges). If Pass2 turns up
later it **replaces** the Pass1 grid for that hour. Each stored grid records its upstream file in its
catalog metadata:

```sql
SELECT reference_time, zarr_metadata->>'source_file'
FROM datasets WHERE model = 'mrms-qpe' AND parameter = 'QPE_01H' ORDER BY 1 DESC;
```

At the time of writing 9 of 67 stored QPE_01H hours are Pass1.

## Why a separate model (for maintainers)

Retention is per model, and for observation models `retention.hours` is *also* how far back the
downloader looks on startup. One `mrms` model therefore could not keep "2 hours of 2-minute radar grids"
and "72 hours of hourly grids". `config/models/mrms.yaml` keeps REFL and PRECIP_RATE at 2 h;
`config/models/mrms-qpe.yaml` holds the QPE products at 72 h. The two numbers are equal on purpose: if
cleanup kept less than the downloader fetches, every poll would re-download what cleanup deleted (the
DART problem in `obs_retention.rs`). `keep_latest_observations: 24` protects the newest day from expiry
if the feed stalls.

Storage: 67-69 grids per product, QPE_01H 2.9 MB, QPE_24H 12 MB, QPE_72H 22 MB each, **~2.5 GB
total** (the radar model's 143 grids are 0.4 GB).

### Things that would have bitten any window over ~48 h (fixed)

- The downloader listed only "today" and "the earliest day" of its window, so the days in between
  were never fetched. It now lists every date.
- The S3 `StartAfter` key for full days was `...-000000`, which is exclusive and so skipped each day's
  00:00 file. Full days now list from the top; the earliest day starts one second early.
- Pass1 and Pass2 hourly QPE use different GRIB2 parameter numbers (30 and 37). Without the
  `fallback_grib2` mapping in `mrms-qpe.yaml` a Pass1 file ingests as nothing.
- `model == "mrms"` checks elsewhere (point sampling for GetFeatureInfo, the "no isolines on
  radar" checks, the ingestion bbox table, storage paths) now recognise `mrms-qpe`. Missing the
  sampler would have read the wrong grid cells through the global lat/lon fallback.

### A series point is null, not a neighbour's value

A point series asks every requested parameter for every time *any* parameter has. The catalog's
`find_by_time` returns the **nearest** grid with no tolerance, so a parameter with no grid at an hour used
to repeat the neighbouring hour's value under this hour's timestamp, and a repeated rainfall total looks
like real rain. `value_at_instant` (edr-api `position.rs`) now returns `null` when the grid that answered
is not the requested instant's. It applies to observation data only (for forecast grids, `time` is the run
time) and to series; a single-instant query still gets the nearest grid.

## Deploying / re-deploying this (runbook)

The first rollout had two operational surprises worth knowing.

1. **The downloader remembers completed URLs.** `completed_downloads` (SQLite in the `downloader_state`
   volume) is keyed by URL, and the old `mrms` model had recorded every QPE URL of the past week as done.
   The new model would have skipped its entire backfill for those products. Before the first start of
   `mrms-qpe`, clear them:
   ```bash
   docker compose stop downloader
   docker run --rm -v weather-wms_downloader_state:/d python:3.11-slim python -c "
   import sqlite3; c=sqlite3.connect('/d/downloads.db', timeout=60)
   with c:
       c.execute(\"delete from completed_downloads where url like '%MultiSensor_QPE_%'\")
       c.execute(\"delete from downloads where url like '%MultiSensor_QPE_%'\")"
   docker compose up -d downloader
   ```
2. **Downloader startup vs. autoheal.** At startup the downloader first retries every file that finished
   downloading 5 min-2 h ago but was never confirmed ingested, one at a time and *before* it serves
   `/health` or starts polling. After an ingester restart that set can be dozens of 250-430 MB
   full-disk GOES frames (~1 min each), longer than autoheal's health window, so autoheal restarts the
   container and the retry starts over, forever. All downloads are paused meanwhile. The workaround used:
   delete those rows (and files) from `completed_downloads` so normal polling re-fetches them. A proper fix
   is to run the startup cleanup in the background; it is not done here.
3. Expect `database is locked` warnings in the downloader log during a large backfill (SQLite
   contention in its 340 MB state file). A download that exhausts its 5 retries is just listed again on the
   next 10-minute poll.

## Checking it

`python3 scripts/check_mrms_qpe.py` (defaults to production) asserts the collections, 72 h of hourly
instances current within 4 h, a one-request point series on whole hours with at most 4 missing, no
repeated non-zero hourly totals, and the renamed WMS layers rendering. Its pure checks are tested in
`scripts/test_check_mrms_qpe.py`.
