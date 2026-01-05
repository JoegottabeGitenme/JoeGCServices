#!/bin/bash
# =============================================================================
# Nuclei Vulnerability Scanner with Custom OGC Templates
# =============================================================================
# Fast template-based vulnerability scanner using ProjectDiscovery Nuclei.
# 
# Features:
# - Standard Nuclei templates for common vulnerabilities (TIME-LIMITED)
# - Custom OGC WMS/WMTS/EDR templates for API-specific testing
# - Dynamic endpoint discovery from GetCapabilities/collections
# - Optional interactsh integration for OOB detection (HTTP-only mode)
# - Time-based blind SQL injection detection
#
# Performance optimizations:
# - Maximum 15 minutes per scan phase
# - Limited template count to prevent runaway scans
# - Separate base URL scan from discovered endpoint scan
#
# Custom templates location: scripts/security/nuclei-templates/
# =============================================================================

set -euo pipefail

TARGET="$1"
OUTPUT_DIR="$2"
AUTH_B64="${3:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CUSTOM_TEMPLATES="${SCRIPT_DIR}/nuclei-templates"

TARGETS_FILE="${OUTPUT_DIR}/raw/nuclei-targets.txt"
BASE_TARGETS_FILE="${OUTPUT_DIR}/raw/nuclei-base-targets.txt"
RESULTS_JSON="${OUTPUT_DIR}/raw/nuclei-report.json"
RESULTS_JSONL="${OUTPUT_DIR}/raw/nuclei-results.jsonl"
MARKDOWN_DIR="${OUTPUT_DIR}/nuclei"

# Time limits (in minutes)
STANDARD_SCAN_TIMEOUT=15
OGC_SCAN_TIMEOUT=10
OOB_SCAN_TIMEOUT=5

echo "[NUCLEI] Starting vulnerability scan with custom OGC templates..."
echo "[NUCLEI] Time limits: Standard=${STANDARD_SCAN_TIMEOUT}m, OGC=${OGC_SCAN_TIMEOUT}m, OOB=${OOB_SCAN_TIMEOUT}m"

# =============================================================================
# Phase 1: Prepare target lists
# =============================================================================
echo "[NUCLEI] Phase 1: Preparing target lists..."

# Create base targets (minimal list for comprehensive template scanning)
cat > "$BASE_TARGETS_FILE" << EOF
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

echo "[NUCLEI] Base targets: $(wc -l < "$BASE_TARGETS_FILE") URLs"

# Run discovery for extended targets (used only for custom OGC templates)
if [[ -x "${SCRIPT_DIR}/discover-ogc-endpoints.sh" ]]; then
    "${SCRIPT_DIR}/discover-ogc-endpoints.sh" "$TARGET" "$OUTPUT_DIR" || true
fi

# Use discovered targets for OGC-specific scans
if [[ -f "${OUTPUT_DIR}/ogc-targets.txt" ]]; then
    cp "${OUTPUT_DIR}/ogc-targets.txt" "$TARGETS_FILE"
    EXTENDED_TARGET_COUNT=$(wc -l < "$TARGETS_FILE")
    echo "[NUCLEI] Extended targets (for OGC templates): ${EXTENDED_TARGET_COUNT} URLs"
else
    cp "$BASE_TARGETS_FILE" "$TARGETS_FILE"
    echo "[NUCLEI] Using base targets for all scans"
fi

# =============================================================================
# Phase 2: Run standard Nuclei templates (FAST - base targets only)
# =============================================================================
echo "[NUCLEI] Phase 2: Running standard vulnerability templates (max ${STANDARD_SCAN_TIMEOUT} minutes)..."
echo "[NUCLEI] Using base targets only to keep scan time reasonable..."

# Run with timeout wrapper and more restrictive settings
timeout ${STANDARD_SCAN_TIMEOUT}m docker run --rm \
    -v "${OUTPUT_DIR}:/output" \
    --network host \
    projectdiscovery/nuclei:latest \
    -l "/output/raw/nuclei-base-targets.txt" \
    -H "Authorization: Basic ${AUTH_B64}" \
    -tags cve,exposure,misconfig \
    -etags dos,fuzz,intrusive,brute-force \
    -severity critical,high,medium \
    -tc 200 \
    -rl 100 \
    -c 25 \
    -timeout 10 \
    -retries 1 \
    -stats \
    -si 30 \
    -nc \
    -jsonl \
    -o "/output/raw/nuclei-standard.jsonl" \
    2>&1 | tee "${OUTPUT_DIR}/raw/nuclei-standard-scan.log" || {
        echo "[NUCLEI] Standard scan completed or timed out after ${STANDARD_SCAN_TIMEOUT} minutes"
    }

echo "[NUCLEI] Standard templates phase complete"

# =============================================================================
# Phase 3: Run focused security templates (injection, traversal, ssrf)
# =============================================================================
echo "[NUCLEI] Phase 3: Running focused security templates..."

