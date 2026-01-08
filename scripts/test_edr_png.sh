#!/bin/bash
# Test EDR PNG output functionality
#
# Usage:
#   ./scripts/test_edr_png.sh [options] [base_url]
#
# Options:
#   -c, --collection NAME   Use specific collection (e.g., hrrr-surface)
#   -p, --parameter NAME    Use specific parameter (e.g., TMP)
#   -h, --help              Show this help
#
# Examples:
#   ./scripts/test_edr_png.sh                              # Auto-select collection
#   ./scripts/test_edr_png.sh -c hrrr-surface              # Use specific collection
#   ./scripts/test_edr_png.sh -c gfs-surface -p TMP        # Collection + parameter
#   ./scripts/test_edr_png.sh http://api.example.com       # Custom server

set -e

# Parse arguments
SPECIFIED_COLLECTION=""
SPECIFIED_PARAM=""
BASE_URL="http://localhost:8083"

while [[ $# -gt 0 ]]; do
    case $1 in
        -c|--collection)
            SPECIFIED_COLLECTION="$2"
            shift 2
            ;;
        -p|--parameter)
            SPECIFIED_PARAM="$2"
            shift 2
            ;;
        -h|--help)
            head -17 "$0" | tail -15
            exit 0
            ;;
        http://*|https://*)
            BASE_URL="$1"
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

EDR_URL="${BASE_URL}/edr"
OUTPUT_DIR="/tmp/edr-png-test"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

mkdir -p "$OUTPUT_DIR"

echo -e "${BLUE}=== EDR PNG Output Test ===${NC}"
echo "Base URL: $EDR_URL"
echo "Output dir: $OUTPUT_DIR"
echo ""

# Check if server is running
echo -e "${YELLOW}Checking server health...${NC}"
if ! curl -sf "${BASE_URL}/health" > /dev/null 2>&1; then
    echo -e "${RED}Server not responding at ${BASE_URL}/health${NC}"
    echo "Make sure the EDR API is running:"
    echo "  cargo run -p edr-api"
    exit 1
fi
echo -e "${GREEN}Server is healthy${NC}"
echo ""

# Get available collections
echo -e "${YELLOW}Fetching collections...${NC}"
COLLECTIONS=$(curl -sf "${EDR_URL}/collections" | jq -r '.collections[].id' 2>/dev/null || echo "")
if [ -z "$COLLECTIONS" ]; then
    echo -e "${RED}No collections found or failed to fetch${NC}"
    exit 1
fi
echo "Available collections:"
echo "$COLLECTIONS" | while read -r coll; do echo "  - $coll"; done
echo ""

# Use specified collection or auto-select
if [ -n "$SPECIFIED_COLLECTION" ]; then
    if echo "$COLLECTIONS" | grep -q "^${SPECIFIED_COLLECTION}$"; then
        TEST_COLLECTION="$SPECIFIED_COLLECTION"
    else
        echo -e "${RED}Collection '$SPECIFIED_COLLECTION' not found${NC}"
        exit 1
    fi
else
    # Preferred collection order (try these first as they're more likely to have data)
    PREFERRED_COLLECTIONS="hrrr-surface gfs-surface hrrr-isobaric gfs-isobaric mrms-single-level"

    # Find a working collection by trying preferred ones first
    TEST_COLLECTION=""
    for pref in $PREFERRED_COLLECTIONS; do
        if echo "$COLLECTIONS" | grep -q "^${pref}$"; then
            TEST_COLLECTION="$pref"
            break
        fi
    done

    # Fall back to first available if none of the preferred ones exist
    if [ -z "$TEST_COLLECTION" ]; then
        TEST_COLLECTION=$(echo "$COLLECTIONS" | head -1)
    fi
fi

echo -e "${YELLOW}Testing with collection: ${TEST_COLLECTION}${NC}"

# Get collection info to find a parameter
COLLECTION_INFO=$(curl -sf "${EDR_URL}/collections/${TEST_COLLECTION}")

# Get available parameters for this collection
AVAILABLE_PARAMS=$(echo "$COLLECTION_INFO" | jq -r '.parameter_names | keys[]' 2>/dev/null || echo "")
echo "Available parameters: $(echo $AVAILABLE_PARAMS | tr '\n' ' ')"

# Use specified parameter or auto-select first available
if [ -n "$SPECIFIED_PARAM" ]; then
    if echo "$AVAILABLE_PARAMS" | grep -q "^${SPECIFIED_PARAM}$"; then
        FIRST_PARAM="$SPECIFIED_PARAM"
    else
        echo -e "${YELLOW}Warning: Parameter '$SPECIFIED_PARAM' not in collection, using first available${NC}"
        FIRST_PARAM=$(echo "$AVAILABLE_PARAMS" | head -1)
    fi
