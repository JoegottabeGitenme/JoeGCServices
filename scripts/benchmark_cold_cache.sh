#!/bin/bash
#
# Benchmark WMS/WMTS cold cache performance at varying zoom levels
#
# This script:
# 1. Clears all caches between requests using reset_test_state.sh
# 2. Makes both GetMap and WMTS tile requests for GFS data
# 3. Records time-to-first-byte and total time for each request
# 4. Tests zoom levels 0-8 to measure pyramid level efficiency
#
# Usage:
#   ./benchmark_cold_cache.sh                    # Run all benchmarks
#   ./benchmark_cold_cache.sh --layer gfs_TMP    # Test specific layer
#   ./benchmark_cold_cache.sh --zooms 0,2,4,6    # Test specific zoom levels
#   ./benchmark_cold_cache.sh --iterations 3    # Multiple iterations per test

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Script directory and project root
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

cd "$PROJECT_ROOT"

# Configuration
API_URL="http://localhost:8080"
DEFAULT_LAYER="gfs_TMP"
DEFAULT_ZOOMS="0,1,2,3,4,5,6,7,8,9,10,11,12,13,14"
DEFAULT_ITERATIONS=1
OUTPUT_DIR="benchmark_results"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Parse arguments
LAYER="$DEFAULT_LAYER"
ZOOMS="$DEFAULT_ZOOMS"
ITERATIONS="$DEFAULT_ITERATIONS"
SKIP_RESET=0
VERBOSE=0
FORCE=0

while [[ $# -gt 0 ]]; do
    case $1 in
        --layer)
            LAYER="$2"
            shift 2
            ;;
        --zooms)
            ZOOMS="$2"
            shift 2
            ;;
        --iterations)
            ITERATIONS="$2"
            shift 2
            ;;
        --skip-reset)
            SKIP_RESET=1
            shift
            ;;
        --verbose)
            VERBOSE=1
            shift
            ;;
        --force)
            FORCE=1
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Benchmarks WMS/WMTS cold cache performance at varying zoom levels."
            echo "Clears all caches between requests to measure true cold cache latency."
            echo ""
            echo "Options:"
            echo "  --layer LAYER       Layer to test (default: gfs_TMP)"
            echo "  --zooms ZOOMS       Comma-separated zoom levels (default: 0,1,2,3,4,5,6,7,8)"
            echo "  --iterations N      Number of iterations per test (default: 1)"
            echo "  --skip-reset        Skip cache reset between requests (warm cache test)"
            echo "  --force             Run even if layer not found in capabilities"
            echo "  --verbose           Show detailed curl output"
            echo "  -h, --help          Show this help"
            echo ""
            echo "Examples:"
            echo "  $0                                    # Default GFS temperature test"
            echo "  $0 --layer gfs_PRMSL --zooms 0,2,4,6  # Test pressure at specific zooms"
            echo "  $0 --iterations 3                     # 3 iterations for averaging"
            echo "  $0 --skip-reset                       # Warm cache comparison"
            echo ""
            echo "Prerequisites:"
            echo "  1. WMS API must be running (docker-compose up)"
            echo "  2. Data must be ingested for the layer being tested"
            echo "     Run: cargo run --package ingester -- --model gfs"
            echo ""
            echo "Output:"
            echo "  Results are saved to benchmark_results/cold_cache_benchmark_*.csv"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[OK]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_result() {
    echo -e "${CYAN}[RESULT]${NC} $1"
}

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Results file (CSV format)
RESULTS_FILE="$OUTPUT_DIR/cold_cache_benchmark_${TIMESTAMP}.csv"
echo "request_type,layer,zoom,tile_col,tile_row,width,height,ttfb_ms,total_ms,http_code,size_bytes,iteration" > "$RESULTS_FILE"

echo ""
echo -e "${CYAN}=======================================${NC}"
echo -e "${CYAN}  Cold Cache Performance Benchmark${NC}"
echo -e "${CYAN}=======================================${NC}"
echo ""
log_info "Layer: $LAYER"
log_info "Zoom levels: $ZOOMS"
log_info "Iterations per test: $ITERATIONS"
log_info "Results file: $RESULTS_FILE"
echo ""

