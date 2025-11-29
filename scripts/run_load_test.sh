#!/bin/bash
#
# Run load tests against the Weather WMS/WMTS service
#
# This script provides a convenient wrapper around the load-test tool
# with common scenarios and output formatting.
#
# Usage:
#   ./run_load_test.sh                    # Run quick smoke test
#   ./run_load_test.sh cold_cache         # Run cold cache test
#   ./run_load_test.sh --scenario <file>  # Run custom scenario
#   ./run_load_test.sh --help             # Show help

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Script directory and project root
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

# Default values
SCENARIO="quick"
OUTPUT_FORMAT="table"
RESET_CACHE=false
SAVE_RESULTS=false
RESULTS_DIR="$PROJECT_ROOT/validation/load-test/results"

# Show help
show_help() {
    cat << EOF
Usage: $0 [OPTIONS] [SCENARIO]

Run load tests against the Weather WMS/WMTS service.

SCENARIOS:
  quick               Quick smoke test (10s, 1 concurrent) - DEFAULT
  cold_cache          Test cache-miss performance (60s, 10 concurrent)
  warm_cache          Test cache-hit performance (60s, 10 concurrent)
  stress              High concurrency stress test (60s, 200 concurrent)
  layer_comparison    Compare different layer types (120s, 20 concurrent)
  zoom_sweep          Test all zoom levels (custom scenario)

OPTIONS:
  -s, --scenario FILE     Use custom scenario YAML file
  -o, --output FORMAT     Output format: table, json, csv (default: table)
  -r, --reset-cache       Reset Redis cache before test
  -S, --save              Save results to results/ directory
  -h, --help              Show this help message

EXAMPLES:
  # Quick smoke test
  $0

  # Cold cache test with results saved
  $0 cold_cache --save

  # Custom scenario with JSON output
  $0 --scenario my_test.yaml --output json

  # Stress test with cache reset
  $0 stress --reset-cache

NOTES:
  - Ensure services are running: ./scripts/start.sh
  - Results are saved to: validation/load-test/results/
  - CSV output can be appended to track performance over time

EOF
}

# Parse arguments
POSITIONAL_ARGS=()
while [[ $# -gt 0 ]]; do
    case $1 in
        -s|--scenario)
            CUSTOM_SCENARIO="$2"
            shift 2
            ;;
        -o|--output)
            OUTPUT_FORMAT="$2"
            shift 2
            ;;
        -r|--reset-cache)
            RESET_CACHE=true
            shift
            ;;
        -S|--save)
            SAVE_RESULTS=true
            shift
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        -*|--*)
            log_error "Unknown option: $1"
            show_help
            exit 1
            ;;
        *)
            POSITIONAL_ARGS+=("$1")
            shift
            ;;
    esac
done

# Restore positional parameters
set -- "${POSITIONAL_ARGS[@]}"

