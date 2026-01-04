#!/bin/bash
# =============================================================================
# Nuclei Vulnerability Scanner
# =============================================================================
# Fast template-based vulnerability scanner using ProjectDiscovery Nuclei.
# Scans for CVEs, misconfigurations, exposures, and common vulnerabilities.
# =============================================================================

set -euo pipefail

TARGET="$1"
OUTPUT_DIR="$2"
AUTH_B64="${3:-}"

TARGETS_FILE="${OUTPUT_DIR}/raw/nuclei-targets.txt"
RESULTS_JSON="${OUTPUT_DIR}/raw/nuclei-report.json"
MARKDOWN_DIR="${OUTPUT_DIR}/nuclei"

echo "[NUCLEI] Preparing target URLs..."

# Generate target URLs list
cat > "$TARGETS_FILE" << EOF
${TARGET}/
${TARGET}/wms?SERVICE=WMS&REQUEST=GetCapabilities
${TARGET}/wmts?SERVICE=WMTS&REQUEST=GetCapabilities
${TARGET}/edr
${TARGET}/edr/collections
${TARGET}/health
${TARGET}/admin
${TARGET}/api/metrics
${TARGET}/grafana/
${TARGET}/downloader/
${TARGET}/loadtest
EOF

echo "[NUCLEI] Starting vulnerability scan with $(wc -l < "$TARGETS_FILE") target URLs..."

# Build auth header if available
AUTH_ARGS=""
if [[ -n "$AUTH_B64" ]]; then
    AUTH_ARGS="-H 'Authorization: Basic ${AUTH_B64}'"
fi

# Run Nuclei via Docker
# Tags: cve, owasp, misconfig, exposure, xss, sqli, rce, lfi, ssrf
docker run --rm \
    -v "${OUTPUT_DIR}:/output" \
    projectdiscovery/nuclei:latest \
    -l "/output/raw/nuclei-targets.txt" \
    -H "Authorization: Basic ${AUTH_B64}" \
    -tags cve,owasp,misconfig,exposure,xss,sqli,rce,lfi,ssrf,creds-disclosure,token-spray \
    -severity critical,high,medium,low,info \
    -rl 50 \
    -c 10 \
    -timeout 15 \
    -retries 2 \
    -stats \
    -nc \
    -je "/output/raw/nuclei-report.json" \
    -me "/output/nuclei/" \
    2>&1 | tee "${OUTPUT_DIR}/raw/nuclei-scan.log" || true

# Count findings by severity
if [[ -f "$RESULTS_JSON" ]]; then
    CRITICAL=$(grep -c '"severity":"critical"' "$RESULTS_JSON" 2>/dev/null || echo "0")
    HIGH=$(grep -c '"severity":"high"' "$RESULTS_JSON" 2>/dev/null || echo "0")
    MEDIUM=$(grep -c '"severity":"medium"' "$RESULTS_JSON" 2>/dev/null || echo "0")
    LOW=$(grep -c '"severity":"low"' "$RESULTS_JSON" 2>/dev/null || echo "0")
    INFO=$(grep -c '"severity":"info"' "$RESULTS_JSON" 2>/dev/null || echo "0")
    
    echo "[NUCLEI] Findings: ${CRITICAL} critical, ${HIGH} high, ${MEDIUM} medium, ${LOW} low, ${INFO} info"
else
    echo "[NUCLEI] No findings file generated (may indicate no vulnerabilities found)"
    # Create empty results file
    echo '[]' > "$RESULTS_JSON"
fi

echo "[NUCLEI] Results saved to ${OUTPUT_DIR}/nuclei/"
