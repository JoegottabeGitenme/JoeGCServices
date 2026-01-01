#!/bin/bash
#
# OGC WMTS 1.0.0 Compliance Test Runner
#
# This script runs the official OGC ETS (Executable Test Suite) for WMTS 1.0.0
# using Docker with TEAM Engine and the REST API.
#
# Usage:
#   ./run_wmts_compliance.sh                               # Test local WMTS (default)
#   ./run_wmts_compliance.sh --url http://example.com/wmts # Test custom URL
#   ./run_wmts_compliance.sh --open                        # Open HTML report after
#
# Requirements:
#   - Docker and Docker Compose
#   - WMTS service running on localhost:8080 (or custom URL)

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
BOLD='\033[1m'
NC='\033[0m' # No Color

# Configuration
TEAMENGINE_PORT=9094
COMPOSE_PROJECT="ogc-wmts-test"
DOCKER_COMPOSE_FILE="docker-compose.yml"

# Default test parameters
WMTS_URL="${WMTS_URL:-http://localhost:8080/wmts}"
OPEN_REPORT=false
OUTPUT_DIR="results/wmts"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --url|-u)
            WMTS_URL="$2"
            shift 2
            ;;
        --output|-o)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --open)
            OPEN_REPORT=true
            shift
            ;;
        --help|-h)
            echo "OGC WMTS 1.0.0 Compliance Test Runner"
            echo ""
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  -u, --url URL           WMTS endpoint URL (default: http://localhost:8080/wmts)"
            echo "  -o, --output DIR        Output directory (default: results/wmts)"
            echo "  --open                  Open HTML report in browser when done"
            echo "  -h, --help              Show this help message"
            echo ""
            echo "Requirements:"
            echo "  - Docker and Docker Compose"
            echo "  - WMTS service running"
            echo ""
            echo "Examples:"
            echo "  $0                                        # Test local WMTS"
            echo "  $0 --url http://myserver:8080/wmts        # Test custom WMTS"
            echo "  $0 --open                                 # Test and open report"
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
    echo -e "${CYAN}${BOLD}$1${NC}"
    echo -e "${CYAN}$(printf '=%.0s' $(seq 1 ${#1}))${NC}"
}

# Check Docker installation
check_docker() {
    log_info "Checking Docker installation..."
    
    if ! command -v docker &>/dev/null; then
        log_error "Docker is not installed or not in PATH"
        log_error "Please install Docker: https://docs.docker.com/get-docker/"
        exit 1
    fi
    
    if ! docker info &>/dev/null; then
        log_error "Docker daemon is not running"
        log_error "Please start Docker and try again"
        exit 1
    fi
    
    if ! docker compose version &>/dev/null; then
        log_error "Docker Compose is not available"
        exit 1
    fi
    
    log_success "Docker is available"
}

# Build GetCapabilities URL
get_capabilities_url() {
    local base_url="$1"
    if [[ "$base_url" == *"?"* ]]; then
        echo "${base_url}&SERVICE=WMTS&REQUEST=GetCapabilities"
    else
        echo "${base_url}?SERVICE=WMTS&REQUEST=GetCapabilities"
    fi
}

# Check if WMTS is accessible
check_wmts() {
    local caps_url
    caps_url=$(get_capabilities_url "$WMTS_URL")
    
    log_info "Checking WMTS at ${WMTS_URL}..."
    
    local response
    response=$(curl -sf -w "%{http_code}" -o /tmp/wmts-caps-check.xml "$caps_url" 2>/dev/null) || response="000"
    
    if [[ "$response" == "200" ]]; then
        if grep -q "Capabilities" /tmp/wmts-caps-check.xml 2>/dev/null; then
            log_success "WMTS is accessible and returns valid capabilities"
            return 0
        fi
    fi
    
    log_error "WMTS is not accessible at ${WMTS_URL}"
    log_error "HTTP response code: $response"
    log_error ""
    log_error "Make sure the WMTS is running. Start it with:"
    log_error "  cargo run --release -p wms-api"
    exit 1
}