else
    FIRST_PARAM=$(echo "$AVAILABLE_PARAMS" | head -1)
fi

if [ -z "$FIRST_PARAM" ]; then
    echo -e "${RED}No parameters available in collection${NC}"
    exit 1
fi
echo "Using parameter: $FIRST_PARAM"
echo ""

# Define test polygons based on collection type
# Note: Spaces are URL-encoded as %20
if [[ "$TEST_COLLECTION" == *"goes"* ]]; then
    # GOES covers Pacific/Western US
    TEST_POLYGON="POLYGON((-125%2035,-120%2035,-120%2040,-125%2040,-125%2035))"
    BBOX_DESC="California coast (small)"
    # GOES doesn't have PNG area limit configured, use smaller CONUS
    CONUS_POLYGON="POLYGON((-125%2025,-100%2025,-100%2050,-125%2050,-125%2025))"
    CONUS_DESC="Western CONUS"
elif [[ "$TEST_COLLECTION" == *"gfs"* ]]; then
    # GFS is global - use small area for basic tests
    TEST_POLYGON="POLYGON((-100%2035,-98%2035,-98%2037,-100%2037,-100%2035))"
    BBOX_DESC="Oklahoma/Kansas (small)"
    # GFS has high PNG limit, can do full CONUS
    CONUS_POLYGON="POLYGON((-125%2024,-66%2024,-66%2050,-125%2050,-125%2024))"
    CONUS_DESC="Full CONUS"
else
    # HRRR/MRMS/NDFD cover CONUS - use small area for basic tests
    TEST_POLYGON="POLYGON((-100%2035,-98%2035,-98%2037,-100%2037,-100%2035))"
    BBOX_DESC="Oklahoma/Kansas (small)"
    # HRRR has 2500 sq deg PNG limit, can do full CONUS
    CONUS_POLYGON="POLYGON((-125%2024,-66%2024,-66%2050,-125%2050,-125%2024))"
    CONUS_DESC="Full CONUS"
fi

echo -e "${BLUE}--- Test 1: Basic PNG Request ---${NC}"
echo "Area: $BBOX_DESC"
PNG_FILE="${OUTPUT_DIR}/test1_basic.png"
RESPONSE_HEADERS="${OUTPUT_DIR}/test1_headers.txt"