timeout 10m docker run --rm \
    -v "${OUTPUT_DIR}:/output" \
    --network host \
    projectdiscovery/nuclei:latest \
    -l "/output/raw/nuclei-base-targets.txt" \
    -H "Authorization: Basic ${AUTH_B64}" \
    -tags sqli,lfi,ssrf,rce,xxe,ssti,injection \
    -etags dos,fuzz,intrusive \
    -severity critical,high,medium \
    -tc 100 \
    -rl 50 \
    -c 10 \
    -timeout 15 \
    -retries 1 \
    -stats \
    -si 30 \
    -nc \
    -jsonl \
    -o "/output/raw/nuclei-injection.jsonl" \
    2>&1 | tee "${OUTPUT_DIR}/raw/nuclei-injection-scan.log" || {
        echo "[NUCLEI] Injection scan completed or timed out"
    }

echo "[NUCLEI] Focused security templates phase complete"

# =============================================================================
# Phase 4: Run custom OGC templates (uses BASE URL only - templates have full paths)
# =============================================================================
echo "[NUCLEI] Phase 4: Running custom OGC API templates (max ${OGC_SCAN_TIMEOUT} minutes)..."

if [[ -d "$CUSTOM_TEMPLATES" ]]; then
    TEMPLATE_COUNT=$(find "$CUSTOM_TEMPLATES" -name "*.yaml" | wc -l)
    echo "[NUCLEI] Found ${TEMPLATE_COUNT} custom OGC templates"
    
    # IMPORTANT: Custom OGC templates use {{BaseURL}}/wms, {{BaseURL}}/edr/... paths
    # So we must only pass the BASE site URL, not the full endpoint URLs
    # Otherwise we get double paths like /edr/collections/edr/collections/...
    OGC_BASE_TARGET="${OUTPUT_DIR}/raw/nuclei-ogc-base.txt"
    echo "${TARGET}/" > "$OGC_BASE_TARGET"
    echo "[NUCLEI] Using base URL only for OGC templates: ${TARGET}/"
    
    # Run custom templates - these are specifically designed for our API
    timeout ${OGC_SCAN_TIMEOUT}m docker run --rm \
        -v "${OUTPUT_DIR}:/output" \
        -v "${CUSTOM_TEMPLATES}:/custom-templates:ro" \
        --network host \
        projectdiscovery/nuclei:latest \
        -l "/output/raw/nuclei-ogc-base.txt" \
        -t "/custom-templates/" \
        -H "Authorization: Basic ${AUTH_B64}" \
        -severity critical,high,medium,low,info \
        -rl 30 \
        -c 5 \
        -timeout 45 \
        -retries 1 \
        -stats \
        -si 15 \
        -nc \
        -jsonl \
        -o "/output/raw/nuclei-ogc.jsonl" \
        2>&1 | tee "${OUTPUT_DIR}/raw/nuclei-ogc-scan.log" || {
            echo "[NUCLEI] OGC scan completed or timed out after ${OGC_SCAN_TIMEOUT} minutes"
        }
    
    echo "[NUCLEI] Custom OGC templates scan complete"
else
    echo "[NUCLEI] WARNING: Custom templates directory not found at ${CUSTOM_TEMPLATES}"
    touch "${OUTPUT_DIR}/raw/nuclei-ogc.jsonl"
fi

# =============================================================================
# Phase 5: Run interactsh-enabled templates (HTTP-only OOB)
# =============================================================================
echo "[NUCLEI] Phase 5: Running OOB detection templates (max ${OOB_SCAN_TIMEOUT} minutes)..."

if [[ -d "$CUSTOM_TEMPLATES" ]]; then
    # Use base URL only for OOB templates (same reason as Phase 4)
    OGC_BASE_TARGET="${OUTPUT_DIR}/raw/nuclei-ogc-base.txt"
    if [[ ! -f "$OGC_BASE_TARGET" ]]; then
        echo "${TARGET}/" > "$OGC_BASE_TARGET"
    fi
    
    timeout ${OOB_SCAN_TIMEOUT}m docker run --rm \
        -v "${OUTPUT_DIR}:/output" \
        -v "${CUSTOM_TEMPLATES}:/custom-templates:ro" \
        --network host \
        projectdiscovery/nuclei:latest \
        -l "/output/raw/nuclei-ogc-base.txt" \
        -t "/custom-templates/ogc-wms/wms-ssrf-sld.yaml" \
        -t "/custom-templates/ogc-edr/edr-ssrf-coords.yaml" \
        -H "Authorization: Basic ${AUTH_B64}" \
        -iserver "oast.fun" \
        -severity critical,high,medium \
        -rl 20 \
        -c 3 \
        -timeout 60 \
        -stats \
        -si 15 \
        -nc \
        -jsonl \
        -o "/output/raw/nuclei-oob.jsonl" \
        2>&1 | tee "${OUTPUT_DIR}/raw/nuclei-oob-scan.log" || {
            echo "[NUCLEI] OOB scan completed or timed out after ${OOB_SCAN_TIMEOUT} minutes"
        }
