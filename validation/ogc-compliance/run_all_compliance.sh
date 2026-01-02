#!/bin/bash
#
# OGC Compliance Test Runner - All Services
#
# This script runs OGC compliance tests for WMS, WMTS, and EDR services
# and provides a consolidated summary of results.
#
# Usage:
#   ./run_all_compliance.sh                    # Test all services with defaults
#   ./run_all_compliance.sh --skip-edr         # Skip EDR tests
#   ./run_all_compliance.sh --wms-url URL      # Custom WMS URL
#   ./run_all_compliance.sh --open             # Open reports when done
#
# Requirements:
#   - Docker (for WMS and WMTS tests)
#   - Java 17+ (for EDR tests only)

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Default URLs
WMS_URL="${WMS_URL:-http://localhost:8080/wms}"
WMTS_URL="${WMTS_URL:-http://localhost:8080/wmts}"
EDR_URL="${EDR_URL:-http://localhost:8083/edr}"

# Test flags
RUN_WMS=true
RUN_WMTS=true
RUN_EDR=true
OPEN_REPORTS=false

# Results tracking
WMS_RESULT=""
WMTS_RESULT=""
EDR_RESULT=""
WMS_PASSED=0
WMS_FAILED=0
WMTS_PASSED=0
WMTS_FAILED=0
EDR_PASSED=0
EDR_FAILED=0

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --wms-url)
            WMS_URL="$2"
            shift 2
            ;;
        --wmts-url)
            WMTS_URL="$2"
            shift 2
            ;;
        --edr-url)
            EDR_URL="$2"
            shift 2
            ;;
        --skip-wms)
            RUN_WMS=false
            shift
            ;;
        --skip-wmts)
            RUN_WMTS=false
            shift
            ;;
        --skip-edr)
            RUN_EDR=false
            shift
            ;;
        --open)
            OPEN_REPORTS=true
            shift
            ;;
        --help|-h)
            echo "OGC Compliance Test Runner - All Services"
            echo ""
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --wms-url URL           WMS endpoint URL (default: http://localhost:8080/wms)"
            echo "  --wmts-url URL          WMTS endpoint URL (default: http://localhost:8080/wmts)"
            echo "  --edr-url URL           EDR API URL (default: http://localhost:8083/edr)"
            echo "  --skip-wms              Skip WMS compliance tests"
            echo "  --skip-wmts             Skip WMTS compliance tests"
            echo "  --skip-edr              Skip EDR compliance tests"
            echo "  --open                  Open HTML reports in browser when done"
            echo "  -h, --help              Show this help message"
            echo ""
            echo "Requirements:"
            echo "  - Docker (for WMS and WMTS tests via TEAM Engine)"
            echo "  - Java 17+ (for EDR tests only)"
            echo ""
            echo "Environment Variables:"
            echo "  WMS_URL                 Default WMS endpoint URL"
            echo "  WMTS_URL                Default WMTS endpoint URL"
            echo "  EDR_URL                 Default EDR API URL"
            echo "  JAVA_HOME               Java installation directory"
            echo ""
            echo "Examples:"
            echo "  $0                          # Test all services"
            echo "  $0 --skip-edr               # Test WMS and WMTS only"
            echo "  $0 --wms-url http://x/wms   # Custom WMS URL"
            echo "  $0 --open                   # Open reports when done"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[OK]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_header() {
    echo ""
    echo -e "${MAGENTA}${BOLD}$1${NC}"
    echo -e "${MAGENTA}$(printf '=%.0s' $(seq 1 ${#1}))${NC}"
}

# Extract results from a test run
extract_results() {
    local output_dir="$1"
    local total=0 passed=0 failed=0 skipped=0
    
    # First try TestNG results (EDR)
    local results_file
    results_file=$(find "$output_dir" -name "testng-results.xml" -type f -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
    
    if [[ -n "$results_file" && -f "$results_file" ]]; then
        total=$(grep -oP 'total="\K[^"]+' "$results_file" 2>/dev/null) || total=0
        passed=$(grep -oP 'passed="\K[^"]+' "$results_file" 2>/dev/null) || passed=0
        failed=$(grep -oP 'failed="\K[^"]+' "$results_file" 2>/dev/null) || failed=0
        skipped=$(grep -oP 'skipped="\K[^"]+' "$results_file" 2>/dev/null) || skipped=0
        echo "${total}:${passed}:${failed}:${skipped}"
        return
    fi
    
    # Try TEAM Engine REST API XML results (Docker-based WMS/WMTS)
    local te_results="$output_dir/test-results.xml"
    if [[ -f "$te_results" ]]; then
        passed=$(grep -c '<endtest[^>]*result="1"' "$te_results" 2>/dev/null) || passed=0
        failed=$(grep -c '<endtest[^>]*result="6"' "$te_results" 2>/dev/null) || failed=0
        total=$((passed + failed))
        echo "${total}:${passed}:${failed}:0"
        return
    fi
    
    # Fallback: Try CTL console.log (legacy)
    local console_log="$output_dir/console.log"
    if [[ -f "$console_log" ]]; then
        passed=$(grep -c 'Test [^:]*:[^ ]* Passed$' "$console_log" 2>/dev/null) || passed=0
        failed=$(grep -c 'Test [^:]*:[^ ]* Failed' "$console_log" 2>/dev/null) || failed=0
        total=$((passed + failed))
        echo "${total}:${passed}:${failed}:0"
        return
    fi
    
    echo "0:0:0:0"
}

# Get failed test names from results
get_failed_tests() {
    local output_dir="$1"
    
    # First try TestNG results (EDR)
    local results_file
    results_file=$(find "$output_dir" -name "testng-results.xml" -type f -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
    
    if [[ -n "$results_file" && -f "$results_file" ]]; then
        grep -oP 'name="\K[^"]+(?="[^>]*status="FAIL")' "$results_file" 2>/dev/null | head -5
        return
    fi
    
    # Try TEAM Engine REST API XML results (Docker-based WMS/WMTS)
    local te_results="$output_dir/test-results.xml"
    if [[ -f "$te_results" ]]; then
        grep -B10 'result="6"' "$te_results" 2>/dev/null | \
            grep -oP 'local-name="\K[^"]+' | sort -u | head -5
        return
    fi
    
    # Fallback: Try CTL console.log (legacy)
    local console_log="$output_dir/console.log"
    if [[ -f "$console_log" ]]; then
        grep 'Test [^:]*:[^ ]* Failed' "$console_log" 2>/dev/null | grep -oP 'Test \K[^:]*:[^ ]+' | head -5
        return
    fi
}

# Run WMS tests
run_wms_tests() {
    log_header "WMS 1.3.0 Compliance Tests"
    
    if ! "$SCRIPT_DIR/run_wms_compliance.sh" --url "$WMS_URL" 2>&1 | grep -E "^\[|^  |COMPLIANCE:|Pass Rate:"; then
        WMS_RESULT="ERROR"
        return 1
    fi
    
    local results
    results=$(extract_results "results/wms")
    WMS_PASSED=$(echo "$results" | cut -d: -f2)
    WMS_FAILED=$(echo "$results" | cut -d: -f3)
    
    if [[ "$WMS_FAILED" -eq 0 && "$WMS_PASSED" -gt 0 ]]; then
        WMS_RESULT="PASSED"
    elif [[ "$WMS_FAILED" -gt 0 ]]; then
        WMS_RESULT="FAILED"
    else
        WMS_RESULT="INCOMPLETE"
    fi
}

# Run WMTS tests
run_wmts_tests() {
    log_header "WMTS 1.0.0 Compliance Tests"
    
    if ! "$SCRIPT_DIR/run_wmts_compliance.sh" --url "$WMTS_URL" 2>&1 | grep -E "^\[|^  |COMPLIANCE:|Pass Rate:"; then
        WMTS_RESULT="ERROR"
        return 1
    fi
    
    local results
    results=$(extract_results "results/wmts")
    WMTS_PASSED=$(echo "$results" | cut -d: -f2)
    WMTS_FAILED=$(echo "$results" | cut -d: -f3)
    
    if [[ "$WMTS_FAILED" -eq 0 && "$WMTS_PASSED" -gt 0 ]]; then
        WMTS_RESULT="PASSED"
    elif [[ "$WMTS_FAILED" -gt 0 ]]; then
        WMTS_RESULT="FAILED"
    else
        WMTS_RESULT="INCOMPLETE"
    fi
}

# Run EDR tests
run_edr_tests() {
    log_header "EDR 1.0 Compliance Tests"
    
    if ! "$SCRIPT_DIR/run_edr_compliance.sh" --url "$EDR_URL" 2>&1 | grep -E "^\[|^  |COMPLIANCE:|Pass Rate:"; then
        EDR_RESULT="ERROR"
        return 1
    fi
    
    local results
    results=$(extract_results "results/edr")
    EDR_PASSED=$(echo "$results" | cut -d: -f2)
    EDR_FAILED=$(echo "$results" | cut -d: -f3)
    
    if [[ "$EDR_FAILED" -eq 0 && "$EDR_PASSED" -gt 0 ]]; then
        EDR_RESULT="PASSED"
    elif [[ "$EDR_FAILED" -gt 0 ]]; then
        EDR_RESULT="FAILED"
    else
        EDR_RESULT="INCOMPLETE"
    fi
}

# Print status with color
print_status() {
    local service="$1"
    local status="$2"
    local passed="$3"
    local failed="$4"
    
    local color
    case "$status" in
        PASSED) color="$GREEN" ;;
        FAILED) color="$RED" ;;
        SKIPPED) color="$YELLOW" ;;
        *) color="$YELLOW" ;;
    esac
    
    printf "  %-8s ${color}%-12s${NC} %3s passed, %3s failed\n" "$service" "$status" "$passed" "$failed"
}

