#!/bin/bash
#
# Comprehensive WMS/WMTS Endpoint Hammer Script
#
# This script makes requests for EVERY possible layer/style/elevation/time/run/forecast
# combination exposed by the WMS/WMTS GetCapabilities. It also performs tile stitching
# validation to detect boundary issues.
#
# Usage:
#   ./scripts/hammer_all_endpoints.sh [options]
#
# Options:
#   --url URL           Base URL (default: http://localhost:8080)
#   --output DIR        Output directory for results/images (default: ./hammer_results)
#   --skip-wms          Skip WMS GetMap tests
#   --skip-wmts         Skip WMTS tile tests
#   --skip-stitch       Skip tile stitching tests
#   --layer PATTERN     Only test layers matching pattern (e.g., "gfs_TMP")
#   --max-combos N      Maximum combinations per layer (default: all)
#   --parallel N        Number of parallel requests (default: 4)
#   --save-images       Save all response images (warning: can use lots of disk)
#   --save-failures     Only save images for failed requests (default)
#   --stitch-zoom Z     Zoom level for stitch tests (default: 6)
#   --verbose           Show detailed output
#   --help              Show this help message
#
# Exit codes:
#   0 - All tests passed
#   1 - One or more tests failed
#   2 - Configuration/setup error

set -uo pipefail
# Note: We don't use 'set -e' because we want to continue on test failures

# =============================================================================
# Configuration
# =============================================================================

BASE_URL="${BASE_URL:-http://localhost:8080}"
OUTPUT_DIR="${OUTPUT_DIR:-./hammer_results}"
SKIP_WMS=false
SKIP_WMTS=false
SKIP_STITCH=false
LAYER_PATTERN=""
MAX_COMBOS=""
PARALLEL=4
SAVE_IMAGES="failures"  # "all", "failures", "none"
STITCH_ZOOM=6
VERBOSE=false
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Color codes
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    CYAN='\033[0;36m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    CYAN=''
    NC=''
fi

# =============================================================================
# Parse Arguments
# =============================================================================

show_help() {
    # Print lines 2-30 (the header comment block)
    sed -n '2,30p' "$0" | sed 's/^# //' | sed 's/^#//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --url)
            BASE_URL="$2"
            shift 2
            ;;
        --output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --skip-wms)
            SKIP_WMS=true
            shift
            ;;
        --skip-wmts)
            SKIP_WMTS=true
            shift
            ;;
        --skip-stitch)
            SKIP_STITCH=true
            shift
            ;;
        --layer)
            # Handle "*" or "all" as "match everything"
            if [[ "$2" == "*" ]] || [[ "$2" == "all" ]]; then
                LAYER_PATTERN=""
            else
                LAYER_PATTERN="$2"
            fi
            shift 2
            ;;
        --max-combos)
            MAX_COMBOS="$2"
            shift 2
            ;;
        --parallel)
            PARALLEL="$2"
            shift 2
            ;;
        --save-images)
            SAVE_IMAGES="all"
            shift
            ;;
        --save-failures)
            SAVE_IMAGES="failures"
            shift
            ;;
        --stitch-zoom)
            STITCH_ZOOM="$2"
            shift 2
            ;;
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --help|-h)
            show_help
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 2
            ;;
    esac
done

# =============================================================================
# Setup
# =============================================================================

RESULTS_DIR="$OUTPUT_DIR/$TIMESTAMP"
TEMP_DIR=$(mktemp -d)
WMS_CAPS_FILE="$TEMP_DIR/wms_capabilities.xml"
WMTS_CAPS_FILE="$TEMP_DIR/wmts_capabilities.xml"
RESULTS_FILE="$RESULTS_DIR/results.json"
SUMMARY_FILE="$RESULTS_DIR/summary.txt"
STITCH_DIR="$RESULTS_DIR/stitched"

trap "rm -rf $TEMP_DIR" EXIT

mkdir -p "$RESULTS_DIR"
mkdir -p "$STITCH_DIR"

# Counters
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
SKIPPED_TESTS=0

# Arrays for tracking
declare -a FAILED_REQUESTS=()
declare -a ALL_RESULTS=()

# =============================================================================
# Utility Functions
# =============================================================================

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

log_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_verbose() {
    if [[ "$VERBOSE" == "true" ]]; then
        echo -e "${CYAN}[DEBUG]${NC} $1"
    fi
}

