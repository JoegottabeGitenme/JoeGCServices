#!/bin/bash
#
# Verification script that tests all WMS/WMTS capabilities
# This script dynamically discovers layers from GetCapabilities and tests each one.
#
# Usage: ./scripts/verify_all_capabilities.sh [base_url]
#
# Exit codes:
#   0 - All tests passed
#   1 - One or more tests failed
#

set -euo pipefail

BASE_URL="${1:-http://localhost:8080}"
RESULTS_DIR="validation/verification-results"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="$RESULTS_DIR/verification_$TIMESTAMP.json"
TEMP_DIR=$(mktemp -d)

trap "rm -rf $TEMP_DIR" EXIT

mkdir -p "$RESULTS_DIR"

echo "=============================================="
echo "WMS/WMTS Verification Test"
echo "Base URL: $BASE_URL"
echo "Timestamp: $TIMESTAMP"
echo "=============================================="
echo

# Initialize counters
TOTAL=0
PASSED=0
FAILED=0
SKIPPED=0

# Results array for JSON
declare -a RESULTS
declare -a FAILED_TESTS

# Color codes (if terminal supports it)
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    NC='\033[0m' # No Color
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    NC=''
fi

# Test function for endpoints
test_endpoint() {
    local name="$1"
    local url="$2"
    local expected_status="${3:-200}"
    local expected_content_type="${4:-image/png}"
    
    TOTAL=$((TOTAL + 1))
    
    # Make request with timeout
    local http_code
    local content_type
    
    if ! response=$(curl -s -w "\n%{http_code}\n%{content_type}" -o /tmp/test_response --max-time 30 "$url" 2>/dev/null); then
        echo -e "${RED}x FAIL${NC}: $name (connection error)"
        FAILED=$((FAILED + 1))
        FAILED_TESTS+=("$name")
        RESULTS+=("{\"name\":\"$name\",\"status\":\"fail\",\"reason\":\"connection_error\"}")
        return 1
    fi
    
    # Parse response
    http_code=$(echo "$response" | tail -2 | head -1)
    content_type=$(echo "$response" | tail -1)
    
    if [[ "$http_code" == "$expected_status" ]]; then
        if [[ "$content_type" == *"$expected_content_type"* ]] || [[ "$expected_content_type" == "any" ]]; then
            echo -e "${GREEN}v PASS${NC}: $name"
            PASSED=$((PASSED + 1))
            RESULTS+=("{\"name\":\"$name\",\"status\":\"pass\",\"http_status\":$http_code}")
            return 0
        else
            echo -e "${RED}x FAIL${NC}: $name (wrong content-type: $content_type, expected: $expected_content_type)"
            FAILED=$((FAILED + 1))
            FAILED_TESTS+=("$name")
            RESULTS+=("{\"name\":\"$name\",\"status\":\"fail\",\"reason\":\"wrong_content_type\",\"actual\":\"$content_type\",\"http_status\":$http_code}")
            return 1
        fi
    else
        echo -e "${RED}x FAIL${NC}: $name (HTTP $http_code, expected: $expected_status)"
        FAILED=$((FAILED + 1))
        FAILED_TESTS+=("$name")
        RESULTS+=("{\"name\":\"$name\",\"status\":\"fail\",\"reason\":\"http_error\",\"http_status\":$http_code}")
        return 1
    fi
}

# Test WMTS tile endpoint
test_wmts_tile() {
    local layer="$1"
    local style="${2:-default}"
    local z="${3:-5}"
    local x="${4:-7}"
    local y="${5:-11}"
    
    local url="$BASE_URL/wmts?SERVICE=WMTS&REQUEST=GetTile&LAYER=$layer&STYLE=$style&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=$z&TILEROW=$y&TILECOL=$x&FORMAT=image/png"
    test_endpoint "WMTS $layer ($style)" "$url" 200 "image/png"
}

# Test WMS GetMap endpoint
test_wms_getmap() {
    local layer="$1"
    local style="${2:-default}"
    local crs="${3:-EPSG:4326}"
    local bbox="${4:-25,-125,50,-65}"
    
    local url="$BASE_URL/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=$layer&STYLES=$style&CRS=$crs&BBOX=$bbox&WIDTH=256&HEIGHT=256&FORMAT=image/png"
    test_endpoint "WMS $layer ($style)" "$url" 200 "image/png"
}