# Start TEAM Engine containers
start_containers() {
    log_info "Starting TEAM Engine containers..."
    
    # Start proxy and TEAM Engine containers
    docker compose -f "$DOCKER_COMPOSE_FILE" -p "$COMPOSE_PROJECT" --profile wmts up -d 2>/dev/null
    
    log_info "Waiting for TEAM Engine to start..."
    local max_attempts=30
    local attempt=1
    
    while [[ $attempt -le $max_attempts ]]; do
        if curl -sf "http://localhost:${TEAMENGINE_PORT}/teamengine/" &>/dev/null; then
            log_success "TEAM Engine is ready"
            return 0
        fi
        sleep 1
        ((attempt++))
    done
    
    log_error "TEAM Engine failed to start"
    stop_containers
    exit 1
}

# Stop TEAM Engine containers
stop_containers() {
    log_info "Stopping TEAM Engine containers..."
    docker compose -f "$DOCKER_COMPOSE_FILE" -p "$COMPOSE_PROJECT" --profile wmts down 2>/dev/null || true
}

# Run the tests via REST API
run_tests() {
    log_header "Running OGC WMTS 1.0.0 Compliance Tests"
    
    mkdir -p "$OUTPUT_DIR"
    
    # Build capabilities URL for TEAM Engine
    # Use the nginx proxy IP (172.28.0.10) which rewrites localhost URLs in responses
    local caps_url="http://172.28.0.10:8080/wmts?SERVICE=WMTS&REQUEST=GetCapabilities"
    local encoded_url
    encoded_url=$(python3 -c "import urllib.parse; print(urllib.parse.quote('$caps_url', safe=''))")
    
    log_info "Target URL: ${WMTS_URL}"
    log_info "Test URL (via proxy): ${caps_url}"
    log_info "Output: ${OUTPUT_DIR}/"
    echo ""
    
    # Run tests via REST API
    log_info "Executing tests (this may take a minute)..."
    
    local result_file="$OUTPUT_DIR/test-results.xml"
    
    if ! curl -sf -u ogctest:ogctest --max-time 300 \
        "http://localhost:${TEAMENGINE_PORT}/teamengine/rest/suites/wmts/run?capabilities-url=$encoded_url" \
        -H 'Accept: application/xml' \
        -o "$result_file" 2>/dev/null; then
        log_error "Failed to execute tests via REST API"
        return 1
    fi
    
    log_success "Tests completed"
    return 0
}

# Parse and display results
parse_results() {
    log_header "Test Results"
    
    local result_file="$OUTPUT_DIR/test-results.xml"
    
    if [[ ! -f "$result_file" ]]; then
        log_warn "Could not find test results"
        return 1
    fi
    
    # Count pass/fail from endtest elements
    local pass_count fail_count total
    pass_count=$(grep -c '<endtest[^>]*result="1"' "$result_file" 2>/dev/null) || pass_count=0
    fail_count=$(grep -c '<endtest[^>]*result="6"' "$result_file" 2>/dev/null) || fail_count=0
    total=$((pass_count + fail_count))
    
    echo ""
    echo -e "  ${BOLD}Total:${NC}   $total"
    echo -e "  ${GREEN}Passed:${NC}  $pass_count"
    echo -e "  ${RED}Failed:${NC}  $fail_count"
    echo ""
    
    # Show failed tests
    if [[ "$fail_count" -gt 0 ]]; then
        echo -e "${RED}${BOLD}Failed Tests:${NC}"
        # Extract test names from failed tests
        grep -B10 'result="6"' "$result_file" 2>/dev/null | \
            grep -oP 'local-name="\K[^"]+' | \
            sort -u | head -10 | while read -r test_name; do
            echo -e "  ${RED}- $test_name${NC}"
        done
        if [[ "$fail_count" -gt 10 ]]; then
            echo -e "  ${RED}... and more${NC}"
        fi
        echo ""
    fi
    
    # Calculate pass rate
    if [[ "$total" -gt 0 ]]; then
        local pass_rate
        pass_rate=$((pass_count * 100 / total))
        echo -e "  ${BOLD}Pass Rate:${NC} ${pass_rate}%"
    fi
    
    # Overall status
    echo ""
    if [[ "$fail_count" -eq 0 && "$pass_count" -gt 0 ]]; then
        echo -e "${GREEN}${BOLD}COMPLIANCE: PASSED${NC}"
        return 0
    elif [[ "$fail_count" -gt 0 ]]; then
        echo -e "${RED}${BOLD}COMPLIANCE: FAILED${NC}"
        return 1
    else
        echo -e "${YELLOW}${BOLD}COMPLIANCE: INCOMPLETE${NC}"
        return 2
    fi
}

