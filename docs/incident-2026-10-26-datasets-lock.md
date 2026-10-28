# Incident 2026-10-26: `datasets` lock queue (~1 h, WMS down, EDR slow)

## Timeline (UTC)

- **~17:30** During a health check of the NUC, a diagnostic query (a per-model freshness
  report with a correlated subquery over `datasets`) was run through
  `ssh … docker exec psql`. The ssh command timed out client-side, but **the query kept
  running inside Postgres**, holding an `AccessShareLock` on `datasets`. Nothing bounded it
  server-side.
- **17:45** edr-api and wms-api were recreated to apply new cache sizes
  (`docker-compose.prod.yml`, see below). edr-api came up. **wms-api's startup migration**
  runs DDL on `datasets` (`ALTER TABLE … ADD COLUMN IF NOT EXISTS` and similar). That needs an
  `AccessExclusiveLock` even when it changes nothing, so it queued behind the diagnostic query.
- **17:45–18:44** Every other statement on `datasets` queued behind that waiting DDL. That
  covered edr-api reads, ingester `INSERT`s and trail-physics reads.
  - WMS returned **502** (wms-api never finished starting; autoheal restarted it every
    ~100 s).
  - EDR requests that touched `datasets` hung until Cloudflare's **524**.
  - Ingest throughput dropped from ~60 to 1–33 per 10 minutes. The downloader logged 21
    ingest-trigger timeouts; those files are retried by the cleanup task.
- **~18:44** The diagnostic query finished by itself, after about 70 minutes. The migration ran
  in milliseconds, wms-api started at 18:44:30, and the queue drained: 426 ingests completed
  in the next 10 minutes. No data was lost and no ingest failed.

## Causes

1. **An unbounded diagnostic query.** Client-side timeouts (ssh, `timeout`) do not stop a
   statement in Postgres. Every ad-hoc production query now sets a server-side bound first:
   `SET statement_timeout = '15s'` in the same session.
2. **Startup DDL can take the whole table hostage.** A migration that waits for an exclusive
   lock blocks every later reader and writer of that table, so one slow reader is enough to
   stall the system. *Follow-up:* run migrations with a short `lock_timeout` (fail and retry
   rather than queue everyone), or skip DDL when the schema is already current.

## The change being applied at the time

API cache sizing and the wms-api health check (`deploy/production/docker-compose.prod.yml`).
The change itself was fine and is live; it is described in the PR that adds this file.