# =========================================
# Section 1: Service Health Checks
# =========================================
echo -e "${BLUE}--- Service Health Checks ---${NC}"

test_endpoint "Health Check" "$BASE_URL/health" 200 "any"
test_endpoint "Ready Check" "$BASE_URL/ready" 200 "any"
test_endpoint "Metrics Endpoint" "$BASE_URL/metrics" 200 "any"

echo

# =========================================
# Section 2: Fetch and Parse Capabilities
# =========================================
echo -e "${BLUE}--- Fetching Capabilities Documents ---${NC}"

WMS_CAPS_FILE="$TEMP_DIR/wms_capabilities.xml"
WMTS_CAPS_FILE="$TEMP_DIR/wmts_capabilities.xml"

# Fetch WMS GetCapabilities
echo "Fetching WMS GetCapabilities..."
if ! curl -s --max-time 60 "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities" -o "$WMS_CAPS_FILE"; then
    echo -e "${RED}ERROR: Failed to fetch WMS GetCapabilities${NC}"
    exit 1
fi
test_endpoint "WMS GetCapabilities" "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities" 200 "xml"

# Fetch WMTS GetCapabilities
echo "Fetching WMTS GetCapabilities..."
if ! curl -s --max-time 60 "$BASE_URL/wmts?SERVICE=WMTS&REQUEST=GetCapabilities" -o "$WMTS_CAPS_FILE"; then
    echo -e "${RED}ERROR: Failed to fetch WMTS GetCapabilities${NC}"
    exit 1
fi
test_endpoint "WMTS GetCapabilities" "$BASE_URL/wmts?SERVICE=WMTS&REQUEST=GetCapabilities" 200 "xml"

echo

# =========================================
# Section 3: Parse Layers from WMS Capabilities
# =========================================
echo -e "${BLUE}--- Discovering Layers from WMS GetCapabilities ---${NC}"

# Extract layer names from WMS GetCapabilities
# Look for <Name> elements and filter to weather layer patterns
# 1. Insert newline before each <Name> tag
# 2. Filter lines containing <Name>
# 3. Extract content between <Name> and </Name>
# 4. Filter to weather layer patterns (gfs_, hrrr_, mrms_, goes*_)
# This approach works on both macOS and Linux
LAYERS=$(sed 's/<Name>/\
<Name>/g' "$WMS_CAPS_FILE" | \
    grep '<Name>' | \
    sed 's/.*<Name>\([^<]*\)<\/Name>.*/\1/' | \
    grep -E '^(gfs|hrrr|mrms|goes)[0-9]*_' | \
    sort -u)

LAYER_COUNT=$(echo "$LAYERS" | wc -l | tr -d ' ')
echo "Discovered $LAYER_COUNT layers from WMS GetCapabilities"
echo

# Extract styles for each layer (simplified - assumes default style exists)
# In a more robust version, we'd parse the Style elements per layer

# =========================================
# Section 4: Test All Discovered Layers (WMTS)
# =========================================
echo -e "${BLUE}--- Testing All Discovered Layers (WMTS) ---${NC}"

# Function to get style for a layer based on naming conventions
get_style_for_layer() {
    local layer="$1"
    
    case "$layer" in
        *_TMP|*_TEMP)
            echo "temperature"
            ;;
        *_WIND_BARBS)
            echo "wind_barbs"
            ;;
        *_PRMSL|*_PRES)
            echo "atmospheric"
            ;;
        *_RH|*_PWAT)
            echo "humidity"
            ;;
        *_CAPE)
            echo "cape"
            ;;
        *_TCDC|*_LCDC|*_MCDC|*_HCDC)
            echo "cloud"
            ;;
        *_VIS)
            echo "visibility"
            ;;
        *_GUST)
            echo "wind"
            ;;
        *_REFC|*_REFL|*_RETOP)
            echo "reflectivity"
            ;;
        *_PRECIP_RATE)
            echo "precip_rate"
            ;;
        *_APCP|*_QPE*)
            echo "precipitation"
            ;;
        *_LTNG)
            echo "lightning"
            ;;
        *_MXUPHL|*_HLCY)
            echo "helicity"
            ;;
        goes*_CMI_C01|goes*_CMI_C02|goes*_CMI_C03)
            echo "goes_visible"
            ;;
        goes*_CMI_C*)
            echo "goes_ir"
            ;;
        *)
            echo "default"
            ;;
    esac
}

