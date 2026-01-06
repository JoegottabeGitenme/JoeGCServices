#!/bin/bash
# Test EDR PNG output functionality
#
# Usage:
#   ./scripts/test_edr_png.sh [base_url]
#
# Examples:
#   ./scripts/test_edr_png.sh                          # Uses localhost:8083
#   ./scripts/test_edr_png.sh http://api.example.com   # Custom server

set -e

BASE_URL="${1:-http://localhost:8083}"
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

# Pick first collection for testing
TEST_COLLECTION=$(echo "$COLLECTIONS" | head -1)
echo -e "${YELLOW}Testing with collection: ${TEST_COLLECTION}${NC}"

# Get collection info to find a parameter
COLLECTION_INFO=$(curl -sf "${EDR_URL}/collections/${TEST_COLLECTION}")
FIRST_PARAM=$(echo "$COLLECTION_INFO" | jq -r '.parameter_names | keys[0]' 2>/dev/null || echo "TMP")
echo "Using parameter: $FIRST_PARAM"
echo ""

# Define test polygon (small area over CONUS for HRRR, or Pacific for GOES)
if [[ "$TEST_COLLECTION" == *"goes"* ]]; then
    # GOES covers Pacific/Western US
    TEST_POLYGON="POLYGON((-125 35,-120 35,-120 40,-125 40,-125 35))"
    BBOX_DESC="California coast"
else
    # HRRR covers CONUS
    TEST_POLYGON="POLYGON((-100 35,-98 35,-98 37,-100 37,-100 35))"
    BBOX_DESC="Oklahoma/Kansas"
fi

echo -e "${BLUE}--- Test 1: Basic PNG Request ---${NC}"
echo "Area: $BBOX_DESC"
PNG_FILE="${OUTPUT_DIR}/test1_basic.png"
RESPONSE_HEADERS="${OUTPUT_DIR}/test1_headers.txt"

HTTP_CODE=$(curl -sf -o "$PNG_FILE" -D "$RESPONSE_HEADERS" -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png" \
    2>/dev/null || echo "000")

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
    
    # Parse header values for display
    echo ""
    echo "Decoded metadata:"
    MIN=$(grep -i "x-edr-min" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    MAX=$(grep -i "x-edr-max" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    WIDTH=$(grep -i "x-edr-width" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    HEIGHT=$(grep -i "x-edr-height" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    UNITS=$(grep -i "x-edr-units" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
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

HTTP_CODE=$(curl -sf -o "$PNG_FILE" -D "$RESPONSE_HEADERS" -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&width=256&height=256" \
    2>/dev/null || echo "000")

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}Success! HTTP $HTTP_CODE${NC}"
    echo "File size: $(ls -lh "$PNG_FILE" | awk '{print $5}')"
    WIDTH=$(grep -i "x-edr-width" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    HEIGHT=$(grep -i "x-edr-height" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    echo "Dimensions: ${WIDTH}x${HEIGHT}"
else
    echo -e "${RED}Failed! HTTP $HTTP_CODE${NC}"
    cat "$PNG_FILE" 2>/dev/null || true
fi
echo ""

echo -e "${BLUE}--- Test 3: Large Resize (1024x1024) ---${NC}"
PNG_FILE="${OUTPUT_DIR}/test3_large.png"
RESPONSE_HEADERS="${OUTPUT_DIR}/test3_headers.txt"

HTTP_CODE=$(curl -sf -o "$PNG_FILE" -D "$RESPONSE_HEADERS" -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&width=1024&height=1024" \
    2>/dev/null || echo "000")

if [ "$HTTP_CODE" = "200" ]; then
    echo -e "${GREEN}Success! HTTP $HTTP_CODE${NC}"
    echo "File size: $(ls -lh "$PNG_FILE" | awk '{print $5}')"
    WIDTH=$(grep -i "x-edr-width" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    HEIGHT=$(grep -i "x-edr-height" "$RESPONSE_HEADERS" | cut -d: -f2 | tr -d ' \r')
    echo "Dimensions: ${WIDTH}x${HEIGHT}"
else
    echo -e "${RED}Failed! HTTP $HTTP_CODE${NC}"
    cat "$PNG_FILE" 2>/dev/null || true
fi
echo ""

echo -e "${BLUE}--- Test 4: Verify CORS Headers ---${NC}"
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

echo -e "${BLUE}--- Test 5: Error Cases ---${NC}"

# Test: Missing width when height provided
echo -n "Missing width with height: "
HTTP_CODE=$(curl -sf -o /dev/null -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&height=256" \
    2>/dev/null || echo "000")
if [ "$HTTP_CODE" = "400" ]; then
    echo -e "${GREEN}400 (correct)${NC}"
else
    echo -e "${RED}$HTTP_CODE (expected 400)${NC}"
fi

# Test: Dimensions too large
echo -n "Dimensions > 4096: "
HTTP_CODE=$(curl -sf -o /dev/null -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM}&f=png&width=5000&height=5000" \
    2>/dev/null || echo "000")
if [ "$HTTP_CODE" = "400" ]; then
    echo -e "${GREEN}400 (correct)${NC}"
else
    echo -e "${RED}$HTTP_CODE (expected 400)${NC}"
fi

# Test: Multiple parameters (should fail for PNG)
echo -n "Multiple parameters: "
HTTP_CODE=$(curl -sf -o /dev/null -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/area?coords=${TEST_POLYGON}&parameter-name=${FIRST_PARAM},UGRD&f=png" \
    2>/dev/null || echo "000")
if [ "$HTTP_CODE" = "400" ]; then
    echo -e "${GREEN}400 (correct)${NC}"
else
    echo -e "${RED}$HTTP_CODE (expected 400)${NC}"
fi

# Test: PNG on position endpoint (should fail)
echo -n "PNG on position endpoint: "
HTTP_CODE=$(curl -sf -o /dev/null -w "%{http_code}" \
    "${EDR_URL}/collections/${TEST_COLLECTION}/position?coords=POINT(-99 36)&parameter-name=${FIRST_PARAM}&f=png" \
    2>/dev/null || echo "000")
if [ "$HTTP_CODE" = "400" ]; then
    echo -e "${GREEN}400 (correct)${NC}"
else
    echo -e "${RED}$HTTP_CODE (expected 400)${NC}"
fi

echo ""
echo -e "${BLUE}--- Test 6: Cache-Control Header ---${NC}"
CACHE_CONTROL=$(grep -i "cache-control" "${OUTPUT_DIR}/test1_headers.txt" | cut -d: -f2 | tr -d ' \r')
echo "Cache-Control: $CACHE_CONTROL"
if [[ "$CACHE_CONTROL" == *"max-age="* ]]; then
    MAX_AGE=$(echo "$CACHE_CONTROL" | grep -o 'max-age=[0-9]*' | cut -d= -f2)
    echo "max-age: ${MAX_AGE}s ($(echo "scale=1; $MAX_AGE/60" | bc) minutes)"
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
