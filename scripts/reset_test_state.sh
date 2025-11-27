#!/bin/bash
#
# Reset system to consistent state for benchmarking and testing
#
# This script:
# 1. Flushes Redis tile cache
# 2. Clears any pending render jobs
# 3. Optionally restarts WMS API service
#
# Usage:
#   ./reset_test_state.sh           # Clear cache only
#   ./reset_test_state.sh --restart # Clear cache and restart API

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

# Check if Redis is running
if ! docker-compose ps redis 2>/dev/null | grep -q "Up"; then
    log_error "Redis is not running. Please start services first:"
    echo "  ./start.sh"
    exit 1
fi

# 1. Flush Redis cache
log_info "Step 1/3: Flushing Redis tile cache..."
KEYS_BEFORE=$(docker-compose exec -T redis redis-cli DBSIZE 2>/dev/null | tr -d '\r')
log_info "  Cache keys before: ${KEYS_BEFORE:-0}"

if docker-compose exec -T redis redis-cli FLUSHALL &>/dev/null; then
    KEYS_AFTER=$(docker-compose exec -T redis redis-cli DBSIZE 2>/dev/null | tr -d '\r')
    log_success "  Cache cleared! Keys remaining: ${KEYS_AFTER:-0}"
else
    log_error "Failed to flush Redis cache"
    exit 1
fi

# 2. Clear any pending render jobs (if using a job queue)
log_info "Step 2/3: Clearing render queue..."
QUEUE_LEN=$(docker-compose exec -T redis redis-cli LLEN render:queue 2>/dev/null | tr -d '\r' || echo "0")
if [ "$QUEUE_LEN" != "0" ]; then
    docker-compose exec -T redis redis-cli DEL render:queue &>/dev/null || true
    log_success "  Cleared ${QUEUE_LEN} pending render jobs"
else
    log_info "  No pending jobs in queue"
fi

# 3. Optional: Restart WMS API to clear in-memory state
if [ "$1" = "--restart" ]; then
    log_info "Step 3/3: Restarting WMS API service..."
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
    log_info "Step 3/3: Skipping API restart (use --restart to enable)"
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
