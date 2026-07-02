#!/usr/bin/env bash
# Shared helpers for load-test orchestration (runs on the NUC).

# Resolve the loadtest root (parent of scripts/)
LT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export LT_ROOT

# Config (override via environment)
K6_IMAGE="${K6_IMAGE:-grafana/k6:latest}"
PY_IMAGE="${PY_IMAGE:-python:3.11-slim}"
K6_NETWORK="${K6_NETWORK:-shared-services}"
COMPOSE_NET="${COMPOSE_NET:-weather-wms_default}"
WMS_ENV_FILE="${WMS_ENV_FILE:-/opt/weather-wms/.env}"
REDIS_CONTAINER="${REDIS_CONTAINER:-weather-wms-redis-1}"
PUBLISH_DIR="${PUBLISH_DIR:-/opt/gateway/static/loadtest}"

info() { printf '\033[1;34m▸ %s\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m✔ %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m! %s\033[0m\n' "$*"; }
err()  { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; }

ensure_images() {
  docker image inspect "$K6_IMAGE" >/dev/null 2>&1 || docker pull "$K6_IMAGE"
  docker image inspect "$PY_IMAGE" >/dev/null 2>&1 || docker pull "$PY_IMAGE"
}

# scale_dur <seconds> -> seconds * SCALE (SCALE default 1), min 15s
scale_dur() {
  awk -v s="${SCALE:-1}" -v d="$1" 'BEGIN { v = int(d * s); if (v < 15) v = 15; print v }'
}

# run_k6 <stage-name> <scenario.js> [extra -e args...]
# Writes summary JSON to $RUN_DIR/<stage-name>.json
run_k6() {
  local stage="$1" scenario="$2"
  shift 2
  info "Stage: $stage ($scenario $*)"
  local args=()
  local kv
  for kv in "$@"; do
    args+=(-e "$kv")
  done
  # shellcheck disable=SC2086
  docker run --rm -u "$(id -u):$(id -g)" --network "$K6_NETWORK" \
    -v "$LT_ROOT:/lt" \
    -e SCENARIO_NAME="$stage" \
    -e SUMMARY_PATH="/lt/results/$RUN_NAME/$stage.json" \
    "${args[@]}" \
    "$K6_IMAGE" run --quiet "/lt/scenarios/$scenario" \
    || warn "Stage $stage exited non-zero (threshold breach or abort — expected for ramps)"
}

# Flush L2 (Redis) and in-memory caches for a true cold start
flush_caches() {
  info "Flushing caches (Redis + wms-api in-memory)..."
  local redis_pw=""
  if [[ -f "$WMS_ENV_FILE" ]]; then
    redis_pw="$(grep -E '^REDIS_PASSWORD=' "$WMS_ENV_FILE" | cut -d= -f2- || true)"
  fi
  if [[ -n "$redis_pw" ]]; then
    docker exec "$REDIS_CONTAINER" redis-cli -a "$redis_pw" --no-auth-warning FLUSHALL >/dev/null \
      && ok "Redis flushed" || warn "Redis flush failed"
  else
    docker exec "$REDIS_CONTAINER" redis-cli FLUSHALL >/dev/null \
      && ok "Redis flushed" || warn "Redis flush failed (no password found in $WMS_ENV_FILE?)"
  fi
  docker run --rm --network "$K6_NETWORK" curlimages/curl:latest \
    -s -X POST http://wms-api:8080/api/cache/clear >/dev/null \
    && ok "wms-api caches cleared" || warn "wms-api cache clear failed"
}

# Background docker-stats sampler -> $RUN_DIR/docker-stats.jsonl
start_stats_sampler() {
  (
    while true; do
      docker stats --no-stream --format '{{json .}}' 2>/dev/null \
        | grep -E 'weather-wms-(wms-api|edr-api|postgres|redis|minio|ingester)|gateway-nginx' \
        >> "$RUN_DIR/docker-stats.jsonl" || true
      sleep 20
    done
  ) &
  STATS_PID=$!
  info "docker-stats sampler started (pid $STATS_PID)"
}

stop_stats_sampler() {
  if [[ -n "${STATS_PID:-}" ]]; then
    kill "$STATS_PID" 2>/dev/null || true
    wait "$STATS_PID" 2>/dev/null || true
    STATS_PID=""
  fi
}

# Generate report (python container on the compose network to reach prometheus)
generate_report() {
  local mode="$1"
  info "Generating $mode report..."
  docker run --rm -u "$(id -u):$(id -g)" --network "$COMPOSE_NET" \
    -v "$LT_ROOT:/lt" \
    "$PY_IMAGE" python "/lt/report/generate_report.py" \
    --run-dir "/lt/results/$RUN_NAME" --mode "$mode" \
    || warn "Report generation failed"
}

# Publish report to the gateway static dir
publish_report() {
  local mode="$1"
  mkdir -p "$PUBLISH_DIR/history"
  if [[ -f "$RUN_DIR/report.html" ]]; then
    if [[ "$mode" == "nightly" ]]; then
      cp "$RUN_DIR/report.html" "$PUBLISH_DIR/index.html"
      cp "$RUN_DIR/report.html" "$PUBLISH_DIR/history/$RUN_NAME.html"
    else
      cp "$RUN_DIR/report.html" "$PUBLISH_DIR/probe.html"
    fi
    ok "Report published to $PUBLISH_DIR"
  else
    warn "No report.html to publish"
  fi
}

# Keep only the most recent N result dirs for a given prefix
prune_results() {
  local prefix="$1" keep="$2"
  # shellcheck disable=SC2012
  ls -1d "$LT_ROOT/results/$prefix"-* 2>/dev/null | sort | head -n "-$keep" | while read -r d; do
    rm -rf "$d"
  done
}