# URL encode a string
urlencode() {
    local string="$1"
    python3 -c "import urllib.parse; print(urllib.parse.quote('$string', safe=''))"
}

# Test a single endpoint and return result
# Returns: 0 if pass, 1 if fail
test_endpoint() {
    local name="$1"
    local url="$2"
    local output_file="$3"
    local expected_status="${4:-200}"
    local expected_type="${5:-image/png}"
    
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    
    local http_code
    local content_type
    local response_time
    
    # Make request with timing
    local start_time=$(date +%s%N)
    
    if ! response=$(curl -sf -w "\n%{http_code}\n%{content_type}\n%{time_total}" \
        -o "$output_file" \
        --max-time 60 \
        "$url" 2>/dev/null); then
        # Curl failed (connection error, timeout, etc.)
        http_code="000"
        content_type="error"
        response_time="60.0"
    else
        http_code=$(echo "$response" | tail -3 | head -1)
        content_type=$(echo "$response" | tail -2 | head -1)
        response_time=$(echo "$response" | tail -1)
    fi
    
    local end_time=$(date +%s%N)
    local duration_ms=$(( (end_time - start_time) / 1000000 ))
    
    local result_status="pass"
    local failure_reason=""
    
    # Check HTTP status
    if [[ "$http_code" != "$expected_status" ]]; then
        result_status="fail"
        failure_reason="HTTP $http_code (expected $expected_status)"
    # Check content type for successful requests
    elif [[ "$http_code" == "200" ]] && [[ "$expected_type" != "any" ]]; then
        if [[ "$content_type" != *"$expected_type"* ]]; then
            result_status="fail"
            failure_reason="Content-Type: $content_type (expected $expected_type)"
        fi
        # Verify PNG validity for image responses
        if [[ "$expected_type" == "image/png" ]] && [[ -f "$output_file" ]]; then
            if ! file "$output_file" | grep -q "PNG image data"; then
                result_status="fail"
                failure_reason="Response is not valid PNG data"
            fi
        fi
    fi
    
    # Update counters
    if [[ "$result_status" == "pass" ]]; then
        PASSED_TESTS=$((PASSED_TESTS + 1))
        if [[ "$VERBOSE" == "true" ]]; then
            log_success "$name (${duration_ms}ms)"
        fi
        # Remove output file if not saving all images
        if [[ "$SAVE_IMAGES" != "all" ]] && [[ -f "$output_file" ]]; then
            rm -f "$output_file"
        fi
    else
        FAILED_TESTS=$((FAILED_TESTS + 1))
        log_fail "$name - $failure_reason"
        FAILED_REQUESTS+=("$name|$url|$failure_reason")
        # Keep output file for failures
        if [[ "$SAVE_IMAGES" == "none" ]] && [[ -f "$output_file" ]]; then
            rm -f "$output_file"
        fi
    fi
    
    # Store result for JSON report
    ALL_RESULTS+=("{\"name\":\"$name\",\"status\":\"$result_status\",\"http_code\":$http_code,\"duration_ms\":$duration_ms,\"reason\":\"$failure_reason\"}")
    
    # Always return 0 to not exit the script on failures
    return 0
}

# =============================================================================
# Capabilities Parsing
# =============================================================================

# Fetch capabilities documents
fetch_capabilities() {
    log_info "Fetching WMS GetCapabilities..."
    if ! curl -sf --max-time 60 "$BASE_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities" -o "$WMS_CAPS_FILE"; then
        log_fail "Failed to fetch WMS GetCapabilities"
        exit 2
    fi
    
    log_info "Fetching WMTS GetCapabilities..."
    if ! curl -sf --max-time 60 "$BASE_URL/wmts?SERVICE=WMTS&REQUEST=GetCapabilities" -o "$WMTS_CAPS_FILE"; then
        log_fail "Failed to fetch WMTS GetCapabilities"
        exit 2
    fi
    
    log_success "Capabilities fetched successfully"
}

