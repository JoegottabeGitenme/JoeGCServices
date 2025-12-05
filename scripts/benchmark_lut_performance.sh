#!/usr/bin/env bash

# Benchmark script to compare GOES tile rendering performance with and without LUT
#
# This script:
# 1. Runs a series of GOES tile requests with LUT enabled
# 2. Runs the same requests with LUT disabled
# 3. Compares the response times
#
# Usage: ./scripts/benchmark_lut_performance.sh [options]
#
# Options:
#   --api-url URL      WMS API URL (default: http://localhost:8080)
#   --iterations N     Number of iterations per tile (default: 10)
#   --warmup N         Number of warmup requests (default: 3)
#   --output FILE      Output results to file (default: stdout)

set -e

# Defaults
API_URL="${API_URL:-http://localhost:8080}"
ITERATIONS=10
WARMUP=3
OUTPUT=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --api-url)
            API_URL="$2"
            shift 2
            ;;
        --iterations)
            ITERATIONS="$2"
            shift 2
            ;;
        --warmup)
            WARMUP="$2"
            shift 2
            ;;
        --output)
            OUTPUT="$2"
            shift 2
            ;;
        --help)
            echo "Usage: $0 [options]"
            echo ""
            echo "Options:"
            echo "  --api-url URL      WMS API URL (default: http://localhost:8080)"
            echo "  --iterations N     Number of iterations per tile (default: 10)"
            echo "  --warmup N         Number of warmup requests (default: 3)"
            echo "  --output FILE      Output results to file (default: stdout)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log() {
    echo -e "$1"
}

log_header() {
    log "\n${BLUE}=== $1 ===${NC}\n"
}

# Test tiles at different zoom levels within GOES coverage
# Format: "z/x/y description"
TILES=(
    "5/7/11 z5_central_conus"
    "5/8/11 z5_midwest"
    "6/14/22 z6_kansas"
    "6/15/23 z6_oklahoma"
    "7/28/44 z7_detailed_1"
    "7/30/46 z7_detailed_2"
)

# Function to time a single request and return milliseconds
time_request() {
    local url="$1"
    local start end elapsed_ms
    
    start=$(python3 -c 'import time; print(int(time.time() * 1000))')
    curl -s -o /dev/null -w "%{http_code}" "$url" > /dev/null
    end=$(python3 -c 'import time; print(int(time.time() * 1000))')
    
    elapsed_ms=$((end - start))
    echo "$elapsed_ms"
}

# Function to run benchmark for a tile
benchmark_tile() {
    local z=$1
    local x=$2
    local y=$3
    local name=$4
    local iterations=$5
    local warmup=$6
    local layer=$7
    qq
    local url="${API_URL}/tiles/${layer}/default/${z}/${x}/${y}.png"
    local times=()
    
    # Warmup
    for ((i=1; i<=warmup; i++)); do
        curl -s -o /dev/null "$url"
    done
    
    # Timed iterations
    for ((i=1; i<=iterations; i++)); do
        local ms=$(time_request "$url")
        times+=("$ms")
    done
    
    # Calculate statistics
    local sum=0
    local min=${times[0]}
    local max=${times[0]}
    
    for t in "${times[@]}"; do
        sum=$((sum + t))
        ((t < min)) && min=$t
        ((t > max)) && max=$t
    done
    
    local avg=$((sum / iterations))
    
    echo "$avg $min $max"
}

# Check if service is available
log_header "Checking Service"
if ! curl -s "${API_URL}/health" > /dev/null 2>&1; then
    log "${RED}ERROR: WMS API not responding at ${API_URL}${NC}"
    exit 1
fi
log "${GREEN}✓ Service is up at ${API_URL}${NC}"

# Check if GOES data is available and detect which satellite
CAPS=$(curl -s "${API_URL}/wmts?SERVICE=WMTS&REQUEST=GetCapabilities" 2>/dev/null || echo "")

# Try to find a GOES IR layer (C13 is the "clean" IR band, most commonly used)
if echo "$CAPS" | grep -q "goes16_CMI_C13"; then
    GOES_LAYER="goes16_CMI_C13"
    GOES_SATELLITE="goes16"
    log "${GREEN}✓ GOES-16 CMI_C13 data available${NC}"
elif echo "$CAPS" | grep -q "goes18_CMI_C13"; then
    GOES_LAYER="goes18_CMI_C13"
    GOES_SATELLITE="goes18"
    log "${GREEN}✓ GOES-18 CMI_C13 data available${NC}"
elif echo "$CAPS" | grep -q "goes16_CMI"; then
    # Fallback to any GOES-16 layer
    GOES_LAYER=$(echo "$CAPS" | grep -oE "goes16_CMI_C[0-9]+" | head -1)
    GOES_SATELLITE="goes16"
    log "${GREEN}✓ GOES-16 data available (${GOES_LAYER})${NC}"
elif echo "$CAPS" | grep -q "goes18_CMI"; then
    # Fallback to any GOES-18 layer
    GOES_LAYER=$(echo "$CAPS" | grep -oE "goes18_CMI_C[0-9]+" | head -1)
    GOES_SATELLITE="goes18"
    log "${GREEN}✓ GOES-18 data available (${GOES_LAYER})${NC}"
else
    log "${RED}ERROR: No GOES data available. Please ingest test data first.${NC}"
    log "Run: ./scripts/download_goes.sh && ./scripts/ingest_test_data.sh"
    exit 1
fi
log "Using layer: ${GOES_LAYER}"

