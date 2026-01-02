#!/bin/bash
#
# Test script to verify the ETS EDRGEOJSON bug fix
#
# This script:
# 1. Clones the ets-ogcapi-edr10 repository
# 2. Applies the fix (req/ -> conf/)
# 3. Builds the fixed JAR
# 4. Runs tests with both original and fixed JARs
# 5. Compares results to prove the fix works
#
# Requirements:
#   - Java 17+
#   - Maven 3.x
#   - EDR API running on localhost:8083
#
# Usage:
#   ./test_ets_fix.sh

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK_DIR="/tmp/ets-fix-test"
ETS_REPO="https://github.com/opengeospatial/ets-ogcapi-edr10.git"
EDR_API_URL="${EDR_API_URL:-http://localhost:8083/edr}"

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }
log_header() { echo -e "\n${BOLD}$1${NC}\n$(printf '=%.0s' $(seq 1 ${#1}))"; }

# Check prerequisites
check_prerequisites() {
    log_header "Checking Prerequisites"
    
    if ! command -v java &>/dev/null; then
        log_error "Java is not installed"
        exit 1
    fi
    log_success "Java: $(java -version 2>&1 | head -1)"
    
    if ! command -v mvn &>/dev/null; then
        log_error "Maven is not installed"
        exit 1
    fi
    log_success "Maven: $(mvn --version | head -1)"
    
    if ! curl -sf "$EDR_API_URL" > /dev/null 2>&1; then
        log_error "EDR API is not running at $EDR_API_URL"
        log_error "Start it with: cargo run --release -p edr-api"
        exit 1
    fi
    log_success "EDR API is running at $EDR_API_URL"
    
    # Check if API declares edr-geojson conformance
    if ! curl -sf "$EDR_API_URL/conformance" | grep -q "conf/edr-geojson"; then
        log_error "EDR API does not declare conf/edr-geojson conformance class"
        exit 1
    fi
    log_success "EDR API declares conf/edr-geojson conformance"
}

# Setup working directory
setup_workdir() {
    log_header "Setting Up Working Directory"
    
    rm -rf "$WORK_DIR"
    mkdir -p "$WORK_DIR"/{original,fixed}
    log_success "Created $WORK_DIR"
}

# Clone and fix the ETS repository
clone_and_fix() {
    log_header "Cloning ETS Repository"
    
    cd "$WORK_DIR"
    git clone --depth 1 "$ETS_REPO" ets-ogcapi-edr10
    log_success "Cloned ets-ogcapi-edr10"
    
    log_info "Applying fix (req/edr-geojson -> conf/edr-geojson)..."
    
    REQUIREMENT_CLASS_FILE="ets-ogcapi-edr10/src/main/java/org/opengis/cite/ogcapiedr10/conformance/RequirementClass.java"
    
    # Show the bug
    echo -e "${YELLOW}Before fix:${NC}"
    grep -n "EDRGEOJSON" "$REQUIREMENT_CLASS_FILE"
    
    # Apply the fix using sed
    sed -i 's|/req/edr-geojson|/conf/edr-geojson|g' "$REQUIREMENT_CLASS_FILE"
    
    # Verify the fix
    echo -e "${GREEN}After fix:${NC}"
    grep -n "EDRGEOJSON" "$REQUIREMENT_CLASS_FILE"
    
    log_success "Fix applied"
}

# Build the fixed JAR
build_fixed_jar() {
    log_header "Building Fixed JAR"
    
    cd "$WORK_DIR/ets-ogcapi-edr10"
    mvn clean package -DskipTests -q
    
    FIXED_JAR="$WORK_DIR/ets-ogcapi-edr10/target/ets-ogcapi-edr10-1.4-SNAPSHOT-aio.jar"
    if [[ ! -f "$FIXED_JAR" ]]; then
        log_error "Failed to build JAR"
        exit 1
    fi
    
    log_success "Built: $FIXED_JAR ($(du -h "$FIXED_JAR" | cut -f1))"
    
    # Verify the fix is in the compiled class
    log_info "Verifying fix in compiled JAR..."
    if unzip -p "$FIXED_JAR" org/opengis/cite/ogcapiedr10/conformance/RequirementClass.class | strings | grep -q "conf/edr-geojson"; then
        log_success "Fix verified in compiled class"
    else
        log_error "Fix not found in compiled class"
        exit 1
    fi
}

