# Incident: disk exhaustion → total API outage (Aug 2026)

**Impact:** all public endpoints returned HTTP 502 (WMS, WMTS, EDR).
**Duration:** Postgres died 2026-07-28 22:27 UTC; detected and resolved
2026-08-05 (~8 days of degraded/failed service).
**Data loss:** none.

---

## Symptom

`GET /wms?...&LAYERS=gfs_TMP&...` → `502`. All other endpoints too.

## Root cause: a four-link failure chain

| # | Link | Detail |
|---|---|---|
| 1 | **Redis OOM-killed** (2026-07-08 10:01) | The prod overlay capped the Redis container at 1 GB memory but set **no `maxmemory`**, so Redis grew into the hard limit and was killed (exit 137) instead of evicting. |
| 2 | **wms-api refused to start** | `state.rs` did `TileCache::connect(...).await?` — an unreachable Redis was a **fatal startup error**, so wms-api entered a permanent crash loop. |
| 3 | **Data retention silently stopped** | The `CleanupTask` (which enforces `config/models/*.yaml` retention by deleting expired datasets from MinIO + Postgres) **runs inside wms-api**. With wms-api down, nothing pruned data for ~3 weeks. MinIO grew **165 GB → 623 GB**. |
| 4 | **Disk hit 100% → Postgres died** | Postgres crashed mid-WAL-redo: `could not write to file "pg_xact/06F3": No space left on device`. It could not restart (no space), so every API lost its database and nginx had no healthy upstream → **502**. |

The trigger was a single Redis OOM; the *outage* was caused by retention being
coupled to a service that couldn't tolerate that dependency failing.

Contributing factor: forecast-horizon expansion (GFS 120h→384h, NBM 48h→264h,
2026-07-02) raised the steady-state footprint, shortening the runway once
retention stalled. Retention failure was the dominant cause, not the expansion.

## Disk at time of incident

| Volume | Size |
|---|---|
| `minio_data` | **623.3 GB** (all stale — newest data 8 days old) |
| `downloader_data` | 48.4 GB (un-ingested staging files) |
| Docker images / container layers | ~80 GB |
| `postgres_data` | 6.5 GB |
| `redis_data` | 2.3 GB |

## Recovery performed

1. Stopped the crash-looping stack (and autoheal) to stop thrash.
2. Pruned stopped containers + dangling images (**~35 GB**).
3. Cleared `downloader_data` staging files (**~48 GB**) — transient, re-downloaded.
4. Deleted all of MinIO `grids/` (**~600 GB**) with MinIO stopped, preserving
   `.minio.sys` and the `weather-data` bucket. All of it was past every model's
   retention window and re-downloadable.
5. Started Postgres → **WAL recovery completed cleanly**, no restore needed.
   Verified intact: 865,019 storm events · 45,052 locations · 3,235 counties ·
   10,272,272 observations.
6. `TRUNCATE datasets` — 235,605 catalog rows pointed at deleted objects; the
   catalog re-populates from live ingest.
7. Brought the stack up in dependency order; all 14 services healthy.

Result: **789 GB free (6% used)**, all endpoints 200, data re-ingesting.

## Fixes (defense in depth — each breaks a different link)

| Link broken | Fix |
|---|---|
| 1 | **Redis evicts instead of dying**: `--maxmemory 768mb --maxmemory-policy allkeys-lru` (below the 1 GB container limit) and `--save ''` (cache data is disposable; persistence was also growing the volume). |
| 2 | **Redis is now optional**: `TileCache` holds an `Option<connection>`; `connect()` degrades to a disabled cache (gets miss, sets no-op) and logs a warning instead of erroring. wms-api keeps serving from L1 + object storage **and keeps running retention**. Regression test: `test_connect_unreachable_redis_returns_disabled_cache`. |
| 3 | **Server-side retention backstop**: MinIO lifecycle (ILM) expiry rules per model prefix, applied automatically by the `minio-setup` init container and via `scripts/setup_minio_lifecycle.sh`. These work even when wms-api is down. Deliberately generous vs app retention (3–4 days for ≤48 h models, 32 days for NLDAS/GLDAS) so they only cap unbounded growth. |
| 4 | **Disk guardrail**: `scripts/check_disk_space.sh` via cron every 15 min — warns at 80%, and at 90% reports whether wms-api/retention is healthy and reclaims downloader staging. Logs to `/opt/weather-wms/logs/disk.log`. |

## Operational notes

```bash
# Inspect lifecycle rules
./scripts/setup_minio_lifecycle.sh --list

# Check disk status manually
/opt/weather-wms/scripts/check_disk_space.sh --dry-run

# Disk monitor history
tail /opt/weather-wms/logs/disk.log
```

**Key lesson:** a background job responsible for bounding resource growth must
not live behind an optional dependency. If retention can't run, the system must
either fail loudly or have an independent backstop — now it has both.

## Follow-ups (not blocking)

- Re-measure steady-state MinIO footprint now that retention works, and confirm
  the 384 h GFS / 264 h NBM horizons fit comfortably in 884 GB.
- Consider moving the CleanupTask out of wms-api into its own service or a
  scheduled job so it is structurally independent of API health.
- Local dev box also hit 100% during recovery: `weather-wms/target` had 189 GB
  of stale debug artifacts (`cargo clean --profile dev` reclaimed 202 GB).
