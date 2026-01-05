#!/bin/bash
# =============================================================================
# Security Headers Checker
# =============================================================================
# Analyzes HTTP security headers on all endpoints including:
# - Strict-Transport-Security (HSTS)
# - X-Content-Type-Options
# - X-Frame-Options
# - X-XSS-Protection
# - Content-Security-Policy
# - Referrer-Policy
# - Permissions-Policy
# =============================================================================

set -euo pipefail

TARGET="$1"
OUTPUT_DIR="$2"
AUTH_B64="${3:-}"

RESULTS_JSON="${OUTPUT_DIR}/raw/headers-report.json"

echo "[HEADERS] Checking security headers on ${TARGET}..."

# Required security headers
REQUIRED_HEADERS="Strict-Transport-Security X-Content-Type-Options X-Frame-Options X-XSS-Protection Referrer-Policy"
RECOMMENDED_HEADERS="Content-Security-Policy Permissions-Policy"

# Endpoints to check
PUBLIC_ENDPOINTS="/ /wms?SERVICE=WMS&REQUEST=GetCapabilities /edr /health"
PROTECTED_ENDPOINTS="/admin /api/metrics /grafana/"

# Initialize counters
TOTAL_CHECKS=0
PASSED=0
WARNINGS=0
FAILED=0

# Temp file for collecting endpoint results
ENDPOINTS_TMP=$(mktemp)

check_endpoint() {
    local endpoint="$1"
    local use_auth="$2"
    local url="${TARGET}${endpoint}"
    
    # Fetch headers
    local headers
    if [[ "$use_auth" == "true" && -n "$AUTH_B64" ]]; then
        headers=$(curl -sI -L --max-redirs 3 --max-time 10 -H "Authorization: Basic ${AUTH_B64}" "$url" 2>/dev/null | tr -d '\r') || headers=""
    else
        headers=$(curl -sI -L --max-redirs 3 --max-time 10 "$url" 2>/dev/null | tr -d '\r') || headers=""
    fi

    local http_status=$(echo "$headers" | grep -E "^HTTP/" | tail -1 | awk '{print $2}')
    http_status="${http_status:-0}"
    
    # Start building endpoint JSON
    local checks_json=""
    local headers_json=""
    
    # Check required headers
    for header in $REQUIRED_HEADERS; do
        local value=$(echo "$headers" | grep -i "^${header}:" | head -1 | cut -d: -f2- | xargs 2>/dev/null || echo "")
        local status="missing"
        local severity="high"
        
        ((TOTAL_CHECKS++)) || true
        
        if [[ -n "$value" ]]; then
            status="pass"
            severity="info"
            ((PASSED++)) || true
        else
            ((FAILED++)) || true
        fi
        
        # Escape quotes in value
        value=$(echo "$value" | sed 's/"/\\"/g')
        
        [[ -n "$headers_json" ]] && headers_json+=","
        headers_json+="\"${header}\": \"${value:-null}\""
        
        [[ -n "$checks_json" ]] && checks_json+=","
        checks_json+="{\"header\": \"${header}\", \"status\": \"${status}\", \"severity\": \"${severity}\", \"value\": \"${value:-missing}\"}"
    done
    
    # Check recommended headers
    for header in $RECOMMENDED_HEADERS; do
        local value=$(echo "$headers" | grep -i "^${header}:" | head -1 | cut -d: -f2- | xargs 2>/dev/null || echo "")
        local status="warning"
        local severity="low"
        
        ((TOTAL_CHECKS++)) || true
        
        if [[ -n "$value" ]]; then
            status="pass"
            severity="info"
            ((PASSED++)) || true
        else
            ((WARNINGS++)) || true
        fi
        
        value=$(echo "$value" | sed 's/"/\\"/g')
        
        [[ -n "$headers_json" ]] && headers_json+=","
        headers_json+="\"${header}\": \"${value:-null}\""
        
        [[ -n "$checks_json" ]] && checks_json+=","
        checks_json+="{\"header\": \"${header}\", \"status\": \"${status}\", \"severity\": \"${severity}\", \"value\": \"${value:-missing}\"}"
    done
    
    echo "{\"endpoint\": \"${endpoint}\", \"status\": ${http_status}, \"headers\": {${headers_json}}, \"checks\": [${checks_json}]}"
}

# Check all endpoints
FIRST=true
for endpoint in $PUBLIC_ENDPOINTS; do
    [[ "$FIRST" != "true" ]] && echo "," >> "$ENDPOINTS_TMP"
    FIRST=false
    check_endpoint "$endpoint" "false" >> "$ENDPOINTS_TMP"
done

if [[ -n "$AUTH_B64" ]]; then
    for endpoint in $PROTECTED_ENDPOINTS; do
        echo "," >> "$ENDPOINTS_TMP"
        check_endpoint "$endpoint" "true" >> "$ENDPOINTS_TMP"
    done
fi

# Build final JSON
{
    echo '{'
    echo '  "scan_type": "security_headers",'
    echo '  "target": "'"$TARGET"'",'
    echo '  "timestamp": "'"$(date -Iseconds)"'",'
    echo '  "endpoints": ['
    cat "$ENDPOINTS_TMP"
    echo '  ],'
    echo '  "summary": {'
    echo '    "total": '$TOTAL_CHECKS','
    echo '    "passed": '$PASSED','
    echo '    "warnings": '$WARNINGS','
    echo '    "failed": '$FAILED
    echo '  }'
    echo '}'
} > "$RESULTS_JSON"

rm -f "$ENDPOINTS_TMP"

echo "[HEADERS] Results: ${PASSED} passed, ${WARNINGS} warnings, ${FAILED} failed (${TOTAL_CHECKS} total checks)"
echo "[HEADERS] Report saved to ${RESULTS_JSON}"
