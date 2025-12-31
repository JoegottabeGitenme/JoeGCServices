#!/bin/bash
#
# EDR API Load Test Script
#
# Dynamically discovers collections and parameters from the EDR API,
# then runs load tests with realistic request patterns.
#
# Usage:
#   ./scripts/run_edr_load_test.sh [options]
#
# Options:
#   --url URL         EDR API base URL (default: http://localhost:8083/edr)
#   --duration SECS   Test duration in seconds (default: 60)
#   --concurrency N   Number of concurrent requests (default: 10)
#   --query-type TYPE Only run position, area, radius, trajectory, corridor, or all (default: all)
#   --output DIR      Output directory for results (default: ./edr_load_results)
#   --validate        Validate discovered endpoints before load testing
#   --verbose         Show detailed output
#   --help            Show this help message

set -euo pipefail

# =============================================================================
# Configuration
# =============================================================================

BASE_URL="${EDR_BASE_URL:-http://localhost:8083/edr}"
DURATION=60
CONCURRENCY=10
QUERY_TYPE="all"  # position, area, radius, trajectory, corridor, or all
OUTPUT_DIR="./edr_load_results"
VALIDATE=false
VERBOSE=false
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Colors
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' BLUE='' NC=''
fi

# =============================================================================
# Parse Arguments
# =============================================================================

show_help() {
    sed -n '2,18p' "$0" | sed 's/^# //' | sed 's/^#//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --url) BASE_URL="$2"; shift 2 ;;
        --duration) DURATION="$2"; shift 2 ;;
        --concurrency) CONCURRENCY="$2"; shift 2 ;;
        --query-type) QUERY_TYPE="$2"; shift 2 ;;
        --output) OUTPUT_DIR="$2"; shift 2 ;;
        --validate) VALIDATE=true; shift ;;
        --verbose|-v) VERBOSE=true; shift ;;
        --help|-h) show_help ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# =============================================================================
# Setup
# =============================================================================

RESULTS_DIR="$OUTPUT_DIR/$TIMESTAMP"
mkdir -p "$RESULTS_DIR"

# Stats tracking - use files for cross-process communication
STATS_DIR=$(mktemp -d)
echo "0" > "$STATS_DIR/total"
echo "0" > "$STATS_DIR/success"
echo "0" > "$STATS_DIR/fail"
: > "$STATS_DIR/latencies"

# Discovered test scenarios (populated by discover_endpoints)
SCENARIOS_FILE="$STATS_DIR/scenarios.txt"
: > "$SCENARIOS_FILE"

cleanup_stats() {
    rm -rf "$STATS_DIR"
}
trap cleanup_stats EXIT

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; }
log_verbose() { [[ "$VERBOSE" == "true" ]] && echo -e "[DEBUG] $1" || true; }

# URL encode
urlencode() {
    python3 -c "import urllib.parse; print(urllib.parse.quote('$1', safe=''))"
}

# =============================================================================
# Discovery Functions
# =============================================================================