# Check service health
log_info "Checking WMS service health..."
if ! curl -s --connect-timeout 5 "$API_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities" > /dev/null 2>&1; then
    echo -e "${RED}ERROR: WMS service is not responding at $API_URL${NC}"
    exit 1
fi
log_success "WMS service is healthy"

# Check if layer exists in capabilities
CAPS=$(curl -s "$API_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities")
if ! echo "$CAPS" | grep -q "$LAYER"; then
    log_warn "Layer '$LAYER' not found in capabilities."
    
    # Show available layers
    AVAILABLE_LAYERS=$(echo "$CAPS" | grep -oP '(?<=<Name>)[^<]+(?=</Name>)' | grep -E '^(gfs|hrrr|mrms|goes)' 2>/dev/null || true)
    if [ -n "$AVAILABLE_LAYERS" ]; then
        echo "  Available data layers:"
        echo "$AVAILABLE_LAYERS" | sed 's/^/    /'
    else
        echo "  No data layers available. You need to ingest data first:"
        echo "    cargo run --package ingester -- --model gfs"
    fi
    
    if [ "$FORCE" = "1" ]; then
        log_warn "Continuing anyway due to --force flag (requests will likely fail)"
    else
        echo ""
        echo "  Use --force to run anyway, or ingest data first."
        exit 1
    fi
else
    log_success "Layer '$LAYER' found in capabilities"
fi
echo ""

# Function to reset caches
reset_caches() {
    if [ "$SKIP_RESET" = "1" ]; then
        return 0
    fi
    
    # Use the existing reset script but suppress output
    "$SCRIPT_DIR/reset_test_state.sh" > /dev/null 2>&1 || {
        # Fallback: directly flush Redis if script fails
        REDIS_CONTAINER=$(docker-compose ps -q redis 2>/dev/null)
        if [ -n "$REDIS_CONTAINER" ]; then
            timeout 5 docker exec "$REDIS_CONTAINER" redis-cli FLUSHALL > /dev/null 2>&1 || true
        fi
        # Clear L1 cache via API
        curl -s -X POST "$API_URL/api/cache/clear" > /dev/null 2>&1 || true
    }
    
    # Brief pause to let system settle
    sleep 0.2
}

# Function to get tile coordinates for a zoom level (center of world)
get_tile_coords() {
    local zoom=$1
    # For EPSG:3857, tiles at zoom z have 2^z tiles per axis
    # Center tile is at (2^(z-1), 2^(z-1)) for z > 0
    # We'll pick a tile that covers a good portion of CONUS
    
    # Approximate CONUS center: lon=-98, lat=39
    # In Web Mercator tile scheme:
    local max_tile=$((1 << zoom))
    
    # Convert lon/lat to tile coordinates
    # x = floor((lon + 180) / 360 * 2^zoom)
    # y = floor((1 - ln(tan(lat_rad) + 1/cos(lat_rad)) / pi) / 2 * 2^zoom)
    
    # For simplicity, use pre-calculated values for CONUS center
    case $zoom in
        0) echo "0 0" ;;
        1) echo "0 0" ;;
        2) echo "1 1" ;;
        3) echo "2 2" ;;
        4) echo "4 5" ;;
        5) echo "8 11" ;;
        6) echo "16 23" ;;
        7) echo "32 47" ;;
        8) echo "64 94" ;;
        9) echo "128 189" ;;
        10) echo "256 378" ;;
        *) echo "0 0" ;;
    esac
}

# Function to get bbox for GetMap at a zoom level
get_bbox_for_zoom() {
    local zoom=$1
    # Return progressively smaller bboxes for higher zoom levels
    # Format: minlon,minlat,maxlon,maxlat (EPSG:4326 axis order for WMS 1.3.0)
    # Note: For CRS=EPSG:4326 in WMS 1.3.0, bbox order is minlat,minlon,maxlat,maxlon
    case $zoom in
        0) echo "-90,-180,90,180" ;;      # Global
        1) echo "-90,-180,90,0" ;;         # Western hemisphere
        2) echo "0,-135,60,-45" ;;          # North America
        3) echo "20,-120,55,-70" ;;         # CONUS
        4) echo "30,-110,50,-85" ;;         # Central US
        5) echo "35,-105,45,-90" ;;         # Central plains
        6) echo "38,-100,43,-94" ;;         # Kansas area
        7) echo "39,-98,41,-96" ;;          # ~200km box
        8) echo "39.5,-97.5,40.5,-96.5" ;;  # ~100km box
        9) echo "39.75,-97.25,40.25,-96.75" ;;
        10) echo "39.9,-97.1,40.1,-96.9" ;;
        *) echo "-90,-180,90,180" ;;
    esac
}

