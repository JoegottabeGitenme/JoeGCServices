#!/bin/bash
#
# OGC API EDR 1.0 Compliance Test Runner (CLI)
#
# This script runs the official OGC ETS (Executable Test Suite) for EDR 1.0
# using the all-in-one JAR directly - no Docker or TEAM Engine required.
#
# Usage:
#   ./run-ets-cli.sh                           # Test local EDR API (default)
#   ./run-ets-cli.sh --url http://example.com/edr  # Test custom URL
#   ./run-ets-cli.sh --collections 5           # Test 5 collections
#   ./run-ets-cli.sh --all-collections         # Test all collections
#   ./run-ets-cli.sh --open                    # Open HTML report after
#
# Requirements:
#   - Java 17+ (openjdk or similar)

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
ETS_VERSION="1.3"
AIO_JAR="lib/ets-ogcapi-edr10-${ETS_VERSION}-aio.jar"
MAVEN_URL="https://repo1.maven.org/maven2/org/opengis/cite/ets-ogcapi-edr10/${ETS_VERSION}/ets-ogcapi-edr10-${ETS_VERSION}-aio.jar"

# Default test parameters
EDR_API_URL="${EDR_API_URL:-http://localhost:8083/edr}"
API_DEFINITION=""  # Auto-detect from landing page
NUM_COLLECTIONS=3
OPEN_REPORT=false
OUTPUT_DIR="results"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --url|-u)
            EDR_API_URL="$2"
            shift 2
            ;;
        --api-definition)
            API_DEFINITION="$2"
            shift 2
            ;;
        --collections|-c)
            NUM_COLLECTIONS="$2"
            shift 2
            ;;
        --all-collections|-a)
            NUM_COLLECTIONS="-1"
            shift
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
            echo "OGC API EDR 1.0 Compliance Test Runner"
            echo ""
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  -u, --url URL           EDR API URL to test (default: http://localhost:8083/edr)"
            echo "  --api-definition URL    OpenAPI definition URL (auto-detected if not specified)"
            echo "  -c, --collections N     Number of collections to test (default: 3)"
            echo "  -a, --all-collections   Test all collections"
            echo "  -o, --output DIR        Output directory (default: results)"
            echo "  --open                  Open HTML report in browser when done"
            echo "  -h, --help              Show this help message"
            echo ""
            echo "Environment Variables:"
            echo "  EDR_API_URL             Default EDR API URL"
            echo "  JAVA_HOME               Java installation directory"
            echo ""
            echo "Examples:"
            echo "  $0                                    # Test local API"
            echo "  $0 --url https://api.example.com/edr  # Test remote API"
            echo "  $0 --all-collections --open           # Full test with report"
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

# Check Java installation
check_java() {
    log_info "Checking Java installation..."
    
    if ! command -v java &>/dev/null; then
        log_error "Java is not installed or not in PATH"
        log_error "Please install Java 17+ (e.g., openjdk-17-jre)"
        exit 1
    fi
    
    local java_version
    java_version=$(java -version 2>&1 | head -1 | cut -d'"' -f2 | cut -d'.' -f1)
    
    if [[ "$java_version" -lt 17 ]]; then
        log_error "Java 17+ required, found version $java_version"
        exit 1
    fi
    
    log_success "Java $java_version found"
}

# Download AIO JAR if needed
ensure_jar() {
    if [[ -f "$AIO_JAR" ]]; then
        log_success "ETS JAR found: $AIO_JAR"
        return 0
    fi
    
    log_info "Downloading ETS all-in-one JAR..."
    mkdir -p lib
    
    if ! curl -fSL -o "$AIO_JAR" "$MAVEN_URL"; then
        log_error "Failed to download ETS JAR from Maven"
        log_error "URL: $MAVEN_URL"
        exit 1
    fi
    
    log_success "Downloaded ETS JAR ($(du -h "$AIO_JAR" | cut -f1))"
}

# Check if EDR API is accessible
check_edr_api() {
    log_info "Checking EDR API at ${EDR_API_URL}..."
    
    local max_attempts=10
    local attempt=1
    
    while [[ $attempt -le $max_attempts ]]; do
        local response
        response=$(curl -sf -w "%{http_code}" -o /tmp/edr-landing.json "$EDR_API_URL" 2>/dev/null) || response="000"
        
        if [[ "$response" == "200" ]]; then
            log_success "EDR API is accessible"
            
            # Auto-detect API definition URL if not specified
            if [[ -z "$API_DEFINITION" ]]; then
                API_DEFINITION=$(jq -r '.links[] | select(.rel == "service-desc" or .rel == "api") | .href' /tmp/edr-landing.json 2>/dev/null | head -1)
                if [[ -n "$API_DEFINITION" && "$API_DEFINITION" != "null" ]]; then
                    log_info "Auto-detected API definition: $API_DEFINITION"
                else
                    API_DEFINITION="${EDR_API_URL}/api"
                    log_info "Using default API definition: $API_DEFINITION"
                fi
            fi
            return 0
        fi
        
        if [[ $attempt -eq $max_attempts ]]; then
            log_error "EDR API is not accessible at ${EDR_API_URL}"
            log_error "HTTP response code: $response"
            log_error ""
            log_error "Make sure the EDR API is running. Start it with:"
            log_error "  cargo run --release -p edr-api"
            exit 1
        fi
        
        log_info "Waiting for EDR API... (attempt $attempt/$max_attempts)"
        sleep 2
        ((attempt++))
    done
}

# Generate test properties file
generate_props() {
    local props_file="$OUTPUT_DIR/test-run-props.xml"
    
    mkdir -p "$OUTPUT_DIR"
    
    cat > "$props_file" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE properties SYSTEM "http://java.sun.com/dtd/properties.dtd">
<properties version="1.0">
    <comment>OGC API EDR 1.0 Test Run Configuration</comment>
    <entry key="iut">${EDR_API_URL}</entry>
    <entry key="apiDefinition">${API_DEFINITION}</entry>
    <entry key="noofcollections">${NUM_COLLECTIONS}</entry>
</properties>
EOF
    
    log_success "Generated test properties: $props_file"
}

# Run the tests
run_tests() {
    log_header "Running OGC API EDR 1.0 Compliance Tests"
    
    log_info "Target URL: ${EDR_API_URL}"
    log_info "API Definition: ${API_DEFINITION}"
    log_info "Collections: ${NUM_COLLECTIONS} (-1 = all)"
    log_info "Output: ${OUTPUT_DIR}/"
    echo ""
    
    local props_file="$OUTPUT_DIR/test-run-props.xml"
    
    # Run the ETS
    java -jar "$AIO_JAR" -o "$OUTPUT_DIR" "$props_file"
    
    local exit_code=$?
    return $exit_code
}

# Parse and display results
parse_results() {
    log_header "Test Results"
    
    local results_file
    # Find the most recently modified testng-results.xml (the ETS creates nested dirs)
    results_file=$(find "$OUTPUT_DIR" -name "testng-results.xml" -type f -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
    
    if [[ -z "$results_file" || ! -f "$results_file" ]]; then
        log_warn "Could not find testng-results.xml"
        return 1
    fi
    
    log_info "Results file: $results_file"
    
    # Extract stats from XML
    local total passed failed skipped
    total=$(grep -oP 'total="\K[^"]+' "$results_file" 2>/dev/null || echo "0")
    passed=$(grep -oP 'passed="\K[^"]+' "$results_file" 2>/dev/null || echo "0")
    failed=$(grep -oP 'failed="\K[^"]+' "$results_file" 2>/dev/null || echo "0")
    skipped=$(grep -oP 'skipped="\K[^"]+' "$results_file" 2>/dev/null || echo "0")
    
    echo ""
    echo -e "  ${BOLD}Total:${NC}   $total"
    echo -e "  ${GREEN}Passed:${NC}  $passed"
    echo -e "  ${RED}Failed:${NC}  $failed"
    echo -e "  ${YELLOW}Skipped:${NC} $skipped"
    echo ""
    
    # Show failed tests
    if [[ "$failed" -gt 0 ]]; then
        echo -e "${RED}${BOLD}Failed Tests:${NC}"
        grep -oP 'name="\K[^"]+(?="[^>]*status="FAIL")' "$results_file" 2>/dev/null | while read -r test_name; do
            echo -e "  ${RED}- $test_name${NC}"
        done
        echo ""
    fi
    
    # Calculate pass rate
    if [[ "$total" -gt 0 ]]; then
        local pass_rate
        pass_rate=$((passed * 100 / total))
        echo -e "  ${BOLD}Pass Rate:${NC} ${pass_rate}%"
    fi
    
    # Overall status
    echo ""
    if [[ "$failed" -eq 0 && "$passed" -gt 0 ]]; then
        echo -e "${GREEN}${BOLD}COMPLIANCE: PASSED${NC}"
        return 0
    elif [[ "$failed" -gt 0 ]]; then
        echo -e "${RED}${BOLD}COMPLIANCE: FAILED${NC}"
        return 1
    else
        echo -e "${YELLOW}${BOLD}COMPLIANCE: INCOMPLETE${NC}"
        return 2
    fi
}

# Generate simple HTML summary
generate_html_summary() {
    local results_file
    results_file=$(find "$OUTPUT_DIR" -name "testng-results.xml" -type f -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
    
    if [[ -z "$results_file" ]]; then
        return
    fi
    
    local total passed failed skipped
    total=$(grep -oP 'total="\K[^"]+' "$results_file" 2>/dev/null || echo "0")
    passed=$(grep -oP 'passed="\K[^"]+' "$results_file" 2>/dev/null || echo "0")
    failed=$(grep -oP 'failed="\K[^"]+' "$results_file" 2>/dev/null || echo "0")
    skipped=$(grep -oP 'skipped="\K[^"]+' "$results_file" 2>/dev/null || echo "0")
    
    local status="UNKNOWN"
    local status_color="#6c757d"
    if [[ "$failed" -eq 0 && "$passed" -gt 0 ]]; then
        status="PASSED"
        status_color="#28a745"
    elif [[ "$failed" -gt 0 ]]; then
        status="FAILED"
        status_color="#dc3545"
    fi
    
    cat > "$OUTPUT_DIR/report.html" << EOF
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>OGC API EDR 1.0 Compliance Report</title>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #f5f5f5;
            padding: 20px;
            line-height: 1.6;
        }
        .container { max-width: 1000px; margin: 0 auto; }
        .header {
            background: linear-gradient(135deg, #1a365d 0%, #2c5282 100%);
            color: white;
            padding: 30px;
            border-radius: 10px;
            margin-bottom: 20px;
        }
        .header h1 { font-size: 24px; margin-bottom: 10px; }
        .header .subtitle { opacity: 0.9; }
        .status-badge {
            display: inline-block;
            padding: 8px 20px;
            border-radius: 20px;
            font-weight: bold;
            font-size: 14px;
            background: ${status_color};
            margin-top: 15px;
        }
        .card {
            background: white;
            border-radius: 10px;
            padding: 20px;
            margin-bottom: 20px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
        }
        .card h2 {
            font-size: 18px;
            color: #1a365d;
            margin-bottom: 15px;
            border-bottom: 2px solid #e2e8f0;
            padding-bottom: 10px;
        }
        .stats {
            display: grid;
            grid-template-columns: repeat(4, 1fr);
            gap: 15px;
        }
        .stat {
            text-align: center;
            padding: 15px;
            background: #f8fafc;
            border-radius: 8px;
        }
        .stat-value {
            font-size: 32px;
            font-weight: bold;
            color: #1a365d;
        }
        .stat-label { color: #64748b; font-size: 14px; }
        .stat.passed .stat-value { color: #28a745; }
        .stat.failed .stat-value { color: #dc3545; }
        .stat.skipped .stat-value { color: #ffc107; }
        .info-table { width: 100%; border-collapse: collapse; }
        .info-table td { padding: 10px 0; border-bottom: 1px solid #e2e8f0; }
        .info-table td:first-child { color: #64748b; width: 150px; }
        .footer {
            text-align: center;
            color: #64748b;
            font-size: 14px;
            margin-top: 30px;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>OGC API - Environmental Data Retrieval 1.0</h1>
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
                    <div class="stat-value">${passed}</div>
                    <div class="stat-label">Passed</div>
                </div>
                <div class="stat failed">
                    <div class="stat-value">${failed}</div>
                    <div class="stat-label">Failed</div>
                </div>
                <div class="stat skipped">
                    <div class="stat-value">${skipped}</div>
                    <div class="stat-label">Skipped</div>
                </div>
            </div>
        </div>

        <div class="card">
            <h2>Test Configuration</h2>
            <table class="info-table">
                <tr>
                    <td>Test Suite</td>
                    <td>OGC API - EDR 1.0 ETS v${ETS_VERSION}</td>
                </tr>
                <tr>
                    <td>Implementation</td>
                    <td><a href="${EDR_API_URL}">${EDR_API_URL}</a></td>
                </tr>
                <tr>
                    <td>API Definition</td>
                    <td><a href="${API_DEFINITION}">${API_DEFINITION}</a></td>
                </tr>
                <tr>
                    <td>Collections Tested</td>
                    <td>${NUM_COLLECTIONS} (-1 = all)</td>
                </tr>
                <tr>
                    <td>Generated</td>
                    <td>$(date -Iseconds)</td>
                </tr>
            </table>
        </div>

        <div class="footer">
            <p>Generated by Weather WMS OGC Compliance Framework</p>
            <p><a href="https://cite.ogc.org/">OGC CITE</a> | ETS v${ETS_VERSION}</p>
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
        log_warn "Report file not found"
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
    echo "         OGC Compliance Test Summary"
    echo "=========================================="
    echo ""
    echo "EDR API:    ${EDR_API_URL}"
    echo "Results:    ${SCRIPT_DIR}/${OUTPUT_DIR}/"
    echo ""
    
    if [[ -f "$OUTPUT_DIR/report.html" ]]; then
        echo "HTML Report: file://${SCRIPT_DIR}/${OUTPUT_DIR}/report.html"
    fi
    
    local results_file
    results_file=$(find "$OUTPUT_DIR" -name "testng-results.xml" -type f 2>/dev/null | head -1)
    if [[ -n "$results_file" ]]; then
        echo "XML Results: ${results_file}"
    fi
    
    echo ""
}

# Main execution
main() {
    echo ""
    echo "================================================"
    echo "    OGC API EDR 1.0 Compliance Test Suite"
    echo "================================================"
    echo ""
    
    check_java
    ensure_jar
    check_edr_api
    generate_props
    
    local test_result=0
    run_tests || test_result=$?
    
    parse_results || true
    generate_html_summary
    print_summary
    
    if [[ "$OPEN_REPORT" == "true" ]]; then
        open_report
    fi
    
    exit $test_result
}

main "$@"