# Print consolidated summary
print_summary() {
    echo ""
    echo -e "${MAGENTA}${BOLD}======================================================${NC}"
    echo -e "${MAGENTA}${BOLD}           OGC COMPLIANCE TEST SUMMARY${NC}"
    echo -e "${MAGENTA}${BOLD}======================================================${NC}"
    echo ""
    
    # Service results table
    echo -e "${BOLD}Service Results:${NC}"
    echo ""
    
    if [[ "$RUN_WMS" == "true" ]]; then
        print_status "WMS" "$WMS_RESULT" "$WMS_PASSED" "$WMS_FAILED"
    else
        print_status "WMS" "SKIPPED" "-" "-"
    fi
    
    if [[ "$RUN_WMTS" == "true" ]]; then
        print_status "WMTS" "$WMTS_RESULT" "$WMTS_PASSED" "$WMTS_FAILED"
    else
        print_status "WMTS" "SKIPPED" "-" "-"
    fi
    
    if [[ "$RUN_EDR" == "true" ]]; then
        print_status "EDR" "$EDR_RESULT" "$EDR_PASSED" "$EDR_FAILED"
    else
        print_status "EDR" "SKIPPED" "-" "-"
    fi
    
    echo ""
    
    # Show failed tests summary (limited to avoid clutter)
    local has_failures=false
    
    if [[ "$WMS_FAILED" -gt 0 ]]; then
        has_failures=true
        echo -e "${RED}${BOLD}WMS Failed Tests:${NC}"
        get_failed_tests "results/wms" | while read -r test; do
            echo -e "  ${RED}- $test${NC}"
        done
        [[ "$WMS_FAILED" -gt 5 ]] && echo -e "  ${RED}... and $((WMS_FAILED - 5)) more${NC}"
        echo ""
    fi
    
    if [[ "$WMTS_FAILED" -gt 0 ]]; then
        has_failures=true
        echo -e "${RED}${BOLD}WMTS Failed Tests:${NC}"
        get_failed_tests "results/wmts" | while read -r test; do
            echo -e "  ${RED}- $test${NC}"
        done
        [[ "$WMTS_FAILED" -gt 5 ]] && echo -e "  ${RED}... and $((WMTS_FAILED - 5)) more${NC}"
        echo ""
    fi
    
    if [[ "$EDR_FAILED" -gt 0 ]]; then
        has_failures=true
        echo -e "${RED}${BOLD}EDR Failed Tests:${NC}"
        get_failed_tests "results/edr" | while read -r test; do
            echo -e "  ${RED}- $test${NC}"
        done
        [[ "$EDR_FAILED" -gt 5 ]] && echo -e "  ${RED}... and $((EDR_FAILED - 5)) more${NC}"
        echo ""
    fi
    
    # Overall result
    local overall_status="PASSED"
    local overall_color="$GREEN"
    
    if [[ "$WMS_RESULT" == "FAILED" || "$WMTS_RESULT" == "FAILED" || "$EDR_RESULT" == "FAILED" ]]; then
        overall_status="FAILED"
        overall_color="$RED"
    elif [[ "$WMS_RESULT" == "INCOMPLETE" || "$WMTS_RESULT" == "INCOMPLETE" || "$EDR_RESULT" == "INCOMPLETE" ]]; then
        overall_status="INCOMPLETE"
        overall_color="$YELLOW"
    elif [[ "$WMS_RESULT" == "ERROR" || "$WMTS_RESULT" == "ERROR" || "$EDR_RESULT" == "ERROR" ]]; then
        overall_status="ERROR"
        overall_color="$RED"
    fi
    
    echo -e "${BOLD}Overall:${NC} ${overall_color}${BOLD}${overall_status}${NC}"
    echo ""
    
    # Reports location
    echo -e "${BOLD}Reports:${NC}"
    [[ "$RUN_WMS" == "true" ]] && echo "  WMS:  file://${SCRIPT_DIR}/results/wms/report.html"
    [[ "$RUN_WMTS" == "true" ]] && echo "  WMTS: file://${SCRIPT_DIR}/results/wmts/report.html"
    [[ "$RUN_EDR" == "true" ]] && echo "  EDR:  file://${SCRIPT_DIR}/results/edr/report.html"
    echo ""
}