# Function to calculate image size based on zoom (simulate typical usage)
get_size_for_zoom() {
    local zoom=$1
    # Higher zoom = smaller geographic area but we still want 256x256 tiles
    # For GetMap, we might request larger images at low zoom
    case $zoom in
        0|1) echo "512 256" ;;
        2|3) echo "512 512" ;;
        *) echo "256 256" ;;
    esac
}

# Function to make a timed request
# Returns: ttfb_ms total_ms http_code size_bytes
make_timed_request() {
    local url=$1
    local output_file=$(mktemp)
    
    # Use curl with timing info
    # %{time_starttransfer} = time to first byte
    # %{time_total} = total time
    # %{http_code} = HTTP status code
    # %{size_download} = response size
    
    local result
    result=$(curl -s -w "%{time_starttransfer}|%{time_total}|%{http_code}|%{size_download}" \
        --connect-timeout 10 \
        --max-time 60 \
        -o "$output_file" \
        "$url" 2>/dev/null) || true
    
    rm -f "$output_file"
    
    # Parse result (handle empty or malformed responses)
    local ttfb=$(echo "$result" | cut -d'|' -f1)
    local total=$(echo "$result" | cut -d'|' -f2)
    local code=$(echo "$result" | cut -d'|' -f3)
    local size=$(echo "$result" | cut -d'|' -f4)
    
    # Default values if parsing failed
    [ -z "$ttfb" ] && ttfb="0"
    [ -z "$total" ] && total="0"
    [ -z "$code" ] && code="000"
    [ -z "$size" ] && size="0"
    
    # Convert to milliseconds using awk (more portable than bc)
    local ttfb_ms=$(awk "BEGIN {printf \"%.0f\", $ttfb * 1000}")
    local total_ms=$(awk "BEGIN {printf \"%.0f\", $total * 1000}")
    
    echo "$ttfb_ms $total_ms $code $size"
}

# Arrays to store results for summary
declare -a GETMAP_RESULTS
declare -a WMTS_RESULTS

echo "======================================="
echo "  Running Cold Cache Benchmarks"
echo "======================================="
echo ""

# Convert zoom string to array
IFS=',' read -ra ZOOM_ARRAY <<< "$ZOOMS"