# Create test properties file
create_test_props() {
    cat > "$WORK_DIR/test-props.xml" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE properties SYSTEM "http://java.sun.com/dtd/properties.dtd">
<properties version="1.0">
    <comment>OGC API EDR 1.0 Test Run Configuration</comment>
    <entry key="iut">${EDR_API_URL}</entry>
    <entry key="apiDefinition">${EDR_API_URL}/api</entry>
    <entry key="noofcollections">1</entry>
</properties>
EOF
    log_success "Created test properties: $WORK_DIR/test-props.xml"
}

# Run tests with a specific JAR and return results file path
run_tests() {
    local jar_path="$1"
    local output_dir="$2"
    local label="$3"
    
    log_info "Running tests with $label JAR..." >&2
    
    java -jar "$jar_path" -o "$output_dir" "$WORK_DIR/test-props.xml" > "$output_dir/test-output.log" 2>&1 || true
    
    local results_file
    results_file=$(find "$output_dir" -name "testng-results.xml" -type f 2>/dev/null | head -1)
    
    if [[ -z "$results_file" ]]; then
        echo "ERROR: No results file found" >&2
        return 1
    fi
    
    # Return just the path, nothing else
    printf '%s' "$results_file"
}

# Extract and display test result
extract_result() {
    local results_file="$1"
    local label="$2"
    
    # Extract the full test-method line for validateResponseForEDRGeoJSON
    local test_line
    test_line=$(grep 'name="validateResponseForEDRGeoJSON"' "$results_file" 2>/dev/null | head -1)
    
    if [[ -z "$test_line" ]]; then
        echo "$label|NOT_FOUND|0|Test not found in results"
        return
    fi
    
    # Extract status (handles any attribute order)
    local status
    status=$(echo "$test_line" | grep -oP 'status="\K[^"]+' || echo "UNKNOWN")
    
    # Extract duration
    local duration
    duration=$(echo "$test_line" | grep -oP 'duration-ms="\K[^"]+' || echo "0")
    
    local message=""
    if [[ "$status" == "SKIP" ]]; then
        message=$(grep -A5 'name="validateResponseForEDRGeoJSON"' "$results_file" | grep -oP 'CDATA\[\K[^\]]+' | head -1 || echo "")
    elif [[ "$status" == "FAIL" ]]; then
        message=$(grep -A15 'name="validateResponseForEDRGeoJSON"' "$results_file" | grep -oP 'CDATA\[\K[^\]]+' | head -1 || echo "Test executed")
    elif [[ "$status" == "PASS" ]]; then
        message="Test passed"
    fi
    
    echo "$label|$status|$duration|$message"
}

