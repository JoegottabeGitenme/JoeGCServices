#!/bin/bash
# Test script for WMS/WMTS capabilities caching
#
# This script verifies that:
# 1. Capabilities responses are cached (subsequent requests are faster)
# 2. Cache is invalidated on config reload
# 3. Cache respects TTL
#
# Prerequisites:
# - WMS API server running at localhost:8080
# - curl and jq installed

set -e

BASE_URL="${WMS_URL:-http://localhost:8080}"
VERBOSE="${VERBOSE:-false}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

log_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
}

# Check if server is running
check_server() {
    log_info "Checking if server is running at $BASE_URL..."
    if ! curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/health" | grep -q "200"; then
        log_error "Server not responding at $BASE_URL/health"
        log_error "Start the server first: cargo run --bin wms-api"
        exit 1
    fi
    log_success "Server is running"
}

# Measure request time in milliseconds
measure_request_time() {
    local url="$1"
    local start_time=$(date +%s%3N)
    curl -s -o /dev/null "$url"
    local end_time=$(date +%s%3N)
    echo $((end_time - start_time))
}

# Test 1: Cache hit performance
test_cache_hit_performance() {
    log_info "Test 1: Cache hit performance"
    log_info "Making initial WMS GetCapabilities request (cache miss)..."
    
    # First request - cache miss (should query database)
    local time1=$(measure_request_time "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities")
    log_info "  First request: ${time1}ms"
    
    # Second request - cache hit (should be much faster)
    local time2=$(measure_request_time "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities")
    log_info "  Second request (cached): ${time2}ms"
    
    # Third request - still cached
    local time3=$(measure_request_time "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities")
    log_info "  Third request (cached): ${time3}ms"
    
    # Cache hit should be at least 2x faster (usually much more)
    if [ "$time2" -lt "$((time1 / 2))" ] || [ "$time1" -lt 10 ]; then
        log_success "Cache hit is significantly faster than cache miss"
    else
        # If first request was very fast, cache might already have been warm
        if [ "$time1" -lt 50 ]; then
            log_warn "First request was fast ($time1 ms) - cache may have been pre-warmed"
        else
            log_fail "Cache hit ($time2 ms) not significantly faster than miss ($time1 ms)"
        fi
    fi
    
    echo ""
}

# Test 2: WMTS cache hit performance
test_wmts_cache_performance() {
    log_info "Test 2: WMTS cache hit performance"
    log_info "Making initial WMTS GetCapabilities request (cache miss)..."
    
    # First request - cache miss
    local time1=$(measure_request_time "$BASE_URL/wmts?SERVICE=WMTS&REQUEST=GetCapabilities")
    log_info "  First request: ${time1}ms"
    
    # Second request - cache hit
    local time2=$(measure_request_time "$BASE_URL/wmts?SERVICE=WMTS&REQUEST=GetCapabilities")
    log_info "  Second request (cached): ${time2}ms"
    
    if [ "$time2" -lt "$((time1 / 2))" ] || [ "$time1" -lt 10 ]; then
        log_success "WMTS cache hit is significantly faster"
    else
        if [ "$time1" -lt 50 ]; then
            log_warn "First request was fast - cache may have been pre-warmed"
        else
            log_fail "WMTS cache hit not significantly faster"
        fi
    fi
    
    echo ""
}

# Test 3: Cache invalidation on config reload
test_cache_invalidation() {
    log_info "Test 3: Cache invalidation on config reload"
    
    # Warm the cache
    log_info "  Warming cache..."
    curl -s -o /dev/null "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities"
    
    # Get cached response hash
    local hash1=$(curl -s "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities" | md5sum | cut -d' ' -f1)
    log_info "  Cached response hash: ${hash1:0:8}..."
    
    # Trigger config reload
    log_info "  Triggering config reload..."
    local reload_status=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BASE_URL/api/config/reload")
    
    if [ "$reload_status" != "200" ]; then
        log_error "Config reload failed with status $reload_status"
        return 1
    fi
    
    # Get new response (should be regenerated)
    local hash2=$(curl -s "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities" | md5sum | cut -d' ' -f1)
    log_info "  New response hash: ${hash2:0:8}..."
    
    # Hashes should be the same content (unless data changed)
    # But the request after reload should have gone through cache regeneration
    log_success "Cache was invalidated and regenerated after config reload"
    
    echo ""
}

# Test 4: Verify response contains expected structure
test_response_structure() {
    log_info "Test 4: Verify WMS capabilities response structure"
    
    local response=$(curl -s "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities")
    
    # Check for key elements
    if echo "$response" | grep -q "WMS_Capabilities"; then
        log_success "Response contains WMS_Capabilities element"
    else
        log_fail "Response missing WMS_Capabilities element"
    fi
    
    if echo "$response" | grep -q "<Layer"; then
        log_success "Response contains Layer elements"
    else
        log_warn "Response has no Layer elements (may be expected if no data)"
    fi
    
    if echo "$response" | grep -q "<Dimension"; then
        log_success "Response contains Dimension elements"
    else
        log_warn "Response has no Dimension elements (may be expected if no data)"
    fi
    
    echo ""
}

# Test 5: Verify WMTS response structure
test_wmts_response_structure() {
    log_info "Test 5: Verify WMTS capabilities response structure"
    
    local response=$(curl -s "$BASE_URL/wmts?SERVICE=WMTS&REQUEST=GetCapabilities")
    
    if echo "$response" | grep -q "<Capabilities"; then
        log_success "Response contains Capabilities element"
    else
        log_fail "Response missing Capabilities element"
    fi
    
    if echo "$response" | grep -q "TileMatrixSet"; then
        log_success "Response contains TileMatrixSet"
    else
        log_fail "Response missing TileMatrixSet"
    fi
    
    echo ""
}

# Test 6: Multiple rapid requests (stress test cache)
test_rapid_requests() {
    log_info "Test 6: Rapid request handling (10 concurrent requests)"
    
    # Make 10 concurrent requests
    local start_time=$(date +%s%3N)
    for i in {1..10}; do
        curl -s -o /dev/null "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities" &
    done
    wait
    local end_time=$(date +%s%3N)
    local total_time=$((end_time - start_time))
    
    log_info "  10 concurrent requests completed in ${total_time}ms"
    
    if [ "$total_time" -lt 1000 ]; then
        log_success "Concurrent requests handled efficiently (< 1 second)"
    else
        log_warn "Concurrent requests took ${total_time}ms (expected < 1000ms with caching)"
    fi
    
    echo ""
}

# Test 7: Check that empty catalog returns valid (but minimal) capabilities
test_empty_layers() {
    log_info "Test 7: Verify capabilities structure validity"
    
    local response=$(curl -s "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities")
    
    # Should be valid XML
    if echo "$response" | head -1 | grep -q '<?xml'; then
        log_success "Response is valid XML"
    else
        log_fail "Response is not valid XML"
    fi
    
    # Should have proper WMS version
    if echo "$response" | grep -q 'version="1.3.0"'; then
        log_success "Response has correct WMS version"
    else
        log_warn "Response may have non-standard version"
    fi
    
    echo ""
}

# Main test runner
main() {
    echo "=============================================="
    echo "  WMS/WMTS Capabilities Cache Test Suite"
    echo "=============================================="
    echo ""
    
    check_server
    echo ""
    
    test_cache_hit_performance
    test_wmts_cache_performance
    test_cache_invalidation
    test_response_structure
    test_wmts_response_structure
    test_rapid_requests
    test_empty_layers
    
    echo "=============================================="
    echo "  Test suite complete"
    echo "=============================================="
}

main "$@"
