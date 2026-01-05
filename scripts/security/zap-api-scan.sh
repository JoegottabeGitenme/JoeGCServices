#!/bin/bash
# =============================================================================
# OWASP ZAP API Scan with OpenAPI Specs
# =============================================================================
# Uses ZAP's API scanning mode with OpenAPI specifications to properly test
# WMS, WMTS, and EDR API endpoints. This complements the full spider-based
# scan by using API schema information to:
# - Understand parameter types and formats
# - Generate appropriate test cases for each parameter
# - Skip irrelevant browser-focused checks (XSS in APIs)
# - Properly fuzz query parameters
#
# Requirements:
# - Docker
# - OpenAPI specs in services/edr-api/src/openapi.yaml and docs/api/openapi.yaml
# =============================================================================

set -euo pipefail

TARGET="$1"
OUTPUT_DIR="$2"
AUTH_B64="${3:-}"
PROJECT_ROOT="${4:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

ZAP_OUTPUT_DIR="${OUTPUT_DIR}/zap-api"
mkdir -p "$ZAP_OUTPUT_DIR"

echo "[ZAP-API] Starting API-focused security scan..."
echo "[ZAP-API] Target: ${TARGET}"
echo "[ZAP-API] This supplements the full ZAP scan with OpenAPI-aware testing..."

# Verify OpenAPI specs exist
EDR_OPENAPI="${PROJECT_ROOT}/services/edr-api/src/openapi.yaml"
WMS_OPENAPI="${PROJECT_ROOT}/docs/api/openapi.yaml"

if [[ ! -f "$EDR_OPENAPI" ]]; then
    echo "[ZAP-API] WARNING: EDR OpenAPI spec not found at ${EDR_OPENAPI}"
    EDR_OPENAPI=""
fi

if [[ ! -f "$WMS_OPENAPI" ]]; then
    echo "[ZAP-API] WARNING: WMS OpenAPI spec not found at ${WMS_OPENAPI}"
    WMS_OPENAPI=""
fi

# Create ZAP configuration file for API scanning
ZAP_CONFIG="${ZAP_OUTPUT_DIR}/zap-api-config.conf"
cat > "$ZAP_CONFIG" << 'EOF'
# ZAP API Scan Configuration
# ==========================
# Focus on API-relevant vulnerabilities, disable browser-focused checks

# SQL Injection - Enable with high priority
40018   WARN    (SQL Injection)
40019   WARN    (SQL Injection - MySQL)
40020   WARN    (SQL Injection - Hypersonic SQL)
40021   WARN    (SQL Injection - Oracle)
40022   WARN    (SQL Injection - PostgreSQL)
40024   WARN    (SQL Injection - SQLite)

# Command/Code Injection
90019   WARN    (Server Side Code Injection)
90020   WARN    (Remote OS Command Injection)

# Path Traversal / LFI
6       WARN    (Path Traversal)
40003   WARN    (Directory Browsing)

# SSRF / External Service Interaction
40046   WARN    (Server Side Request Forgery)

# Authentication Issues
10011   WARN    (Cookie Without Secure Flag)
10054   WARN    (Cookie without SameSite Attribute)
10055   WARN    (CSP)
10056   WARN    (X-Debug-Token)

# Information Disclosure
10023   WARN    (Information Disclosure - Debug Error Messages)
10024   WARN    (Information Disclosure - Sensitive Information in URL)
10025   WARN    (Information Disclosure - Sensitive Information in HTTP Referrer Header)
10027   WARN    (Information Disclosure - Suspicious Comments)
10032   WARN    (Viewstate)
10035   WARN    (Strict-Transport-Security Header)
10036   WARN    (Server Leaks Version Information)
10037   WARN    (Server Leaks Information via X-Powered-By)
10038   WARN    (Content Security Policy Header Not Set)
10039   WARN    (X-Backend-Server Header)
10040   WARN    (Secure Pages Include Mixed Content)
10041   WARN    (HTTP to HTTPS Insecure Transition in Form Post)
10042   WARN    (HTTPS to HTTP Insecure Transition in Form Post)
10043   WARN    (User Controllable JavaScript Event - XSS)
10050   WARN    (Retrieved from Cache)
10052   WARN    (X-ChromeLogger-Data Header)
10054   WARN    (Cookie Without SameSite)
10055   WARN    (CSP Scanner)
10057   WARN    (Username Hash Found)
10061   WARN    (X-AspNet-Version Response Header)
10062   WARN    (PII Disclosure)
10096   WARN    (Timestamp Disclosure)
10097   WARN    (Hash Disclosure)
10098   WARN    (Cross-Domain Misconfiguration)