# Generate HTML report
generate_html_report() {
    local result_file="$OUTPUT_DIR/test-results.xml"
    
    local pass_count=0 fail_count=0 total=0
    
    if [[ -f "$result_file" ]]; then
        pass_count=$(grep -c '<endtest[^>]*result="1"' "$result_file" 2>/dev/null) || pass_count=0
        fail_count=$(grep -c '<endtest[^>]*result="6"' "$result_file" 2>/dev/null) || fail_count=0
        total=$((pass_count + fail_count))
    fi
    
    local status="UNKNOWN"
    local status_color="#6c757d"
    if [[ "$fail_count" -eq 0 && "$pass_count" -gt 0 ]]; then
        status="PASSED"
        status_color="#28a745"
    elif [[ "$fail_count" -gt 0 ]]; then
        status="FAILED"
        status_color="#dc3545"
    fi
    
    local pass_rate=0
    if [[ "$total" -gt 0 ]]; then
        pass_rate=$((pass_count * 100 / total))
    fi
    
    # Extract failed test names
    local failed_tests=""
    if [[ -f "$result_file" && "$fail_count" -gt 0 ]]; then
        failed_tests=$(grep -B10 'result="6"' "$result_file" 2>/dev/null | \
            grep -oP 'local-name="\K[^"]+' | sort -u | head -20 | \
            while read -r name; do echo "<li>$name</li>"; done)
    fi
    
    cat > "$OUTPUT_DIR/report.html" << EOF
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>OGC WMTS 1.0.0 Compliance Report</title>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f5f5f5; padding: 20px; line-height: 1.6; }
        .container { max-width: 1000px; margin: 0 auto; }
        .header { background: linear-gradient(135deg, #1e3a5f 0%, #2d5986 100%); color: white; padding: 30px; border-radius: 10px; margin-bottom: 20px; }
        .header h1 { font-size: 24px; margin-bottom: 10px; }
        .header .subtitle { opacity: 0.9; }
        .status-badge { display: inline-block; padding: 8px 20px; border-radius: 20px; font-weight: bold; font-size: 14px; background: ${status_color}; margin-top: 15px; }
        .card { background: white; border-radius: 10px; padding: 20px; margin-bottom: 20px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
        .card h2 { font-size: 18px; color: #1e3a5f; margin-bottom: 15px; border-bottom: 2px solid #e2e8f0; padding-bottom: 10px; }
        .stats { display: grid; grid-template-columns: repeat(4, 1fr); gap: 15px; }
        .stat { text-align: center; padding: 15px; background: #f8fafc; border-radius: 8px; }
        .stat-value { font-size: 32px; font-weight: bold; color: #1e3a5f; }
        .stat-label { color: #64748b; font-size: 14px; }
        .stat.passed .stat-value { color: #28a745; }
        .stat.failed .stat-value { color: #dc3545; }
        .stat.rate .stat-value { color: #007bff; }
        .info-table { width: 100%; border-collapse: collapse; }
        .info-table td { padding: 10px 0; border-bottom: 1px solid #e2e8f0; }
        .info-table td:first-child { color: #64748b; width: 180px; }
        .failed-list { list-style: none; padding: 0; }
        .failed-list li { padding: 8px 12px; background: #fff5f5; border-left: 3px solid #dc3545; margin-bottom: 5px; font-family: monospace; font-size: 13px; }
        .footer { text-align: center; color: #64748b; font-size: 14px; margin-top: 30px; }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>OGC Web Map Tile Service 1.0.0</h1>
            <div class="subtitle">Compliance Test Report</div>
            <span class="status-badge">${status}</span>
        </div>

        <div class="card">
            <h2>Test Summary</h2>
            <div class="stats">
                <div class="stat">
                    <div class="stat-value">${total}</div>
                    <div class="stat-label">Total Tests</div>
                </div>
                <div class="stat passed">
                    <div class="stat-value">${pass_count}</div>
                    <div class="stat-label">Passed</div>
                </div>
                <div class="stat failed">
                    <div class="stat-value">${fail_count}</div>
                    <div class="stat-label">Failed</div>
                </div>
                <div class="stat rate">
                    <div class="stat-value">${pass_rate}%</div>
                    <div class="stat-label">Pass Rate</div>
                </div>
            </div>
        </div>

        <div class="card">
            <h2>Test Configuration</h2>
            <table class="info-table">
                <tr><td>Test Suite</td><td>OGC WMTS 1.0.0 ETS (TEAM Engine)</td></tr>
                <tr><td>Implementation</td><td><a href="${WMTS_URL}">${WMTS_URL}</a></td></tr>
                <tr><td>Generated</td><td>$(date -Iseconds)</td></tr>
            </table>
        </div>

        $(if [[ -n "$failed_tests" ]]; then echo "
        <div class=\"card\">
            <h2>Failed Tests</h2>
            <ul class=\"failed-list\">
                $failed_tests
            </ul>
        </div>
        "; fi)

        <div class="footer">
            <p>Generated by Weather WMS OGC Compliance Framework</p>
            <p><a href="https://cite.ogc.org/">OGC CITE</a></p>
        </div>
    </div>
</body>
</html>
EOF
    
    log_success "Generated HTML report: $OUTPUT_DIR/report.html"
}

# Open report in browser
open_report() {
    local report_file="$OUTPUT_DIR/report.html"
    
    if [[ ! -f "$report_file" ]]; then
        return
    fi
    
    if command -v xdg-open &>/dev/null; then
        xdg-open "$report_file"
    elif command -v open &>/dev/null; then
        open "$report_file"
    else
        log_info "Open in browser: file://${SCRIPT_DIR}/${report_file}"
    fi
}

# Print final summary
print_summary() {
    echo ""
    echo "=========================================="
    echo "       WMTS Compliance Test Summary"
    echo "=========================================="
    echo ""
    echo "WMTS URL:   ${WMTS_URL}"
    echo "Results:    ${SCRIPT_DIR}/${OUTPUT_DIR}/"
    echo ""
    echo "HTML Report: file://${SCRIPT_DIR}/${OUTPUT_DIR}/report.html"
    echo "XML Results: ${SCRIPT_DIR}/${OUTPUT_DIR}/test-results.xml"
    echo ""
}

# Cleanup on exit
cleanup() {
    stop_containers
}

# Main execution
main() {
    echo ""
    echo "================================================"
    echo "      OGC WMTS 1.0.0 Compliance Test Suite"
    echo "================================================"
    echo ""
    
    # Set up cleanup trap
    trap cleanup EXIT
    
    check_docker
    check_wmts
    start_containers
    
    local test_result=0
    run_tests || test_result=$?
    
    parse_results || true
    generate_html_report
    print_summary
    
    if [[ "$OPEN_REPORT" == "true" ]]; then
        open_report
    fi
    
    exit $test_result
}

main "$@"