# Don't use -f so we can capture error responses
HTTP_CODE=$(curl -s -o "$PNG_FILE" -D "$RESPONSE_HEADERS" -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png" \
    2>/dev/null)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}Success! HTTP $HTTP_CODE${NC}"
    
    # Show PNG info
    if command -v file &> /dev/null; then
        echo "File info: $(file "$PNG_FILE")"
    fi
    echo "File size: $(ls -lh "$PNG_FILE" | awk '{print $5}')"
    
    # Show response headers
    echo ""
    echo "Response headers:"
    grep -i "x-edr\|cache-control\|content-type" "$RESPONSE_HEADERS" | while read -r line; do
        echo "  $line"
    done
    
    # Parse header values for display (use ^x-edr to avoid matching expose-headers line)
    echo ""
    echo "Decoded metadata:"
    MIN=$(grep -i "^x-edr-min:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    MAX=$(grep -i "^x-edr-max:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    WIDTH=$(grep -i "^x-edr-width:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    HEIGHT=$(grep -i "^x-edr-height:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    UNITS=$(grep -i "^x-edr-units:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    echo "  Value range: $MIN to $MAX $UNITS"
    echo "  Dimensions: ${WIDTH}x${HEIGHT}"
else
    echo -e "${RED}Failed! HTTP $HTTP_CODE${NC}"
    cat "$PNG_FILE" 2>/dev/null || true
fi
echo ""

echo -e "${BLUE}--- Test 2: Resized PNG (256x256) ---${NC}"
PNG_FILE="${OUTPUT_DIR}/test2_resized.png"
RESPONSE_HEADERS="${OUTPUT_DIR}/test2_headers.txt"

HTTP_CODE=$(curl -s -o "$PNG_FILE" -D "$RESPONSE_HEADERS" -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&width=256&height=256" \
    2>/dev/null)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}Success! HTTP $HTTP_CODE${NC}"
    echo "File size: $(ls -lh "$PNG_FILE" | awk '{print $5}')"
    WIDTH=$(grep -i "^x-edr-width:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    HEIGHT=$(grep -i "^x-edr-height:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    echo "Dimensions: ${WIDTH}x${HEIGHT}"
else
    echo -e "${RED}Failed! HTTP $HTTP_CODE${NC}"
    cat "$PNG_FILE" 2>/dev/null || true
fi
echo ""

echo -e "${BLUE}--- Test 3: Large Resize (1024x1024) ---${NC}"
PNG_FILE="${OUTPUT_DIR}/test3_large.png"
RESPONSE_HEADERS="${OUTPUT_DIR}/test3_headers.txt"

HTTP_CODE=$(curl -s -o "$PNG_FILE" -D "$RESPONSE_HEADERS" -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&width=1024&height=1024" \
    2>/dev/null)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}Success! HTTP $HTTP_CODE${NC}"
    echo "File size: $(ls -lh "$PNG_FILE" | awk '{print $5}')"
    WIDTH=$(grep -i "^x-edr-width:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    HEIGHT=$(grep -i "^x-edr-height:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    echo "Dimensions: ${WIDTH}x${HEIGHT}"
else
    echo -e "${RED}Failed! HTTP $HTTP_CODE${NC}"
    cat "$PNG_FILE" 2>/dev/null || true
fi
echo ""

echo -e "${BLUE}--- Test 4: 8-bit Encoding (depth=8) ---${NC}"
PNG_FILE_8BIT="${OUTPUT_DIR}/test4_8bit.png"
PNG_FILE_16BIT="${OUTPUT_DIR}/test4_16bit.png"
RESPONSE_HEADERS_8BIT="${OUTPUT_DIR}/test4_8bit_headers.txt"
RESPONSE_HEADERS_16BIT="${OUTPUT_DIR}/test4_16bit_headers.txt"

# Get 16-bit version for comparison
HTTP_CODE_16=$(curl -s -o "$PNG_FILE_16BIT" -D "$RESPONSE_HEADERS_16BIT" -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&width=256&height=256" \
    2>/dev/null)

# Get 8-bit version
HTTP_CODE_8=$(curl -s -o "$PNG_FILE_8BIT" -D "$RESPONSE_HEADERS_8BIT" -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&width=256&height=256&depth=8" \
    2>/dev/null)

if [ "$HTTP_CODE_8" = "200" ]; then
    echo -e "${GREEN}Success! HTTP $HTTP_CODE_8${NC}"
    SIZE_8BIT=$(ls -l "$PNG_FILE_8BIT" | awk '{print $5}')
    SIZE_16BIT=$(ls -l "$PNG_FILE_16BIT" | awk '{print $5}')
    ENCODING=$(grep -i "^x-edr-encoding:" "$RESPONSE_HEADERS_8BIT" | cut -d: -f2 | tr -d ' \r')
    echo "Encoding: $ENCODING"
    echo "8-bit size:  $(ls -lh "$PNG_FILE_8BIT" | awk '{print $5}') ($SIZE_8BIT bytes)"
    echo "16-bit size: $(ls -lh "$PNG_FILE_16BIT" | awk '{print $5}') ($SIZE_16BIT bytes)"
    if [ "$SIZE_8BIT" -lt "$SIZE_16BIT" ]; then
        SAVINGS=$(( (SIZE_16BIT - SIZE_8BIT) * 100 / SIZE_16BIT ))
        echo -e "${GREEN}8-bit is ${SAVINGS}% smaller${NC}"
    else
        echo -e "${YELLOW}Warning: 8-bit not smaller than 16-bit${NC}"
    fi
else
    echo -e "${RED}Failed! HTTP $HTTP_CODE_8${NC}"
    cat "$PNG_FILE_8BIT" 2>/dev/null || true
fi
echo ""

echo -e "${BLUE}--- Test 5: Full CONUS Coverage ---${NC}"
echo "Area: $CONUS_DESC"
PNG_FILE="${OUTPUT_DIR}/test5_conus.png"
RESPONSE_HEADERS="${OUTPUT_DIR}/test5_headers.txt"

# Request full CONUS with 8-bit encoding for smaller size
HTTP_CODE=$(curl -s -o "$PNG_FILE" -D "$RESPONSE_HEADERS" -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${CONUS_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&depth=8&width=1024&height=512" \
    2>/dev/null)

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}Success! HTTP $HTTP_CODE${NC}"
    echo "File size: $(ls -lh "$PNG_FILE" | awk '{print $5}')"
    WIDTH=$(grep -i "^x-edr-width:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    HEIGHT=$(grep -i "^x-edr-height:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    BBOX=$(grep -i "^x-edr-bbox:" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    echo "Dimensions: ${WIDTH}x${HEIGHT}"
    echo "BBox: $BBOX"
elif [ "$HTTP_CODE" = "413" ]; then
    echo -e "${YELLOW}Area too large (413) - PNG area limit may not be configured${NC}"
    cat "$PNG_FILE" 2>/dev/null | head -5 || true
else
    echo -e "${RED}Failed! HTTP $HTTP_CODE${NC}"
    cat "$PNG_FILE" 2>/dev/null || true
fi
echo ""

echo -e "${BLUE}--- Test 6: Verify CORS Headers ---${NC}"
# Use OPTIONS to check CORS preflight
CORS_HEADERS=$(curl -sf -I -X OPTIONS \
    -H "Origin: http://example.com" \
    -H "Access-Control-Request-Method: GET" \
    -H "Access-Control-Request-Headers: Accept" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area" 2>/dev/null || echo "")

if echo "$CORS_HEADERS" | grep -qi "access-control"; then
    echo -e "${GREEN}CORS headers present${NC}"
    echo "$CORS_HEADERS" | grep -i "access-control" | while read -r line; do
        echo "  $line"
    done
else
    echo -e "${YELLOW}CORS preflight may not be explicitly handled (permissive CORS)${NC}"
fi
echo ""

echo -e "${BLUE}--- Test 7: Error Cases ---${NC}"

# Test: Missing width when height provided
echo -n "Missing width with height: "
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&height=256" \
    2>/dev/null)
if [ "$HTTP_CODE" = "400" ]; then
    echo -e "${GREEN}400 (correct)${NC}"
else
    echo -e "${RED}$HTTP_CODE (expected 400)${NC}"
fi

# Test: Dimensions too large
echo -n "Dimensions > 4096: "
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&width=5000&height=5000" \
    2>/dev/null)
if [ "$HTTP_CODE" = "400" ]; then
    echo -e "${GREEN}400 (correct)${NC}"
else
    echo -e "${RED}$HTTP_CODE (expected 400)${NC}"
fi

# Test: Multiple parameters (should fail for PNG)
# Get a second parameter if available
SECOND_PARAM=$(echo "$AVAILABLE_PARAMS" | sed -n '2p')
if [ -n "$SECOND_PARAM" ]; then
    echo -n "Multiple parameters: "
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
        "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM},${SECOND_PARAM}&f=png" \
        2>/dev/null)
    if [ "$HTTP_CODE" = "400" ]; then
        echo -e "${GREEN}400 (correct)${NC}"
    else
        echo -e "${RED}$HTTP_CODE (expected 400)${NC}"
    fi
else
    echo "Multiple parameters: ${YELLOW}skipped (only one param available)${NC}"
fi

# Test: PNG on position endpoint (should fail)
echo -n "PNG on position endpoint: "
HTTP_CODE=$(curl -s --max-time 10 -o /dev/null -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/position?coords=POINT(-99%2036)&parameter-name=${FIRST_PARAM}&f=png" \
    2>/dev/null)
if [ "$HTTP_CODE" = "400" ]; then
    echo -e "${GREEN}400 (correct)${NC}"
else
    echo -e "${RED}$HTTP_CODE (expected 400)${NC}"
fi

echo ""

# Test: Invalid depth value
echo -n "Invalid depth value: "
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&depth=12" \
    2>/dev/null)
if [ "$HTTP_CODE" = "400" ]; then
    echo -e "${GREEN}400 (correct)${NC}"
else
    echo -e "${RED}$HTTP_CODE (expected 400)${NC}"
fi

echo ""
echo -e "${BLUE}--- Test 8: Cache-Control Header ---${NC}"
CACHE_CONTROL=$(grep -i "^cache-control:" "${OUTPUT_DIR}/test1_headers.txt" | cut -d: -f2 | tr -d ' \r')
echo "Cache-Control: $CACHE_CONTROL"
if [[ "$CACHE_CONTROL" == *"max-age="* ]]; then
    MAX_AGE=$(echo "$CACHE_CONTROL" | grep -o 'max-age=[0-9]*' | cut -d= -f2)
    MINUTES=$((MAX_AGE / 60))
    echo "max-age: ${MAX_AGE}s (${MINUTES} minutes)"
fi
echo ""

echo -e "${BLUE}=== Summary ===${NC}"
echo "Output files saved to: $OUTPUT_DIR"
ls -la "$OUTPUT_DIR"/*.png 2>/dev/null | awk '{print "  " $9 " (" $5 ")"}'
echo ""

# Optional: Open PNG in viewer if available
if command -v xdg-open &> /dev/null && [ -n "$DISPLAY" ]; then
    echo -e "${YELLOW}Hint: View PNG with:${NC}"
    echo "  xdg-open ${OUTPUT_DIR}/test1_basic.png"
elif command -v open &> /dev/null; then
    echo -e "${YELLOW}Hint: View PNG with:${NC}"
    echo "  open ${OUTPUT_DIR}/test1_basic.png"
fi

echo ""
echo -e "${GREEN}Done!${NC}"