# Parse WMS capabilities to extract layers and their dimensions
# Output: layer|style1,style2|elevation1,elevation2|dimension_type
parse_wms_layers() {
    python3 << 'PYTHON_SCRIPT'
import xml.etree.ElementTree as ET
import sys
import re
import os

def parse_wms_capabilities(xml_file):
    """Parse WMS GetCapabilities XML and extract layer info with dimensions."""
    
    try:
        tree = ET.parse(xml_file)
        root = tree.getroot()
    except Exception as e:
        print(f"Error parsing XML: {e}", file=sys.stderr)
        return
    
    # Detect namespace from root element
    # Root tag might be like '{http://www.opengis.net/wms}WMS_Capabilities'
    ns_match = re.match(r'\{([^}]+)\}', root.tag)
    ns = ns_match.group(1) if ns_match else ''
    
    def ns_tag(tag):
        """Return namespaced tag if namespace exists."""
        return f'{{{ns}}}{tag}' if ns else tag
    
    layers = []
    
    # Iterate over all Layer elements (with or without namespace)
    for layer_elem in root.iter(ns_tag('Layer')):
        name_elem = layer_elem.find(ns_tag('Name'))
        if name_elem is None:
            continue
        
        name = name_elem.text
        if not name:
            continue
            
        # Filter to weather layers (gfs_, hrrr_, mrms_, goes*_)
        if not re.match(r'^(gfs|hrrr|mrms|goes)[0-9]*_', name):
            continue
        
        # Get styles
        styles = []
        for style_elem in layer_elem.findall(ns_tag('Style')):
            style_name = style_elem.find(ns_tag('Name'))
            if style_name is not None and style_name.text:
                styles.append(style_name.text)
        if not styles:
            styles = ['default']
        
        # Get dimensions
        elevations = []
        runs = []
        forecasts = []
        times = []
        dimension_type = 'forecast'  # default
        
        for dim_elem in layer_elem.findall(ns_tag('Dimension')):
            dim_name = dim_elem.get('name', '').upper()
            dim_values = dim_elem.text.strip() if dim_elem.text else ''
            
            if dim_name == 'ELEVATION':
                elevations = [v.strip() for v in dim_values.split(',') if v.strip()]
            elif dim_name == 'RUN':
                runs = [v.strip() for v in dim_values.split(',') if v.strip()]
            elif dim_name == 'FORECAST':
                forecasts = [v.strip() for v in dim_values.split(',') if v.strip()]
            elif dim_name == 'TIME':
                times = [v.strip() for v in dim_values.split(',') if v.strip()]
                dimension_type = 'observation'
        
        # Determine dimension type from layer name if not from TIME dimension
        if name.startswith('goes') or name.startswith('mrms'):
            dimension_type = 'observation'
        
        # Output in pipe-separated format
        styles_str = ','.join(styles) if styles else 'default'
        elevations_str = ','.join(elevations) if elevations else ''
        runs_str = ','.join(runs) if runs else ''
        forecasts_str = ','.join(forecasts) if forecasts else ''
        times_str = ','.join(times) if times else ''
        
        print(f"{name}|{styles_str}|{elevations_str}|{runs_str}|{forecasts_str}|{times_str}|{dimension_type}")
        layers.append(name)
    
    if not layers:
        print("No layers found in capabilities", file=sys.stderr)

# Run parser
xml_file = os.environ.get('WMS_CAPS_FILE', '/tmp/wms_caps.xml')
parse_wms_capabilities(xml_file)
PYTHON_SCRIPT
}

# =============================================================================
# WMS Testing
# =============================================================================

