#!/usr/bin/env bash
#
# Disk-space guardrail for the weather-wms host.
# =============================================================================
# WHY THIS EXISTS
#
# In Aug 2026 the NUC filled to 100% unnoticed. Postgres then died mid-WAL-redo
# ("No space left on device"), every API returned 502, and recovery required
# manual intervention. Nothing was watching the disk.
#
# This script warns early and, at a critical threshold, takes the one safe
# automatic action available: clearing the downloader's staging directory
# (transient GRIB/NetCDF files that are re-downloaded on demand). It never
# touches Postgres, MinIO object data, or anything else stateful.
#
# Install via cron (see scripts/install_disk_monitor.sh or the crontab entry):
#   */15 * * * * /opt/weather-wms/scripts/check_disk_space.sh >> /var/log/... 2>&1
#
# Usage:
#   ./scripts/check_disk_space.sh          # check + act if needed
#   ./scripts/check_disk_space.sh --dry-run
# =============================================================================
set -euo pipefail

WARN_PCT="${WARN_PCT:-80}"
CRIT_PCT="${CRIT_PCT:-90}"
MOUNT="${MOUNT:-/}"
DRY_RUN=0
[[ "${1:-}" == "--dry-run" ]] && DRY_RUN=1

used_pct="$(df --output=pcent "$MOUNT" | tail -1 | tr -dc '0-9')"
avail_h="$(df -h --output=avail "$MOUNT" | tail -1 | tr -d ' ')"
ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

log() { echo "[$ts] $*"; }

if (( used_pct < WARN_PCT )); then
  log "OK: disk ${used_pct}% used, ${avail_h} available"
  exit 0
fi

if (( used_pct < CRIT_PCT )); then
  log "WARNING: disk ${used_pct}% used (warn>=${WARN_PCT}%), ${avail_h} available"
  log "  Top volumes:"
  docker system df -v 2>/dev/null \
    | sed -n '/Local Volumes/,/Build Cache/p' \
    | sort -k3 -rh | head -6 | sed 's/^/    /'
  exit 0
fi

# ---- critical ---------------------------------------------------------------
log "CRITICAL: disk ${used_pct}% used (crit>=${CRIT_PCT}%), only ${avail_h} available"
log "  Retention may have stalled. Checking wms-api (owner of the cleanup task)..."
if ! docker ps --filter name=weather-wms-wms-api --filter health=healthy --format '{{.Names}}' | grep -q wms-api; then
  log "  !! wms-api is NOT healthy - data retention is very likely NOT running."
  log "     This is the failure mode that filled the disk in Aug 2026."
fi

log "  Reclaiming: downloader staging files (transient, re-downloaded on demand)"
if (( DRY_RUN )); then
  log "  [dry-run] would clear weather-wms_downloader_data"
else
  docker run --rm -v weather-wms_downloader_data:/d alpine \
    sh -c 'find /d -maxdepth 1 -type f -delete' 2>/dev/null \
    && log "  cleared downloader staging" \
    || log "  could not clear downloader staging"

  docker image prune -f >/dev/null 2>&1 && log "  pruned dangling images" || true

  new_pct="$(df --output=pcent "$MOUNT" | tail -1 | tr -dc '0-9')"
  new_avail="$(df -h --output=avail "$MOUNT" | tail -1 | tr -d ' ')"
  log "  after reclaim: ${new_pct}% used, ${new_avail} available"
  if (( new_pct >= CRIT_PCT )); then
    log "  !! STILL CRITICAL - manual intervention needed."
    log "     MinIO object data is the usual culprit; verify lifecycle rules:"
    log "       ./scripts/setup_minio_lifecycle.sh --list"
  fi
fi
