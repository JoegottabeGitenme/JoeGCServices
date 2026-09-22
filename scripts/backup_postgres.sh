#!/usr/bin/env bash
#
# Nightly PostgreSQL backup for the weather-wms host.
# =============================================================================
# WHY THIS EXISTS
#
# Audited during the trail-conditions design session (2026-09): the only
# backup of the production database was a single manual pre-migration dump
# from 2026-06-05 (backups/pre-storm-events-*.sql.gz). Everything else in
# this stack is intentionally rebuildable from source (HRRR/NLDAS re-ingest,
# OSM re-sync, static terrain layers re-derived) — Postgres is the one store
# that is NOT reproducible. It holds storm events, observations, the
# locations/trailhead registry, and (going forward) trail condition reports,
# which double as the training labels for the trail-conditions classifier.
# Losing it silently would be worse than the Aug 2026 disk-exhaustion outage.
#
# This script takes a nightly `pg_dump`, gzips it, and prunes old copies.
#
# SCOPE: local disk only. It protects against container/compose nukes,
# accidental `DROP`, and bad migrations — NOT against full disk/hardware
# loss, which would take the backups directory down with everything else.
# An off-NUC copy (rclone to cloud storage or a workstation) is a tracked
# follow-up, not yet implemented.
#
# Install via cron (see scripts/install_backup_cron.sh):
#   0 2 * * * /opt/weather-wms/scripts/backup_postgres.sh >> /opt/weather-wms/logs/backup.log 2>&1
#
# Usage:
#   ./scripts/backup_postgres.sh              # run a backup now
#   ./scripts/backup_postgres.sh --dry-run     # show what would happen
# =============================================================================
set -euo pipefail

WORKDIR="${WEATHER_WMS_DIR:-/opt/weather-wms}"
BACKUP_DIR="${BACKUP_DIR:-$WORKDIR/backups}"
RETENTION_DAYS="${RETENTION_DAYS:-14}"
LOCK_FILE="$BACKUP_DIR/.backup.lock"
DRY_RUN=0
[[ "${1:-}" == "--dry-run" ]] && DRY_RUN=1

ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { echo "[$(ts)] $*"; }

# Re-exec under flock so an overrunning backup can't overlap with the next
# night's run (same pattern as the loadtest cron jobs).
if [[ "${BACKUP_LOCKED:-0}" != "1" ]]; then
  mkdir -p "$BACKUP_DIR"
  exec env BACKUP_LOCKED=1 flock -n "$LOCK_FILE" "$0" "$@"
fi

cd "$WORKDIR"

if [[ -f .env ]]; then
  # shellcheck disable=SC1091
  source .env
fi

POSTGRES_USER="${POSTGRES_USER:-weatherwms}"
POSTGRES_DB="${POSTGRES_DB:-weatherwms}"

COMPOSE_FILES=(-f docker-compose.yml -f deploy/production/docker-compose.prod.yml)
STAMP="$(date -u +%Y%m%d-%H%M%S)"
OUT_FILE="$BACKUP_DIR/weatherwms-${STAMP}.sql.gz"

if [[ "$DRY_RUN" -eq 1 ]]; then
  log "DRY RUN: would dump database '$POSTGRES_DB' as user '$POSTGRES_USER' to $OUT_FILE"
  log "DRY RUN: would prune backups older than $RETENTION_DAYS days in $BACKUP_DIR"
  exit 0
fi

log "Starting backup of '$POSTGRES_DB' -> $OUT_FILE"

if ! docker compose "${COMPOSE_FILES[@]}" exec -T postgres \
    pg_dump -U "$POSTGRES_USER" -d "$POSTGRES_DB" --no-owner --no-acl \
    | gzip > "$OUT_FILE.partial"; then
  log "ERROR: pg_dump failed"
  rm -f "$OUT_FILE.partial"
  exit 1
fi

mv "$OUT_FILE.partial" "$OUT_FILE"
size_h="$(du -h "$OUT_FILE" | cut -f1)"
log "OK: wrote $OUT_FILE ($size_h)"

# Prune backups older than RETENTION_DAYS. Never touch the manual
# pre-storm-events dump or any file not matching our naming pattern.
deleted=0
while IFS= read -r -d '' old; do
  rm -f "$old"
  log "Pruned old backup: $old"
  deleted=$((deleted + 1))
done < <(find "$BACKUP_DIR" -maxdepth 1 -name 'weatherwms-*.sql.gz' -mtime "+$RETENTION_DAYS" -print0)

log "Backup complete. Pruned $deleted old file(s)."
