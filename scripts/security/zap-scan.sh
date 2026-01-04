#!/bin/bash
# =============================================================================
# OWASP ZAP Full Active Scan
# =============================================================================
# Comprehensive web application security scanner that performs:
# - Spider/crawling to discover endpoints
# - Ajax spider for JavaScript-heavy pages
# - Passive scanning (security headers, cookies, etc.)
# - Active scanning (SQL injection, XSS, CSRF, etc.)
# =============================================================================

set -euo pipefail

TARGET="$1"
OUTPUT_DIR="$2"
AUTH_B64="${3:-}"

ZAP_OUTPUT_DIR="${OUTPUT_DIR}/zap"
mkdir -p "$ZAP_OUTPUT_DIR"

echo "[ZAP] Starting OWASP ZAP full active scan..."
echo "[ZAP] This may take 15-30 minutes depending on the target size..."

# Build ZAP config for authentication
ZAP_AUTH_CONFIG=""
if [[ -n "$AUTH_B64" ]]; then
    ZAP_AUTH_CONFIG="-config replacer.full_list(0).description=BasicAuth \
        -config replacer.full_list(0).enabled=true \
        -config replacer.full_list(0).matchtype=REQ_HEADER \
        -config replacer.full_list(0).matchstr=Authorization \
        -config replacer.full_list(0).regex=false \
        -config replacer.full_list(0).replacement='Basic ${AUTH_B64}'"
fi

# Phase 1: Scan public endpoints (no auth)
echo "[ZAP] Phase 1: Scanning public endpoints..."

docker run --rm \
    -v "${ZAP_OUTPUT_DIR}:/zap/wrk:rw" \
    -t ghcr.io/zaproxy/zaproxy:stable \
    zap-full-scan.py \
    -t "${TARGET}" \
    -r "zap-public-report.html" \
    -J "zap-public-report.json" \
    -w "zap-public-report.md" \
    -a \
    -j \
    -m 5 \
    -I \
    2>&1 | tee "${OUTPUT_DIR}/raw/zap-public-scan.log" || true

# Phase 2: Scan authenticated endpoints (if credentials available)
if [[ -n "$AUTH_B64" ]]; then
    echo "[ZAP] Phase 2: Scanning authenticated endpoints..."
    
    # Create a context file with auth
    cat > "${ZAP_OUTPUT_DIR}/auth-context.xml" << 'CONTEXT_EOF'
<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<configuration>
    <context>
        <name>AuthContext</name>
        <desc/>
        <inscope>true</inscope>
        <incregexes>.*</incregexes>
        <tech>
            <include>Db</include>
            <include>Db.CouchDB</include>
            <include>Db.Firebird</include>
            <include>Db.HypersonicSQL</include>
            <include>Db.IBM DB2</include>
            <include>Db.Microsoft Access</include>
            <include>Db.Microsoft SQL Server</include>
            <include>Db.MongoDB</include>
            <include>Db.MySQL</include>
            <include>Db.Oracle</include>
            <include>Db.PostgreSQL</include>
            <include>Db.SAP MaxDB</include>
            <include>Db.SQLite</include>
            <include>Db.Sybase</include>
            <include>Language</include>
            <include>Language.ASP</include>
            <include>Language.C</include>
            <include>Language.JSP/Servlet</include>
            <include>Language.Java</include>
            <include>Language.JavaScript</include>
            <include>Language.PHP</include>
            <include>Language.Python</include>
            <include>Language.Ruby</include>
            <include>Language.XML</include>
            <include>OS</include>
            <include>OS.Linux</include>
            <include>OS.MacOS</include>
            <include>OS.Windows</include>
            <include>SCM</include>
            <include>SCM.Git</include>
            <include>SCM.SVN</include>
            <include>WS</include>
            <include>WS.Apache</include>
            <include>WS.IIS</include>
            <include>WS.Tomcat</include>
        </tech>
        <urlparser>
            <class>org.zaproxy.zap.model.StandardParameterParser</class>
            <config>{"kvps":"&amp;","kvs":"=","struct":[]}</config>
        </urlparser>
        <postparser>
            <class>org.zaproxy.zap.model.StandardParameterParser</class>
            <config>{"kvps":"&amp;","kvs":"=","struct":[]}</config>
        </postparser>
        <authentication>
            <type>2</type>
            <strategy>EACH_RESP</strategy>
            <pollurl/>
            <polldata/>
            <pollheaders/>
            <pollfreq>60</pollfreq>
            <pollunits>REQUESTS</pollunits>
        </authentication>
        <forceduser>-1</forceduser>
        <session>
            <type>0</type>
        </session>
        <authorization>
            <type>0</type>
            <basic>
                <header/>
                <body/>
                <logic>AND</logic>
                <code>-1</code>
            </basic>
        </authorization>
    </context>