# Get unique models from layers
MODELS=$(echo "$LAYERS" | cut -d'_' -f1 | sort -u)

# Test each model's layers
for model in $MODELS; do
    echo
    # Convert model to uppercase for display (portable way)
    model_upper=$(echo "$model" | tr '[:lower:]' '[:upper:]')
    echo -e "${YELLOW}Model: ${model_upper}${NC}"
    
    # Get layers for this model
    model_layers=$(echo "$LAYERS" | grep "^${model}_")
    
    for layer in $model_layers; do
        style=$(get_style_for_layer "$layer")
        test_wmts_tile "$layer" "$style"
    done
done

echo

# =========================================
# Section 5: Test Sample WMS GetMap Requests
# =========================================
echo -e "${BLUE}--- Testing WMS GetMap (Sample Layers) ---${NC}"

# Test first layer from each model with WMS GetMap
for model in $MODELS; do
    first_layer=$(echo "$LAYERS" | grep "^${model}_" | head -1)
    
    if [[ -n "$first_layer" ]]; then
        style=$(get_style_for_layer "$first_layer")
        test_wms_getmap "$first_layer" "$style" "EPSG:4326" "25,-125,50,-65"
    fi
done

echo

# =========================================
# Section 6: Test WMS with EPSG:3857
# =========================================
echo -e "${BLUE}--- Testing WMS GetMap (EPSG:3857) ---${NC}"

# Test a few layers with Web Mercator projection
sample_layers=$(echo "$LAYERS" | head -3)
for layer in $sample_layers; do
    style="default"
    case "$layer" in
        *_TMP) style="temperature" ;;
        *_REFC|*_REFL) style="reflectivity" ;;
    esac
    
    test_endpoint "WMS $layer (EPSG:3857)" \
        "$BASE_URL/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=$layer&STYLES=$style&CRS=EPSG:3857&BBOX=-13914937,2875744,-7235766,6446276&WIDTH=256&HEIGHT=256&FORMAT=image/png" \
        200 "image/png"
done

echo

# =========================================
# Section 7: Test WMS GetFeatureInfo
# =========================================
echo -e "${BLUE}--- Testing WMS GetFeatureInfo ---${NC}"

# Use first available layer for GetFeatureInfo tests
first_layer=$(echo "$LAYERS" | head -1)

test_endpoint "GetFeatureInfo (JSON)" \
    "$BASE_URL/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=$first_layer&STYLES=default&CRS=EPSG:4326&BBOX=25,-125,50,-65&WIDTH=256&HEIGHT=256&QUERY_LAYERS=$first_layer&INFO_FORMAT=application/json&I=128&J=128" \
    200 "application/json"

test_endpoint "GetFeatureInfo (HTML)" \
    "$BASE_URL/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=$first_layer&STYLES=default&CRS=EPSG:4326&BBOX=25,-125,50,-65&WIDTH=256&HEIGHT=256&QUERY_LAYERS=$first_layer&INFO_FORMAT=text/html&I=128&J=128" \
    200 "text/html"

test_endpoint "GetFeatureInfo (XML)" \
    "$BASE_URL/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=$first_layer&STYLES=default&CRS=EPSG:4326&BBOX=25,-125,50,-65&WIDTH=256&HEIGHT=256&QUERY_LAYERS=$first_layer&INFO_FORMAT=text/xml&I=128&J=128" \
    200 "text/xml"

echo

# =========================================
# Section 8: Test XYZ Tile Endpoints
# =========================================
echo -e "${BLUE}--- Testing XYZ Tile Endpoints ---${NC}"