else
    touch "${OUTPUT_DIR}/raw/nuclei-oob.jsonl"
fi

echo "[NUCLEI] OOB detection scan complete"

# =============================================================================
# Phase 6: Combine and summarize results
# =============================================================================
echo "[NUCLEI] Phase 6: Combining results..."

# Ensure all output files exist
touch "${OUTPUT_DIR}/raw/nuclei-standard.jsonl" 2>/dev/null || true
touch "${OUTPUT_DIR}/raw/nuclei-injection.jsonl" 2>/dev/null || true
touch "${OUTPUT_DIR}/raw/nuclei-ogc.jsonl" 2>/dev/null || true
touch "${OUTPUT_DIR}/raw/nuclei-oob.jsonl" 2>/dev/null || true

# Merge all JSONL files
cat "${OUTPUT_DIR}/raw/nuclei-standard.jsonl" \
    "${OUTPUT_DIR}/raw/nuclei-injection.jsonl" \
    "${OUTPUT_DIR}/raw/nuclei-ogc.jsonl" \
    "${OUTPUT_DIR}/raw/nuclei-oob.jsonl" \
    2>/dev/null | sort -u > "$RESULTS_JSONL" || true

# Convert JSONL to JSON array for report generator compatibility
if [[ -f "$RESULTS_JSONL" && -s "$RESULTS_JSONL" ]]; then
    jq -s '.' "$RESULTS_JSONL" > "$RESULTS_JSON" 2>/dev/null || echo '[]' > "$RESULTS_JSON"
else
    echo '[]' > "$RESULTS_JSON"
fi

# Count findings by severity
if [[ -f "$RESULTS_JSONL" && -s "$RESULTS_JSONL" ]]; then
    CRITICAL=$(grep -c '"severity":"critical"' "$RESULTS_JSONL" 2>/dev/null || echo "0")
    HIGH=$(grep -c '"severity":"high"' "$RESULTS_JSONL" 2>/dev/null || echo "0")
    MEDIUM=$(grep -c '"severity":"medium"' "$RESULTS_JSONL" 2>/dev/null || echo "0")
    LOW=$(grep -c '"severity":"low"' "$RESULTS_JSONL" 2>/dev/null || echo "0")
    INFO=$(grep -c '"severity":"info"' "$RESULTS_JSONL" 2>/dev/null || echo "0")
else
    CRITICAL=0
    HIGH=0
    MEDIUM=0
    LOW=0
    INFO=0
fi

echo "[NUCLEI] Findings: ${CRITICAL} critical, ${HIGH} high, ${MEDIUM} medium, ${LOW} low, ${INFO} info"

# Create summary JSON
BASE_TARGET_COUNT=$(wc -l < "$BASE_TARGETS_FILE")
EXTENDED_TARGET_COUNT=$(wc -l < "$TARGETS_FILE")

cat > "${OUTPUT_DIR}/raw/nuclei-summary.json" << EOF
{
    "scan_type": "nuclei",
    "target": "${TARGET}",
    "timestamp": "$(date -Iseconds)",
    "time_limits": {
        "standard_minutes": ${STANDARD_SCAN_TIMEOUT},
        "ogc_minutes": ${OGC_SCAN_TIMEOUT},
        "oob_minutes": ${OOB_SCAN_TIMEOUT}
    },
    "templates": {
        "standard": true,
        "injection": true,
        "custom_ogc": $([ -d "$CUSTOM_TEMPLATES" ] && echo "true" || echo "false"),
        "oob_detection": true
    },
    "targets": {
        "base": ${BASE_TARGET_COUNT},
        "extended": ${EXTENDED_TARGET_COUNT}
    },
    "findings": {
        "critical": ${CRITICAL},
        "high": ${HIGH},
        "medium": ${MEDIUM},
        "low": ${LOW},
        "info": ${INFO}
    }
}
EOF

# Create markdown summary of OGC-specific findings
mkdir -p "${OUTPUT_DIR}/nuclei"
{
    echo "# Nuclei Security Scan Results"
    echo ""
    echo "Generated: $(date)"
    echo "Target: ${TARGET}"
    echo ""
    echo "## Summary"
    echo ""
    echo "| Severity | Count |"
    echo "|----------|-------|"
    echo "| Critical | ${CRITICAL} |"
    echo "| High | ${HIGH} |"
    echo "| Medium | ${MEDIUM} |"
    echo "| Low | ${LOW} |"
    echo "| Info | ${INFO} |"
    echo ""
    
    if [[ -f "$RESULTS_JSONL" && -s "$RESULTS_JSONL" ]]; then
        echo "## Findings"
        echo ""
        jq -r '"- **\(.info.severity | ascii_upcase)**: \(.info.name) - `\(.matched)`"' "$RESULTS_JSONL" 2>/dev/null | sort | uniq || true
    fi
} > "${OUTPUT_DIR}/nuclei/findings.md"

echo "[NUCLEI] Results saved to ${OUTPUT_DIR}/nuclei/"
echo "[NUCLEI] Scan complete"
