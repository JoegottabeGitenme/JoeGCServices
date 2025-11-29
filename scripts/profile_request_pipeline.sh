#!/bin/bash
# Profile the complete WMS/WMTS request pipeline with detailed timing.
#
# This script:
# 1. Enables detailed tracing in the WMS API
# 2. Makes sample requests
# 3. Extracts and displays timing information from logs
#
# Usage:
#   ./scripts/profile_request_pipeline.sh              # Default test tiles
#   ./scripts/profile_request_pipeline.sh gradient     # Temperature gradient tiles
#   ./scripts/profile_request_pipeline.sh barbs        # Wind barb tiles
#   ./scripts/profile_request_pipeline.sh contour      # Contour tiles

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

MODE=${1:-"all"}
OUTPUT_DIR="$PROJECT_ROOT/profiling"
LOG_FILE="$OUTPUT_DIR/pipeline_$(date +%Y%m%d_%H%M%S).log"

mkdir -p "$OUTPUT_DIR"

echo "=== WMS Request Pipeline Profiling ==="
echo "Mode: $MODE"
echo "Log: $LOG_FILE"
echo ""

cleanup() {
    echo ""
    echo "Cleaning up..."
    [ -n "$WMS_PID" ] && kill "$WMS_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Build release binary
echo "Building WMS API..."
cargo build --release --package wms-api 2>/dev/null

# Start server with detailed tracing
echo "Starting WMS API with detailed tracing..."
export RUST_LOG="wms_api=debug,renderer=debug,storage=debug"
./target/release/wms-api > "$LOG_FILE" 2>&1 &
WMS_PID=$!
sleep 3

# Verify server is running
if ! kill -0 $WMS_PID 2>/dev/null; then
    echo "ERROR: WMS API failed to start"
    cat "$LOG_FILE"
    exit 1
fi

echo "Server started (PID: $WMS_PID)"
echo ""

# Define test requests based on mode
declare -a REQUESTS

case "$MODE" in
    gradient|temperature)
        REQUESTS=(
            "gfs_TMP|temperature|5|10|12|Temperature gradient tile"
            "gfs_TMP|temperature|6|20|25|Higher zoom temp tile"
            "gfs_TMP|temperature|7|40|50|Detailed temp tile"
            "hrrr_TMP|temperature|5|10|12|HRRR temperature"
        )
        ;;
    barbs|wind)
        REQUESTS=(
            "gfs_WIND_BARBS|wind_barbs|5|10|12|GFS wind barbs"
            "gfs_WIND_BARBS|wind_barbs|6|20|25|Higher zoom barbs"
            "hrrr_WIND_BARBS|wind_barbs|5|10|12|HRRR wind barbs"
        )
        ;;
    contour|isolines)
        REQUESTS=(
            "gfs_TMP|temperature_isolines|5|10|12|Temp contours"
            "gfs_PRMSL|isolines|5|10|12|Pressure contours"
        )
        ;;
    *)
        REQUESTS=(
            "gfs_TMP|temperature|5|10|12|Temperature gradient"
            "gfs_TMP|temperature|6|20|25|Higher zoom temp"
            "gfs_WIND_BARBS|wind_barbs|5|10|12|Wind barbs"
            "gfs_TMP|temperature_isolines|5|10|12|Temp isolines"
        )
        ;;
esac

echo "=== Making Test Requests ==="
echo ""

# Arrays to store timing results
declare -a RESULTS

for req in "${REQUESTS[@]}"; do
    IFS='|' read -r layer style z y x description <<< "$req"
    
    URL="http://localhost:8080/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0"
    URL="${URL}&LAYER=${layer}&STYLE=${style}&FORMAT=image/png"
    URL="${URL}&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=${z}&TILEROW=${y}&TILECOL=${x}"
    
    echo "Request: $description"
    echo "  Layer: $layer, Style: $style, Tile: z=$z, y=$y, x=$x"
    
    # Make request with timing
    START_TIME=$(date +%s%3N)
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$URL")
    END_TIME=$(date +%s%3N)
    TOTAL_MS=$((END_TIME - START_TIME))
    
    echo "  HTTP: $HTTP_CODE, Total: ${TOTAL_MS}ms"
    
    # Small delay to let logs flush
    sleep 0.5
    
    RESULTS+=("$description|$layer|$TOTAL_MS|$HTTP_CODE")
    echo ""
done

# Wait a moment for all logs to be written
sleep 1

echo ""
echo "=== Pipeline Timing Breakdown ==="
echo ""

# Extract timing information from logs
echo "Parsing log file..."
echo ""

# Look for timing spans in the logs
echo "--- Catalog Lookup Times ---"
grep -E "catalog_lookup|find_by_forecast" "$LOG_FILE" 2>/dev/null | tail -20 || echo "No catalog lookup logs found"
echo ""

echo "--- Data Loading Times ---"
grep -E "load_grid_data|grib_cache|storage_get" "$LOG_FILE" 2>/dev/null | tail -20 || echo "No data loading logs found"
echo ""

echo "--- Resampling Times ---"
grep -E "resample|bilinear" "$LOG_FILE" 2>/dev/null | tail -20 || echo "No resampling logs found"
echo ""

echo "--- Rendering Times ---"
grep -E "render_|apply_color|gradient|barb|contour" "$LOG_FILE" 2>/dev/null | tail -20 || echo "No rendering logs found"
echo ""

echo "--- PNG Encoding Times ---"
grep -E "png_encode|create_png|deflate" "$LOG_FILE" 2>/dev/null | tail -10 || echo "No PNG encoding logs found"
echo ""

echo "=== Request Summary ==="
echo ""
printf "%-30s %-20s %10s %8s\n" "Description" "Layer" "Time (ms)" "Status"
printf "%s\n" "--------------------------------------------------------------------------------"
for result in "${RESULTS[@]}"; do
    IFS='|' read -r desc layer time code <<< "$result"
    printf "%-30s %-20s %10s %8s\n" "$desc" "$layer" "$time" "$code"
done

echo ""
echo "=== Metrics Endpoint ==="
echo ""
echo "Current metrics from /api/metrics:"
curl -s "http://localhost:8080/api/metrics" 2>/dev/null | python3 -m json.tool 2>/dev/null || \
    curl -s "http://localhost:8080/api/metrics" || \
    echo "Could not fetch metrics"

echo ""
echo "=== Profiling Complete ==="
echo "Full log: $LOG_FILE"
echo ""
echo "Additional analysis:"
echo "  grep 'render_weather_data' $LOG_FILE    # Find render calls"
echo "  grep 'duration\|elapsed\|ms' $LOG_FILE  # Find timing info"
echo "  grep 'ERROR\|WARN' $LOG_FILE            # Find issues"
