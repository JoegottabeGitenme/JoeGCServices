#!/bin/bash
#
# Run all load test scenarios sequentially
#
# This script runs every available load test scenario, clearing the cache
# between each run to ensure fair comparisons. Results are saved to the
# load test dashboard.
#
# Usage:
#   ./run_all_load_tests.sh              # Run all scenarios
#   ./run_all_load_tests.sh --quick      # Run only quick scenarios (< 60s)
#   ./run_all_load_tests.sh --no-reset   # Skip cache reset between tests

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
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

log_scenario() {
    echo -e "${CYAN}[SCENARIO]${NC} $1"
}

# Script directory and project root
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

# Configuration
RESET_CACHE=true
QUICK_MODE=false
SCENARIOS_DIR="$PROJECT_ROOT/validation/load-test/scenarios"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --no-reset)
            RESET_CACHE=false
            shift
            ;;
        --quick)
            QUICK_MODE=true
            shift
            ;;
        -h|--help)
            cat << EOF
Usage: $0 [OPTIONS]

Run all load test scenarios sequentially with cache clearing between runs.

OPTIONS:
  --quick         Run only quick scenarios (< 60 seconds)
  --no-reset      Skip cache reset between tests
  -h, --help      Show this help message

EXAMPLES:
  # Run all scenarios
  $0

  # Run only quick scenarios
  $0 --quick

  # Run all scenarios without cache reset
  $0 --no-reset

NOTES:
  - All results are saved to validation/load-test/results/
  - Each test result includes current optimization settings
  - Dashboard updates automatically at http://localhost:8080/loadtest
  - Full run takes approximately 30-60 minutes depending on scenarios

EOF
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

cd "$PROJECT_ROOT"

echo ""
log_info "=========================================="
log_info "Run All Load Tests"
log_info "=========================================="
log_info "Reset cache:   $RESET_CACHE"
log_info "Quick mode:    $QUICK_MODE"
log_info "Scenarios dir: $SCENARIOS_DIR"
log_info "=========================================="
echo ""

# Check if WMS service is running
log_info "Checking if WMS service is running..."
if ! curl -s -f "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities" > /dev/null 2>&1; then
    log_error "WMS service is not responding at http://localhost:8080"
    log_info "Start services with: ./scripts/start.sh"
    exit 1
fi
log_success "WMS service is running"
echo ""

# Define quick scenarios (< 60 seconds)
QUICK_SCENARIOS=(
    "quick"
    "gradient_only"
    "isolines_only"
    "wind_barbs_only"
    "goes_single_tile_temporal"
    "hrrr_single_tile_temporal"
    "mrms_single_tile_temporal"
)

# Get all available scenarios
ALL_SCENARIOS=()
for scenario_file in "$SCENARIOS_DIR"/*.yaml; do
    if [ -f "$scenario_file" ]; then
        scenario_name=$(basename "$scenario_file" .yaml)
        ALL_SCENARIOS+=("$scenario_name")
    fi
done

# Select which scenarios to run
if [ "$QUICK_MODE" = true ]; then
    SCENARIOS_TO_RUN=("${QUICK_SCENARIOS[@]}")
    log_info "Running ${#SCENARIOS_TO_RUN[@]} quick scenarios"
else
    SCENARIOS_TO_RUN=("${ALL_SCENARIOS[@]}")
    log_info "Running ${#SCENARIOS_TO_RUN[@]} scenarios"
fi

# Track results
TOTAL_SCENARIOS=${#SCENARIOS_TO_RUN[@]}
COMPLETED=0
FAILED=0
SKIPPED=0

START_TIME=$(date +%s)

# Run each scenario
for scenario in "${SCENARIOS_TO_RUN[@]}"; do
    SCENARIO_NUM=$((COMPLETED + FAILED + SKIPPED + 1))
    
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    log_scenario "[$SCENARIO_NUM/$TOTAL_SCENARIOS] Running: $scenario"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    
    # Check if scenario file exists
    SCENARIO_FILE="$SCENARIOS_DIR/${scenario}.yaml"
    if [ ! -f "$SCENARIO_FILE" ]; then
        log_warn "Scenario file not found: $SCENARIO_FILE"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi
    
    # Reset cache if enabled
    if [ "$RESET_CACHE" = true ]; then
        log_info "Resetting cache..."
        if [ -f "$SCRIPT_DIR/reset_test_state.sh" ]; then
            "$SCRIPT_DIR/reset_test_state.sh" > /dev/null 2>&1
            log_success "Cache reset complete"
        else
            log_warn "reset_test_state.sh not found, skipping cache reset"
        fi
        echo ""
        
        # Wait a moment for cache to clear
        sleep 2
    fi
    
    # Run the test
    SCENARIO_START=$(date +%s)
    
    if "$SCRIPT_DIR/run_load_test.sh" "$scenario" --save --output json; then
        SCENARIO_END=$(date +%s)
        SCENARIO_DURATION=$((SCENARIO_END - SCENARIO_START))
        
        log_success "Completed: $scenario (${SCENARIO_DURATION}s)"
        COMPLETED=$((COMPLETED + 1))
    else
        log_error "Failed: $scenario"
        FAILED=$((FAILED + 1))
    fi
    
    echo ""
done

END_TIME=$(date +%s)
TOTAL_DURATION=$((END_TIME - START_TIME))

# Calculate duration in human-readable format
MINUTES=$((TOTAL_DURATION / 60))
SECONDS=$((TOTAL_DURATION % 60))

# Print summary
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
log_success "=========================================="
log_success "All Load Tests Complete!"
log_success "=========================================="
echo ""
log_info "Summary:"
log_info "  Total scenarios:  $TOTAL_SCENARIOS"
log_success "  Completed:        $COMPLETED"
if [ $FAILED -gt 0 ]; then
    log_error "  Failed:           $FAILED"
fi
if [ $SKIPPED -gt 0 ]; then
    log_warn "  Skipped:          $SKIPPED"
fi
log_info "  Total duration:   ${MINUTES}m ${SECONDS}s"
echo ""
log_info "Results saved to: $PROJECT_ROOT/validation/load-test/results/"
log_info "View dashboard at: http://localhost:8080/loadtest"
echo ""
log_success "=========================================="
echo ""

# Exit with error if any tests failed
if [ $FAILED -gt 0 ]; then
    exit 1
fi
