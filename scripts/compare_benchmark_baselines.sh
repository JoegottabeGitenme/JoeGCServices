#!/bin/bash
# Compare Benchmark Baselines Script
#
# Compares two saved benchmark baselines and shows the differences.
#
# Usage:
#   ./scripts/compare_benchmark_baselines.sh <baseline1> <baseline2>
#   ./scripts/compare_benchmark_baselines.sh <baseline> --current
#
# Examples:
#   ./scripts/compare_benchmark_baselines.sh initial after-shm
#   ./scripts/compare_benchmark_baselines.sh initial --current

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BASELINES_DIR="$PROJECT_ROOT/benchmarks/baselines"
CRITERION_DIR="$PROJECT_ROOT/target/criterion"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Parse arguments
BASELINE1="${1:-}"
BASELINE2="${2:-}"

if [ -z "$BASELINE1" ] || [ -z "$BASELINE2" ]; then
    echo "Usage: $0 <baseline1> <baseline2>"
    echo "       $0 <baseline> --current"
    echo ""
    echo "Compares two benchmark baselines and shows performance differences."
    echo ""
    echo "Available baselines:"
    if [ -d "$BASELINES_DIR" ]; then
        ls -1 "$BASELINES_DIR" 2>/dev/null | while read -r name; do
            if [ -f "$BASELINES_DIR/$name/metadata.json" ]; then
                desc=$(jq -r '.description // "No description"' "$BASELINES_DIR/$name/metadata.json" 2>/dev/null)
                ts=$(jq -r '.timestamp // "Unknown"' "$BASELINES_DIR/$name/metadata.json" 2>/dev/null)
                echo "  $name - $desc ($ts)"
            fi
        done
    else
        echo "  (none)"
    fi
    exit 1
fi

# Handle --current flag
USE_CURRENT=0
if [ "$BASELINE2" = "--current" ]; then
    USE_CURRENT=1
    BASELINE2="(current)"
    if [ ! -d "$CRITERION_DIR" ]; then
        echo "Error: No current benchmark data found. Run benchmarks first."
        exit 1
    fi
fi

# Check if baseline1 exists
BASELINE1_DIR="$BASELINES_DIR/$BASELINE1"
if [ ! -d "$BASELINE1_DIR" ]; then
    echo "Error: Baseline '$BASELINE1' not found"
    exit 1
fi

# Check if baseline2 exists (if not using current)
if [ $USE_CURRENT -eq 0 ]; then
    BASELINE2_DIR="$BASELINES_DIR/$BASELINE2"
    if [ ! -d "$BASELINE2_DIR" ]; then
        echo "Error: Baseline '$BASELINE2' not found"
        exit 1
    fi
fi

echo "============================================================"
echo "  Benchmark Comparison: $BASELINE1 vs $BASELINE2"
echo "============================================================"
echo ""

# Show metadata
echo "Baseline 1: $BASELINE1"
if [ -f "$BASELINE1_DIR/metadata.json" ]; then
    jq -r '"  Timestamp: \(.timestamp)\n  Description: \(.description)\n  Git: \(.git_commit[0:8])"' "$BASELINE1_DIR/metadata.json" 2>/dev/null
fi
echo ""

echo "Baseline 2: $BASELINE2"
if [ $USE_CURRENT -eq 0 ] && [ -f "$BASELINE2_DIR/metadata.json" ]; then
    jq -r '"  Timestamp: \(.timestamp)\n  Description: \(.description)\n  Git: \(.git_commit[0:8])"' "$BASELINE2_DIR/metadata.json" 2>/dev/null
elif [ $USE_CURRENT -eq 1 ]; then
    echo "  (Current benchmark results in target/criterion/)"
fi
echo ""

echo "============================================================"
echo "  Performance Comparison"
echo "============================================================"
echo ""

# Function to get metric from a baseline
get_metric() {
    local baseline_dir="$1"
    local metric_path="$2"
    local file="$baseline_dir/data/$metric_path/new/estimates.json"
    
    if [ -f "$file" ]; then
        jq -r '.mean.point_estimate' "$file" 2>/dev/null
    else
        echo ""
    fi
}

# Function to get metric from current criterion data
get_current_metric() {
    local metric_path="$1"
    local file="$CRITERION_DIR/$metric_path/new/estimates.json"
    
    if [ -f "$file" ]; then
        jq -r '.mean.point_estimate' "$file" 2>/dev/null
    else
        echo ""
    fi
}

