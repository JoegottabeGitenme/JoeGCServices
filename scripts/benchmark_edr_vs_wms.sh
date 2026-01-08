#!/bin/bash
# Benchmark: EDR PNG vs WMS Tiles for CONUS/Global Coverage
#
# Compares bandwidth and latency between:
# - EDR PNG area query (single request, data-encoded PNG)
# - WMS tile requests (multiple requests, rendered tiles)
#
# Usage:
#   ./scripts/benchmark_edr_vs_wms.sh [model]
#
# Arguments:
#   model   - hrrr (default) or gfs
#
# Examples:
#   ./scripts/benchmark_edr_vs_wms.sh          # Test HRRR (CONUS)
#   ./scripts/benchmark_edr_vs_wms.sh hrrr     # Test HRRR (CONUS)
#   ./scripts/benchmark_edr_vs_wms.sh gfs      # Test GFS (Global/CONUS)
#
# Requirements:
#   - curl with timing support
#   - bc for calculations

set -e

MODEL="${1:-hrrr}"

# Service URLs (EDR on 8083, WMS on 8080)
EDR_URL="http://localhost:8083/edr"
WMS_URL="http://localhost:8080/wms"
OUTPUT_DIR="/tmp/edr-wms-benchmark-${MODEL}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# CONUS bounding box (approximate)
CONUS_WEST=-125
CONUS_SOUTH=24
CONUS_EAST=-66
CONUS_NORTH=50

# Calculate CONUS dimensions
CONUS_WIDTH=$((CONUS_EAST - CONUS_WEST))   # 59 degrees
CONUS_HEIGHT=$((CONUS_NORTH - CONUS_SOUTH)) # 26 degrees

# WMS tile size
TILE_SIZE=256

# Model-specific parameters
case "$MODEL" in
    hrrr)
        LAYER="hrrr_TMP"
        COLLECTION="hrrr-height-agl"
        PARAMETER="TMP"
        STYLE="gradient"
        MODEL_DESC="HRRR (3km resolution, CONUS)"
        ;;
    gfs)
        LAYER="gfs_TMP"
        COLLECTION="gfs-height-agl"
        PARAMETER="TMP"
        STYLE="gradient"
        MODEL_DESC="GFS (0.25° resolution, Global)"
        ;;
    *)
        echo "Unknown model: $MODEL"
        echo "Usage: $0 [hrrr|gfs]"
        exit 1
        ;;
esac