for zoom in "${ZOOM_ARRAY[@]}"; do
    echo -e "${YELLOW}--- Zoom Level $zoom ---${NC}"
    
    # Get parameters for this zoom level
    read -r width height <<< "$(get_size_for_zoom $zoom)"
    bbox=$(get_bbox_for_zoom $zoom)
    read -r tile_col tile_row <<< "$(get_tile_coords $zoom)"
    
    for iteration in $(seq 1 $ITERATIONS); do
        if [ "$ITERATIONS" -gt 1 ]; then
            echo "  Iteration $iteration/$ITERATIONS"
        fi
        
        # =========================================
        # Test WMS GetMap request
        # =========================================
        reset_caches
        
        # Build GetMap URL
        # Note: For EPSG:4326 in WMS 1.3.0, bbox should be lat,lon order
        # But many servers accept lon,lat - we'll use what works
        GETMAP_URL="$API_URL/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap"
        GETMAP_URL="${GETMAP_URL}&LAYERS=${LAYER}&STYLES=&CRS=EPSG:4326"
        GETMAP_URL="${GETMAP_URL}&BBOX=${bbox}&WIDTH=${width}&HEIGHT=${height}"
        GETMAP_URL="${GETMAP_URL}&FORMAT=image/png&TRANSPARENT=true"
        
        if [ "$VERBOSE" = "1" ]; then
            echo "  GetMap URL: $GETMAP_URL"
        fi
        
        read -r ttfb total code size <<< "$(make_timed_request "$GETMAP_URL")"
        
        echo "  GetMap:  ${total}ms (TTFB: ${ttfb}ms) - ${size} bytes - HTTP $code"
        
        # Record result (tile_col and tile_row are empty for GetMap)
        echo "getmap,$LAYER,$zoom,,,$width,$height,$ttfb,$total,$code,$size,$iteration" >> "$RESULTS_FILE"
        GETMAP_RESULTS+=("$zoom:$total")
        
        # =========================================
        # Test WMTS tile request
        # =========================================
        reset_caches
        
        # Build WMTS URL (KVP style)
        WMTS_URL="$API_URL/wmts?SERVICE=WMTS&VERSION=1.0.0&REQUEST=GetTile"
        WMTS_URL="${WMTS_URL}&LAYER=${LAYER}&STYLE=default"
        WMTS_URL="${WMTS_URL}&FORMAT=image/png&TILEMATRIXSET=WebMercatorQuad"
        WMTS_URL="${WMTS_URL}&TILEMATRIX=${zoom}&TILEROW=${tile_row}&TILECOL=${tile_col}"
        
        if [ "$VERBOSE" = "1" ]; then
            echo "  WMTS URL: $WMTS_URL"
        fi
        
        read -r ttfb total code size <<< "$(make_timed_request "$WMTS_URL")"
        
        echo "  WMTS:    ${total}ms (TTFB: ${ttfb}ms) - ${size} bytes - HTTP $code"
        
        # Record result
        echo "wmts,$LAYER,$zoom,$tile_col,$tile_row,256,256,$ttfb,$total,$code,$size,$iteration" >> "$RESULTS_FILE"
        WMTS_RESULTS+=("$zoom:$total")
    done
    echo ""
done

# =========================================
# Summary
# =========================================
echo "======================================="
echo "  Summary"
echo "======================================="
echo ""

echo "Results saved to: $RESULTS_FILE"
echo ""

# Calculate averages by zoom level
echo "Average response times by zoom level (ms):"
echo ""
printf "%-6s | %-12s | %-12s\n" "Zoom" "GetMap" "WMTS"
printf "%-6s-+-%-12s-+-%-12s\n" "------" "------------" "------------"

for zoom in "${ZOOM_ARRAY[@]}"; do
    getmap_times=""
    wmts_times=""
    
    for result in "${GETMAP_RESULTS[@]}"; do
        if [[ "$result" == "$zoom:"* ]]; then
            time="${result#*:}"
            if [ -z "$getmap_times" ]; then
                getmap_times="$time"
            else
                getmap_times="$getmap_times+$time"
            fi
        fi
    done
    
    for result in "${WMTS_RESULTS[@]}"; do
        if [[ "$result" == "$zoom:"* ]]; then
            time="${result#*:}"
            if [ -z "$wmts_times" ]; then
                wmts_times="$time"
            else
                wmts_times="$wmts_times+$time"
            fi
        fi
    done
    
    # Calculate averages using awk
    if [ -n "$getmap_times" ]; then
        getmap_avg=$(awk "BEGIN {printf \"%.0f\", ($getmap_times)/$ITERATIONS}")
    else
        getmap_avg="N/A"
    fi
    
    if [ -n "$wmts_times" ]; then
        wmts_avg=$(awk "BEGIN {printf \"%.0f\", ($wmts_times)/$ITERATIONS}")
    else
        wmts_avg="N/A"
    fi
    
    printf "%-6s | %-12s | %-12s\n" "$zoom" "$getmap_avg" "$wmts_avg"
done

echo ""
echo "======================================="
echo ""

# Show file contents for easy copy/paste
if [ "$VERBOSE" = "1" ]; then
    echo "Raw CSV data:"
    cat "$RESULTS_FILE"
    echo ""
fi

echo "To view results:"
echo "  cat $RESULTS_FILE"
echo ""
echo "To plot with gnuplot:"
echo "  gnuplot -e \"set terminal png; set output 'benchmark.png'; set datafile separator ','; plot '$RESULTS_FILE' using 3:9 with linespoints title 'Total Time'\""
echo ""