# Function to format time
format_time() {
    local ns="$1"
    if [ -z "$ns" ] || [ "$ns" = "null" ]; then
        echo "N/A"
        return
    fi
    
    if (( $(echo "$ns > 1000000" | bc -l 2>/dev/null || echo 0) )); then
        echo "$(echo "scale=2; $ns / 1000000" | bc) ms"
    elif (( $(echo "$ns > 1000" | bc -l 2>/dev/null || echo 0) )); then
        echo "$(echo "scale=2; $ns / 1000" | bc) µs"
    else
        echo "$ns ns"
    fi
}

# Function to calculate and display comparison
compare_metric() {
    local name="$1"
    local path="$2"
    
    local val1=$(get_metric "$BASELINE1_DIR" "$path")
    local val2
    if [ $USE_CURRENT -eq 1 ]; then
        val2=$(get_current_metric "$path")
    else
        val2=$(get_metric "$BASELINE2_DIR" "$path")
    fi
    
    if [ -z "$val1" ] && [ -z "$val2" ]; then
        return
    fi
    
    local time1=$(format_time "$val1")
    local time2=$(format_time "$val2")
    
    # Calculate percentage change
    local change=""
    local color="$NC"
    if [ -n "$val1" ] && [ -n "$val2" ] && [ "$val1" != "null" ] && [ "$val2" != "null" ]; then
        local pct=$(echo "scale=1; (($val2 - $val1) / $val1) * 100" | bc 2>/dev/null)
        if [ -n "$pct" ]; then
            if (( $(echo "$pct < -5" | bc -l 2>/dev/null || echo 0) )); then
                color="$GREEN"
                change="(${pct}% faster)"
            elif (( $(echo "$pct > 5" | bc -l 2>/dev/null || echo 0) )); then
                color="$RED"
                change="(+${pct}% slower)"
            else
                color="$YELLOW"
                change="(~${pct}%)"
            fi
        fi
    fi
    
    printf "%-40s %12s -> %12s ${color}%s${NC}\n" "$name" "$time1" "$time2" "$change"
}

echo "TEMP FILE I/O:"
compare_metric "  Write+Read+Delete (2.8MB)" "temp_file_io/system_temp_write_read_delete/2.8MB_typical"
compare_metric "  Write Only (2.8MB)" "temp_file_io/system_temp_write_only/2.8MB_typical"
compare_metric "  Memory Copy Baseline (2.8MB)" "temp_file_io/memory_copy_baseline/2.8MB_typical"
echo ""

echo "NETCDF I/O PATTERN:"
compare_metric "  With Sync" "netcdf_io_pattern/current_pattern_with_sync"
compare_metric "  No Sync" "netcdf_io_pattern/no_sync_pattern"
compare_metric "  3x Sequential" "netcdf_io_pattern/sequential_3x_operations"
echo ""

echo "PROJECTION TRANSFORMS:"
compare_metric "  geo_to_grid (65K)" "goes_projection/geo_to_grid/65536"
compare_metric "  geo_to_grid (262K)" "goes_projection/geo_to_grid/262144"
compare_metric "  geo_to_scan (65K)" "goes_projection/geo_to_scan/65536"
echo ""

echo "RESAMPLING (256x256 output):"
compare_metric "  Bilinear Only (CONUS)" "goes_resample/bilinear_only/full_conus_z4"
compare_metric "  With Projection (CONUS)" "goes_resample/with_projection/full_conus_z4"
compare_metric "  Bilinear Only (Central US)" "goes_resample/bilinear_only/central_us_z7"
compare_metric "  With Projection (Central US)" "goes_resample/with_projection/central_us_z7"
echo ""

echo "COLOR MAPPING:"
compare_metric "  IR Enhanced (256x256)" "goes_color/ir_enhanced/256x256"
compare_metric "  Visible Grayscale (256x256)" "goes_color/visible_grayscale/256x256"
compare_metric "  IR Enhanced (512x512)" "goes_color/ir_enhanced/512x512"
echo ""

echo "FULL PIPELINE:"
compare_metric "  IR Tile 256x256" "goes_pipeline/ir_tile_256x256"
compare_metric "  Visible Tile 256x256" "goes_pipeline/visible_tile_256x256"
compare_metric "  Resample Only" "goes_pipeline/resample_only_256x256"
compare_metric "  Color+PNG Only" "goes_pipeline/color_and_png_only_256x256"
echo ""

echo "PNG ENCODING:"
compare_metric "  256x256" "goes_png/encode/256x256"
compare_metric "  512x512" "goes_png/encode/512x512"
echo ""

echo "============================================================"
echo "Legend: ${GREEN}Green = faster${NC}, ${RED}Red = slower${NC}, ${YELLOW}Yellow = ~same${NC}"
echo "============================================================"
