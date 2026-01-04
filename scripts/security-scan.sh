#!/bin/bash
# =============================================================================
# Weather WMS - Security Scan Orchestrator
# =============================================================================
# Comprehensive security scanning using OWASP ZAP, Nuclei, testssl.sh, and
# custom checks against the deployed Weather WMS services.
#
# Usage:
#   ./scripts/security-scan.sh                    # Scan default target
#   ./scripts/security-scan.sh --target URL       # Scan specific target
#   ./scripts/security-scan.sh --only zap         # Run only ZAP scan
#   ./scripts/security-scan.sh --skip nuclei      # Skip Nuclei scan
#   ./scripts/security-scan.sh --help             # Show help
#
# Requirements:
#   - Docker (for ZAP, Nuclei, testssl.sh)
#   - curl
#   - jq (for JSON processing)
#   - cargo-audit (optional, for Rust dependency scanning)
# =============================================================================

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color
BOLD='\033[1m'

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SECURITY_DIR="${SCRIPT_DIR}/security"

# Default configuration
DEFAULT_TARGET="https://folkweather.com"
TARGET=""
AUTH_USER=""
AUTH_PASS=""
AUTH_B64=""

# Scan control
SKIP_SCANS=()
ONLY_SCAN=""

# Output directory
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
OUTPUT_DIR="${SCRIPT_DIR}/security-scan-results/${TIMESTAMP}"

# Timing
SCAN_START_TIME=""
declare -A SCAN_TIMES

# =============================================================================
# Helper Functions
# =============================================================================

print_banner() {
    echo ""
    echo -e "${CYAN}${BOLD}"
    echo "╔═══════════════════════════════════════════════════════════════╗"
    echo "║           Weather WMS Security Scan                           ║"
    echo "╚═══════════════════════════════════════════════════════════════╝"
    echo -e "${NC}"
}

print_step() {
    echo -e "${CYAN}[STEP]${NC} $1"
}

print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_config() {
    echo -e "${BLUE}[CONFIG]${NC} $1"
}

show_help() {
    cat << EOF
Weather WMS Security Scanner

Usage: $(basename "$0") [OPTIONS]

Options:
  --target URL      Target URL to scan (default: ${DEFAULT_TARGET})
  --only SCAN       Run only specified scan (zap, nuclei, tls, headers, auth, rate, cargo)
  --skip SCAN       Skip specified scan (can be used multiple times)
  --output DIR      Output directory (default: auto-generated with timestamp)
  --help            Show this help message

Available Scans:
  tls       - TLS/SSL configuration analysis (testssl.sh)
  headers   - Security headers check
  cargo     - Rust dependency CVE scan (cargo-audit)
  nuclei    - Vulnerability scanning with Nuclei templates
  zap       - OWASP ZAP full active scan
  auth      - Authentication bypass tests
  rate      - Rate limiting verification

Examples:
  $(basename "$0")                          # Run all scans against default target
  $(basename "$0") --target https://example.com
  $(basename "$0") --only zap               # Run only ZAP scan
  $(basename "$0") --skip zap --skip nuclei # Skip ZAP and Nuclei

Credentials:
  The script automatically loads credentials from .env.nuc or .env file.
  Required variables: ADMIN_USER, ADMIN_PASSWORD
EOF
}

# Parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --target)
                TARGET="$2"
                shift 2
                ;;
            --only)
                ONLY_SCAN="$2"
                shift 2
                ;;
            --skip)
                SKIP_SCANS+=("$2")
                shift 2
                ;;
            --output)
                OUTPUT_DIR="$2"
                shift 2
                ;;
            --help|-h)
                show_help
                exit 0
                ;;
            *)
                print_error "Unknown option: $1"
                show_help
                exit 1
                ;;
        esac
    done

    # Set default target if not specified
    if [[ -z "$TARGET" ]]; then
        TARGET="$DEFAULT_TARGET"
    fi
}

# Load credentials from .env files
load_credentials() {
    local env_file=""
    
    # Try .env.nuc first, then .env
    if [[ -f "${PROJECT_ROOT}/.env.nuc" ]]; then
        env_file="${PROJECT_ROOT}/.env.nuc"
    elif [[ -f "${PROJECT_ROOT}/.env" ]]; then
        env_file="${PROJECT_ROOT}/.env"
    fi

    if [[ -n "$env_file" ]]; then
        print_config "Loading credentials from $(basename "$env_file")"
        
        # Source the file to get variables
        set -a
        source "$env_file"
        set +a
        
        AUTH_USER="${ADMIN_USER:-}"
        AUTH_PASS="${ADMIN_PASSWORD:-}"
    fi

    if [[ -z "$AUTH_USER" || -z "$AUTH_PASS" ]]; then
        print_warning "Credentials not found - authenticated endpoints will be skipped"
        AUTH_B64=""
    else
        AUTH_B64=$(echo -n "${AUTH_USER}:${AUTH_PASS}" | base64)
        print_config "Auth: Credentials loaded for user '${AUTH_USER}'"
    fi
}

