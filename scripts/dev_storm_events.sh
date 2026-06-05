#!/bin/bash
# Local storm-events pipeline dev script.
#
# Starts the minimal infrastructure (postgres + minio), builds and runs
# ingester + edr-api locally, then seeds test data so you can hit the EDR
# endpoints immediately — no internet download required.
#
# For testing with REAL (minimal) data instead of the seed script, pass --real:
#   ./scripts/dev_storm_events.sh --real
#   This swaps in the dev config (backfill_start_year: 2026, ~1.7 MB) and
#   starts the downloader so it fetches the current partial-year file once.
#
# Usage:
#   ./scripts/dev_storm_events.sh           # seed mode (fastest, no download)
#   ./scripts/dev_storm_events.sh --real    # real data, minimal (1.7 MB)
#   ./scripts/dev_storm_events.sh --stop    # kill background processes + infra
#
# Requirements:
#   - docker + docker compose
#   - cargo (workspace already built via `cargo check`)

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT}"

MODE="seed"
[[ "${1:-}" == "--real" ]] && MODE="real"
[[ "${1:-}" == "--stop" ]] && MODE="stop"

DB_URL="postgresql://weatherwms:weatherwms@localhost:5432/weatherwms"
S3_ENDPOINT="http://localhost:9000"
S3_KEY="minioadmin"
INGESTER_URL="http://localhost:8082"
EDR_URL="http://localhost:8083"

# Shared env used by both Rust services.
COMMON_ENV=(
    DATABASE_URL="${DB_URL}"
    S3_ENDPOINT="${S3_ENDPOINT}"
    S3_BUCKET="weather-data"
    S3_ACCESS_KEY="${S3_KEY}"
    S3_SECRET_KEY="${S3_KEY}"
    CONFIG_DIR="config"
    RUST_LOG="info,sqlx=warn"
)

# ─── STOP ─────────────────────────────────────────────────────────────────────
if [[ "${MODE}" == "stop" ]]; then
    echo "==> Stopping background services"
    pkill -f 'target/.*/ingester' 2>/dev/null && echo "  killed ingester" || true
    pkill -f 'target/.*/edr-api'  2>/dev/null && echo "  killed edr-api"  || true
    pkill -f 'target/.*/downloader' 2>/dev/null && echo "  killed downloader" || true
    echo "==> Stopping infra containers"
    docker compose stop postgres minio 2>/dev/null || true
    echo "Done."
    exit 0
fi

# ─── INFRA ────────────────────────────────────────────────────────────────────
echo "==> Starting postgres + minio"
docker compose up -d postgres minio minio-setup

echo "==> Waiting for postgres..."
until docker compose exec -T postgres pg_isready -U weatherwms -q 2>/dev/null; do
    sleep 1; printf '.'
done
echo " ready."

# ─── BUILD ────────────────────────────────────────────────────────────────────
echo "==> Building ingester + edr-api (dev profile)..."
cargo build -p ingester -p edr-api 2>&1 | grep -E 'Compiling|Finished|error' || true

# ─── INGESTER ─────────────────────────────────────────────────────────────────
echo "==> Starting ingester on :8082"
pkill -f 'target/.*/ingester' 2>/dev/null || true
sleep 1
env "${COMMON_ENV[@]}" ./target/debug/ingester \
    > /tmp/ingester.log 2>&1 &
INGESTER_PID=$!
echo "    PID ${INGESTER_PID} — logs: /tmp/ingester.log"

# Wait for ingester to be ready (it runs migrations on startup)
echo "==> Waiting for ingester..."
for i in $(seq 1 30); do
    if curl -sf "${INGESTER_URL}/health" > /dev/null 2>&1; then
        echo " ready."
        break
    fi
    sleep 1; printf '.'
    if [[ $i -eq 30 ]]; then
        echo ""
        echo "ERROR: ingester did not start in 30 s. Check /tmp/ingester.log"
        exit 1
    fi
done

# ─── TIGER COUNTIES (optional but stamps county_fips) ─────────────────────────
if ! docker compose exec -T postgres psql "postgresql://weatherwms:weatherwms@localhost:5432/weatherwms" -c "SELECT 1 FROM tiger_counties LIMIT 1" > /dev/null 2>&1; then
    echo "==> Loading TIGER county polygons (one-time, ~60 s)..."
    # Uses Python + psql only — no ogr2ogr/GDAL needed.
    DATABASE_URL="${DB_URL}" WORK_DIR="/tmp/tiger_py" \
        ./scripts/load_tiger_counties_py.sh 2>&1 | tail -8
else
    echo "==> TIGER counties already loaded, skipping."
fi

# ─── EDR-API ──────────────────────────────────────────────────────────────────
echo "==> Starting edr-api on :8083"
pkill -f 'target/.*/edr-api' 2>/dev/null || true
sleep 1
env "${COMMON_ENV[@]}" \
    EDR_BASE_URL="${EDR_URL}/edr" \
    ./target/debug/edr-api \
    > /tmp/edr-api.log 2>&1 &
