#!/usr/bin/env bash
# ============================================================================
# Daytime probe — light mixed traffic through the gateway, pass/fail via
# k6 thresholds. Scheduled by loadtest-probe.timer (every ~3h with jitter).
# ============================================================================
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

RUN_NAME="probe-$(date -u +%Y%m%d-%H%M%S)"
RUN_DIR="$LT_ROOT/results/$RUN_NAME"
mkdir -p "$RUN_DIR"

info "Probe run: $RUN_NAME"
ensure_images

run_k6 "probe" "probe.js" \
  "TARGET=gateway" \
  "RATE=${RATE:-15}" \
  "DURATION=${DURATION:-3m}" \
  "TIME_VARIETY=0.2"

generate_report "probe"
publish_report "probe"
prune_results "probe" 50

ok "Probe complete: $RUN_DIR"