# Disable XSS checks for pure APIs (not rendering HTML)
40012   IGNORE  (Cross Site Scripting - Reflected)
40014   IGNORE  (Cross Site Scripting - Persistent)
40016   IGNORE  (Cross Site Scripting - Persistent - Prime)
40017   IGNORE  (Cross Site Scripting - Persistent - Spider)

# Disable CSRF for stateless APIs
10202   IGNORE  (Absence of Anti-CSRF Tokens)

# Disable clickjacking for APIs
10020   IGNORE  (X-Frame-Options Header)
EOF

echo "[ZAP-API] Created configuration file"

# =============================================================================
# Scan EDR API with OpenAPI spec
# =============================================================================
if [[ -n "$EDR_OPENAPI" ]]; then
    echo "[ZAP-API] Scanning EDR API with OpenAPI specification..."
    
    # Determine target URL for EDR (override spec's localhost with actual target)
    EDR_TARGET="${TARGET}/edr"
    
    timeout 35m docker run --rm \
        -v "${ZAP_OUTPUT_DIR}:/zap/wrk:rw" \
        -v "${EDR_OPENAPI}:/zap/edr-openapi.yaml:ro" \
        -v "${ZAP_CONFIG}:/zap/config.conf:ro" \
        --network host \
        -t ghcr.io/zaproxy/zaproxy:stable \
        zap-api-scan.py \
            -t "/zap/edr-openapi.yaml" \
            -f openapi \
            -O "${EDR_TARGET}" \
            -c "/zap/config.conf" \
            -r "zap-edr-api-report.html" \
            -J "zap-edr-api-report.json" \
            -w "zap-edr-api-report.md" \
            -z "-config api.disablekey=true" \
            -I \
            -T 30 \
            2>&1 | tee "${OUTPUT_DIR}/raw/zap-api-edr-scan.log" || {
                echo "[ZAP-API] EDR scan completed or timed out"
            }
    
    echo "[ZAP-API] EDR API scan complete"
fi

# =============================================================================
# Scan WMS/WMTS API with OpenAPI spec
# =============================================================================
if [[ -n "$WMS_OPENAPI" ]]; then
    echo "[ZAP-API] Scanning WMS/WMTS API with OpenAPI specification..."
    
    timeout 35m docker run --rm \
        -v "${ZAP_OUTPUT_DIR}:/zap/wrk:rw" \
        -v "${WMS_OPENAPI}:/zap/wms-openapi.yaml:ro" \
        -v "${ZAP_CONFIG}:/zap/config.conf:ro" \
        --network host \
        -t ghcr.io/zaproxy/zaproxy:stable \
        zap-api-scan.py \
            -t "/zap/wms-openapi.yaml" \
            -f openapi \
            -O "${TARGET}" \
            -c "/zap/config.conf" \
            -r "zap-wms-api-report.html" \
            -J "zap-wms-api-report.json" \
            -w "zap-wms-api-report.md" \
            -z "-config api.disablekey=true" \
            -I \
            -T 30 \
            2>&1 | tee "${OUTPUT_DIR}/raw/zap-api-wms-scan.log" || {
                echo "[ZAP-API] WMS scan completed or timed out"
            }
    
    echo "[ZAP-API] WMS/WMTS API scan complete"
fi

# =============================================================================
# Authenticated API scan (if credentials provided)
# =============================================================================
if [[ -n "$AUTH_B64" ]]; then
    echo "[ZAP-API] Scanning authenticated endpoints..."
    
    # Create authenticated target list
    cat > "${ZAP_OUTPUT_DIR}/auth-targets.txt" << EOF