mkdir -p "$OUTPUT_DIR"
rm -f "$OUTPUT_DIR"/*.png "$OUTPUT_DIR"/*.txt 2>/dev/null || true

echo -e "${BLUE}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║         EDR PNG vs WMS Tiles - Bandwidth Benchmark             ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Model: $MODEL_DESC"
echo "EDR URL: $EDR_URL"
echo "WMS URL: $WMS_URL"
echo "CONUS bounds: ${CONUS_WEST},${CONUS_SOUTH} to ${CONUS_EAST},${CONUS_NORTH}"
echo "CONUS size: ${CONUS_WIDTH}° x ${CONUS_HEIGHT}° = $((CONUS_WIDTH * CONUS_HEIGHT)) sq degrees"
echo "Collection: $COLLECTION | Layer: $LAYER | Parameter: $PARAMETER"
echo ""

# Check if servers are running
echo -e "${YELLOW}Checking server health...${NC}"
if ! curl -sf "http://localhost:8083/health" > /dev/null 2>&1; then
    echo -e "${RED}EDR API not responding at http://localhost:8083/health${NC}"
    exit 1
fi
echo -e "${GREEN}EDR API is healthy${NC}"

if ! curl -sf "http://localhost:8080/health" > /dev/null 2>&1; then
    echo -e "${RED}WMS API not responding at http://localhost:8080/health${NC}"
    exit 1
fi
echo -e "${GREEN}WMS API is healthy${NC}"
echo ""

# ============================================================================
# EDR PNG Requests
# ============================================================================

echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${CYAN}                    EDR PNG Area Queries                        ${NC}"
echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# URL encode the polygon
POLYGON="POLYGON((${CONUS_WEST}%20${CONUS_SOUTH},${CONUS_EAST}%20${CONUS_SOUTH},${CONUS_EAST}%20${CONUS_NORTH},${CONUS_WEST}%20${CONUS_NORTH},${CONUS_WEST}%20${CONUS_SOUTH}))"

# Test 1: EDR 16-bit PNG (native resolution)
echo -e "${YELLOW}1. EDR PNG 16-bit (native resolution)${NC}"
EDR_16BIT_FILE="$OUTPUT_DIR/edr_16bit_native.png"
EDR_16BIT_TIME=$(curl -s -o "$EDR_16BIT_FILE" -w "%{time_total}" \
    "${EDR_URL}/collections/${COLLECTION}/area?coords=${POLYGON}&parameter-name=${PARAMETER}&f=png" 2>/dev/null || echo "0")

if [ -f "$EDR_16BIT_FILE" ] && [ -s "$EDR_16BIT_FILE" ]; then
    EDR_16BIT_SIZE=$(stat -f%z "$EDR_16BIT_FILE" 2>/dev/null || stat -c%s "$EDR_16BIT_FILE" 2>/dev/null || echo "0")
    EDR_16BIT_DIMS=$(file "$EDR_16BIT_FILE" 2>/dev/null | grep -oE '[0-9]+ x [0-9]+' || echo "unknown")
    echo "   Size: $(numfmt --to=iec-i --suffix=B $EDR_16BIT_SIZE 2>/dev/null || echo "${EDR_16BIT_SIZE} bytes")"
    echo "   Dimensions: $EDR_16BIT_DIMS"
    echo "   Time: ${EDR_16BIT_TIME}s"
    echo "   Requests: 1"
else
    echo -e "   ${RED}Failed to fetch${NC}"
    EDR_16BIT_SIZE=0
fi
echo ""

# Test 2: EDR 8-bit PNG (native resolution)
echo -e "${YELLOW}2. EDR PNG 8-bit (native resolution)${NC}"
EDR_8BIT_FILE="$OUTPUT_DIR/edr_8bit_native.png"
EDR_8BIT_TIME=$(curl -s -o "$EDR_8BIT_FILE" -w "%{time_total}" \
    "${EDR_URL}/collections/${COLLECTION}/area?coords=${POLYGON}&parameter-name=${PARAMETER}&f=png&depth=8" 2>/dev/null || echo "0")

if [ -f "$EDR_8BIT_FILE" ] && [ -s "$EDR_8BIT_FILE" ]; then
    EDR_8BIT_SIZE=$(stat -f%z "$EDR_8BIT_FILE" 2>/dev/null || stat -c%s "$EDR_8BIT_FILE" 2>/dev/null || echo "0")
    EDR_8BIT_DIMS=$(file "$EDR_8BIT_FILE" 2>/dev/null | grep -oE '[0-9]+ x [0-9]+' || echo "unknown")
    echo "   Size: $(numfmt --to=iec-i --suffix=B $EDR_8BIT_SIZE 2>/dev/null || echo "${EDR_8BIT_SIZE} bytes")"
    echo "   Dimensions: $EDR_8BIT_DIMS"
    echo "   Time: ${EDR_8BIT_TIME}s"
    echo "   Requests: 1"
else
    echo -e "   ${RED}Failed to fetch${NC}"
    EDR_8BIT_SIZE=0
fi
echo ""

# Test 3: EDR 8-bit PNG (1024x512 - web-friendly size)
echo -e "${YELLOW}3. EDR PNG 8-bit (1024x512 resized)${NC}"
EDR_RESIZED_FILE="$OUTPUT_DIR/edr_8bit_1024x512.png"
EDR_RESIZED_TIME=$(curl -s -o "$EDR_RESIZED_FILE" -w "%{time_total}" \
    "${EDR_URL}/collections/${COLLECTION}/area?coords=${POLYGON}&parameter-name=${PARAMETER}&f=png&depth=8&width=1024&height=512" 2>/dev/null || echo "0")

if [ -f "$EDR_RESIZED_FILE" ] && [ -s "$EDR_RESIZED_FILE" ]; then
    EDR_RESIZED_SIZE=$(stat -f%z "$EDR_RESIZED_FILE" 2>/dev/null || stat -c%s "$EDR_RESIZED_FILE" 2>/dev/null || echo "0")
    echo "   Size: $(numfmt --to=iec-i --suffix=B $EDR_RESIZED_SIZE 2>/dev/null || echo "${EDR_RESIZED_SIZE} bytes")"
    echo "   Dimensions: 1024 x 512"
    echo "   Time: ${EDR_RESIZED_TIME}s"
    echo "   Requests: 1"
else
    echo -e "   ${RED}Failed to fetch${NC}"
    EDR_RESIZED_SIZE=0
fi
echo ""

# ============================================================================
# WMS Tile Requests
# ============================================================================

echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${CYAN}                      WMS Tile Requests                         ${NC}"
echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# Function to fetch WMS tiles for a grid
fetch_wms_tiles() {
    local cols=$1
    local rows=$2
    local tile_width=$3
    local tile_height=$4
    local prefix=$5
    
    local total_size=0
    local total_time=0
    local count=0
    
    # Calculate tile bounds
    local lon_step=$(echo "scale=6; $CONUS_WIDTH / $cols" | bc)
    local lat_step=$(echo "scale=6; $CONUS_HEIGHT / $rows" | bc)
    
    for ((c=0; c<cols; c++)); do
        for ((r=0; r<rows; r++)); do
            local minx=$(echo "scale=6; $CONUS_WEST + $c * $lon_step" | bc)
            local maxx=$(echo "scale=6; $CONUS_WEST + ($c + 1) * $lon_step" | bc)
            local miny=$(echo "scale=6; $CONUS_SOUTH + $r * $lat_step" | bc)
            local maxy=$(echo "scale=6; $CONUS_SOUTH + ($r + 1) * $lat_step" | bc)
            
            local tile_file="$OUTPUT_DIR/${prefix}_${c}_${r}.png"
            local tile_time=$(curl -s -o "$tile_file" -w "%{time_total}" \
                "${WMS_URL}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${LAYER}&STYLES=${STYLE}&CRS=CRS:84&BBOX=${minx},${miny},${maxx},${maxy}&WIDTH=${tile_width}&HEIGHT=${tile_height}&FORMAT=image/png" 2>/dev/null || echo "0")
            
            if [ -f "$tile_file" ] && [ -s "$tile_file" ]; then
                local tile_size=$(stat -f%z "$tile_file" 2>/dev/null || stat -c%s "$tile_file" 2>/dev/null || echo "0")
                total_size=$((total_size + tile_size))
                total_time=$(echo "$total_time + $tile_time" | bc)
                count=$((count + 1))
            fi
        done
    done
    
    echo "$total_size $count $total_time"
}

# Test 4: WMS 2x2 grid (4 tiles, each ~30x13 degrees)
echo -e "${YELLOW}4. WMS 2x2 Grid (4 tiles @ 256x256)${NC}"
WMS_2X2=$(fetch_wms_tiles 2 2 256 256 "wms_2x2")
WMS_2X2_SIZE=$(echo $WMS_2X2 | cut -d' ' -f1)
WMS_2X2_COUNT=$(echo $WMS_2X2 | cut -d' ' -f2)
WMS_2X2_TIME=$(echo $WMS_2X2 | cut -d' ' -f3)
echo "   Total Size: $(numfmt --to=iec-i --suffix=B $WMS_2X2_SIZE 2>/dev/null || echo "${WMS_2X2_SIZE} bytes")"
echo "   Requests: $WMS_2X2_COUNT"
echo "   Total Time: ${WMS_2X2_TIME}s (sequential)"
echo "   Resolution: ~512x512 effective"
echo ""

# Test 5: WMS 4x4 grid (16 tiles)
echo -e "${YELLOW}5. WMS 4x4 Grid (16 tiles @ 256x256)${NC}"
WMS_4X4=$(fetch_wms_tiles 4 4 256 256 "wms_4x4")
WMS_4X4_SIZE=$(echo $WMS_4X4 | cut -d' ' -f1)
WMS_4X4_COUNT=$(echo $WMS_4X4 | cut -d' ' -f2)
WMS_4X4_TIME=$(echo $WMS_4X4 | cut -d' ' -f3)
echo "   Total Size: $(numfmt --to=iec-i --suffix=B $WMS_4X4_SIZE 2>/dev/null || echo "${WMS_4X4_SIZE} bytes")"
echo "   Requests: $WMS_4X4_COUNT"
echo "   Total Time: ${WMS_4X4_TIME}s (sequential)"
echo "   Resolution: ~1024x1024 effective"
echo ""

# Test 6: WMS 8x4 grid (32 tiles - similar to web map zoom level)
echo -e "${YELLOW}6. WMS 8x4 Grid (32 tiles @ 256x256)${NC}"
WMS_8X4=$(fetch_wms_tiles 8 4 256 256 "wms_8x4")
WMS_8X4_SIZE=$(echo $WMS_8X4 | cut -d' ' -f1)
WMS_8X4_COUNT=$(echo $WMS_8X4 | cut -d' ' -f2)
WMS_8X4_TIME=$(echo $WMS_8X4 | cut -d' ' -f3)
echo "   Total Size: $(numfmt --to=iec-i --suffix=B $WMS_8X4_SIZE 2>/dev/null || echo "${WMS_8X4_SIZE} bytes")"
echo "   Requests: $WMS_8X4_COUNT"
echo "   Total Time: ${WMS_8X4_TIME}s (sequential)"
echo "   Resolution: ~2048x1024 effective"
echo ""

# Test 7: Single large WMS tile (for direct comparison)
echo -e "${YELLOW}7. WMS Single Tile (1024x512)${NC}"
WMS_SINGLE_FILE="$OUTPUT_DIR/wms_single_1024x512.png"
WMS_SINGLE_TIME=$(curl -s -o "$WMS_SINGLE_FILE" -w "%{time_total}" \
    "${WMS_URL}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${LAYER}&STYLES=${STYLE}&CRS=CRS:84&BBOX=${CONUS_WEST},${CONUS_SOUTH},${CONUS_EAST},${CONUS_NORTH}&WIDTH=1024&HEIGHT=512&FORMAT=image/png" 2>/dev/null || echo "0")

if [ -f "$WMS_SINGLE_FILE" ] && [ -s "$WMS_SINGLE_FILE" ]; then
    WMS_SINGLE_SIZE=$(stat -f%z "$WMS_SINGLE_FILE" 2>/dev/null || stat -c%s "$WMS_SINGLE_FILE" 2>/dev/null || echo "0")
    echo "   Size: $(numfmt --to=iec-i --suffix=B $WMS_SINGLE_SIZE 2>/dev/null || echo "${WMS_SINGLE_SIZE} bytes")"
    echo "   Dimensions: 1024 x 512"
    echo "   Time: ${WMS_SINGLE_TIME}s"
    echo "   Requests: 1"
else
    echo -e "   ${RED}Failed to fetch${NC}"
    WMS_SINGLE_SIZE=0
fi
echo ""

# ============================================================================
# Summary Comparison
# ============================================================================

echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${CYAN}                       Summary Comparison                       ${NC}"
echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

printf "%-35s %12s %10s %12s\n" "Method" "Size" "Requests" "Time"
printf "%-35s %12s %10s %12s\n" "-----------------------------------" "------------" "----------" "------------"
printf "%-35s %12s %10s %12s\n" "EDR PNG 16-bit (native)" "$(numfmt --to=iec-i --suffix=B $EDR_16BIT_SIZE 2>/dev/null || echo "$EDR_16BIT_SIZE")" "1" "${EDR_16BIT_TIME}s"
printf "%-35s %12s %10s %12s\n" "EDR PNG 8-bit (native)" "$(numfmt --to=iec-i --suffix=B $EDR_8BIT_SIZE 2>/dev/null || echo "$EDR_8BIT_SIZE")" "1" "${EDR_8BIT_TIME}s"
printf "%-35s %12s %10s %12s\n" "EDR PNG 8-bit (1024x512)" "$(numfmt --to=iec-i --suffix=B $EDR_RESIZED_SIZE 2>/dev/null || echo "$EDR_RESIZED_SIZE")" "1" "${EDR_RESIZED_TIME}s"
printf "%-35s %12s %10s %12s\n" "WMS 2x2 (4 tiles)" "$(numfmt --to=iec-i --suffix=B $WMS_2X2_SIZE 2>/dev/null || echo "$WMS_2X2_SIZE")" "$WMS_2X2_COUNT" "${WMS_2X2_TIME}s"
printf "%-35s %12s %10s %12s\n" "WMS 4x4 (16 tiles)" "$(numfmt --to=iec-i --suffix=B $WMS_4X4_SIZE 2>/dev/null || echo "$WMS_4X4_SIZE")" "$WMS_4X4_COUNT" "${WMS_4X4_TIME}s"
printf "%-35s %12s %10s %12s\n" "WMS 8x4 (32 tiles)" "$(numfmt --to=iec-i --suffix=B $WMS_8X4_SIZE 2>/dev/null || echo "$WMS_8X4_SIZE")" "$WMS_8X4_COUNT" "${WMS_8X4_TIME}s"
printf "%-35s %12s %10s %12s\n" "WMS Single (1024x512)" "$(numfmt --to=iec-i --suffix=B $WMS_SINGLE_SIZE 2>/dev/null || echo "$WMS_SINGLE_SIZE")" "1" "${WMS_SINGLE_TIME}s"

echo ""
echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${CYAN}                         Key Insights                           ${NC}"
echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""

# Direct comparison: EDR 8-bit 1024x512 vs WMS single 1024x512
if [ "$EDR_RESIZED_SIZE" -gt 0 ] && [ "$WMS_SINGLE_SIZE" -gt 0 ]; then
    echo -e "${GREEN}Direct Comparison (1024x512):${NC}"
    echo "  EDR PNG 8-bit:  $(numfmt --to=iec-i --suffix=B $EDR_RESIZED_SIZE 2>/dev/null || echo "$EDR_RESIZED_SIZE bytes")"
    echo "  WMS rendered:   $(numfmt --to=iec-i --suffix=B $WMS_SINGLE_SIZE 2>/dev/null || echo "$WMS_SINGLE_SIZE bytes")"
    
    if [ "$EDR_RESIZED_SIZE" -lt "$WMS_SINGLE_SIZE" ]; then
        DIFF=$((WMS_SINGLE_SIZE - EDR_RESIZED_SIZE))
        PCT=$((DIFF * 100 / WMS_SINGLE_SIZE))
        echo -e "  ${GREEN}EDR is ${PCT}% smaller${NC}"
    else
        DIFF=$((EDR_RESIZED_SIZE - WMS_SINGLE_SIZE))
        PCT=$((DIFF * 100 / EDR_RESIZED_SIZE))
        echo -e "  ${YELLOW}WMS is ${PCT}% smaller${NC}"
    fi
    echo ""
fi

# Typical web usage comparison: EDR single vs WMS 4x4 grid
if [ "$EDR_RESIZED_SIZE" -gt 0 ] && [ "$WMS_4X4_SIZE" -gt 0 ]; then
    echo -e "${GREEN}Typical Web Map Comparison (1024x1024 effective):${NC}"
    echo "  EDR PNG (1 request):      $(numfmt --to=iec-i --suffix=B $EDR_RESIZED_SIZE 2>/dev/null || echo "$EDR_RESIZED_SIZE bytes")"
    echo "  WMS 4x4 ($WMS_4X4_COUNT requests):   $(numfmt --to=iec-i --suffix=B $WMS_4X4_SIZE 2>/dev/null || echo "$WMS_4X4_SIZE bytes")"
    echo "  Request reduction:        ${WMS_4X4_COUNT}x → 1x"
    
    if [ "$EDR_RESIZED_SIZE" -lt "$WMS_4X4_SIZE" ]; then
        BANDWIDTH_SAVED=$((WMS_4X4_SIZE - EDR_RESIZED_SIZE))
        BANDWIDTH_PCT=$((BANDWIDTH_SAVED * 100 / WMS_4X4_SIZE))
        echo -e "  Bandwidth saved:          ${GREEN}${BANDWIDTH_PCT}% ($(numfmt --to=iec-i --suffix=B $BANDWIDTH_SAVED 2>/dev/null || echo "$BANDWIDTH_SAVED bytes"))${NC}"
    else
        BANDWIDTH_EXTRA=$((EDR_RESIZED_SIZE - WMS_4X4_SIZE))
        BANDWIDTH_PCT=$((BANDWIDTH_EXTRA * 100 / WMS_4X4_SIZE))
        echo -e "  Extra bandwidth:          ${YELLOW}+${BANDWIDTH_PCT}% ($(numfmt --to=iec-i --suffix=B $BANDWIDTH_EXTRA 2>/dev/null || echo "$BANDWIDTH_EXTRA bytes"))${NC}"
    fi
    echo ""
fi

# Native resolution comparison
if [ "$EDR_8BIT_SIZE" -gt 0 ] && [ "$WMS_8X4_SIZE" -gt 0 ]; then
    echo -e "${GREEN}Higher Resolution Comparison (~2048x1024):${NC}"
    echo "  EDR PNG 8-bit (native):   $(numfmt --to=iec-i --suffix=B $EDR_8BIT_SIZE 2>/dev/null || echo "$EDR_8BIT_SIZE bytes")"
    echo "  WMS 8x4 ($WMS_8X4_COUNT requests):   $(numfmt --to=iec-i --suffix=B $WMS_8X4_SIZE 2>/dev/null || echo "$WMS_8X4_SIZE bytes")"
    echo "  Request reduction:        ${WMS_8X4_COUNT}x → 1x"
    
    if [ "$EDR_8BIT_SIZE" -lt "$WMS_8X4_SIZE" ]; then
        BANDWIDTH_SAVED=$((WMS_8X4_SIZE - EDR_8BIT_SIZE))
        BANDWIDTH_PCT=$((BANDWIDTH_SAVED * 100 / WMS_8X4_SIZE))
        echo -e "  Bandwidth saved:          ${GREEN}${BANDWIDTH_PCT}% ($(numfmt --to=iec-i --suffix=B $BANDWIDTH_SAVED 2>/dev/null || echo "$BANDWIDTH_SAVED bytes"))${NC}"
    else
        BANDWIDTH_EXTRA=$((EDR_8BIT_SIZE - WMS_8X4_SIZE))
        BANDWIDTH_PCT=$((BANDWIDTH_EXTRA * 100 / WMS_8X4_SIZE))
        echo -e "  Extra bandwidth:          ${YELLOW}+${BANDWIDTH_PCT}% ($(numfmt --to=iec-i --suffix=B $BANDWIDTH_EXTRA 2>/dev/null || echo "$BANDWIDTH_EXTRA bytes"))${NC}"
    fi
    echo ""
fi

echo -e "${GREEN}Trade-offs:${NC}"
echo "  WMS tiles:"
echo "    + Smaller individual files (indexed color)"
echo "    + Cacheable at CDN level per-tile"
echo "    + Works with standard mapping libraries"
echo "    - Many HTTP requests (latency overhead)"
echo "    - Fixed colormap (server-rendered)"
echo ""
echo "  EDR PNG:"
echo "    + Single HTTP request"
echo "    + 16-bit precision available"
echo "    + Client-side colormap flexibility"
echo "    + Direct GPU texture upload"
echo "    - Larger per-request payload"
echo "    - Requires custom WebGL rendering"
echo ""

echo -e "${BLUE}Output files saved to: $OUTPUT_DIR${NC}"
ls -lh "$OUTPUT_DIR"/*.png 2>/dev/null | head -10
echo ""

echo -e "${GREEN}Done!${NC}"
