#!/usr/bin/env bash
#
# Upload an assembled WS1 static stack (assemble_static_stack.py's output,
# a Zarr v3 directory) to the NUC's MinIO, under the reaper-safe `static/`
# prefix (verified: outside CleanupTask's catalog-driven scope, outside
# SyncTask's shredded/raw/grids/ listing, outside the ILM expiry rules in
# scripts/setup_minio_lifecycle.sh -- see docs/trail-conditions-design.md).
#
# =============================================================================
# WHY THIS IS TWO STEPS, NOT ONE
#
# 1. rsync the local Zarr directory to the NUC's filesystem (this
#    sandboxed environment has no SSH access to the production NUC --
#    confirmed this session, consistent with every prior session's own
#    deployment story -- so this script is written to be RUN from a
#    context that does have that access, e.g. by the user or a CI/CD
#    runner with the real SSH key, not executed here).
# 2. `mc mirror` FROM the NUC's own filesystem INTO MinIO, using the same
#    "run mc in a throwaway container on the compose network" pattern
#    already established in scripts/setup_minio_lifecycle.sh (mc_run) --
#    not a new, divergent pattern.
#
# This mirrors scripts/deploy-remote.sh's own VIIRS static-data precedent
# (rsync the file to the NUC's data/ directory, then a second step to get
# it into the running system) rather than inventing a different mechanism.
#
# Usage (run on a machine with real SSH access to the NUC):
#   ./upload_static_stack.sh ./data/static/colorado-10m-pilot.zarr colorado-10m/pilot
#
# =============================================================================
set -euo pipefail

LOCAL_ZARR_PATH="${1:?Usage: $0 <local-zarr-path> <remote-prefix-under-static/>}"
REMOTE_PREFIX="${2:?Usage: $0 <local-zarr-path> <remote-prefix-under-static/>}"

REMOTE_HOST="${REMOTE_HOST:-nuc}"
REMOTE_DIR="${REMOTE_DIR:-/opt/weather-wms}"
BUCKET="${BUCKET:-weather-data}"
MC_IMAGE="${MC_IMAGE:-minio/mc:latest}"
COMPOSE_NET="${COMPOSE_NET:-weather-wms_default}"
ENV_FILE="${ENV_FILE:-/opt/weather-wms/.env}"

info() { printf '\033[1;34m▸ %s\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m✔ %s\033[0m\n' "$*"; }

if [[ ! -d "$LOCAL_ZARR_PATH" ]]; then
  echo "Error: $LOCAL_ZARR_PATH does not exist or is not a directory (expected a Zarr v3 store, see assemble_static_stack.py)" >&2
  exit 1
fi

zarr_name="$(basename "$LOCAL_ZARR_PATH")"
remote_zarr_path="$REMOTE_DIR/data/static_staging/$zarr_name"

info "Step 1/2: rsync $LOCAL_ZARR_PATH -> $REMOTE_HOST:$remote_zarr_path"
ssh "$REMOTE_HOST" "mkdir -p $REMOTE_DIR/data/static_staging"
rsync -avz --progress "$LOCAL_ZARR_PATH/" "$REMOTE_HOST:$remote_zarr_path/"
ok "rsync complete"

info "Step 2/2: mc mirror (NUC filesystem -> MinIO $BUCKET/static/$REMOTE_PREFIX)"
ssh "$REMOTE_HOST" bash -s <<REMOTE_SCRIPT
set -euo pipefail
S3_ACCESS_KEY="\$(grep -E '^S3_ACCESS_KEY=' '$ENV_FILE' | cut -d= -f2- || echo minioadmin)"
S3_SECRET_KEY="\$(grep -E '^S3_SECRET_KEY=' '$ENV_FILE' | cut -d= -f2- || echo minioadmin)"
docker run --rm --network "$COMPOSE_NET" \
  -v "$remote_zarr_path:/upload_source:ro" \
  -e "MC_HOST_minio=http://\${S3_ACCESS_KEY}:\${S3_SECRET_KEY}@minio:9000" \
  "$MC_IMAGE" mirror --overwrite /upload_source "minio/$BUCKET/static/$REMOTE_PREFIX"
REMOTE_SCRIPT
ok "mirror complete"

info "Verifying: reading the grid_spec attr back from MinIO via the deployed environment..."
ssh "$REMOTE_HOST" bash -s <<REMOTE_VERIFY
set -euo pipefail
S3_ACCESS_KEY="\$(grep -E '^S3_ACCESS_KEY=' '$ENV_FILE' | cut -d= -f2- || echo minioadmin)"
S3_SECRET_KEY="\$(grep -E '^S3_SECRET_KEY=' '$ENV_FILE' | cut -d= -f2- || echo minioadmin)"
docker run --rm --network "$COMPOSE_NET" \
  -e "MC_HOST_minio=http://\${S3_ACCESS_KEY}:\${S3_SECRET_KEY}@minio:9000" \
  "$MC_IMAGE" ls "minio/$BUCKET/static/$REMOTE_PREFIX/"
REMOTE_VERIFY

echo
info "Uploaded to minio/$BUCKET/static/$REMOTE_PREFIX/"
info "Cleaning up the NUC-side staging copy (rsync target, no longer needed once mirrored into MinIO):"
echo "  ssh $REMOTE_HOST rm -rf $remote_zarr_path"