# Check required dependencies
check_dependencies() {
    local missing=()

    if ! command -v docker &> /dev/null; then
        missing+=("docker")
    fi

    if ! command -v curl &> /dev/null; then
        missing+=("curl")
    fi

    if ! command -v jq &> /dev/null; then
        missing+=("jq")
    fi

    if [[ ${#missing[@]} -gt 0 ]]; then
        print_error "Missing required dependencies: ${missing[*]}"
        print_info "Please install them and try again"
        exit 1
    fi

    # Check Docker is running
    if ! docker info &> /dev/null; then
        print_error "Docker is not running"
        exit 1
    fi

    print_success "All dependencies satisfied"
}

# Check if a scan should be run
should_run_scan() {
    local scan_name="$1"

    # If --only is specified, only run that scan
    if [[ -n "$ONLY_SCAN" ]]; then
        [[ "$ONLY_SCAN" == "$scan_name" ]]
        return
    fi

    # Check if scan is in skip list
    for skip in "${SKIP_SCANS[@]}"; do
        if [[ "$skip" == "$scan_name" ]]; then
            return 1
        fi
    done

    return 0
}

# Create output directory
setup_output_dir() {
    mkdir -p "$OUTPUT_DIR"
    print_config "Output: ${OUTPUT_DIR}"
    
    # Create subdirectories
    mkdir -p "${OUTPUT_DIR}/zap"
    mkdir -p "${OUTPUT_DIR}/nuclei"
    mkdir -p "${OUTPUT_DIR}/raw"
}

# Record scan time
record_time() {
    local scan_name="$1"
    local start_time="$2"
    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    SCAN_TIMES["$scan_name"]=$duration
}

format_duration() {
    local seconds=$1
    local minutes=$((seconds / 60))
    local secs=$((seconds % 60))
    if [[ $minutes -gt 0 ]]; then
        echo "${minutes}m ${secs}s"
    else
        echo "${secs}s"
    fi
}

# =============================================================================
# Scan Execution Functions
# =============================================================================

run_tls_scan() {
    if ! should_run_scan "tls"; then
        print_info "Skipping TLS scan"
        return 0
    fi

    print_step "Running TLS scan (testssl.sh)..."
    local start_time=$(date +%s)

    if [[ -x "${SECURITY_DIR}/tls-scan.sh" ]]; then
        "${SECURITY_DIR}/tls-scan.sh" "$TARGET" "$OUTPUT_DIR" || true
    else
        print_warning "TLS scan script not found or not executable"
    fi

    record_time "tls" "$start_time"
    print_success "TLS scan complete ($(format_duration ${SCAN_TIMES[tls]}))"
}

run_header_check() {
    if ! should_run_scan "headers"; then
        print_info "Skipping headers check"
        return 0
    fi

    print_step "Running security headers check..."
    local start_time=$(date +%s)

    if [[ -x "${SECURITY_DIR}/header-check.sh" ]]; then
        "${SECURITY_DIR}/header-check.sh" "$TARGET" "$OUTPUT_DIR" "$AUTH_B64" || true
    else
        print_warning "Header check script not found or not executable"
    fi

    record_time "headers" "$start_time"
    print_success "Headers check complete ($(format_duration ${SCAN_TIMES[headers]}))"
}

run_cargo_audit() {
    if ! should_run_scan "cargo"; then
        print_info "Skipping cargo audit"
        return 0
    fi

    print_step "Running cargo audit (Rust dependency CVE scan)..."
    local start_time=$(date +%s)

    if [[ -x "${SECURITY_DIR}/cargo-audit.sh" ]]; then
        "${SECURITY_DIR}/cargo-audit.sh" "$PROJECT_ROOT" "$OUTPUT_DIR" || true
    else
        print_warning "Cargo audit script not found or not executable"
    fi

    record_time "cargo" "$start_time"
    print_success "Cargo audit complete ($(format_duration ${SCAN_TIMES[cargo]}))"
}

run_nuclei_scan() {
    if ! should_run_scan "nuclei"; then
        print_info "Skipping Nuclei scan"
        return 0
    fi

    print_step "Running Nuclei vulnerability scan..."
    local start_time=$(date +%s)

    if [[ -x "${SECURITY_DIR}/nuclei-scan.sh" ]]; then
        "${SECURITY_DIR}/nuclei-scan.sh" "$TARGET" "$OUTPUT_DIR" "$AUTH_B64" || true
    else
        print_warning "Nuclei scan script not found or not executable"
    fi

    record_time "nuclei" "$start_time"
    print_success "Nuclei scan complete ($(format_duration ${SCAN_TIMES[nuclei]}))"
}

run_zap_scan() {
    if ! should_run_scan "zap"; then
        print_info "Skipping ZAP scan"
        return 0
    fi

    print_step "Running OWASP ZAP full scan (this may take 15-30 minutes)..."
    local start_time=$(date +%s)

    if [[ -x "${SECURITY_DIR}/zap-scan.sh" ]]; then
        "${SECURITY_DIR}/zap-scan.sh" "$TARGET" "$OUTPUT_DIR" "$AUTH_B64" || true
    else
        print_warning "ZAP scan script not found or not executable"
    fi

    record_time "zap" "$start_time"
    print_success "ZAP scan complete ($(format_duration ${SCAN_TIMES[zap]}))"
}

run_auth_check() {
    if ! should_run_scan "auth"; then
        print_info "Skipping auth bypass check"
        return 0
    fi

    print_step "Running authentication bypass checks..."
    local start_time=$(date +%s)

    if [[ -x "${SECURITY_DIR}/auth-check.sh" ]]; then
        "${SECURITY_DIR}/auth-check.sh" "$TARGET" "$OUTPUT_DIR" || true
    else
        print_warning "Auth check script not found or not executable"
    fi

    record_time "auth" "$start_time"
    print_success "Auth check complete ($(format_duration ${SCAN_TIMES[auth]}))"
}

run_rate_limit_check() {
    if ! should_run_scan "rate"; then
        print_info "Skipping rate limit check"
        return 0
    fi

    print_step "Running rate limit verification..."
    local start_time=$(date +%s)

    if [[ -x "${SECURITY_DIR}/rate-limit-check.sh" ]]; then
        "${SECURITY_DIR}/rate-limit-check.sh" "$TARGET" "$OUTPUT_DIR" || true
    else
        print_warning "Rate limit check script not found or not executable"
    fi

    record_time "rate" "$start_time"
    print_success "Rate limit check complete ($(format_duration ${SCAN_TIMES[rate]}))"
}

generate_report() {
    print_step "Generating HTML report..."

    if [[ -x "${SECURITY_DIR}/generate-report.sh" ]]; then
        "${SECURITY_DIR}/generate-report.sh" "$OUTPUT_DIR" "$TARGET" "$TIMESTAMP"
    else
        print_warning "Report generator not found - individual reports are still available"
    fi
}

# Print final summary
print_summary() {
    local total_duration=$(($(date +%s) - SCAN_START_TIME))

    echo ""
    echo -e "${CYAN}${BOLD}"
    echo "═══════════════════════════════════════════════════════════════"
    echo "                      SCAN SUMMARY"
    echo "═══════════════════════════════════════════════════════════════"
    echo -e "${NC}"
    
    echo -e "  ${BOLD}Target:${NC}           ${TARGET}"
    echo -e "  ${BOLD}Total Duration:${NC}   $(format_duration $total_duration)"
    echo ""

    # Print timing for each scan
    echo -e "  ${BOLD}Scan Times:${NC}"
    for scan in tls headers cargo nuclei zap auth rate; do
        if [[ -v SCAN_TIMES[$scan] ]]; then
            printf "    %-12s %s\n" "$scan:" "$(format_duration ${SCAN_TIMES[$scan]})"
        fi
    done

    echo ""
    echo -e "${CYAN}═══════════════════════════════════════════════════════════════${NC}"
    echo ""

    if [[ -f "${OUTPUT_DIR}/index.html" ]]; then
        print_success "Report saved to:"
        echo "  ${OUTPUT_DIR}/index.html"
    else
        print_info "Individual scan results saved to:"
        echo "  ${OUTPUT_DIR}/"
    fi
    echo ""
}

# =============================================================================
# Main Execution
# =============================================================================

main() {
    SCAN_START_TIME=$(date +%s)

    print_banner
    parse_args "$@"

    print_config "Target: ${TARGET}"
    
    load_credentials
    check_dependencies
    setup_output_dir

    echo ""

    # Phase 1: Run independent scans in parallel
    print_info "Phase 1: Running independent scans in parallel..."
    
    run_tls_scan &
    TLS_PID=$!
    
    run_header_check &
    HEADER_PID=$!
    
    run_cargo_audit &
    CARGO_PID=$!

    # Wait for parallel scans to complete
    wait $TLS_PID 2>/dev/null || true
    wait $HEADER_PID 2>/dev/null || true
    wait $CARGO_PID 2>/dev/null || true

    echo ""

    # Phase 2: Run sequential scans (these are heavier and more aggressive)
    print_info "Phase 2: Running sequential scans..."
    
    run_nuclei_scan
    run_zap_scan
    run_auth_check
    run_rate_limit_check

    echo ""

    # Phase 3: Generate combined report
    print_info "Phase 3: Generating report..."
    generate_report

    # Print summary
    print_summary
}

# Run main function
main "$@"