# Discover all collections and their queryable parameters
discover_endpoints() {
    log_info "Discovering EDR collections and parameters..."
    
    # Fetch collections list to a temp file
    local collections_file="$STATS_DIR/collections.json"
    curl -sf "$BASE_URL/collections" > "$collections_file" 2>/dev/null || {
        log_fail "Failed to fetch collections from $BASE_URL/collections"
        exit 1
    }
    
    # Parse collections and fetch details for each
    python3 << PYTHON
import json
import urllib.request
import sys

base_url = "$BASE_URL"

with open("$collections_file") as f:
    data = json.load(f)

collections = data.get('collections', [])
scenarios = []
total_params = 0

for coll in collections:
    coll_id = coll.get('id', '')
    if not coll_id:
        continue
    
    # Fetch full collection details to get parameters
    try:
        with urllib.request.urlopen(f"{base_url}/collections/{coll_id}") as resp:
            coll_detail = json.load(resp)
    except Exception as e:
        print(f"  Warning: Failed to fetch {coll_id}: {e}", file=sys.stderr)
        continue
    
    # Get bbox for random point generation
    bbox = coll_detail.get('extent', {}).get('spatial', {}).get('bbox', [[-180, -90, 180, 90]])[0]
    min_lon, min_lat, max_lon, max_lat = bbox[0], bbox[1], bbox[2], bbox[3]
    
    # Get parameters from detailed collection response
    params = list(coll_detail.get('parameter_names', {}).keys())
    if not params:
        continue
    
    total_params += len(params)
    
    # Check if collection has vertical levels
    vertical = coll_detail.get('extent', {}).get('vertical', {})
    levels = []
    if vertical:
        intervals = vertical.get('interval', [])
        for interval in intervals:
            if interval and interval[0] is not None:
                levels.append(str(int(interval[0])))
    
    # Get supported query types from data_queries
    data_queries = coll_detail.get('data_queries', {})
    supports_position = 'position' in data_queries
    supports_area = 'area' in data_queries
    supports_radius = 'radius' in data_queries
    supports_trajectory = 'trajectory' in data_queries
    supports_corridor = 'corridor' in data_queries
    
    # Generate scenarios for each parameter
    for param in params:
        level = levels[0] if levels else ""
        
        # Position query scenario
        if supports_position:
            scenarios.append(f"position|{coll_id}|{param}|{level}|{min_lon}|{max_lon}|{min_lat}|{max_lat}")
        
        # Area query scenario
        if supports_area:
            scenarios.append(f"area|{coll_id}|{param}|{level}|{min_lon}|{max_lon}|{min_lat}|{max_lat}")
        
        # Radius query scenario
        if supports_radius:
            scenarios.append(f"radius|{coll_id}|{param}|{level}|{min_lon}|{max_lon}|{min_lat}|{max_lat}")
        
        # Trajectory query scenario
        if supports_trajectory:
            scenarios.append(f"trajectory|{coll_id}|{param}|{level}|{min_lon}|{max_lon}|{min_lat}|{max_lat}")
        
        # Corridor query scenario
        if supports_corridor:
            scenarios.append(f"corridor|{coll_id}|{param}|{level}|{min_lon}|{max_lon}|{min_lat}|{max_lat}")

# Write scenarios to file
with open("$SCENARIOS_FILE", 'w') as f:
    for s in scenarios:
        f.write(s + '\n')

# Count scenarios by query type
position_count = sum(1 for s in scenarios if s.startswith('position|'))
area_count = sum(1 for s in scenarios if s.startswith('area|'))
radius_count = sum(1 for s in scenarios if s.startswith('radius|'))
trajectory_count = sum(1 for s in scenarios if s.startswith('trajectory|'))
corridor_count = sum(1 for s in scenarios if s.startswith('corridor|'))

print(f"Discovered {len(collections)} collections, {total_params} parameters, {len(scenarios)} test scenarios")
print(f"  Query types: {position_count} position, {area_count} area, {radius_count} radius, {trajectory_count} trajectory, {corridor_count} corridor")
PYTHON
    
    local scenario_count=$(wc -l < "$SCENARIOS_FILE")
    if [[ $scenario_count -eq 0 ]]; then
        log_fail "No test scenarios discovered"
        exit 1
    fi
    
    log_info "  Test scenarios: $scenario_count"
}

