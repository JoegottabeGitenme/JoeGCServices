#!/bin/bash
#
# Reset system to consistent state for benchmarking and testing
#
# This script:
# 1. Clears L1 in-memory caches (tile cache, GRIB cache, grid cache)
# 2. Flushes Redis tile cache (L2)
# 3. Clears any pending render jobs
# 4. Optionally restarts WMS API service
#
# Usage:
#   ./reset_test_state.sh           # Clear all caches
#   ./reset_test_state.sh --restart # Clear caches and restart API

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Script directory and project root
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

cd "$PROJECT_ROOT"

echo ""
log_info "====================================="
log_info "Resetting System Test State"
log_info "====================================="
echo ""

# Get Redis container name and check if running
REDIS_CONTAINER=$(docker-compose ps -q redis 2>/dev/null)
if [ -z "$REDIS_CONTAINER" ]; then
    log_error "Redis is not running. Please start services first:"
    echo "  ./start.sh"
    exit 1
fi

# 1. Clear L1 in-memory caches via API
log_info "Step 1/4: Clearing L1 in-memory caches..."
# Use timeout command as extra safeguard against curl hangs, plus connect-timeout for fast fail
L1_RESPONSE=$(timeout 10 curl -s --connect-timeout 2 --max-time 5 -X POST "http://localhost:8080/api/cache/clear" 2>/dev/null || echo "")
if echo "$L1_RESPONSE" | grep -q '"success":true'; then
    L1_CLEARED=$(echo "$L1_RESPONSE" | grep -o '"l1_tile_cache":[0-9]*' | cut -d: -f2)
    GRIB_CLEARED=$(echo "$L1_RESPONSE" | grep -o '"grib_cache":[0-9]*' | cut -d: -f2)
    GRID_CLEARED=$(echo "$L1_RESPONSE" | grep -o '"grid_cache":[0-9]*' | cut -d: -f2)
    log_success "  L1 tile cache cleared: ${L1_CLEARED:-0} entries"
    log_success "  GRIB cache cleared: ${GRIB_CLEARED:-0} entries"
    log_success "  Grid cache cleared: ${GRID_CLEARED:-0} entries"
else
    log_warn "  Could not clear L1 cache (API may not be running or timed out)"
    log_warn "  Continuing with Redis cache clear..."
fi

# 2. Flush Redis cache (L2)
# Use docker exec directly instead of docker-compose exec to avoid TTY issues in loops
log_info "Step 2/4: Flushing Redis tile cache (L2)..."
KEYS_BEFORE=$(timeout 5 docker exec "$REDIS_CONTAINER" redis-cli DBSIZE 2>/dev/null | tr -d '\r' | grep -oE '[0-9]+' || echo "0")
log_info "  Cache keys before: ${KEYS_BEFORE:-0}"

if timeout 5 docker exec "$REDIS_CONTAINER" redis-cli FLUSHALL >/dev/null 2>&1; then
    KEYS_AFTER=$(timeout 5 docker exec "$REDIS_CONTAINER" redis-cli DBSIZE 2>/dev/null | tr -d '\r' | grep -oE '[0-9]+' || echo "0")
    log_success "  Redis cache cleared! Keys remaining: ${KEYS_AFTER:-0}"
else
    log_error "Failed to flush Redis cache (timeout or connection error)"
    exit 1
fi

# 3. Clear any pending render jobs (if using a job queue)
log_info "Step 3/4: Clearing render queue..."
QUEUE_LEN=$(timeout 5 docker exec "$REDIS_CONTAINER" redis-cli LLEN render:queue 2>/dev/null | tr -d '\r' | grep -oE '[0-9]+' || echo "0")
if [ "$QUEUE_LEN" != "0" ] && [ "$QUEUE_LEN" != "" ]; then
    timeout 5 docker exec "$REDIS_CONTAINER" redis-cli DEL render:queue >/dev/null 2>&1 || true
    log_success "  Cleared ${QUEUE_LEN} pending render jobs"
else
    log_info "  No pending jobs in queue"
fi

# 4. Optional: Restart WMS API to clear in-memory state
if [ "$1" = "--restart" ]; then
    log_info "Step 4/4: Restarting WMS API service..."
    docker-compose restart wms-api
    
    # Wait for API to be healthy
    log_info "  Waiting for API to be ready..."
    retries=15
    while [ $retries -gt 0 ]; do
        if curl -s -f "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities" &>/dev/null; then
            log_success "  API is healthy!"
            break
        fi
        echo -ne "\r  Waiting... ($retries seconds remaining)"
        sleep 1
        retries=$((retries - 1))
    done
    
    echo ""
    
    if [ $retries -eq 0 ]; then
        log_warn "  API may not be fully ready yet"
    fi
else
    log_info "Step 4/4: Skipping API restart (use --restart to enable)"
fi

echo ""
log_success "====================================="
log_success "System reset complete!"
log_success "====================================="
echo ""
log_info "System is now in a clean state for:"
log_info "  • Performance benchmarking"
log_info "  • Cache miss testing"
log_info "  • Consistent test results"
echo ""
log_info "Next steps:"
log_info "  • Run load tests: ./validation/load-test/run.sh"
log_info "  • Test rendering: ./scripts/test_rendering.sh"
log_info "  • Warm cache: curl GetMap requests"
echo ""
