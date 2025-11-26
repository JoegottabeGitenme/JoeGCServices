#!/bin/bash
# Quick WMTS 1.0.0 Compliance Validation
# Runs lightweight checks to verify OGC WMTS compliance

set -e

# Configuration
WMTS_URL="${WMTS_URL:-http://localhost:8080/wmts}"
VERBOSE="${VERBOSE:-0}"
QUICK="${QUICK:-0}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Counters
TOTAL_CHECKS=0
PASSED_CHECKS=0
FAILED_CHECKS=0

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --verbose|-v)
            VERBOSE=1
            shift
            ;;
        --quick|-q)
            QUICK=1
            shift
            ;;
        --url)
            WMTS_URL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--verbose] [--quick] [--url WMTS_URL]"
            exit 1
            ;;
    esac
done

# Helper functions
log_info() {
    if [ "$VERBOSE" -eq 1 ]; then
        echo -e "${NC}$1${NC}"
    fi
}

log_success() {
    echo -e "${GREEN}✓${NC} $1"
}

log_failure() {
    echo -e "${RED}✗${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

check_pass() {
    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
    PASSED_CHECKS=$((PASSED_CHECKS + 1))
    log_success "$1"
}

check_fail() {
    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
    FAILED_CHECKS=$((FAILED_CHECKS + 1))
    log_failure "$1"
}

# Start validation
echo ""
echo "═══════════════════════════════════════════════════════"
echo "  WMTS 1.0.0 Quick Compliance Validation"
echo "═══════════════════════════════════════════════════════"
echo "  Endpoint: $WMTS_URL"
echo "═══════════════════════════════════════════════════════"
echo ""

# Check 1: GetCapabilities returns valid XML (KVP)
log_info "Checking GetCapabilities (KVP)..."
CAPS_RESPONSE=$(curl -sf "${WMTS_URL}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0" || echo "ERROR")

if [ "$CAPS_RESPONSE" = "ERROR" ]; then
    check_fail "GetCapabilities request failed (HTTP error)"
    exit 1
else
    # Check if response contains XML declaration
    if echo "$CAPS_RESPONSE" | grep -q "<?xml"; then
        check_pass "GetCapabilities returns XML (KVP)"
    else
        check_fail "GetCapabilities does not return valid XML"
        exit 1
    fi
fi

# Check 2: GetCapabilities has Capabilities root element
if echo "$CAPS_RESPONSE" | grep -q "<Capabilities"; then
    check_pass "GetCapabilities has Capabilities root element"
else
    check_fail "GetCapabilities missing Capabilities root element"
fi

# Check 3: Correct WMTS version
if echo "$CAPS_RESPONSE" | grep -q 'version="1.0.0"'; then
    check_pass "WMTS version is 1.0.0"
else
    check_fail "WMTS version is not 1.0.0"
fi

# Check 4: ServiceIdentification present
if echo "$CAPS_RESPONSE" | grep -q "<ows:ServiceIdentification"; then
    check_pass "ServiceIdentification present"
else
    check_fail "ServiceIdentification missing"
fi

# Check 5: Required operations present
if echo "$CAPS_RESPONSE" | grep -q "GetCapabilities"; then
    if echo "$CAPS_RESPONSE" | grep -q "GetTile"; then
        check_pass "Required operations present (GetCapabilities, GetTile)"
    else
        check_fail "GetTile operation missing"
    fi
else
    check_fail "Operations metadata missing"
fi

# Check 6: Layers are defined
LAYER_COUNT=$(echo "$CAPS_RESPONSE" | grep -c "<Layer>" || echo "0")
if [ "$LAYER_COUNT" -gt 0 ]; then
    check_pass "Layers defined ($LAYER_COUNT layers found)"
else
    check_fail "No layers found in capabilities"
fi

# Extract layer identifiers for testing
LAYERS=$(echo "$CAPS_RESPONSE" | grep -oP '(?<=<ows:Identifier>)gfs_[^<]+' | head -5)
LAYER_ARRAY=($LAYERS)
LAYER_COUNT=${#LAYER_ARRAY[@]}

log_info "Found layers: ${LAYER_ARRAY[*]}"

# Check 7: TileMatrixSet defined
if echo "$CAPS_RESPONSE" | grep -q "WebMercatorQuad"; then
    check_pass "WebMercatorQuad TileMatrixSet present"
else
    check_fail "WebMercatorQuad TileMatrixSet missing"
fi

# Check 8: TileMatrixSet has TileMatrix definitions
TILEMATRIX_COUNT=$(echo "$CAPS_RESPONSE" | grep -c "<TileMatrix>" || echo "0")
if [ "$TILEMATRIX_COUNT" -gt 0 ]; then
    check_pass "TileMatrix definitions present ($TILEMATRIX_COUNT levels)"
else
    check_fail "No TileMatrix definitions found"
fi

# Check 9: REST endpoint (GetTile via REST)
if [ "$LAYER_COUNT" -gt 0 ]; then
    TEST_LAYER="${LAYER_ARRAY[0]}"
    log_info "Testing GetTile REST with layer: $TEST_LAYER"
    
    # Test a tile at zoom 2, x=1, y=1
    REST_URL="${WMTS_URL}/rest/${TEST_LAYER}/default/WebMercatorQuad/2/1/1.png"
    
    HTTP_CODE=$(curl -s -o /tmp/wmts_test.png -w "%{http_code}" "$REST_URL")
    
    if [ "$HTTP_CODE" = "200" ]; then
        # Check if it's actually a PNG
        if file /tmp/wmts_test.png | grep -q "PNG image data"; then
            check_pass "GetTile REST returns valid PNG image"
        else
            check_fail "GetTile REST returns HTTP 200 but not a PNG image"
        fi
    else
        check_fail "GetTile REST returned HTTP $HTTP_CODE (expected 200)"
    fi
    
    rm -f /tmp/wmts_test.png
fi

# Check 10: KVP endpoint (GetTile via KVP)
if [ "$LAYER_COUNT" -gt 0 ]; then
    log_info "Testing GetTile KVP..."
    
    KVP_URL="${WMTS_URL}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${TEST_LAYER}&STYLE=default&FORMAT=image/png&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=2&TILEROW=1&TILECOL=1"
    
    HTTP_CODE=$(curl -s -o /tmp/wmts_test_kvp.png -w "%{http_code}" "$KVP_URL")
    
    if [ "$HTTP_CODE" = "200" ]; then
        if file /tmp/wmts_test_kvp.png | grep -q "PNG image data"; then
            check_pass "GetTile KVP returns valid PNG image"
        else
            check_fail "GetTile KVP returns HTTP 200 but not a PNG"
        fi
    else
        check_fail "GetTile KVP returned HTTP $HTTP_CODE"
    fi
    
    rm -f /tmp/wmts_test_kvp.png
fi

# Check 11: Test multiple zoom levels (if not quick mode)
if [ "$QUICK" -eq 0 ] && [ "$LAYER_COUNT" -gt 0 ]; then
    log_info "Testing multiple zoom levels..."
    
    ZOOM_LEVELS=(0 1 2 3)
    ZOOMS_TESTED=0
    ZOOMS_PASSED=0
    
    for zoom in "${ZOOM_LEVELS[@]}"; do
        ZOOMS_TESTED=$((ZOOMS_TESTED + 1))
        
        ZOOM_URL="${WMTS_URL}/rest/${TEST_LAYER}/default/WebMercatorQuad/${zoom}/0/0.png"
        
        HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$ZOOM_URL")
        
        if [ "$HTTP_CODE" = "200" ]; then
            ZOOMS_PASSED=$((ZOOMS_PASSED + 1))
            log_info "  ✓ Zoom level $zoom: OK"
        else
            log_info "  ✗ Zoom level $zoom: FAILED (HTTP $HTTP_CODE)"
        fi
    done
    
    if [ "$ZOOMS_PASSED" -eq "$ZOOMS_TESTED" ]; then
        check_pass "All zoom levels work ($ZOOMS_PASSED/$ZOOMS_TESTED)"
    else
        check_fail "Some zoom levels failed ($ZOOMS_PASSED/$ZOOMS_TESTED passed)"
    fi
fi

# Check 12: Test all layers (if not quick mode)
if [ "$QUICK" -eq 0 ] && [ "$LAYER_COUNT" -gt 1 ]; then
    log_info "Testing all layers..."
    
    LAYERS_TESTED=0
    LAYERS_PASSED=0
    
    for layer in "${LAYER_ARRAY[@]}"; do
        LAYERS_TESTED=$((LAYERS_TESTED + 1))
        
        LAYER_URL="${WMTS_URL}/rest/${layer}/default/WebMercatorQuad/2/1/1.png"
        
        HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$LAYER_URL")
        
        if [ "$HTTP_CODE" = "200" ]; then
            LAYERS_PASSED=$((LAYERS_PASSED + 1))
            log_info "  ✓ Layer $layer: OK"
        else
            log_info "  ✗ Layer $layer: FAILED (HTTP $HTTP_CODE)"
        fi
    done
    
    if [ "$LAYERS_PASSED" -eq "$LAYERS_TESTED" ]; then
        check_pass "All layers return tiles ($LAYERS_PASSED/$LAYERS_TESTED)"
    else
        check_fail "Some layers failed to return tiles ($LAYERS_PASSED/$LAYERS_TESTED passed)"
    fi
fi

# Summary
echo ""
echo "═══════════════════════════════════════════════════════"
echo "  Validation Summary"
echo "═══════════════════════════════════════════════════════"
echo "  Total checks: $TOTAL_CHECKS"
echo -e "  ${GREEN}Passed: $PASSED_CHECKS${NC}"
echo -e "  ${RED}Failed: $FAILED_CHECKS${NC}"
echo "═══════════════════════════════════════════════════════"
echo ""

if [ "$FAILED_CHECKS" -eq 0 ]; then
    echo -e "${GREEN}✓ WMTS 1.0.0 validation PASSED${NC}"
    echo ""
    exit 0
else
    echo -e "${RED}✗ WMTS 1.0.0 validation FAILED${NC}"
    echo ""
    exit 1
fi
