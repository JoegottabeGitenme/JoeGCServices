-- One-off deploy step for "Fix GFS isobaric levels: config was millibars, ingester reads pascals".
--
-- WHY
--   Before that fix the ingester compared gfs.yaml's millibar numbers with the GRIB2 level value,
--   which is in pascals, so the only isobaric messages it kept were the ones whose level is
--   1000 Pa = 10 mb (TMP and HGT), and it stored them as "1000 mb". Every catalog row of model
--   `gfs` whose level is "<n> mb" was written that way: its data is NOT the pressure level in its
--   name. The public gfs-isobaric collection returns ~30,700 gpm / ~226 K for z=1000.
--
--   Rolling out the fixed ingester does not repair them. The catalog upsert key is
--   (model, parameter, level, reference_time, forecast_hour, forecast_minute), so a row is only
--   overwritten when the same run and forecast hour are ingested again, and the downloader does
--   not do that. Until retention ages them out (gfs: 48 h) the stale rows keep being served as
--   real 1000 mb, and a run that is part-way through ingestion at the deploy shows 10 mb data
--   for its early hours and real 1000 mb data for the later ones under one label.
--
-- WHAT
--   Marks those rows status = 'expired'. Every read path filters on status = 'available', so they
--   stop being served at once (the WMS/EDR capabilities caches follow within their TTL, ~2 min).
--   The retention loop's next cycle then deletes their zarr objects and rows; no object-store
--   work is needed here. Rows for forecast hours the fixed ingester wrote are not touched.
--   The hours of an in-flight run that were ingested before the deploy simply have no upper-air
--   data (the downloader does not replay them); the next 6-hourly run is complete.
--
-- WHEN
--   After the fixed ingester is running and the old ingester pod is gone. :cutoff is the moment
--   the fixed ingester started (kubectl get pod <ingester pod> -o jsonpath='{.status.startTime}'),
--   so the rows it wrote itself, which are correct, are left alone. Running it before the old pod
--   is gone would miss the bad rows the old pod writes in the meantime.
--
-- HOW (dry run by default: it prints what would change, then rolls back)
--   psql "$DATABASE_URL" -v cutoff='2026-10-30 01:23:00+00' -f scripts/expire_mislabeled_gfs_isobaric.sql
--   psql "$DATABASE_URL" -v cutoff='2026-10-30 01:23:00+00' -v apply=1 -f scripts/expire_mislabeled_gfs_isobaric.sql
--
-- CHECK AFTERWARDS
--   SELECT status, count(*) FROM datasets WHERE model = 'gfs' AND level LIKE '% mb' GROUP BY 1;
--   z=1000 EDR HGT over a point should read ~0-300 gpm (not ~30,700), and z=500 ~5,000-5,900 gpm.
--
-- SCOPE
--   model = 'gfs' only. The only `gfs` level labels ending in " mb" are the isobaric ones
--   (the rest end in "ground", "surface", "atmosphere", "layer", "sea level"). HRRR's isobaric
--   rows were always correct and are not matched.

\set ON_ERROR_STOP on

\if :{?cutoff}
\else
  \echo 'cutoff is required: -v cutoff=''2026-10-30 01:23:00+00'' (the time the fixed ingester started)'
  \quit
\endif

BEGIN;

\echo
\echo 'GFS isobaric rows ingested before the cutoff (these hold mislabelled data):'
SELECT parameter, level, count(*) AS rows,
       min(reference_time) AS first_run, max(reference_time) AS last_run,
       pg_size_pretty(sum(file_size)) AS size
FROM datasets
WHERE model = 'gfs' AND level LIKE '% mb' AND status = 'available'
  AND ingested_at < :'cutoff'::timestamptz
GROUP BY parameter, level
ORDER BY parameter, level;

UPDATE datasets
SET status = 'expired'
WHERE model = 'gfs' AND level LIKE '% mb' AND status = 'available'
  AND ingested_at < :'cutoff'::timestamptz;

\echo
\echo 'GFS isobaric rows still available after the cutoff (written by the fixed ingester):'
SELECT parameter, count(DISTINCT level) AS levels, count(*) AS rows,
       min(reference_time) AS first_run, max(reference_time) AS last_run
FROM datasets
WHERE model = 'gfs' AND level LIKE '% mb' AND status = 'available'
GROUP BY parameter
ORDER BY parameter;

\if :{?apply}
  COMMIT;
  \echo
  \echo 'Committed: the rows above are expired; retention will delete their files and rows.'
\else
  ROLLBACK;
  \echo
  \echo 'Dry run, rolled back. Re-run with -v apply=1 to expire them.'
\endif
