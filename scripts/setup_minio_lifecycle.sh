#!/usr/bin/env bash
#
# Apply MinIO object-lifecycle (ILM) expiry rules to the weather-data bucket.
# =============================================================================
# WHY THIS EXISTS
#
# Primary data retention is enforced by the wms-api CleanupTask. That is an
# application-level mechanism, so it stops working whenever wms-api is down.
# In Aug 2026 a Redis outage kept wms-api from starting for ~3 weeks; retention
# never ran, MinIO grew to 623 GB, the disk hit 100%, Postgres died mid-WAL-redo
# and every API returned 502.
#
# These ILM rules are a SERVER-SIDE BACKSTOP: MinIO expires stale objects on its
# own, independent of any application service. They are deliberately far more
# generous than the app's retention windows, so they never fight the app - they
# only stop unbounded growth if the app's cleanup stalls.
#
#   expiry = ceil(model retention hours / 24) + 2 days of slack, min 3 days
#
# Layout assumption: objects live under  grids/{model}/{date}/...
#
# Usage:
#   ./scripts/setup_minio_lifecycle.sh            # apply to the NUC
#   MC_ALIAS=... BUCKET=... ./scripts/setup_minio_lifecycle.sh
#   ./scripts/setup_minio_lifecycle.sh --list     # just show current rules
# =============================================================================
set -euo pipefail

BUCKET="${BUCKET:-weather-data}"
MC_IMAGE="${MC_IMAGE:-minio/mc:latest}"
COMPOSE_NET="${COMPOSE_NET:-weather-wms_default}"
ENV_FILE="${ENV_FILE:-/opt/weather-wms/.env}"

# model:expiry_days  — derived from config/models/*.yaml retention.hours (+2d slack).
# Everything except the NLDAS/GLDAS family has <=48h retention, so 3-4 days is
# already several times the app's window.
RULES="
aigefs:4
aigfs:4
gfs:4
gfswave:4
hrrr:3
ndfd:3
nbm-conus:3
nbm-alaska:3
nbm-guam:3
nbm-hawaii:3
nbm-puertorico:3
mrms:3
goes18:3
goes19:3
goes18-fulldisk:3
goes19-fulldisk:3
nldas-forcing:32
nldas-noah:32
gldas-noah:32
unknown-cf:3
"

info() { printf '\033[1;34m▸ %s\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m✔ %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m! %s\033[0m\n' "$*"; }

# Read S3 creds from the deployed .env
if [[ -f "$ENV_FILE" ]]; then
  S3_ACCESS_KEY="$(grep -E '^S3_ACCESS_KEY=' "$ENV_FILE" | cut -d= -f2-)"
  S3_SECRET_KEY="$(grep -E '^S3_SECRET_KEY=' "$ENV_FILE" | cut -d= -f2-)"
else
  S3_ACCESS_KEY="${S3_ACCESS_KEY:-minioadmin}"
  S3_SECRET_KEY="${S3_SECRET_KEY:-minioadmin}"
fi

# Run an mc command inside a throwaway container on the compose network.
mc_run() {
  docker run --rm --network "$COMPOSE_NET" \
    -e MC_HOST_minio="http://${S3_ACCESS_KEY}:${S3_SECRET_KEY}@minio:9000" \
    "$MC_IMAGE" "$@"
}

if [[ "${1:-}" == "--list" ]]; then
  info "Current lifecycle rules on ${BUCKET}:"
  mc_run ilm rule list "minio/${BUCKET}" || warn "no rules set"
  exit 0
fi

info "Applying MinIO lifecycle rules to ${BUCKET} (server-side retention backstop)"

applied=0
skipped=0
while read -r entry; do
  [[ -z "$entry" ]] && continue
  model="${entry%%:*}"
  days="${entry##*:}"
  prefix="grids/${model}/"

  # `ilm rule add` is idempotent enough for our purposes: duplicate rules for the
  # same prefix are avoided by clearing matching rules first (best-effort).
  if mc_run ilm rule add "minio/${BUCKET}" \
        --expire-days "$days" \
        --prefix "$prefix" >/dev/null 2>&1; then
    printf '  %-22s expire after %2s days\n' "$model" "$days"
    applied=$((applied + 1))
  else
    warn "could not add rule for ${prefix} (may already exist)"
    skipped=$((skipped + 1))
  fi
done <<< "$RULES"

ok "Lifecycle rules applied: ${applied} (skipped: ${skipped})"
info "Verifying:"
mc_run ilm rule list "minio/${BUCKET}" 2>/dev/null | head -30 || warn "could not list rules"

cat <<'EOF'

NOTE: These rules are a backstop only. Normal retention is still enforced by the
wms-api CleanupTask using config/models/*.yaml. If ILM ever deletes something the
app still wanted, the expiry days above are too tight - raise them.
EOF