# Open reports in browser
open_reports() {
    local open_cmd=""
    if command -v xdg-open &>/dev/null; then
        open_cmd="xdg-open"
    elif command -v open &>/dev/null; then
        open_cmd="open"
    else
        log_info "Cannot auto-open reports. Open them manually from:"
        echo "  results/wms/report.html"
        echo "  results/wmts/report.html"
        echo "  results/edr/report.html"
        return
    fi
    
    [[ "$RUN_WMS" == "true" && -f "results/wms/report.html" ]] && $open_cmd "results/wms/report.html"
    [[ "$RUN_WMTS" == "true" && -f "results/wmts/report.html" ]] && $open_cmd "results/wmts/report.html"
    [[ "$RUN_EDR" == "true" && -f "results/edr/report.html" ]] && $open_cmd "results/edr/report.html"
}

# Main execution
main() {
    echo ""
    echo -e "${MAGENTA}${BOLD}======================================================${NC}"
    echo -e "${MAGENTA}${BOLD}         OGC COMPLIANCE TEST SUITE${NC}"
    echo -e "${MAGENTA}${BOLD}======================================================${NC}"
    echo ""
    echo -e "${BOLD}Services to test:${NC}"
    [[ "$RUN_WMS" == "true" ]] && echo "  - WMS 1.3.0:  $WMS_URL"
    [[ "$RUN_WMTS" == "true" ]] && echo "  - WMTS 1.0.0: $WMTS_URL"
    [[ "$RUN_EDR" == "true" ]] && echo "  - EDR 1.0:    $EDR_URL"
    echo ""
    
    local start_time
    start_time=$(date +%s)
    
    # Run tests (continue even if one fails)
    if [[ "$RUN_WMS" == "true" ]]; then
        run_wms_tests || true
    fi
    
    if [[ "$RUN_WMTS" == "true" ]]; then
        run_wmts_tests || true
    fi
    
    if [[ "$RUN_EDR" == "true" ]]; then
        run_edr_tests || true
    fi
    
    local end_time
    end_time=$(date +%s)
    local duration=$((end_time - start_time))
    
    # Print summary
    print_summary
    
    echo -e "${BOLD}Duration:${NC} ${duration}s"
    echo ""
    
    # Open reports if requested
    if [[ "$OPEN_REPORTS" == "true" ]]; then
        open_reports
    fi
    
    # Exit with failure if any test failed
    if [[ "$WMS_RESULT" == "FAILED" || "$WMTS_RESULT" == "FAILED" || "$EDR_RESULT" == "FAILED" ]]; then
        exit 1
    fi
}

main "$@"