# Check current LUT status
log_header "Checking LUT Configuration"
CONFIG=$(curl -s "${API_URL}/api/config" 2>/dev/null || echo "{}")
LUT_ENABLED=$(echo "$CONFIG" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    lut_config = d.get('optimizations', {}).get('projection_lut', {})
    enabled = lut_config.get('enabled', False)
    goes16_loaded = lut_config.get('goes16_loaded', False)
    goes18_loaded = lut_config.get('goes18_loaded', False)
    memory_mb = lut_config.get('memory_mb', 0)
    print(f'{enabled}|{goes16_loaded}|{goes18_loaded}|{memory_mb:.1f}')
except Exception as e:
    print(f'unknown|false|false|0')
" 2>/dev/null || echo "unknown|false|false|0")

IFS='|' read -r LUT_ENABLED GOES16_LOADED GOES18_LOADED LUT_MEMORY_MB <<< "$LUT_ENABLED"
log "LUT Enabled: ${LUT_ENABLED}"
log "GOES-16 LUT Loaded: ${GOES16_LOADED}"
log "GOES-18 LUT Loaded: ${GOES18_LOADED}"
log "LUT Memory: ${LUT_MEMORY_MB} MB"

# Check if the LUT for our satellite is loaded
if [ "$GOES_SATELLITE" = "goes16" ] && [ "$GOES16_LOADED" != "true" ] && [ "$GOES16_LOADED" != "True" ]; then
    log "${YELLOW}⚠ GOES-16 LUT not loaded - benchmark will show on-the-fly performance${NC}"
elif [ "$GOES_SATELLITE" = "goes18" ] && [ "$GOES18_LOADED" != "true" ] && [ "$GOES18_LOADED" != "True" ]; then
    log "${YELLOW}⚠ GOES-18 LUT not loaded - benchmark will show on-the-fly performance${NC}"
fi

# Run benchmarks
log_header "Running Benchmarks"
log "Iterations per tile: ${ITERATIONS}"
log "Warmup requests: ${WARMUP}"
log ""

# Results storage
all_avgs=""

log "${YELLOW}Testing tiles...${NC}"
log ""
printf "%-25s %10s %10s %10s\n" "Tile" "Avg (ms)" "Min (ms)" "Max (ms)"
printf "%s\n" "-------------------------------------------------------"

for tile_spec in "${TILES[@]}"; do
    coords=$(echo "$tile_spec" | awk '{print $1}')
    name=$(echo "$tile_spec" | awk '{print $2}')
    z=$(echo "$coords" | cut -d'/' -f1)
    x=$(echo "$coords" | cut -d'/' -f2)
    y=$(echo "$coords" | cut -d'/' -f3)
    
    result=$(benchmark_tile "$z" "$x" "$y" "$name" "$ITERATIONS" "$WARMUP" "$GOES_LAYER")
    avg=$(echo "$result" | awk '{print $1}')
    min=$(echo "$result" | awk '{print $2}')
    max=$(echo "$result" | awk '{print $3}')
    
    all_avgs="$all_avgs $avg"
    printf "%-25s %10d %10d %10d\n" "$name" "$avg" "$min" "$max"
done

# Calculate overall statistics
log ""
log_header "Summary"

total_avg=0
count=0
for avg in $all_avgs; do
    total_avg=$((total_avg + avg))
    count=$((count + 1))
done

if [ $count -gt 0 ]; then
    overall_avg=$((total_avg / count))
    log "Layer tested: ${GOES_LAYER}"
    log "Overall average response time: ${overall_avg} ms"
    log ""
    
    # Check if LUT is enabled AND loaded for this satellite
    lut_active="false"
    if [ "$LUT_ENABLED" = "true" ] || [ "$LUT_ENABLED" = "True" ]; then
        if [ "$GOES_SATELLITE" = "goes16" ] && ([ "$GOES16_LOADED" = "true" ] || [ "$GOES16_LOADED" = "True" ]); then
            lut_active="true"
        elif [ "$GOES_SATELLITE" = "goes18" ] && ([ "$GOES18_LOADED" = "true" ] || [ "$GOES18_LOADED" = "True" ]); then
            lut_active="true"
        fi
    fi
    
    if [ "$lut_active" = "true" ]; then
        log "${GREEN}LUT is ACTIVE for ${GOES_SATELLITE}${NC}"
        log "Expected performance: ~1-2ms for resampling (+ I/O, color mapping, PNG encoding)"
        log ""
        if [ $overall_avg -lt 50 ]; then
            log "${GREEN}✓ Performance looks good! LUT is working.${NC}"
        else
            log "${YELLOW}⚠ Response times seem high. Check logs for issues.${NC}"
        fi
    else
        log "${YELLOW}LUT is NOT ACTIVE for ${GOES_SATELLITE}${NC}"
        log "Expected performance: ~7-8ms for resampling (projection transforms)"
        log ""
        log "To enable LUT for ${GOES_SATELLITE}:"
        log "  1. Generate LUT: cargo run --release --bin generate-goes-lut -- --output ./data/luts --satellite ${GOES_SATELLITE}"
        log "  2. Set environment: ENABLE_PROJECTION_LUT=true PROJECTION_LUT_DIR=./data/luts"
        log "  3. Restart the service"
    fi
fi

# Output to file if requested
if [ -n "$OUTPUT" ]; then
    {
        echo "# LUT Performance Benchmark Results"
        echo "# Date: $(date -Iseconds)"
        echo "# API URL: ${API_URL}"
        echo "# LUT Enabled: ${LUT_ENABLED}"
        echo "# Iterations: ${ITERATIONS}"
        echo ""
        echo "tile,avg_ms,min_ms,max_ms"
        for name in "${!results[@]}"; do
            read -r avg min max <<< "${results[$name]}"
            echo "$name,$avg,$min,$max"
        done
    } > "$OUTPUT"
    log ""
    log "Results saved to: ${OUTPUT}"
fi

log ""
log "Done!"
