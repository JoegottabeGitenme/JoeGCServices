#!/bin/bash

# Quick integration test for LUT functionality
#
# This script tests:
# 1. LUT generation works
# 2. LUT loading works (via service logs)
# 3. GOES tiles render correctly
#
# Usage: ./scripts/test_lut_integration.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
LUT_DIR="${PROJECT_DIR}/data/luts"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() {
    echo -e "$1"
}

log_header() {
    log "\n${BLUE}=== $1 ===${NC}\n"
}

log_success() {
    log "${GREEN}✓ $1${NC}"
}

log_error() {
    log "${RED}✗ $1${NC}"
}

log_warn() {
    log "${YELLOW}⚠ $1${NC}"
}

# Test 1: LUT generation
log_header "Test 1: LUT Generation"

if [ ! -f "${LUT_DIR}/goes16_conus_z0-7.lut" ]; then
    log "Generating GOES-16 LUT (this takes ~6 seconds)..."
    mkdir -p "$LUT_DIR"
    cargo run --release --bin generate-goes-lut -- \
        --output "$LUT_DIR" \
        --satellite goes16 \
        --max-zoom 7 2>&1 | tail -5
    
    if [ -f "${LUT_DIR}/goes16_conus_z0-7.lut" ]; then
        log_success "LUT file generated"
    else
        log_error "LUT file not generated"
        exit 1
    fi
else
    log_success "LUT file already exists: ${LUT_DIR}/goes16_conus_z0-7.lut"
fi

# Show file info
ls -lh "${LUT_DIR}/goes16_conus_z0-7.lut"

# Test 2: Performance test (standalone)
log_header "Test 2: Performance Comparison"

log "Running performance test..."
cargo run --release --bin test-lut-performance 2>&1 | tail -15

# Test 3: Check if service is running and test actual tiles
log_header "Test 3: Service Integration"

API_URL="${API_URL:-http://localhost:8080}"

if curl -s "${API_URL}/health" > /dev/null 2>&1; then
    log_success "Service is running at ${API_URL}"
    
    # Check LUT config
    log "\nChecking service configuration..."
    CONFIG=$(curl -s "${API_URL}/api/config" 2>/dev/null || echo "{}")
    
    LUT_ENABLED=$(echo "$CONFIG" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('optimization_config', {}).get('projection_lut_enabled', False))
except:
    print('unknown')
" 2>/dev/null || echo "unknown")

    if [ "$LUT_ENABLED" = "True" ] || [ "$LUT_ENABLED" = "true" ]; then
        log_success "LUT is enabled in service"
    else
        log_warn "LUT is NOT enabled in service"
        log "  To enable: Set ENABLE_PROJECTION_LUT=true and PROJECTION_LUT_DIR=${LUT_DIR}"
    fi
    
    # Check if GOES data is available
    CAPS=$(curl -s "${API_URL}/wms?SERVICE=WMS&REQUEST=GetCapabilities" 2>/dev/null || echo "")
    if echo "$CAPS" | grep -q "goes16_CMI"; then
        log_success "GOES-16 data is available"
        
        # Time a sample tile request
        log "\nTiming sample tile requests..."
        
        TILES=("5/7/11" "6/14/22" "7/28/44")
        
        for tile in "${TILES[@]}"; do
            IFS='/' read -r z x y <<< "$tile"
            url="${API_URL}/tiles/goes16_CMI_C13/default/${z}/${x}/${y}.png"
            
            # Time the request
            start=$(python3 -c 'import time; print(int(time.time() * 1000))')
            http_code=$(curl -s -o /dev/null -w "%{http_code}" "$url")
            end=$(python3 -c 'import time; print(int(time.time() * 1000))')
            elapsed=$((end - start))
            
            if [ "$http_code" = "200" ]; then
                log "  Tile ${tile}: ${elapsed} ms (HTTP ${http_code})"
            else
                log_warn "  Tile ${tile}: HTTP ${http_code}"
            fi
        done
    else
        log_warn "GOES-16 data not available"
        log "  Run: ./scripts/download_goes.sh && ./scripts/ingest_test_data.sh"
    fi
else
    log_warn "Service not running at ${API_URL}"
    log "  Start with: docker-compose up -d"
    log "  Or run locally with LUT enabled:"
    log "    ENABLE_PROJECTION_LUT=true PROJECTION_LUT_DIR=${LUT_DIR} cargo run --release --bin wms-api"
fi

log_header "Summary"
log "LUT file: ${LUT_DIR}/goes16_conus_z0-7.lut"
log "Expected performance with LUT: ~0.1-0.2 ms for resampling"
log "Expected performance without LUT: ~7-8 ms for resampling"
log ""
log "To run the full benchmark against the service:"
log "  ./scripts/benchmark_lut_performance.sh"