# Validate discovered endpoints by making test requests
validate_endpoints() {
    log_info "Validating discovered endpoints..."
    
    local valid_scenarios="$STATS_DIR/valid_scenarios.txt"
    : > "$valid_scenarios"
    
    local total=0
    local valid=0
    local invalid=0
    
    while IFS='|' read -r query_type coll_id param level min_lon max_lon min_lat max_lat; do
        total=$((total + 1))
        
        # Generate a test point in the center of bbox
        local center_lon=$(python3 -c "print(($min_lon + $max_lon) / 2)")
        local center_lat=$(python3 -c "print(($min_lat + $max_lat) / 2)")
        
        local url
        if [[ "$query_type" == "position" ]]; then
            url="$BASE_URL/collections/$coll_id/position?coords=$(urlencode "POINT($center_lon $center_lat)")&parameter-name=$param"
            [[ -n "$level" ]] && url+="&z=$level"
        elif [[ "$query_type" == "area" ]]; then
            # Small test polygon
            local half=0.5
            local p_min_lon=$(python3 -c "print($center_lon - $half)")
            local p_max_lon=$(python3 -c "print($center_lon + $half)")
            local p_min_lat=$(python3 -c "print($center_lat - $half)")
            local p_max_lat=$(python3 -c "print($center_lat + $half)")
            local polygon="POLYGON(($p_min_lon $p_min_lat,$p_max_lon $p_min_lat,$p_max_lon $p_max_lat,$p_min_lon $p_max_lat,$p_min_lon $p_min_lat))"
            url="$BASE_URL/collections/$coll_id/area?coords=$(urlencode "$polygon")&parameter-name=$param"
            [[ -n "$level" ]] && url+="&z=$level"
        elif [[ "$query_type" == "radius" ]]; then
            # Radius query with 50km radius
            url="$BASE_URL/collections/$coll_id/radius?coords=$(urlencode "POINT($center_lon $center_lat)")&within=50&within-units=km&parameter-name=$param"
            [[ -n "$level" ]] && url+="&z=$level"
        elif [[ "$query_type" == "trajectory" ]]; then
            # Small test linestring for trajectory
            local half=0.5
            local lon1=$(python3 -c "print($center_lon - $half)")
            local lon2=$(python3 -c "print($center_lon + $half)")
            local linestring="LINESTRING($lon1 $center_lat,$lon2 $center_lat)"
            url="$BASE_URL/collections/$coll_id/trajectory?coords=$(urlencode "$linestring")&parameter-name=$param"
            [[ -n "$level" ]] && url+="&z=$level"
        elif [[ "$query_type" == "corridor" ]]; then
            # Small test linestring for corridor
            local half=0.5
            local lon1=$(python3 -c "print($center_lon - $half)")
            local lon2=$(python3 -c "print($center_lon + $half)")
            local linestring="LINESTRING($lon1 $center_lat,$lon2 $center_lat)"
            url="$BASE_URL/collections/$coll_id/corridor?coords=$(urlencode "$linestring")&parameter-name=$param&corridor-width=10&width-units=km&corridor-height=1000&height-units=m"
            [[ -n "$level" ]] && url+="&z=$level"
        fi
        
        local http_code
        http_code=$(curl -sf -w "%{http_code}" -o /dev/null --max-time 10 "$url" 2>/dev/null) || http_code="000"
        
        if [[ "$http_code" == "200" ]]; then
            valid=$((valid + 1))
            echo "$query_type|$coll_id|$param|$level|$min_lon|$max_lon|$min_lat|$max_lat" >> "$valid_scenarios"
            log_verbose "  OK: $query_type $coll_id/$param"
        else
            invalid=$((invalid + 1))
            log_verbose "  SKIP: $query_type $coll_id/$param (HTTP $http_code)"
        fi
        
        # Progress
        printf "\r  Validating: %d/%d (valid: %d, skipped: %d)" "$total" "$(wc -l < "$SCENARIOS_FILE")" "$valid" "$invalid"
        
    done < "$SCENARIOS_FILE"
    
    echo ""
    
    # Replace scenarios with validated ones
    mv "$valid_scenarios" "$SCENARIOS_FILE"
    
    log_info "  Valid scenarios: $valid"
    log_info "  Skipped (no data): $invalid"
    
    if [[ $valid -eq 0 ]]; then
        log_fail "No valid test scenarios found"
        exit 1
    fi
}

# =============================================================================
# Test Functions
# =============================================================================

# Make an EDR request and track results
make_request() {
    local name="$1"
    local url="$2"
    local expected_status="${3:-200}"
    
    local start_time=$(date +%s%N)
    local http_code
    local req_id=$(date +%s%N)
    local response_file="$RESULTS_DIR/response_${req_id}.json"
    
    http_code=$(curl -sf -w "%{http_code}" -o "$response_file" --max-time 30 "$url" 2>/dev/null) || http_code="000"
    
    local end_time=$(date +%s%N)
    local duration_ms=$(( (end_time - start_time) / 1000000 ))
    
    # Atomic increment and append using flock
    (
        flock 200
        echo "$duration_ms" >> "$STATS_DIR/latencies"
        echo $(( $(cat "$STATS_DIR/total") + 1 )) > "$STATS_DIR/total"
    ) 200>"$STATS_DIR/lock"
    
    if [[ "$http_code" == "$expected_status" ]]; then
        (
            flock 200
            echo $(( $(cat "$STATS_DIR/success") + 1 )) > "$STATS_DIR/success"
        ) 200>"$STATS_DIR/lock"
        log_verbose "$name: ${duration_ms}ms (HTTP $http_code)"
        rm -f "$response_file"
    else
        (
            flock 200
            echo $(( $(cat "$STATS_DIR/fail") + 1 )) > "$STATS_DIR/fail"
        ) 200>"$STATS_DIR/lock"
        log_fail "$name: HTTP $http_code (expected $expected_status) - ${duration_ms}ms"
    fi
}