</configuration>
CONTEXT_EOF

    # Scan authenticated endpoints
    docker run --rm \
        -v "${ZAP_OUTPUT_DIR}:/zap/wrk:rw" \
        -t ghcr.io/zaproxy/zaproxy:stable \
        zap-full-scan.py \
        -t "${TARGET}/admin" \
        -r "zap-auth-report.html" \
        -J "zap-auth-report.json" \
        -w "zap-auth-report.md" \
        -a \
        -j \
        -m 5 \
        -I \
        -z "-config replacer.full_list(0).description=BasicAuth \
            -config replacer.full_list(0).enabled=true \
            -config replacer.full_list(0).matchtype=REQ_HEADER \
            -config replacer.full_list(0).matchstr=Authorization \
            -config replacer.full_list(0).regex=false \
            -config replacer.full_list(0).replacement='Basic ${AUTH_B64}'" \
        2>&1 | tee "${OUTPUT_DIR}/raw/zap-auth-scan.log" || true
fi

# Combine results
echo "[ZAP] Combining scan results..."

# Create combined summary
COMBINED_HTML="${ZAP_OUTPUT_DIR}/zap-combined-report.html"

cat > "$COMBINED_HTML" << 'HTML_HEADER'
<!DOCTYPE html>
<html>
<head>
    <title>OWASP ZAP Combined Security Report</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; }
        h1 { color: #333; }
        .section { margin: 20px 0; padding: 15px; border: 1px solid #ddd; border-radius: 5px; }
        iframe { width: 100%; height: 600px; border: 1px solid #ccc; }
    </style>
</head>
<body>
    <h1>OWASP ZAP Security Scan - Combined Report</h1>
    
    <div class="section">
        <h2>Public Endpoints Scan</h2>
        <iframe src="zap-public-report.html"></iframe>
    </div>
HTML_HEADER

if [[ -f "${ZAP_OUTPUT_DIR}/zap-auth-report.html" ]]; then
    cat >> "$COMBINED_HTML" << 'HTML_AUTH'
    <div class="section">
        <h2>Authenticated Endpoints Scan</h2>
        <iframe src="zap-auth-report.html"></iframe>
    </div>
HTML_AUTH
fi

cat >> "$COMBINED_HTML" << 'HTML_FOOTER'
</body>
</html>
HTML_FOOTER

# Count findings from JSON reports
count_findings() {
    local json_file="$1"
    if [[ -f "$json_file" ]]; then
        local high=$(jq '[.site[].alerts[] | select(.riskcode == "3")] | length' "$json_file" 2>/dev/null || echo "0")
        local medium=$(jq '[.site[].alerts[] | select(.riskcode == "2")] | length' "$json_file" 2>/dev/null || echo "0")
        local low=$(jq '[.site[].alerts[] | select(.riskcode == "1")] | length' "$json_file" 2>/dev/null || echo "0")
        local info=$(jq '[.site[].alerts[] | select(.riskcode == "0")] | length' "$json_file" 2>/dev/null || echo "0")
        echo "${high:-0} ${medium:-0} ${low:-0} ${info:-0}"
    else
        echo "0 0 0 0"
    fi
}

PUBLIC_COUNTS=$(count_findings "${ZAP_OUTPUT_DIR}/zap-public-report.json")
read -r PUB_HIGH PUB_MED PUB_LOW PUB_INFO <<< "$PUBLIC_COUNTS"

AUTH_HIGH=0 AUTH_MED=0 AUTH_LOW=0 AUTH_INFO=0
if [[ -f "${ZAP_OUTPUT_DIR}/zap-auth-report.json" ]]; then
    AUTH_COUNTS=$(count_findings "${ZAP_OUTPUT_DIR}/zap-auth-report.json")
    read -r AUTH_HIGH AUTH_MED AUTH_LOW AUTH_INFO <<< "$AUTH_COUNTS"
fi

TOTAL_HIGH=$((PUB_HIGH + AUTH_HIGH))
TOTAL_MED=$((PUB_MED + AUTH_MED))
TOTAL_LOW=$((PUB_LOW + AUTH_LOW))
TOTAL_INFO=$((PUB_INFO + AUTH_INFO))

echo "[ZAP] Findings: ${TOTAL_HIGH} high, ${TOTAL_MED} medium, ${TOTAL_LOW} low, ${TOTAL_INFO} info"
echo "[ZAP] Reports saved to ${ZAP_OUTPUT_DIR}/"

# Create summary JSON for report generator
cat > "${OUTPUT_DIR}/raw/zap-summary.json" << EOF
{
    "scan_type": "owasp_zap",
    "target": "${TARGET}",
    "timestamp": "$(date -Iseconds)",
    "findings": {
        "high": ${TOTAL_HIGH},
        "medium": ${TOTAL_MED},
        "low": ${TOTAL_LOW},
        "info": ${TOTAL_INFO}
    },
    "reports": {
        "public": "zap/zap-public-report.html",
        "authenticated": "zap/zap-auth-report.html",
        "combined": "zap/zap-combined-report.html"
    }
}
EOF
