#!/usr/bin/env bash
# ============================================================================
# Nightly stress suite — full-intensity load test of WMS/WMTS/EDR.
#
# Stages:
#   1. cold        cache flush + modest mixed load (true render-path numbers)
#   2. warm        15 min realistic steady-state
#   3. ramp_wmts   breakpoint search: tiles
#   4. ramp_wms    breakpoint search: GetMap rendering
#   5. ramp_edr    breakpoint search: EDR queries
#   6. spike       sudden burst + recovery
#   7. soak        20 min moderate mixed (leak detection)
#   8. gateway     same mix via nginx (overhead comparison)
#
# Env:
#   SCALE   duration multiplier (default 1.0; use 0.15 for a quick shakeout)
#
# Scheduled by loadtest-nightly.timer (03:00). ~85 min at SCALE=1.
# ============================================================================
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

RUN_NAME="nightly-$(date -u +%Y%m%d-%H%M%S)"
RUN_DIR="$LT_ROOT/results/$RUN_NAME"
mkdir -p "$RUN_DIR"

info "Nightly run: $RUN_NAME (SCALE=${SCALE:-1})"
ensure_images

cleanup() { stop_stats_sampler; }
trap cleanup EXIT

start_stats_sampler

COOLDOWN="$(scale_dur 30)"

# ---- 1. Cold cache ----------------------------------------------------------
flush_caches
run_k6 "01-cold" "mixed.js" \
  "TARGET=direct" "RATE=10" "DURATION=$(scale_dur 300)s" "TIME_VARIETY=0.4"
sleep "$COOLDOWN"

# ---- 2. Warm steady-state -----------------------------------------------------
run_k6 "02-warm" "mixed.js" \
  "TARGET=direct" "RATE=40" "DURATION=$(scale_dur 900)s" "TIME_VARIETY=0.2"
sleep "$COOLDOWN"

# ---- 3-5. Breakpoint ramps ------------------------------------------------------
run_k6 "03-ramp-wmts" "ramp.js" \
  "TARGET=direct" "WORKLOAD=wmts" "START_RATE=20" "MAX_RATE=800" \
  "DURATION=$(scale_dur 600)s"
sleep "$COOLDOWN"

run_k6 "04-ramp-wms" "ramp.js" \
  "TARGET=direct" "WORKLOAD=wms" "START_RATE=5" "MAX_RATE=200" \
  "DURATION=$(scale_dur 600)s"
sleep "$COOLDOWN"

run_k6 "05-ramp-edr" "ramp.js" \
  "TARGET=direct" "WORKLOAD=edr" "START_RATE=5" "MAX_RATE=150" \
  "DURATION=$(scale_dur 600)s"
sleep "$COOLDOWN"

# ---- 6. Spike --------------------------------------------------------------------
run_k6 "06-spike" "spike.js" \
  "TARGET=direct" "SPIKE_RATE=250" "HOLD=$(scale_dur 60)s"
sleep "$COOLDOWN"

# ---- 7. Soak ---------------------------------------------------------------------
run_k6 "07-soak" "mixed.js" \
  "TARGET=direct" "RATE=30" "DURATION=$(scale_dur 1200)s" "TIME_VARIETY=0.2"
sleep "$COOLDOWN"

# ---- 8. Gateway overhead comparison -------------------------------------------------
run_k6 "08-gateway" "mixed.js" \
  "TARGET=gateway" "RATE=25" "DURATION=$(scale_dur 300)s" "TIME_VARIETY=0.2"

stop_stats_sampler

# ---- Report + publish ----------------------------------------------------------------
generate_report "nightly"
publish_report "nightly"
prune_results "nightly" 30

ok "Nightly complete: $RUN_DIR"
[[ -f "$RUN_DIR/report.md" ]] && cat "$RUN_DIR/report.md"