# Generate random point within bbox
random_point() {
    local min_lon=$1 max_lon=$2 min_lat=$3 max_lat=$4
    python3 -c "
import random
lon = random.uniform($min_lon, $max_lon)
lat = random.uniform($min_lat, $max_lat)
print(f'POINT({lon:.4f} {lat:.4f})')
"
}

# Generate random polygon within bbox
random_polygon() {
    local min_lon=$1 max_lon=$2 min_lat=$3 max_lat=$4 size=${5:-1.0}
    python3 -c "
import random
size = $size
# Ensure we have room for the polygon
effective_min_lon = $min_lon + size/2
effective_max_lon = $max_lon - size/2
effective_min_lat = $min_lat + size/2
effective_max_lat = $max_lat - size/2

# Handle case where bbox is smaller than polygon size
if effective_min_lon >= effective_max_lon:
    effective_min_lon = $min_lon
    effective_max_lon = $max_lon
    size = min(size, $max_lon - $min_lon)
if effective_min_lat >= effective_max_lat:
    effective_min_lat = $min_lat
    effective_max_lat = $max_lat
    size = min(size, $max_lat - $min_lat)

center_lon = random.uniform(effective_min_lon, effective_max_lon)
center_lat = random.uniform(effective_min_lat, effective_max_lat)
half = size / 2

min_x = center_lon - half
max_x = center_lon + half
min_y = center_lat - half
max_y = center_lat + half

print(f'POLYGON(({min_x:.4f} {min_y:.4f},{max_x:.4f} {min_y:.4f},{max_x:.4f} {max_y:.4f},{min_x:.4f} {max_y:.4f},{min_x:.4f} {min_y:.4f}))')
"
}

# Generate random linestring within bbox (for corridor queries)
random_linestring() {
    local min_lon=$1 max_lon=$2 min_lat=$3 max_lat=$4 length=${5:-2.0} points=${6:-3}
    python3 -c "
import random
import math

length = $length
num_points = $points

# Ensure we have room for the linestring
effective_min_lon = $min_lon + length/2
effective_max_lon = $max_lon - length/2
effective_min_lat = $min_lat + length/2
effective_max_lat = $max_lat - length/2

# Handle case where bbox is smaller than line length
if effective_min_lon >= effective_max_lon:
    effective_min_lon = $min_lon
    effective_max_lon = $max_lon
    length = min(length, ($max_lon - $min_lon) * 0.8)
if effective_min_lat >= effective_max_lat:
    effective_min_lat = $min_lat
    effective_max_lat = $max_lat
    length = min(length, ($max_lat - $min_lat) * 0.8)

# Start point
start_lon = random.uniform(effective_min_lon, effective_max_lon)
start_lat = random.uniform(effective_min_lat, effective_max_lat)

# Random direction (angle in radians)
angle = random.uniform(0, 2 * math.pi)

# Generate points along a somewhat curved path
coords = []
for i in range(num_points):
    t = i / (num_points - 1) if num_points > 1 else 0
    # Add some curvature
    curve_offset = math.sin(t * math.pi) * (length * 0.2)
    perp_angle = angle + math.pi / 2
    
    lon = start_lon + t * length * math.cos(angle) + curve_offset * math.cos(perp_angle)
    lat = start_lat + t * length * math.sin(angle) + curve_offset * math.sin(perp_angle)
    
    # Clamp to bbox
    lon = max($min_lon, min($max_lon, lon))
    lat = max($min_lat, min($max_lat, lat))
    
    coords.append(f'{lon:.4f} {lat:.4f}')

print(f'LINESTRING({chr(44).join(coords)})')
"
}