# Compare results
compare_results() {
    log_header "Test Results Comparison"
    
    local original_result="$1"
    local fixed_result="$2"
    
    IFS='|' read -r orig_label orig_status orig_duration orig_message <<< "$original_result"
    IFS='|' read -r fix_label fix_status fix_duration fix_message <<< "$fixed_result"
    
    echo ""
    echo -e "${BOLD}validateResponseForEDRGeoJSON Test Results:${NC}"
    echo ""
    printf "%-20s %-10s %-15s %s\n" "JAR" "Status" "Duration (ms)" "Message"
    printf "%-20s %-10s %-15s %s\n" "---" "------" "-------------" "-------"
    
    # Original result
    if [[ "$orig_status" == "SKIP" ]]; then
        printf "%-20s ${YELLOW}%-10s${NC} %-15s %s\n" "Original (buggy)" "$orig_status" "$orig_duration" "${orig_message:0:50}..."
    else
        printf "%-20s %-10s %-15s %s\n" "Original (buggy)" "$orig_status" "$orig_duration" "$orig_message"
    fi
    
    # Fixed result
    if [[ "$fix_status" == "SKIP" ]]; then
        printf "%-20s ${YELLOW}%-10s${NC} %-15s %s\n" "Fixed" "$fix_status" "$fix_duration" "$fix_message"
    elif [[ "$fix_status" == "PASS" ]]; then
        printf "%-20s ${GREEN}%-10s${NC} %-15s %s\n" "Fixed" "$fix_status" "$fix_duration" "$fix_message"
    elif [[ "$fix_status" == "FAIL" ]]; then
        printf "%-20s ${RED}%-10s${NC} %-15s %s\n" "Fixed" "$fix_status" "$fix_duration" "Test executed (implementation issue)"
    fi
    
    echo ""
    
    # Verdict
    if [[ "$orig_status" == "SKIP" && "$fix_status" != "SKIP" ]]; then
        echo -e "${GREEN}${BOLD}SUCCESS: The fix works!${NC}"
        echo ""
        echo "The test was SKIPPED with the original JAR (due to the req/ vs conf/ bug)"
        echo "but EXECUTES with the fixed JAR (duration: ${fix_duration}ms)."
        echo ""
        if [[ "$fix_status" == "FAIL" ]]; then
            echo -e "${YELLOW}Note: The test failure is expected - it means the test is now"
            echo -e "actually running and validating EDR GeoJSON conformance.${NC}"
        fi
        return 0
    else
        echo -e "${RED}${BOLD}UNEXPECTED: Results don't match expected pattern${NC}"
        echo "Original status: $orig_status"
        echo "Fixed status: $fix_status"
        return 1
    fi
}

# Main
main() {
    echo ""
    echo "========================================================"
    echo "    ETS EDRGEOJSON Bug Fix Verification Script"
    echo "========================================================"
    echo ""
    
    check_prerequisites
    setup_workdir
    clone_and_fix
    build_fixed_jar
    create_test_props
    
    log_header "Running Tests"
    
    # Check for original JAR
    ORIGINAL_JAR="$SCRIPT_DIR/lib/ets-ogcapi-edr10-1.3-aio.jar"
    if [[ ! -f "$ORIGINAL_JAR" ]]; then
        log_info "Original JAR not found, downloading..."
        mkdir -p "$SCRIPT_DIR/lib"
        curl -fSL -o "$ORIGINAL_JAR" \
            "https://repo1.maven.org/maven2/org/opengis/cite/ets-ogcapi-edr10/1.3/ets-ogcapi-edr10-1.3-aio.jar"
    fi
    
    # Run with original JAR
    log_info "Running tests with original (buggy) JAR..."
    java -jar "$ORIGINAL_JAR" -o "$WORK_DIR/original" "$WORK_DIR/test-props.xml" > "$WORK_DIR/original/test-output.log" 2>&1 || true
    original_results=$(find "$WORK_DIR/original" -name "testng-results.xml" -type f 2>/dev/null | head -1)
    
    # Run with fixed JAR
    FIXED_JAR="$WORK_DIR/ets-ogcapi-edr10/target/ets-ogcapi-edr10-1.4-SNAPSHOT-aio.jar"
    log_info "Running tests with fixed JAR..."
    java -jar "$FIXED_JAR" -o "$WORK_DIR/fixed" "$WORK_DIR/test-props.xml" > "$WORK_DIR/fixed/test-output.log" 2>&1 || true
    fixed_results=$(find "$WORK_DIR/fixed" -name "testng-results.xml" -type f 2>/dev/null | head -1)
    
    # Debug: show the results files found
    log_info "Original results: $original_results"
    log_info "Fixed results: $fixed_results"
    
    # Extract results
    original_result=$(extract_result "$original_results" "Original")
    fixed_result=$(extract_result "$fixed_results" "Fixed")
    
    # Compare
    compare_results "$original_result" "$fixed_result"
    
    echo ""
    echo "Full results available at: $WORK_DIR"
    echo ""
}

main "$@"