# Test XYZ endpoint for a few layers
sample_layers=$(echo "$LAYERS" | head -3)
for layer in $sample_layers; do
    style="default"
    case "$layer" in
        *_TMP) style="temperature" ;;
        *_REFC|*_REFL) style="reflectivity" ;;
        goes*_CMI_C02) style="goes_visible" ;;
    esac
    
    test_endpoint "XYZ $layer" \
        "$BASE_URL/tiles/$layer/$style/5/7/11.png" \
        200 "image/png"
done

echo

# =========================================
# Section 9: Test Multiple Zoom Levels
# =========================================
echo -e "${BLUE}--- Testing Zoom Level Coverage ---${NC}"

# Use first layer to test various zoom levels
first_layer=$(echo "$LAYERS" | head -1)
for z in 2 4 6 8 10; do
    # Calculate tile coords for CONUS center at this zoom
    x=$((2 ** (z - 1)))
    y=$((2 ** (z - 1)))
    test_wmts_tile "$first_layer" "default" "$z" "$x" "$y"
done

echo

# =========================================
# Section 10: Test All Styles for a Sample Layer
# =========================================
echo -e "${BLUE}--- Testing All Styles (Sample Layer) ---${NC}"

# Find a temperature layer to test various styles
temp_layer=$(echo "$LAYERS" | grep "_TMP" | head -1)
if [[ -n "$temp_layer" ]]; then
    echo "Testing styles for: $temp_layer"
    for style in default temperature isolines numbers; do
        test_wmts_tile "$temp_layer" "$style"
    done
fi

echo

# =========================================
# Summary
# =========================================
echo "=============================================="
echo "VERIFICATION SUMMARY"
echo "=============================================="
echo "Layers Discovered: $LAYER_COUNT"
echo "Total Tests:       $TOTAL"

if [[ $PASSED -gt 0 ]]; then
    PASS_PCT=$(echo "scale=1; $PASSED * 100 / $TOTAL" | bc)
    echo -e "Passed:            ${GREEN}$PASSED${NC} ($PASS_PCT%)"
else
    echo "Passed:            0"
fi

if [[ $FAILED -gt 0 ]]; then
    FAIL_PCT=$(echo "scale=1; $FAILED * 100 / $TOTAL" | bc)
    echo -e "Failed:            ${RED}$FAILED${NC} ($FAIL_PCT%)"
else
    echo -e "Failed:            ${GREEN}0${NC}"
fi

if [[ $SKIPPED -gt 0 ]]; then
    echo -e "Skipped:           ${YELLOW}$SKIPPED${NC}"
fi
echo "=============================================="

# List failed tests
if [[ $FAILED -gt 0 ]]; then
    echo
    echo "Failed Tests:"
    for test in "${FAILED_TESTS[@]}"; do
        echo "  - $test"
    done
fi

# Generate JSON report
{
    echo "{"
    echo "  \"timestamp\": \"$TIMESTAMP\","
    echo "  \"base_url\": \"$BASE_URL\","
    echo "  \"layers_discovered\": $LAYER_COUNT,"
    echo "  \"summary\": {"
    echo "    \"total\": $TOTAL,"
    echo "    \"passed\": $PASSED,"
    echo "    \"failed\": $FAILED,"
    echo "    \"skipped\": $SKIPPED,"
    echo "    \"pass_rate\": $(echo "scale=4; $PASSED / $TOTAL" | bc)"
    echo "  },"
    echo "  \"discovered_layers\": ["
    first=true
    for layer in $LAYERS; do
        if $first; then
            echo "    \"$layer\""
            first=false
        else
            echo "    ,\"$layer\""
        fi
    done
    echo "  ],"
    echo "  \"results\": ["
    if [[ ${#RESULTS[@]} -gt 0 ]]; then
        printf '%s\n' "${RESULTS[@]}" | sed 's/$/,/' | sed '$ s/,$//'
    fi
    echo "  ]"
    echo "}"
} > "$REPORT_FILE"

echo
echo "Report saved to: $REPORT_FILE"

# Exit with error if any tests failed
if [[ $FAILED -gt 0 ]]; then
    echo
    echo -e "${RED}VERIFICATION FAILED${NC}: $FAILED test(s) failed"
    exit 1
else
    echo
    echo -e "${GREEN}VERIFICATION PASSED${NC}: All $PASSED tests passed!"
    exit 0
fi