# Execute a single test scenario
run_scenario() {
    local scenario="$1"
    
    IFS='|' read -r query_type coll_id param level min_lon max_lon min_lat max_lat <<< "$scenario"
    
    local url
    if [[ "$query_type" == "position" ]]; then
        local point=$(random_point "$min_lon" "$max_lon" "$min_lat" "$max_lat")
        url="$BASE_URL/collections/$coll_id/position?coords=$(urlencode "$point")&parameter-name=$param"
        [[ -n "$level" ]] && url+="&z=$level"
    elif [[ "$query_type" == "area" ]]; then
        local polygon=$(random_polygon "$min_lon" "$max_lon" "$min_lat" "$max_lat" 1.0)
        url="$BASE_URL/collections/$coll_id/area?coords=$(urlencode "$polygon")&parameter-name=$param"
        [[ -n "$level" ]] && url+="&z=$level"
    elif [[ "$query_type" == "radius" ]]; then
        local point=$(random_point "$min_lon" "$max_lon" "$min_lat" "$max_lat")
        # Random radius between 25-100km
        local radius=$(python3 -c "import random; print(random.randint(25, 100))")
        url="$BASE_URL/collections/$coll_id/radius?coords=$(urlencode "$point")&within=$radius&within-units=km&parameter-name=$param"
        [[ -n "$level" ]] && url+="&z=$level"
    elif [[ "$query_type" == "trajectory" ]]; then
        local linestring=$(random_linestring "$min_lon" "$max_lon" "$min_lat" "$max_lat" 2.0 3)
        url="$BASE_URL/collections/$coll_id/trajectory?coords=$(urlencode "$linestring")&parameter-name=$param"
        [[ -n "$level" ]] && url+="&z=$level"
    elif [[ "$query_type" == "corridor" ]]; then
        local linestring=$(random_linestring "$min_lon" "$max_lon" "$min_lat" "$max_lat" 2.0 3)
        url="$BASE_URL/collections/$coll_id/corridor?coords=$(urlencode "$linestring")&parameter-name=$param&corridor-width=10&width-units=km&corridor-height=1000&height-units=m"
        [[ -n "$level" ]] && url+="&z=$level"
    fi
    
    make_request "$query_type:$coll_id:$param" "$url"
}

# =============================================================================
# Main Test Loop
# =============================================================================