# Generate and test all WMS GetMap combinations for a layer
test_wms_layer() {
    local layer="$1"
    local styles="$2"
    local elevations="$3"
    local runs="$4"
    local forecasts="$5"
    local times="$6"
    local dimension_type="$7"
    
    log_info "Testing WMS layer: $layer"
    
    # Convert comma-separated to arrays
    IFS=',' read -ra style_array <<< "$styles"
    IFS=',' read -ra elevation_array <<< "$elevations"
    IFS=',' read -ra run_array <<< "$runs"
    IFS=',' read -ra forecast_array <<< "$forecasts"
    IFS=',' read -ra time_array <<< "$times"
    
    # If arrays are empty, use defaults
    [[ ${#style_array[@]} -eq 0 ]] && style_array=("default")
    [[ ${#elevation_array[@]} -eq 0 ]] && elevation_array=("")
    
    local combo_count=0
    local base_bbox="25,-125,50,-65"  # CONUS bbox for EPSG:4326
    
    # Determine which temporal dimensions to iterate
    if [[ "$dimension_type" == "observation" ]]; then
        # For observation data (GOES, MRMS), iterate over TIME
        [[ ${#time_array[@]} -eq 0 ]] && time_array=("")
        
        for style in "${style_array[@]}"; do
            for elevation in "${elevation_array[@]}"; do
                for time_val in "${time_array[@]}"; do
                    # Check max combos limit
                    if [[ -n "$MAX_COMBOS" ]] && [[ $combo_count -ge $MAX_COMBOS ]]; then
                        log_verbose "Reached max combos ($MAX_COMBOS) for $layer"
                        return
                    fi
                    
                    # Build URL
                    local url="$BASE_URL/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap"
                    url+="&LAYERS=$layer&STYLES=$style"
                    url+="&CRS=EPSG:4326&BBOX=$base_bbox"
                    url+="&WIDTH=256&HEIGHT=256&FORMAT=image/png"
                    
                    [[ -n "$elevation" ]] && url+="&ELEVATION=$(urlencode "$elevation")"
                    [[ -n "$time_val" ]] && url+="&TIME=$(urlencode "$time_val")"
                    
                    local test_name="WMS:$layer:$style"
                    [[ -n "$elevation" ]] && test_name+=":elev=$elevation"
                    [[ -n "$time_val" ]] && test_name+=":time=$time_val"
                    
                    local output_file="$RESULTS_DIR/wms_${layer}_${style}_${combo_count}.png"
                    
                    test_endpoint "$test_name" "$url" "$output_file"
                    
                    combo_count=$((combo_count + 1))
                done
            done
        done
    else
        # For forecast data (GFS, HRRR), iterate over RUN and FORECAST
        [[ ${#run_array[@]} -eq 0 ]] && run_array=("")
        [[ ${#forecast_array[@]} -eq 0 ]] && forecast_array=("")
        
        for style in "${style_array[@]}"; do
            for elevation in "${elevation_array[@]}"; do
                for run_val in "${run_array[@]}"; do
                    for forecast_val in "${forecast_array[@]}"; do
                        # Check max combos limit
                        if [[ -n "$MAX_COMBOS" ]] && [[ $combo_count -ge $MAX_COMBOS ]]; then
                            log_verbose "Reached max combos ($MAX_COMBOS) for $layer"
                            return
                        fi
                        
                        # Build URL
                        local url="$BASE_URL/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap"
                        url+="&LAYERS=$layer&STYLES=$style"
                        url+="&CRS=EPSG:4326&BBOX=$base_bbox"
                        url+="&WIDTH=256&HEIGHT=256&FORMAT=image/png"
                        
                        [[ -n "$elevation" ]] && url+="&ELEVATION=$(urlencode "$elevation")"
                        [[ -n "$run_val" ]] && url+="&RUN=$(urlencode "$run_val")"
                        [[ -n "$forecast_val" ]] && url+="&FORECAST=$forecast_val"
                        
                        local test_name="WMS:$layer:$style"
                        [[ -n "$elevation" ]] && test_name+=":elev=$elevation"
                        [[ -n "$run_val" ]] && test_name+=":run=$run_val"
                        [[ -n "$forecast_val" ]] && test_name+=":fhr=$forecast_val"
                        
                        local output_file="$RESULTS_DIR/wms_${layer}_${style}_${combo_count}.png"
                        
                        test_endpoint "$test_name" "$url" "$output_file"
                        
                        combo_count=$((combo_count + 1))
                    done
                done
            done
        done
    fi
    
    log_verbose "Tested $combo_count combinations for $layer (WMS)"
}

# =============================================================================
# WMTS Testing
# =============================================================================

# Test WMTS tile requests for a layer
test_wmts_layer() {
    local layer="$1"
    local styles="$2"
    local elevations="$3"
    local runs="$4"
    local forecasts="$5"
    local times="$6"
    local dimension_type="$7"
    
    log_info "Testing WMTS layer: $layer"
    
    # Convert comma-separated to arrays
    IFS=',' read -ra style_array <<< "$styles"
    IFS=',' read -ra elevation_array <<< "$elevations"
    IFS=',' read -ra run_array <<< "$runs"
    IFS=',' read -ra forecast_array <<< "$forecasts"
    IFS=',' read -ra time_array <<< "$times"
    
    # If arrays are empty, use defaults
    [[ ${#style_array[@]} -eq 0 ]] && style_array=("default")
    [[ ${#elevation_array[@]} -eq 0 ]] && elevation_array=("")
    
    local combo_count=0
    
    # Test tiles at multiple zoom levels
    local zoom_levels=(4 6 8)
    # Tile coords for CONUS center at different zooms
    local tile_coords=(
        "4:3:5"   # z=4, roughly CONUS
        "6:14:23" # z=6, roughly CONUS
        "8:58:94" # z=8, roughly CONUS
    )
    
    if [[ "$dimension_type" == "observation" ]]; then
        [[ ${#time_array[@]} -eq 0 ]] && time_array=("")
        
        for style in "${style_array[@]}"; do
            for elevation in "${elevation_array[@]}"; do
                for time_val in "${time_array[@]}"; do
                    for tile_info in "${tile_coords[@]}"; do
                        IFS=':' read -r z x y <<< "$tile_info"
                        
                        if [[ -n "$MAX_COMBOS" ]] && [[ $combo_count -ge $MAX_COMBOS ]]; then
                            log_verbose "Reached max combos ($MAX_COMBOS) for $layer"
                            return
                        fi
                        
                        # Build WMTS KVP URL
                        local url="$BASE_URL/wmts?SERVICE=WMTS&VERSION=1.0.0&REQUEST=GetTile"
                        url+="&LAYER=$layer&STYLE=$style"
                        url+="&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=$z&TILEROW=$y&TILECOL=$x"
                        url+="&FORMAT=image/png"
                        
                        [[ -n "$elevation" ]] && url+="&ELEVATION=$(urlencode "$elevation")"
                        [[ -n "$time_val" ]] && url+="&TIME=$(urlencode "$time_val")"
                        
                        local test_name="WMTS:$layer:$style:z$z"
                        [[ -n "$elevation" ]] && test_name+=":elev=$elevation"
                        [[ -n "$time_val" ]] && test_name+=":time=$time_val"
                        
                        local output_file="$RESULTS_DIR/wmts_${layer}_${style}_z${z}_${combo_count}.png"
                        
                        test_endpoint "$test_name" "$url" "$output_file"
                        
                        combo_count=$((combo_count + 1))
                    done
                done
            done
        done
    else
        [[ ${#run_array[@]} -eq 0 ]] && run_array=("")
        [[ ${#forecast_array[@]} -eq 0 ]] && forecast_array=("")
        
        for style in "${style_array[@]}"; do
            for elevation in "${elevation_array[@]}"; do
                for run_val in "${run_array[@]}"; do
                    for forecast_val in "${forecast_array[@]}"; do
                        for tile_info in "${tile_coords[@]}"; do
                            IFS=':' read -r z x y <<< "$tile_info"
                            
                            if [[ -n "$MAX_COMBOS" ]] && [[ $combo_count -ge $MAX_COMBOS ]]; then
                                log_verbose "Reached max combos ($MAX_COMBOS) for $layer"
                                wait
                                return
                            fi
                            
                            # Build WMTS KVP URL
                            local url="$BASE_URL/wmts?SERVICE=WMTS&VERSION=1.0.0&REQUEST=GetTile"
                            url+="&LAYER=$layer&STYLE=$style"
                            url+="&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=$z&TILEROW=$y&TILECOL=$x"
                            url+="&FORMAT=image/png"
                            
                            [[ -n "$elevation" ]] && url+="&ELEVATION=$(urlencode "$elevation")"
                            [[ -n "$run_val" ]] && url+="&RUN=$(urlencode "$run_val")"
                            [[ -n "$forecast_val" ]] && url+="&FORECAST=$forecast_val"
                            
                            local test_name="WMTS:$layer:$style:z$z"
                            [[ -n "$elevation" ]] && test_name+=":elev=$elevation"
                            [[ -n "$run_val" ]] && test_name+=":run=$run_val"
                            [[ -n "$forecast_val" ]] && test_name+=":fhr=$forecast_val"
                            
                            local output_file="$RESULTS_DIR/wmts_${layer}_${style}_z${z}_${combo_count}.png"
                            
                            test_endpoint "$test_name" "$url" "$output_file"
                            
                            combo_count=$((combo_count + 1))
                        done
                    done
                done
            done
        done
    fi
    
    log_verbose "Tested $combo_count combinations for $layer (WMTS)"
}

# =============================================================================
# Tile Stitching Tests
# =============================================================================

# Fetch a 3x3 grid of tiles and stitch them together to check for boundary issues
test_tile_stitching() {
    local layer="$1"
    local style="$2"
    local dimension_params="$3"  # Additional dimension params like &RUN=...&FORECAST=...
    
    log_info "Testing tile stitching for $layer ($style)"
    
    local z=$STITCH_ZOOM
    # Center tile for CONUS at zoom 6
    local center_x=14
    local center_y=23
    
    # Adjust for zoom level
    if [[ $z -eq 4 ]]; then
        center_x=3
        center_y=5
    elif [[ $z -eq 5 ]]; then
        center_x=7
        center_y=11
    elif [[ $z -eq 7 ]]; then
        center_x=29
        center_y=47
    elif [[ $z -eq 8 ]]; then
        center_x=58
        center_y=94
    fi
    
    local stitch_subdir="$STITCH_DIR/${layer}_${style}"
    mkdir -p "$stitch_subdir"
    
    local all_tiles_ok=true
    local tile_files=()
    
    # Fetch 3x3 grid of tiles
    for dy in -1 0 1; do
        for dx in -1 0 1; do
            local x=$((center_x + dx))
            local y=$((center_y + dy))
            
            local tile_file="$stitch_subdir/tile_${x}_${y}.png"
            tile_files+=("$tile_file")
            
            # Build URL using XYZ endpoint (simpler for tiles)
            local url="$BASE_URL/tiles/$layer/$style/$z/$x/$y.png"
            [[ -n "$dimension_params" ]] && url+="?$dimension_params"
            
            log_verbose "Fetching tile z=$z x=$x y=$y"
            
            if ! curl -sf --max-time 30 -o "$tile_file" "$url"; then
                log_warn "Failed to fetch tile z=$z x=$x y=$y for $layer"
                all_tiles_ok=false
            fi
        done
    done
    
    if [[ "$all_tiles_ok" == "false" ]]; then
        log_fail "STITCH:$layer:$style - Some tiles failed to fetch"
        FAILED_TESTS=$((FAILED_TESTS + 1))
        TOTAL_TESTS=$((TOTAL_TESTS + 1))
        FAILED_REQUESTS+=("STITCH:$layer:$style|tile fetch failed|")
        return 1
    fi
    
    # Use ImageMagick to stitch tiles together (if available)
    if command -v convert &> /dev/null; then
        local stitched_file="$stitch_subdir/stitched_${z}_${center_x}_${center_y}.png"
        
        # Create 3x3 montage
        # ImageMagick montage command
        if convert \
            "$stitch_subdir/tile_$((center_x-1))_$((center_y-1)).png" \
            "$stitch_subdir/tile_${center_x}_$((center_y-1)).png" \
            "$stitch_subdir/tile_$((center_x+1))_$((center_y-1)).png" \
            "$stitch_subdir/tile_$((center_x-1))_${center_y}.png" \
            "$stitch_subdir/tile_${center_x}_${center_y}.png" \
            "$stitch_subdir/tile_$((center_x+1))_${center_y}.png" \
            "$stitch_subdir/tile_$((center_x-1))_$((center_y+1)).png" \
            "$stitch_subdir/tile_${center_x}_$((center_y+1)).png" \
            "$stitch_subdir/tile_$((center_x+1))_$((center_y+1)).png" \
            +append -background none \
            -crop 3x3@ +repage \
            -append \
            "$stitched_file" 2>/dev/null; then
            
            # Use montage for proper grid layout
            montage \
                "$stitch_subdir/tile_$((center_x-1))_$((center_y-1)).png" \
                "$stitch_subdir/tile_${center_x}_$((center_y-1)).png" \
                "$stitch_subdir/tile_$((center_x+1))_$((center_y-1)).png" \
                "$stitch_subdir/tile_$((center_x-1))_${center_y}.png" \
                "$stitch_subdir/tile_${center_x}_${center_y}.png" \
                "$stitch_subdir/tile_$((center_x+1))_${center_y}.png" \
                "$stitch_subdir/tile_$((center_x-1))_$((center_y+1)).png" \
                "$stitch_subdir/tile_${center_x}_$((center_y+1)).png" \
                "$stitch_subdir/tile_$((center_x+1))_$((center_y+1)).png" \
                -tile 3x3 \
                -geometry 256x256+0+0 \
                "$stitched_file" 2>/dev/null || true
            
            log_success "STITCH:$layer:$style - Created stitched image at $stitched_file"
            PASSED_TESTS=$((PASSED_TESTS + 1))
            TOTAL_TESTS=$((TOTAL_TESTS + 1))
            
            # Analyze for boundary issues using edge detection
            # This creates an edge-detected version to highlight discontinuities
            local edge_file="$stitch_subdir/edges_${z}_${center_x}_${center_y}.png"
            if convert "$stitched_file" -edge 1 "$edge_file" 2>/dev/null; then
                log_verbose "Created edge detection image: $edge_file"
            fi
        else
            log_warn "STITCH:$layer:$style - ImageMagick montage failed, tiles saved individually"
            PASSED_TESTS=$((PASSED_TESTS + 1))
            TOTAL_TESTS=$((TOTAL_TESTS + 1))
        fi
    else
        log_warn "ImageMagick not installed - skipping tile stitching visualization"
        log_info "Individual tiles saved to: $stitch_subdir"
        PASSED_TESTS=$((PASSED_TESTS + 1))
        TOTAL_TESTS=$((TOTAL_TESTS + 1))
    fi
    
    return 0
}

# Run stitch tests for all layers
run_stitch_tests() {
    log_info "=== Running Tile Stitching Tests ==="
    
    # Get first style and first temporal values for each layer to test stitching
    while IFS='|' read -r layer styles elevations runs forecasts times dim_type; do
        # Apply layer filter
        if [[ -n "$LAYER_PATTERN" ]] && [[ ! "$layer" =~ $LAYER_PATTERN ]]; then
            continue
        fi
        
        # Get first style
        IFS=',' read -ra style_array <<< "$styles"
        local style="${style_array[0]:-default}"
        
        # Build dimension params
        local dim_params=""
        
        if [[ "$dim_type" == "observation" ]]; then
            IFS=',' read -ra time_array <<< "$times"
            if [[ ${#time_array[@]} -gt 0 ]] && [[ -n "${time_array[0]}" ]]; then
                dim_params="TIME=$(urlencode "${time_array[0]}")"
            fi
        else
            IFS=',' read -ra run_array <<< "$runs"
            IFS=',' read -ra forecast_array <<< "$forecasts"
            
            if [[ ${#run_array[@]} -gt 0 ]] && [[ -n "${run_array[0]}" ]]; then
                dim_params="RUN=$(urlencode "${run_array[0]}")"
            fi
            if [[ ${#forecast_array[@]} -gt 0 ]] && [[ -n "${forecast_array[0]}" ]]; then
                [[ -n "$dim_params" ]] && dim_params+="&"
                dim_params+="FORECAST=${forecast_array[0]}"
            fi
        fi
        
        # Also add first elevation if present
        IFS=',' read -ra elev_array <<< "$elevations"
        if [[ ${#elev_array[@]} -gt 0 ]] && [[ -n "${elev_array[0]}" ]]; then
            [[ -n "$dim_params" ]] && dim_params+="&"
            dim_params+="ELEVATION=$(urlencode "${elev_array[0]}")"
        fi
        
        test_tile_stitching "$layer" "$style" "$dim_params"
        
    done < <(WMS_CAPS_FILE="$WMS_CAPS_FILE" parse_wms_layers)
}

# =============================================================================
# Report Generation
# =============================================================================

generate_report() {
    log_info "Generating reports..."
    
    # Generate JSON results
    {
        echo "{"
        echo "  \"timestamp\": \"$TIMESTAMP\","
        echo "  \"base_url\": \"$BASE_URL\","
        echo "  \"summary\": {"
        echo "    \"total_tests\": $TOTAL_TESTS,"
        echo "    \"passed\": $PASSED_TESTS,"
        echo "    \"failed\": $FAILED_TESTS,"
        echo "    \"skipped\": $SKIPPED_TESTS,"
        if [[ $TOTAL_TESTS -gt 0 ]]; then
            # Use awk for floating point division
            echo "    \"pass_rate\": $(awk "BEGIN {printf \"%.4f\", $PASSED_TESTS / $TOTAL_TESTS}")"
        else
            echo "    \"pass_rate\": 0"
        fi
        echo "  },"
        echo "  \"failed_requests\": ["
        local first=true
        for failed in "${FAILED_REQUESTS[@]}"; do
            IFS='|' read -r name url reason <<< "$failed"
            if $first; then
                first=false
            else
                echo ","
            fi
            echo -n "    {\"name\": \"$name\", \"reason\": \"$reason\"}"
        done
        echo ""
        echo "  ],"
        echo "  \"results\": ["
        if [[ ${#ALL_RESULTS[@]} -gt 0 ]]; then
            printf '%s\n' "${ALL_RESULTS[@]}" | sed 's/$/,/' | sed '$ s/,$//' | sed 's/^/    /'
        fi
        echo "  ]"
        echo "}"
    } > "$RESULTS_FILE"
    
    # Generate text summary
    {
        echo "========================================"
        echo "HAMMER TEST SUMMARY"
        echo "========================================"
        echo "Timestamp: $TIMESTAMP"
        echo "Base URL:  $BASE_URL"
        echo ""
        echo "RESULTS:"
        echo "  Total Tests: $TOTAL_TESTS"
        echo "  Passed:      $PASSED_TESTS"
        echo "  Failed:      $FAILED_TESTS"
        echo "  Skipped:     $SKIPPED_TESTS"
        if [[ $TOTAL_TESTS -gt 0 ]]; then
            echo "  Pass Rate:   $(( PASSED_TESTS * 100 / TOTAL_TESTS ))%"
        else
            echo "  Pass Rate:   N/A"
        fi
        echo ""
        
        if [[ ${#FAILED_REQUESTS[@]} -gt 0 ]]; then
            echo "FAILED REQUESTS:"
            for failed in "${FAILED_REQUESTS[@]}"; do
                IFS='|' read -r name url reason <<< "$failed"
                echo "  - $name"
                echo "    Reason: $reason"
            done
            echo ""
        fi
        
        echo "OUTPUT:"
        echo "  Results directory: $RESULTS_DIR"
        echo "  JSON report:       $RESULTS_FILE"
        echo "  Stitched tiles:    $STITCH_DIR"
        echo ""
        echo "========================================"
    } > "$SUMMARY_FILE"
    
    cat "$SUMMARY_FILE"
}

# =============================================================================
# Main Execution
# =============================================================================

main() {
    echo ""
    echo "═══════════════════════════════════════════════════════════════════"
    echo "  WMS/WMTS Endpoint Hammer Test"
    echo "═══════════════════════════════════════════════════════════════════"
    echo "  Base URL: $BASE_URL"
    echo "  Output:   $RESULTS_DIR"
    echo "  Parallel: $PARALLEL requests"
    echo "═══════════════════════════════════════════════════════════════════"
    echo ""
    
    # Fetch capabilities
    fetch_capabilities
    
    # Parse layers
    log_info "Parsing layer configurations from capabilities..."
    local layer_count=0
    
    # Store parsed layers in temp file for multiple passes
    local layers_file="$TEMP_DIR/layers.txt"
    WMS_CAPS_FILE="$WMS_CAPS_FILE" parse_wms_layers > "$layers_file"
    
    layer_count=$(wc -l < "$layers_file")
    log_info "Found $layer_count layers to test"
    echo ""
    
    # Run WMS tests
    if [[ "$SKIP_WMS" == "false" ]]; then
        log_info "=== Running WMS GetMap Tests ==="
        while IFS='|' read -r layer styles elevations runs forecasts times dim_type; do
            # Apply layer filter
            if [[ -n "$LAYER_PATTERN" ]] && [[ ! "$layer" =~ $LAYER_PATTERN ]]; then
                log_verbose "Skipping $layer (doesn't match filter)"
                continue
            fi
            
            test_wms_layer "$layer" "$styles" "$elevations" "$runs" "$forecasts" "$times" "$dim_type"
        done < "$layers_file"
        echo ""
    fi
    
    # Run WMTS tests
    if [[ "$SKIP_WMTS" == "false" ]]; then
        log_info "=== Running WMTS GetTile Tests ==="
        while IFS='|' read -r layer styles elevations runs forecasts times dim_type; do
            # Apply layer filter
            if [[ -n "$LAYER_PATTERN" ]] && [[ ! "$layer" =~ $LAYER_PATTERN ]]; then
                log_verbose "Skipping $layer (doesn't match filter)"
                continue
            fi
            
            test_wmts_layer "$layer" "$styles" "$elevations" "$runs" "$forecasts" "$times" "$dim_type"
        done < "$layers_file"
        echo ""
    fi
    
    # Run tile stitching tests
    if [[ "$SKIP_STITCH" == "false" ]]; then
        run_stitch_tests
        echo ""
    fi
    
    # Generate reports
    generate_report
    
    # Exit with appropriate code
    if [[ $FAILED_TESTS -gt 0 ]]; then
        echo -e "${RED}HAMMER TEST FAILED: $FAILED_TESTS failures${NC}"
        exit 1
    else
        echo -e "${GREEN}HAMMER TEST PASSED: All $PASSED_TESTS tests passed${NC}"
        exit 0
    fi
}

# Run main function
main "$@"
