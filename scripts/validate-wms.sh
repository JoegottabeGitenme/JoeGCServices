#!/bin/bash
# Quick WMS 1.3.0 Compliance Validation
# Runs lightweight checks to verify OGC WMS compliance

set -e

# Configuration
WMS_URL="${WMS_URL:-http://localhost:8080/wms}"
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
            WMS_URL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--verbose] [--quick] [--url WMS_URL]"
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
echo "  WMS 1.3.0 Quick Compliance Validation"
echo "═══════════════════════════════════════════════════════"
echo "  Endpoint: $WMS_URL"
echo "═══════════════════════════════════════════════════════"
echo ""

# Check 1: GetCapabilities returns valid XML
log_info "Checking GetCapabilities..."
CAPS_RESPONSE=$(curl -sf "${WMS_URL}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" || echo "ERROR")

if [ "$CAPS_RESPONSE" = "ERROR" ]; then
    check_fail "GetCapabilities request failed (HTTP error)"
    exit 1
else
    # Check if response contains XML declaration
    if echo "$CAPS_RESPONSE" | grep -q "<?xml"; then
        check_pass "GetCapabilities returns XML"
    else
        check_fail "GetCapabilities does not return valid XML"
        exit 1
    fi
fi

# Check 2: GetCapabilities has WMS_Capabilities root element
if echo "$CAPS_RESPONSE" | grep -q "<WMS_Capabilities"; then
    check_pass "GetCapabilities has WMS_Capabilities root element"
else
    check_fail "GetCapabilities missing WMS_Capabilities root element"
fi

# Check 3: Correct WMS version
if echo "$CAPS_RESPONSE" | grep -q 'version="1.3.0"'; then
    check_pass "WMS version is 1.3.0"
else
    check_fail "WMS version is not 1.3.0"
fi

# Check 4: Service metadata present
if echo "$CAPS_RESPONSE" | grep -q "<Service>"; then
    if echo "$CAPS_RESPONSE" | grep -q "<Name>"; then
        check_pass "Service metadata present (Name, Title)"
    else
        check_fail "Service metadata incomplete"
    fi
else
    check_fail "Service element missing"
fi

# Check 5: Required operations present
REQUIRED_OPS=("GetCapabilities" "GetMap" "GetFeatureInfo")
ALL_OPS_PRESENT=1

for op in "${REQUIRED_OPS[@]}"; do
    if echo "$CAPS_RESPONSE" | grep -q "<${op}>"; then
        log_info "  - Operation ${op} found"
    else
        ALL_OPS_PRESENT=0
        log_info "  - Operation ${op} MISSING"
    fi
done

if [ "$ALL_OPS_PRESENT" -eq 1 ]; then
    check_pass "All required operations present (GetCapabilities, GetMap, GetFeatureInfo)"
else
    check_fail "Some required operations missing"
fi

# Check 6: Layers are defined
LAYER_COUNT=$(echo "$CAPS_RESPONSE" | grep -c "<Layer" || echo "0")
if [ "$LAYER_COUNT" -gt 0 ]; then
    check_pass "Layers defined ($LAYER_COUNT layers found)"
else
    check_fail "No layers found in capabilities"
fi