run_tests() {
    log_info "Starting EDR Load Test"
    log_info "  URL: $BASE_URL"
    log_info "  Duration: ${DURATION}s"
    log_info "  Concurrency: $CONCURRENCY"
    log_info "  Query Type: $QUERY_TYPE"
    
    # Load scenarios into array
    local -a scenarios
    mapfile -t scenarios < "$SCENARIOS_FILE"
    local scenario_count=${#scenarios[@]}
    
    # Count and display query types breakdown
    local position_count=0
    local area_count=0
    local radius_count=0
    local trajectory_count=0
    local corridor_count=0
    for s in "${scenarios[@]}"; do
        if [[ "$s" == "position|"* ]]; then
            position_count=$((position_count + 1))
        elif [[ "$s" == "area|"* ]]; then
            area_count=$((area_count + 1))
        elif [[ "$s" == "radius|"* ]]; then
            radius_count=$((radius_count + 1))
        elif [[ "$s" == "trajectory|"* ]]; then
            trajectory_count=$((trajectory_count + 1))
        elif [[ "$s" == "corridor|"* ]]; then
            corridor_count=$((corridor_count + 1))
        fi
    done
    log_info "  Scenarios: $scenario_count total ($position_count position, $area_count area, $radius_count radius, $trajectory_count trajectory, $corridor_count corridor)"
    echo ""
    
    # Filter by query type if specified
    if [[ "$QUERY_TYPE" != "all" ]]; then
        local -a filtered_scenarios
        for s in "${scenarios[@]}"; do
            if [[ "$s" == "$QUERY_TYPE|"* ]]; then
                filtered_scenarios+=("$s")
            fi
        done
        scenarios=("${filtered_scenarios[@]}")
        scenario_count=${#scenarios[@]}
        log_info "  Filtered to $scenario_count $QUERY_TYPE scenarios"
    fi
    
    if [[ $scenario_count -eq 0 ]]; then
        log_fail "No scenarios available for query type: $QUERY_TYPE"
        exit 1
    fi
    
    local end_time=$(($(date +%s) + DURATION))
    
    while [[ $(date +%s) -lt $end_time ]]; do
        # Run concurrent batch
        for ((i=0; i<CONCURRENCY; i++)); do
            {
                # Pick random scenario
                local idx=$((RANDOM % scenario_count))
                run_scenario "${scenarios[$idx]}"
            } &
        done
        
        # Wait for batch to complete
        wait
        
        # Progress update - read from stats files
        local elapsed=$(($(date +%s) - end_time + DURATION))
        local total=$(cat "$STATS_DIR/total" 2>/dev/null || echo 0)
        local success=$(cat "$STATS_DIR/success" 2>/dev/null || echo 0)
        local fail=$(cat "$STATS_DIR/fail" 2>/dev/null || echo 0)
        printf "\r  Progress: %ds / %ds - Requests: %d (ok: %d, fail: %d)" \
            "$elapsed" "$DURATION" "$total" "$success" "$fail"
    done
    
    echo ""
}

# =============================================================================
# Results Summary
# =============================================================================

generate_summary() {
    echo ""
    log_info "=== EDR Load Test Results ==="
    echo ""
    
    # Read final stats from files
    local TOTAL_REQUESTS=$(cat "$STATS_DIR/total" 2>/dev/null || echo 0)
    local SUCCESSFUL_REQUESTS=$(cat "$STATS_DIR/success" 2>/dev/null || echo 0)
    local FAILED_REQUESTS=$(cat "$STATS_DIR/fail" 2>/dev/null || echo 0)
    
    local success_rate=0
    if [[ $TOTAL_REQUESTS -gt 0 ]]; then
        success_rate=$(python3 -c "print(f'{$SUCCESSFUL_REQUESTS / $TOTAL_REQUESTS * 100:.1f}')")
    fi
    
    # Calculate latency stats from file
    local latency_stats
    if [[ -s "$STATS_DIR/latencies" ]]; then
        latency_stats=$(python3 << PYTHON
import statistics
with open("$STATS_DIR/latencies") as f:
    latencies = [int(line.strip()) for line in f if line.strip()]
if latencies:
    print(f"min={min(latencies)}ms, avg={statistics.mean(latencies):.0f}ms, "
          f"p50={statistics.median(latencies):.0f}ms, p95={sorted(latencies)[int(len(latencies)*0.95)]:.0f}ms, "
          f"max={max(latencies)}ms")
else:
    print("N/A")
PYTHON
)
    else
        latency_stats="N/A"
    fi
    
    echo "  Total Requests:    $TOTAL_REQUESTS"
    echo "  Successful:        $SUCCESSFUL_REQUESTS"
    echo "  Failed:            $FAILED_REQUESTS"
    echo "  Success Rate:      ${success_rate}%"
    echo "  Latency:           $latency_stats"
    echo "  Throughput:        $(python3 -c "print(f'{$TOTAL_REQUESTS / $DURATION:.1f}')") req/s"
    echo ""
    
    # Save results to JSON
    cat > "$RESULTS_DIR/summary.json" << EOF
{
  "timestamp": "$TIMESTAMP",
  "config": {
    "base_url": "$BASE_URL",
    "duration_secs": $DURATION,
    "concurrency": $CONCURRENCY,
    "query_type": "$QUERY_TYPE"
  },
  "results": {
    "total_requests": $TOTAL_REQUESTS,
    "successful": $SUCCESSFUL_REQUESTS,
    "failed": $FAILED_REQUESTS,
    "success_rate": $success_rate,
    "throughput_rps": $(python3 -c "print(f'{$TOTAL_REQUESTS / $DURATION:.2f}')")
  }
}
EOF
    
    log_info "Results saved to: $RESULTS_DIR/summary.json"
    
    if [[ $FAILED_REQUESTS -gt 0 ]]; then
        echo -e "${RED}LOAD TEST COMPLETED WITH FAILURES${NC}"
        exit 1
    else
        echo -e "${GREEN}LOAD TEST COMPLETED SUCCESSFULLY${NC}"
        exit 0
    fi
}

# =============================================================================
# Main
# =============================================================================

# Check dependencies
command -v curl &>/dev/null || { log_fail "curl is required"; exit 1; }
command -v python3 &>/dev/null || { log_fail "python3 is required"; exit 1; }

# Discover endpoints
discover_endpoints

# Optionally validate (or always validate to filter out params without data)
if [[ "$VALIDATE" == "true" ]] || [[ "$DURATION" -gt 10 ]]; then
    # For longer tests, always validate to avoid wasting time on bad endpoints
    validate_endpoints
fi

# Run load test
run_tests
generate_summary