${TARGET}/api/metrics
${TARGET}/api/config
${TARGET}/api/stats
${TARGET}/admin
${TARGET}/loadtest
EOF

    # Note: zap-api-scan.py doesn't support direct URL lists, 
    # so we scan the main APIs with auth headers for protected endpoints
    if [[ -n "$WMS_OPENAPI" ]]; then
        docker run --rm \
            -v "${ZAP_OUTPUT_DIR}:/zap/wrk:rw" \
            -v "${WMS_OPENAPI}:/zap/wms-openapi.yaml:ro" \
            -v "${ZAP_CONFIG}:/zap/config.conf:ro" \
            --network host \
            -t ghcr.io/zaproxy/zaproxy:stable \
            zap-api-scan.py \
                -t "/zap/wms-openapi.yaml" \
                -f openapi \
                -O "${TARGET}" \
                -c "/zap/config.conf" \
                -r "zap-api-auth-report.html" \
                -J "zap-api-auth-report.json" \
                -z "-config api.disablekey=true \
                    -config replacer.full_list(0).description=BasicAuth \
                    -config replacer.full_list(0).enabled=true \
                    -config replacer.full_list(0).matchtype=REQ_HEADER \
                    -config replacer.full_list(0).matchstr=Authorization \
                    -config replacer.full_list(0).regex=false \
                    -config replacer.full_list(0).replacement='Basic ${AUTH_B64}'" \
                -I \
                -T 15 \
                2>&1 | tee "${OUTPUT_DIR}/raw/zap-api-auth-scan.log" || true
    fi
fi

# =============================================================================
# Combine results
# =============================================================================
echo "[ZAP-API] Combining scan results..."

# Count findings from all JSON reports
count_json_findings() {
    local json_file="$1"
    if [[ -f "$json_file" ]]; then
        local high=$(jq '[.site[]?.alerts[]? | select(.riskcode == "3")] | length' "$json_file" 2>/dev/null || echo "0")
        local medium=$(jq '[.site[]?.alerts[]? | select(.riskcode == "2")] | length' "$json_file" 2>/dev/null || echo "0")
        local low=$(jq '[.site[]?.alerts[]? | select(.riskcode == "1")] | length' "$json_file" 2>/dev/null || echo "0")
        local info=$(jq '[.site[]?.alerts[]? | select(.riskcode == "0")] | length' "$json_file" 2>/dev/null || echo "0")
        echo "${high:-0} ${medium:-0} ${low:-0} ${info:-0}"
    else
        echo "0 0 0 0"
    fi
}

TOTAL_HIGH=0
TOTAL_MED=0
TOTAL_LOW=0
TOTAL_INFO=0

for report in "${ZAP_OUTPUT_DIR}"/*.json; do
    if [[ -f "$report" ]]; then
        COUNTS=$(count_json_findings "$report")
        read -r H M L I <<< "$COUNTS"
        TOTAL_HIGH=$((TOTAL_HIGH + H))
        TOTAL_MED=$((TOTAL_MED + M))
        TOTAL_LOW=$((TOTAL_LOW + L))
        TOTAL_INFO=$((TOTAL_INFO + I))
    fi
done

echo "[ZAP-API] Total findings: ${TOTAL_HIGH} high, ${TOTAL_MED} medium, ${TOTAL_LOW} low, ${TOTAL_INFO} info"

# Create summary JSON
cat > "${OUTPUT_DIR}/raw/zap-api-summary.json" << EOF
{
    "scan_type": "zap_api_scan",
    "target": "${TARGET}",
    "timestamp": "$(date -Iseconds)",
    "openapi_specs": {
        "edr": "${EDR_OPENAPI:-not found}",
        "wms": "${WMS_OPENAPI:-not found}"
    },
    "findings": {
        "high": ${TOTAL_HIGH},
        "medium": ${TOTAL_MED},
        "low": ${TOTAL_LOW},
        "info": ${TOTAL_INFO}
    },
    "reports": {
        "edr": "zap-api/zap-edr-api-report.html",
        "wms": "zap-api/zap-wms-api-report.html",
        "auth": "zap-api/zap-api-auth-report.html"
    }
}
EOF

echo "[ZAP-API] Reports saved to ${ZAP_OUTPUT_DIR}/"
echo "[ZAP-API] API scan complete"