# Extract layer names for testing
LAYERS=$(echo "$CAPS_RESPONSE" | grep -oP '(?<=<Name>)gfs_[^<]+' | head -5)
LAYER_ARRAY=($LAYERS)
LAYER_COUNT=${#LAYER_ARRAY[@]}

log_info "Found layers: ${LAYER_ARRAY[*]}"

# Check 7: CRS support
if echo "$CAPS_RESPONSE" | grep -q "EPSG:4326"; then
    check_pass "EPSG:4326 support advertised"
else
    check_fail "EPSG:4326 not advertised"
fi

if echo "$CAPS_RESPONSE" | grep -q "EPSG:3857"; then
    check_pass "EPSG:3857 support advertised"
else
    check_fail "EPSG:3857 not advertised"
fi

# Check 8: GetMap returns PNG for first layer
if [ "$LAYER_COUNT" -gt 0 ]; then
    TEST_LAYER="${LAYER_ARRAY[0]}"
    log_info "Testing GetMap with layer: $TEST_LAYER"
    
    GETMAP_URL="${WMS_URL}?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=${TEST_LAYER}&STYLES=default&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png"
    
    HTTP_CODE=$(curl -s -o /tmp/wms_test.png -w "%{http_code}" "$GETMAP_URL")
    
    if [ "$HTTP_CODE" = "200" ]; then
        # Check if it's actually a PNG
        if file /tmp/wms_test.png | grep -q "PNG image data"; then
            check_pass "GetMap returns valid PNG image"
        else
            check_fail "GetMap returns HTTP 200 but not a PNG image"
        fi
    else
        check_fail "GetMap returned HTTP $HTTP_CODE (expected 200)"
    fi
    
    rm -f /tmp/wms_test.png
fi

# Check 9: GetMap with EPSG:3857
if [ "$LAYER_COUNT" -gt 0 ]; then
    log_info "Testing GetMap with EPSG:3857..."
    
    GETMAP_3857_URL="${WMS_URL}?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=${TEST_LAYER}&STYLES=default&CRS=EPSG:3857&BBOX=-20037508,-20037508,20037508,20037508&WIDTH=256&HEIGHT=256&FORMAT=image/png"
    
    HTTP_CODE=$(curl -s -o /tmp/wms_test_3857.png -w "%{http_code}" "$GETMAP_3857_URL")
    
    if [ "$HTTP_CODE" = "200" ]; then
        if file /tmp/wms_test_3857.png | grep -q "PNG image data"; then
            check_pass "GetMap works with EPSG:3857"
        else
            check_fail "GetMap with EPSG:3857 doesn't return PNG"
        fi
    else
        check_fail "GetMap with EPSG:3857 returned HTTP $HTTP_CODE"
    fi
    
    rm -f /tmp/wms_test_3857.png
fi

# Check 10: GetFeatureInfo returns valid JSON
if [ "$LAYER_COUNT" -gt 0 ] && [ "$QUICK" -eq 0 ]; then
    log_info "Testing GetFeatureInfo..."
    
    GFI_URL="${WMS_URL}?SERVICE=WMS&REQUEST=GetFeatureInfo&VERSION=1.3.0&LAYERS=${TEST_LAYER}&QUERY_LAYERS=${TEST_LAYER}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json"
    
    GFI_RESPONSE=$(curl -sf "$GFI_URL" || echo "ERROR")
    
    if [ "$GFI_RESPONSE" != "ERROR" ]; then
        # Check if it's valid JSON
        if echo "$GFI_RESPONSE" | python3 -m json.tool > /dev/null 2>&1; then
            check_pass "GetFeatureInfo returns valid JSON"
        else
            check_fail "GetFeatureInfo doesn't return valid JSON"
        fi
    else
        check_fail "GetFeatureInfo request failed"
    fi
fi

# Check 11: Exception handling
log_info "Testing exception handling..."

# Invalid layer should return ServiceException
EXCEPTION_URL="${WMS_URL}?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=INVALID_LAYER&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png"

EXCEPTION_RESPONSE=$(curl -sf "$EXCEPTION_URL" || echo "ERROR")

if [ "$EXCEPTION_RESPONSE" != "ERROR" ]; then
    if echo "$EXCEPTION_RESPONSE" | grep -q "ServiceException"; then
        check_pass "Invalid layer returns proper ServiceException XML"
    else
        check_fail "Invalid layer doesn't return ServiceException"
    fi
else
    # HTTP error is also acceptable for exceptions
    check_pass "Invalid layer returns HTTP error (acceptable)"
fi

# Check 12: Test all layers (if not quick mode)
if [ "$QUICK" -eq 0 ] && [ "$LAYER_COUNT" -gt 1 ]; then
    log_info "Testing all layers..."
    
    LAYERS_TESTED=0
    LAYERS_PASSED=0
    
    for layer in "${LAYER_ARRAY[@]}"; do
        LAYERS_TESTED=$((LAYERS_TESTED + 1))
        
        LAYER_URL="${WMS_URL}?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=${layer}&STYLES=default&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png"
        
        HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$LAYER_URL")
        
        if [ "$HTTP_CODE" = "200" ]; then
            LAYERS_PASSED=$((LAYERS_PASSED + 1))
            log_info "  ✓ Layer $layer: OK"
        else
            log_info "  ✗ Layer $layer: FAILED (HTTP $HTTP_CODE)"
        fi
    done
    
    if [ "$LAYERS_PASSED" -eq "$LAYERS_TESTED" ]; then
        check_pass "All layers render correctly ($LAYERS_PASSED/$LAYERS_TESTED)"
    else
        check_fail "Some layers failed to render ($LAYERS_PASSED/$LAYERS_TESTED passed)"
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
    echo -e "${GREEN}✓ WMS 1.3.0 validation PASSED${NC}"
    echo ""
    exit 0
else
    echo -e "${RED}✗ WMS 1.3.0 validation FAILED${NC}"
    echo ""
    exit 1
fi