# Get scenario name from first positional arg if provided
if [ $# -gt 0 ]; then
    SCENARIO="$1"
fi

# Determine scenario file path
if [ -n "$CUSTOM_SCENARIO" ]; then
    SCENARIO_FILE="$CUSTOM_SCENARIO"
elif [ -f "$PROJECT_ROOT/validation/load-test/scenarios/${SCENARIO}.yaml" ]; then
    SCENARIO_FILE="$PROJECT_ROOT/validation/load-test/scenarios/${SCENARIO}.yaml"
else
    log_error "Scenario not found: $SCENARIO"
    log_info "Available scenarios:"
    ls -1 "$PROJECT_ROOT/validation/load-test/scenarios/"*.yaml 2>/dev/null | xargs -n1 basename | sed 's/.yaml$//' | sed 's/^/  - /'
    exit 1
fi

echo ""
log_info "=========================================="
log_info "Weather WMS Load Test"
log_info "=========================================="
log_info "Scenario:      $(basename $SCENARIO_FILE .yaml)"
log_info "Output format: $OUTPUT_FORMAT"
log_info "Reset cache:   $RESET_CACHE"
log_info "Save results:  $SAVE_RESULTS"
log_info "=========================================="
echo ""

# Check if services are running
log_info "Checking if WMS service is running..."
if ! curl -s -f "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities" > /dev/null 2>&1; then
    log_error "WMS service is not responding at http://localhost:8080"
    log_info "Start services with: ./scripts/start.sh"
    exit 1
fi
log_success "WMS service is running"
echo ""

# Reset cache if requested
if [ "$RESET_CACHE" = true ]; then
    log_info "Resetting cache..."
    if [ -f "$SCRIPT_DIR/reset_test_state.sh" ]; then
        bash "$SCRIPT_DIR/reset_test_state.sh"
    else
        log_warn "reset_test_state.sh not found, skipping cache reset"
    fi
    echo ""
fi

# Create results directory if saving
if [ "$SAVE_RESULTS" = true ]; then
    mkdir -p "$RESULTS_DIR"
fi

# Build load-test binary if needed
log_info "Building load-test tool..."
cd "$PROJECT_ROOT"
if cargo build --package load-test --release 2>&1 | grep -q "Finished"; then
    log_success "Build complete"
else
    log_error "Build failed"
    exit 1
fi
echo ""

# Run the load test
log_info "Starting load test..."
echo ""

LOAD_TEST_BIN="$PROJECT_ROOT/target/release/load-test"

# Prepare output redirection based on format
if [ "$SAVE_RESULTS" = true ]; then
    TIMESTAMP=$(date +%Y%m%d_%H%M%S)
    
    # Always save JSON to JSONL file for dashboard
    JSONL_FILE="$RESULTS_DIR/runs.jsonl"
    
    case "$OUTPUT_FORMAT" in
        json)
            OUTPUT_FILE="$RESULTS_DIR/${SCENARIO}_${TIMESTAMP}.json"
            log_info "Saving results to: $OUTPUT_FILE"
            RESULT=$("$LOAD_TEST_BIN" run --scenario "$SCENARIO_FILE" --output json 2>&1)
            echo "$RESULT" | tee "$OUTPUT_FILE"
            # Extract just the JSON and append to JSONL for dashboard (compact JSON, one line per record)
            # The JSON starts with { and we want everything from the first { to the last }
            echo "$RESULT" | sed -n '/^{/,/^}/p' | jq -c '.' >> "$JSONL_FILE" 2>/dev/null || true
            ;;
        csv)
            OUTPUT_FILE="$RESULTS_DIR/${SCENARIO}.csv"
            # Append to CSV file (create with header if doesn't exist)
            if [ ! -f "$OUTPUT_FILE" ]; then
                echo "timestamp,config,duration,requests,rps,p50,p90,p99,cache_hit_rate" > "$OUTPUT_FILE"
            fi
            log_info "Appending results to: $OUTPUT_FILE"
            RESULT=$("$LOAD_TEST_BIN" run --scenario "$SCENARIO_FILE" --output json 2>&1)
            # Extract just the JSON part
            JSON_ONLY=$(echo "$RESULT" | sed -n '/^{/,/^}/p')
            # Save JSON to JSONL for dashboard (compact JSON, one line per record)
            echo "$JSON_ONLY" | jq -c '.' >> "$JSONL_FILE" 2>/dev/null || true
            # Extract CSV line and append
            echo "$JSON_ONLY" | jq -r '[.timestamp, .scenario_name, .duration_secs, .total_requests, .requests_per_second, .latency_p50, .latency_p90, .latency_p99, .cache_hit_rate] | @csv' >> "$OUTPUT_FILE"
            # Show table output to console
            "$LOAD_TEST_BIN" run --scenario "$SCENARIO_FILE" --output table
            ;;
        *)
            OUTPUT_FILE="$RESULTS_DIR/${SCENARIO}_${TIMESTAMP}.txt"
            log_info "Saving results to: $OUTPUT_FILE"
            RESULT=$("$LOAD_TEST_BIN" run --scenario "$SCENARIO_FILE" --output table | tee "$OUTPUT_FILE")
            # Also save JSON to JSONL for dashboard (compact JSON, one line per record)
            JSON_RESULT=$("$LOAD_TEST_BIN" run --scenario "$SCENARIO_FILE" --output json 2>&1)
            echo "$JSON_RESULT" | sed -n '/^{/,/^}/p' | jq -c '.' >> "$JSONL_FILE" 2>/dev/null || true
            echo "$RESULT"
            ;;
    esac
else
    # Just run without saving
    "$LOAD_TEST_BIN" run --scenario "$SCENARIO_FILE" --output "$OUTPUT_FORMAT"
fi

echo ""
log_success "=========================================="
log_success "Load test complete!"
log_success "=========================================="
echo ""

# Show results summary if saved
if [ "$SAVE_RESULTS" = true ]; then
    log_info "Results saved to: $OUTPUT_FILE"
    echo ""
    
    # If CSV, show last 5 runs for comparison
    if [ "$OUTPUT_FORMAT" = "csv" ] && [ -f "$OUTPUT_FILE" ]; then
        log_info "Recent test history:"
        echo ""
        head -1 "$OUTPUT_FILE"  # Header
        tail -5 "$OUTPUT_FILE"  # Last 5 runs
        echo ""
    fi
fi

log_info "Next steps:"
log_info "  - View detailed results: cat $OUTPUT_FILE"
log_info "  - Run more tests: $0 --help"
log_info "  - Reset cache: ./scripts/reset_test_state.sh"
echo ""
