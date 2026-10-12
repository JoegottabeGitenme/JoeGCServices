# HRRR 15-minute data (`hrrr-subh`)

Composite reflectivity, precipitation rate and precipitation type every 15 minutes, for the first
18 hours of every HRRR cycle (the most NCEP publishes). EDR only (no WMS layer).

## For the frontend

| Collection | Parameters | Notes |
|---|---|---|
| `hrrr-subh-atmosphere` | `REFC` | Composite reflectivity, dBZ. `-10` means no echo. |
| `hrrr-subh-surface` | `PRATE`, `CRAIN`, `CSNOW`, `CFRZR`, `CICEP` | `PRATE` is kg/m^2/s (multiply by 3600 for mm/h). The four type flags are `0`/`1`, read from the nearest cell, never blended. |

```
GET /edr/collections/hrrr-subh-atmosphere/position
      ?coords=POINT(-104.99 39.74)
      &parameter-name=REFC
      &datetime=2026-10-12T00:00:00Z/2026-10-12T06:00:00Z
```

returns one value per 15 minutes (`domain.axes.t.values` are the valid times). A single instant
(`datetime=2026-10-12T00:30:00Z`) returns that step. Where two cycles cover the same instant the
newer cycle wins (this is guaranteed by the catalog ordering, see
`docs/projected-grid-point-reads.md`), so a loop ending "now" is stitched from the latest run.

* Lead time 0-18 h of each cycle, so a 15-minute loop runs roughly 16-17 h ahead of now (the last file of a cycle appears 1 h 15 min to
  1 h 25 min after its cycle time). Past 18 h (out to
  48 h, synoptic cycles only) use `hrrr-atmosphere` (model `hrrr`), which
  is hourly.
* A cycle's 15-minute data appears about an hour after the cycle time, as HRRR publishes it.
* 12 hours of 15-minute data are kept, and at least the 3 latest cycles.
* The hourly `REFC` in `hrrr-atmosphere` is unchanged.

## What changed underneath

A `wrfsubhf` file holds four lead times (15/30/45/60 min after the previous hour) of every
parameter, which the platform could not represent:

* **Parser** read the forecast time without its unit (GRIB2 Code Table 4.4), so "15 min" meant 15
  hours. It now returns `lead_minutes` (and `forecast_hour = lead_minutes / 60`). Existing products are
  unchanged: HRRR hourly is unit 1 (hours), MRMS unit 0 with value 0.
* **Datasets** are keyed by `(model, parameter, level, reference_time, forecast_hour,
  forecast_minute)`. `forecast_minute` is `SMALLINT NOT NULL DEFAULT 0`, so every existing row keeps
  minute 0. `CatalogEntry::valid_time()` adds it. Lookups keyed on a whole forecast hour
  (`find_by_forecast_hour*`, `find_by_run_and_forecast_hour`, `get_run_forecast_hours`) mean minute 0.
* **Ingester** takes each message's own lead time for models that set
  `ingest: { lead_time_from_messages: true }` (only `hrrr-subh`). For every other model the octets
  mean something else (accumulation and average templates), so it stays off.
  Storage paths gain `m15`/`m30`/`m45` for non-zero minutes; hourly paths are unchanged.
* **Model name**: `hrrr-subh_*` files must not be filed under `hrrr` (`contains("hrrr")` would).
* **Downloader** reads `wrfsubhf00-18` (S3 mirror, selective download by `.idx`: about 2.8 MB of a
  ~230 MB file), with the cycle lookback from `docs/downloader-cycle-lookback.md`.

Cost, measured on production (Zarr per 15-minute step: REFC 869 kB, PRATE 406 kB, CRAIN 172 kB,
CSNOW 67 kB, CICEP 46 kB, CFRZR 44 kB, ~1.6 MB together): 73 steps x 6 parameters = 438 datasets and
~114 MB per cycle, so ~1.4 GB resident at 12 h retention (with the 3-cycle lookback each cycle is also
re-examined, but already-downloaded files cost no request). Download is ~2.5 MB per file, 19 files per
cycle, ~1.2 GB a day.

History: it started at 0-6 h (25 steps, 39 MB a cycle). Frames then went from 15 to 60 minutes apart
about 5 h ahead, which looked like a gap in an animation; 18 h is the most there is, since
`wrfsubhf19` is 404, and the 15 -> 60 minute step now happens at the end of what NCEP offers.

## Deploying / rolling back

The schema change runs from `Catalog::migrate()` when the ingester (or wms-api) starts. It is
idempotent and ordered so there is never a moment without a unique key:

1. `ALTER TABLE datasets ADD COLUMN IF NOT EXISTS forecast_minute SMALLINT NOT NULL DEFAULT 0` (instant
   on PostgreSQL 11+; production is 16.4)
2. `CREATE UNIQUE INDEX IF NOT EXISTS datasets_model_parameter_level_ref_fh_fm_key ON datasets(...,
   forecast_hour, forecast_minute)`
3. `ALTER TABLE datasets DROP CONSTRAINT IF EXISTS datasets_model_parameter_level_reference_time_forecast_hour_key`

**An older ingester cannot register datasets after step 3**: its `ON CONFLICT (..., forecast_hour)` no
longer matches a unique key, so it writes the grid and fails the catalog insert (now logged at
`WARN` as "Could not register dataset in the catalog"). Do not roll the ingester back without
restoring the old key first:

```sql
DELETE FROM datasets WHERE model = 'hrrr-subh';   -- minute rows would violate the old key
DROP INDEX IF EXISTS datasets_model_parameter_level_ref_fh_fm_key;
ALTER TABLE datasets ADD CONSTRAINT datasets_model_parameter_level_reference_time_forecast_hour_key
  UNIQUE (model, parameter, level, reference_time, forecast_hour);
```

(`forecast_minute` can stay; an old binary ignores it.)

Order used: back up `datasets`, sync configs, ship and recreate the ingester (migrates), then edr-api,
then restart the downloader (picks up the new model).
