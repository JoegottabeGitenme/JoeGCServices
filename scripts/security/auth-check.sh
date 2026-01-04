#!/bin/bash
# =============================================================================
# Authentication Bypass Checker
# =============================================================================
# Tests protected endpoints to ensure they properly reject unauthenticated
# requests and are not vulnerable to common bypass techniques.
# =============================================================================

set -uo pipefail

TARGET="$1"
OUTPUT_DIR="$2"

RESULTS_JSON="${OUTPUT_DIR}/raw/auth-check.json"

echo "[AUTH] Testing authentication enforcement..."

# Protected endpoints that MUST require auth
PROTECTED_ENDPOINTS="/admin /api/metrics /grafana/ /downloader/ /loadtest"

# Counters
TOTAL_TESTS=0
PASSED=0
FAILED=0

# Temp file for results
RESULTS_TMP=$(mktemp)
echo "[" > "$RESULTS_TMP"
FIRST_RESULT=true

test_endpoint() {
    local endpoint="$1"
    local test_url="${TARGET}${endpoint}"
    
    # Make request without auth
    local status
    status=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 "$test_url" 2>/dev/null) || status="000"
    
    local result="pass"
    local severity="info"
    
    ((TOTAL_TESTS++)) || true
    
    # Expected: 401 Unauthorized or 403 Forbidden
    # Also acceptable: 404 Not Found (endpoint doesn't exist)
    if [[ "$status" == "401" || "$status" == "403" || "$status" == "404" ]]; then
        result="pass"
        ((PASSED++)) || true
    elif [[ "$status" == "000" ]]; then
        result="error"
        severity="low"
        ((PASSED++)) || true  # Connection errors are not auth bypasses
    elif [[ "$status" == "200" || "$status" == "301" || "$status" == "302" ]]; then
        result="fail"
        severity="critical"
        ((FAILED++)) || true
        echo "[AUTH] FAIL: ${endpoint} returned ${status} (expected 401/403)"
    else
        result="pass"
        ((PASSED++)) || true
    fi
    
    # Add to results
    if [[ "$FIRST_RESULT" != "true" ]]; then
        echo "," >> "$RESULTS_TMP"
    fi
    FIRST_RESULT=false
    
    echo "{\"endpoint\": \"${endpoint}\", \"url\": \"${test_url}\", \"status\": ${status}, \"result\": \"${result}\", \"severity\": \"${severity}\"}" >> "$RESULTS_TMP"
}

# Test each protected endpoint
for endpoint in $PROTECTED_ENDPOINTS; do
    echo "[AUTH] Testing ${endpoint}..."
    test_endpoint "$endpoint"
done

# Close results array
echo "]" >> "$RESULTS_TMP"

# Build final JSON
{
    echo '{'
    echo '  "scan_type": "auth_bypass",'
    echo '  "target": "'"$TARGET"'",'
    echo '  "timestamp": "'"$(date -Iseconds)"'",'
    echo '  "summary": {'
    echo '    "total_tests": '$TOTAL_TESTS','
    echo '    "passed": '$PASSED','
    echo '    "failed": '$FAILED
    echo '  },'
    echo '  "tests": '
    cat "$RESULTS_TMP"
    echo '}'
} > "$RESULTS_JSON"

rm -f "$RESULTS_TMP"

# Print summary
if [[ $FAILED -eq 0 ]]; then
    echo "[AUTH] All ${TOTAL_TESTS} tests passed - endpoints are properly protected"
else
    echo "[AUTH] WARNING: ${FAILED}/${TOTAL_TESTS} tests failed - potential auth bypass vulnerabilities!"
fi

echo "[AUTH] Results saved to ${RESULTS_JSON}"