EDR_PID=$!
echo "    PID ${EDR_PID} — logs: /tmp/edr-api.log"

echo "==> Waiting for edr-api..."
for i in $(seq 1 30); do
    if curl -sf "${EDR_URL}/edr" > /dev/null 2>&1; then
        echo " ready."
        break
    fi
    sleep 1; printf '.'
    if [[ $i -eq 30 ]]; then
        echo ""
        echo "ERROR: edr-api did not start in 30 s. Check /tmp/edr-api.log"
        exit 1
    fi
done

# ─── SEED / REAL DATA ─────────────────────────────────────────────────────────
if [[ "${MODE}" == "real" ]]; then
    echo ""
    echo "==> REAL MODE: swapping in dev config (backfill_start_year: 2026, ~1.7 MB)"
    # Temporarily swap the production config for the dev config.
    cp config/models/storm-events.yaml config/models/storm-events.yaml.prod_bak
    cp config/models/storm-events.dev.yaml config/models/storm-events.yaml

    echo "==> Building + starting downloader (will fetch 2026 file once, then idle)"
    pkill -f 'target/.*/downloader' 2>/dev/null || true
    sleep 1
    cargo build -p downloader 2>&1 | grep -E 'Compiling|Finished|error' || true
    env "${COMMON_ENV[@]}" \
        INGESTER_URL="${INGESTER_URL}" \
        DOWNLOADER_STATE_PATH="/tmp/storm_events_dev_state.db" \
        ./target/debug/downloader \
        > /tmp/downloader.log 2>&1 &
    DL_PID=$!
    echo "    PID ${DL_PID} — logs: /tmp/downloader.log"
    echo "    Waiting up to 3 minutes for the first ingest cycle to complete..."
    for i in $(seq 1 180); do
        if grep -q "Ingested year" /tmp/downloader.log 2>/dev/null; then
            echo " done."
            break
        fi
        sleep 2; printf '.'
    done
    echo ""
    echo "    Restoring production config"
    mv config/models/storm-events.yaml.prod_bak config/models/storm-events.yaml
else
    echo ""
    echo "==> SEED MODE: injecting 25 test events (no internet download)"
    INGESTER_URL="${INGESTER_URL}" ./scripts/seed_storm_events_test.sh
fi

# ─── VERIFY ───────────────────────────────────────────────────────────────────
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Running quick verification queries..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

echo "1) Collections listing:"
curl -sf "${EDR_URL}/edr/collections" | python3 -c "
import sys,json
d=json.load(sys.stdin)
for c in d.get('collections',[]):
    print(f'   {c[\"id\"]:30s}  {c.get(\"title\",\"\")[:50]}')
" 2>/dev/null || echo "   (no collections yet — data may still be ingesting)"

echo ""
echo "2) Hail events within 50 km of OKC (lat=35.5, lon=-97.5):"
curl -sf "${EDR_URL}/edr/collections/hail/radius?coords=POINT(-97.5+35.5)&within=50km" | python3 -c "
import sys,json
d=json.load(sys.stdin)
feats=d.get('features',[])
print(f'   {len(feats)} feature(s)')
for f in feats[:3]:
    p=f.get('properties',{})
    print(f'   event {p.get(\"event_id\")}  mag={p.get(\"magnitude\")} {p.get(\"magnitude_unit\",\"\")}  {p.get(\"begin_time\",\"\")[:10]}')
" 2>/dev/null || echo "   (endpoint not ready or no data yet)"

echo ""
echo "3) Tornado tracks via items endpoint:"
curl -sf "${EDR_URL}/edr/collections/tornado/items?bbox=-99,34,-96,37" | python3 -c "
import sys,json
d=json.load(sys.stdin)
feats=d.get('features',[])
print(f'   {len(feats)} feature(s)')
for f in feats[:3]:
    p=f.get('properties',{})
    g=f.get('geometry',{})
    print(f'   event {p.get(\"event_id\")}  EF{p.get(\"tor_f_scale\")}  {g.get(\"type\")}  {p.get(\"begin_time\",\"\")[:10]}')
" 2>/dev/null || echo "   (endpoint not ready or no data yet)"

echo ""
echo "4) County aggregate (hail, Oklahoma, geometry=false):"
curl -sf "${EDR_URL}/edr/collections/hail/counties?state=OK&geometry=false" | python3 -c "
import sys,json
d=json.load(sys.stdin)
feats=d.get('features',[])
print(f'   {len(feats)} county/counties with hail events')
for f in feats[:5]:
    p=f.get('properties',{})
    print(f'   {p.get(\"county_fips\")}  {p.get(\"name\",\"\"):20s}  total={p.get(\"total\")}  years={list(p.get(\"by_year\",{}).keys())}')
" 2>/dev/null || echo "   (county aggregate may need TIGER counties loaded first)"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Services:"
echo "    ingester  →  ${INGESTER_URL}          logs: /tmp/ingester.log"
echo "    edr-api   →  ${EDR_URL}/edr            logs: /tmp/edr-api.log"
echo ""
echo "  To stop:  ./scripts/dev_storm_events.sh --stop"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
