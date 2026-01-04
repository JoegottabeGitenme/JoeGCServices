#!/bin/bash
# =============================================================================
# Security Scan Report Generator
# =============================================================================
# Combines all scan results into a single HTML report with:
# - Executive summary with severity counts
# - Expandable sections for each scan type
# - Links to full tool-specific reports
# =============================================================================

set -uo pipefail

OUTPUT_DIR="$1"
TARGET="$2"
TIMESTAMP="$3"

REPORT_FILE="${OUTPUT_DIR}/index.html"

echo "[REPORT] Generating combined HTML report..."

# Helper to ensure a value is a valid integer
to_int() {
    local val="$1"
    # Remove any non-numeric characters and default to 0
    val=$(echo "$val" | tr -dc '0-9' | head -c 10)
    echo "${val:-0}"
}

# Count findings from various sources
count_from_json() {
    local file="$1"
    local query="$2"
    local default="${3:-0}"
    
    if [[ -f "$file" ]]; then
        local result
        result=$(jq -r "$query // $default" "$file" 2>/dev/null) || result="$default"
        to_int "$result"
    else
        echo "$default"
    fi
}

# Get findings counts - ZAP
ZAP_HIGH=$(count_from_json "${OUTPUT_DIR}/raw/zap-summary.json" '.findings.high' 0)
ZAP_MEDIUM=$(count_from_json "${OUTPUT_DIR}/raw/zap-summary.json" '.findings.medium' 0)
ZAP_LOW=$(count_from_json "${OUTPUT_DIR}/raw/zap-summary.json" '.findings.low' 0)
ZAP_INFO=$(count_from_json "${OUTPUT_DIR}/raw/zap-summary.json" '.findings.info' 0)

# Nuclei counts - grep may fail if file doesn't exist or no matches
NUCLEI_CRITICAL=0
NUCLEI_HIGH=0
NUCLEI_MEDIUM=0
NUCLEI_LOW=0
NUCLEI_INFO=0

if [[ -f "${OUTPUT_DIR}/raw/nuclei-report.json" ]] && [[ -s "${OUTPUT_DIR}/raw/nuclei-report.json" ]]; then
    NUCLEI_CRITICAL=$(grep -c '"severity":"critical"' "${OUTPUT_DIR}/raw/nuclei-report.json" 2>/dev/null) || NUCLEI_CRITICAL=0
    NUCLEI_HIGH=$(grep -c '"severity":"high"' "${OUTPUT_DIR}/raw/nuclei-report.json" 2>/dev/null) || NUCLEI_HIGH=0
    NUCLEI_MEDIUM=$(grep -c '"severity":"medium"' "${OUTPUT_DIR}/raw/nuclei-report.json" 2>/dev/null) || NUCLEI_MEDIUM=0
    NUCLEI_LOW=$(grep -c '"severity":"low"' "${OUTPUT_DIR}/raw/nuclei-report.json" 2>/dev/null) || NUCLEI_LOW=0
    NUCLEI_INFO=$(grep -c '"severity":"info"' "${OUTPUT_DIR}/raw/nuclei-report.json" 2>/dev/null) || NUCLEI_INFO=0
fi

# Ensure all nuclei values are integers
NUCLEI_CRITICAL=$(to_int "$NUCLEI_CRITICAL")
NUCLEI_HIGH=$(to_int "$NUCLEI_HIGH")
NUCLEI_MEDIUM=$(to_int "$NUCLEI_MEDIUM")
NUCLEI_LOW=$(to_int "$NUCLEI_LOW")
NUCLEI_INFO=$(to_int "$NUCLEI_INFO")

# Headers counts
HEADERS_FAILED=$(count_from_json "${OUTPUT_DIR}/raw/headers-report.json" '.summary.failed' 0)
HEADERS_WARNINGS=$(count_from_json "${OUTPUT_DIR}/raw/headers-report.json" '.summary.warnings' 0)
HEADERS_PASSED=$(count_from_json "${OUTPUT_DIR}/raw/headers-report.json" '.summary.passed' 0)

# Auth check
AUTH_FAILED=$(count_from_json "${OUTPUT_DIR}/raw/auth-check.json" '.summary.failed' 0)
AUTH_PASSED=$(count_from_json "${OUTPUT_DIR}/raw/auth-check.json" '.summary.passed' 0)

# Cargo audit
CARGO_VULNS=$(count_from_json "${OUTPUT_DIR}/raw/cargo-audit.json" '.vulnerabilities.count' 0)
CARGO_WARNINGS=$(count_from_json "${OUTPUT_DIR}/raw/cargo-audit.json" '.warnings.count' 0)

# Ensure all values are integers before arithmetic
ZAP_HIGH=$(to_int "$ZAP_HIGH")
ZAP_MEDIUM=$(to_int "$ZAP_MEDIUM")
ZAP_LOW=$(to_int "$ZAP_LOW")
ZAP_INFO=$(to_int "$ZAP_INFO")
HEADERS_FAILED=$(to_int "$HEADERS_FAILED")
HEADERS_WARNINGS=$(to_int "$HEADERS_WARNINGS")
HEADERS_PASSED=$(to_int "$HEADERS_PASSED")
AUTH_FAILED=$(to_int "$AUTH_FAILED")
AUTH_PASSED=$(to_int "$AUTH_PASSED")
CARGO_VULNS=$(to_int "$CARGO_VULNS")
CARGO_WARNINGS=$(to_int "$CARGO_WARNINGS")

# Calculate totals
TOTAL_CRITICAL=$((NUCLEI_CRITICAL + 0))
TOTAL_HIGH=$((ZAP_HIGH + NUCLEI_HIGH + HEADERS_FAILED + AUTH_FAILED))
TOTAL_MEDIUM=$((ZAP_MEDIUM + NUCLEI_MEDIUM + HEADERS_WARNINGS))
TOTAL_LOW=$((ZAP_LOW + NUCLEI_LOW))
TOTAL_INFO=$((ZAP_INFO + NUCLEI_INFO + HEADERS_PASSED))

# Determine overall status
OVERALL_STATUS="success"
OVERALL_COLOR="#22c55e"
if [[ $TOTAL_CRITICAL -gt 0 ]]; then
    OVERALL_STATUS="critical"
    OVERALL_COLOR="#dc2626"
elif [[ $TOTAL_HIGH -gt 0 ]]; then
    OVERALL_STATUS="high-risk"
    OVERALL_COLOR="#ea580c"
elif [[ $TOTAL_MEDIUM -gt 0 ]]; then
    OVERALL_STATUS="medium-risk"
    OVERALL_COLOR="#eab308"
elif [[ $TOTAL_LOW -gt 0 ]]; then
    OVERALL_STATUS="low-risk"
    OVERALL_COLOR="#3b82f6"
fi

# Generate HTML report
cat > "$REPORT_FILE" << 'HTML_START'
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Security Scan Report - Weather WMS</title>
    <style>
        :root {
            --bg-primary: #0f172a;
            --bg-secondary: #1e293b;
            --bg-tertiary: #334155;
            --text-primary: #f1f5f9;
            --text-secondary: #94a3b8;
            --border-color: #475569;
            --critical: #dc2626;
            --high: #ea580c;
            --medium: #eab308;
            --low: #3b82f6;
            --info: #22c55e;
            --success: #22c55e;
        }
        
        * {
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }
        
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
            background: var(--bg-primary);
            color: var(--text-primary);
            line-height: 1.6;
            padding: 20px;
        }
        
        .container {
            max-width: 1400px;
            margin: 0 auto;
        }
        
        header {
            text-align: center;
            padding: 30px 0;
            border-bottom: 1px solid var(--border-color);
            margin-bottom: 30px;
        }
        
        h1 {
            font-size: 2rem;
            margin-bottom: 10px;
        }
        
        .meta {
            color: var(--text-secondary);
            font-size: 0.9rem;
        }
        
        .summary-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
            gap: 15px;
            margin-bottom: 30px;
        }
        
        .summary-card {
            background: var(--bg-secondary);
            border-radius: 8px;
            padding: 20px;
            text-align: center;
            border: 1px solid var(--border-color);
        }
        
        .summary-card.critical { border-left: 4px solid var(--critical); }
        .summary-card.high { border-left: 4px solid var(--high); }
        .summary-card.medium { border-left: 4px solid var(--medium); }
        .summary-card.low { border-left: 4px solid var(--low); }
        .summary-card.info { border-left: 4px solid var(--info); }
        
        .summary-count {
            font-size: 2.5rem;
            font-weight: bold;
        }
        
        .summary-card.critical .summary-count { color: var(--critical); }
        .summary-card.high .summary-count { color: var(--high); }
        .summary-card.medium .summary-count { color: var(--medium); }
        .summary-card.low .summary-count { color: var(--low); }
        .summary-card.info .summary-count { color: var(--info); }
        
        .summary-label {
            color: var(--text-secondary);
            font-size: 0.85rem;
            text-transform: uppercase;
            letter-spacing: 0.05em;
        }
        
        .status-badge {
            display: inline-block;
            padding: 8px 16px;
            border-radius: 20px;
            font-weight: bold;
            text-transform: uppercase;
            font-size: 0.85rem;
            margin-top: 15px;
        }
        
        .scan-section {
            background: var(--bg-secondary);
            border-radius: 8px;
            margin-bottom: 20px;
            border: 1px solid var(--border-color);
            overflow: hidden;
        }
        
        .scan-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 15px 20px;
            cursor: pointer;
            background: var(--bg-tertiary);
        }
        
        .scan-header:hover {
            background: #3d4a5c;
        }
        
        .scan-title {
            font-size: 1.1rem;
            font-weight: 600;
        }
        
        .scan-badges {
            display: flex;
            gap: 8px;
        }
        
        .badge {
            padding: 4px 10px;
            border-radius: 12px;
            font-size: 0.75rem;
            font-weight: bold;
        }
        
        .badge.critical { background: var(--critical); color: white; }
        .badge.high { background: var(--high); color: white; }
        .badge.medium { background: var(--medium); color: black; }
        .badge.low { background: var(--low); color: white; }
        .badge.info { background: var(--info); color: white; }
        .badge.pass { background: var(--success); color: white; }
        .badge.fail { background: var(--critical); color: white; }
        
        .scan-content {
            padding: 20px;
            display: none;
        }
        
        .scan-content.active {
            display: block;
        }
        
        .findings-table {
            width: 100%;
            border-collapse: collapse;
            margin-top: 15px;
        }
        
        .findings-table th,
        .findings-table td {
            padding: 10px;
            text-align: left;
            border-bottom: 1px solid var(--border-color);
        }
        
        .findings-table th {
            background: var(--bg-tertiary);
            font-weight: 600;
        }
        
        .severity-dot {
            display: inline-block;
            width: 10px;
            height: 10px;
            border-radius: 50%;
            margin-right: 8px;
        }
        
        .severity-dot.critical { background: var(--critical); }
        .severity-dot.high { background: var(--high); }
        .severity-dot.medium { background: var(--medium); }
        .severity-dot.low { background: var(--low); }
        .severity-dot.info { background: var(--info); }
        
        .report-link {
            display: inline-block;
            margin-top: 15px;
            padding: 8px 16px;
            background: var(--bg-tertiary);
            color: var(--text-primary);
            text-decoration: none;
            border-radius: 5px;
            font-size: 0.9rem;
        }
        
        .report-link:hover {
            background: #4a5568;
        }
        
        .check-result {
            display: flex;
            align-items: center;
            padding: 8px 0;
            border-bottom: 1px solid var(--border-color);
        }
        
        .check-icon {
            font-size: 1.2rem;
            margin-right: 10px;
        }
        
        .check-pass .check-icon { color: var(--success); }
        .check-fail .check-icon { color: var(--critical); }
        .check-warn .check-icon { color: var(--medium); }
        
        footer {
            text-align: center;
            padding: 30px;
            color: var(--text-secondary);
            font-size: 0.85rem;
            border-top: 1px solid var(--border-color);
            margin-top: 30px;
        }
        
        .toggle-icon {
            transition: transform 0.2s;
        }
        
        .scan-section.open .toggle-icon {
            transform: rotate(180deg);
        }
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>Security Scan Report</h1>
            <p class="meta">Weather WMS - Comprehensive Security Analysis</p>
HTML_START

# Add dynamic content
cat >> "$REPORT_FILE" << HTML_META
            <p class="meta">Target: <strong>${TARGET}</strong></p>
            <p class="meta">Generated: ${TIMESTAMP}</p>
            <div class="status-badge" style="background: ${OVERALL_COLOR}; color: white;">
                Overall Status: ${OVERALL_STATUS}
            </div>
        </header>
        
        <section class="summary">
            <h2 style="margin-bottom: 20px;">Findings Summary</h2>
            <div class="summary-grid">
                <div class="summary-card critical">
                    <div class="summary-count">${TOTAL_CRITICAL}</div>
                    <div class="summary-label">Critical</div>
                </div>
                <div class="summary-card high">
                    <div class="summary-count">${TOTAL_HIGH}</div>
                    <div class="summary-label">High</div>
                </div>
                <div class="summary-card medium">
                    <div class="summary-count">${TOTAL_MEDIUM}</div>
                    <div class="summary-label">Medium</div>
                </div>
                <div class="summary-card low">
                    <div class="summary-count">${TOTAL_LOW}</div>
                    <div class="summary-label">Low</div>
                </div>
                <div class="summary-card info">
                    <div class="summary-count">${TOTAL_INFO}</div>
                    <div class="summary-label">Info</div>
                </div>
            </div>
        </section>
        
        <section class="scans">
            <h2 style="margin-bottom: 20px;">Scan Results</h2>
            
            <!-- OWASP ZAP -->
            <div class="scan-section">
                <div class="scan-header" onclick="toggleSection(this)">
                    <span class="scan-title">OWASP ZAP - Web Application Security</span>
                    <div class="scan-badges">
                        <span class="badge high">${ZAP_HIGH} High</span>
                        <span class="badge medium">${ZAP_MEDIUM} Medium</span>
                        <span class="badge low">${ZAP_LOW} Low</span>
                        <span class="toggle-icon">&#9660;</span>
                    </div>
                </div>
                <div class="scan-content">
                    <p>Full active security scan including SQL injection, XSS, CSRF, and other OWASP Top 10 vulnerabilities.</p>
HTML_META

if [[ -f "${OUTPUT_DIR}/zap/zap-combined-report.html" ]]; then
    cat >> "$REPORT_FILE" << 'HTML_ZAP'
                    <a href="zap/zap-combined-report.html" class="report-link" target="_blank">View Full ZAP Report</a>
HTML_ZAP
fi

cat >> "$REPORT_FILE" << HTML_ZAP_END
                </div>
            </div>
            
            <!-- Nuclei -->
            <div class="scan-section">
                <div class="scan-header" onclick="toggleSection(this)">
                    <span class="scan-title">Nuclei - Vulnerability Scanner</span>
                    <div class="scan-badges">
                        <span class="badge critical">${NUCLEI_CRITICAL} Critical</span>
                        <span class="badge high">${NUCLEI_HIGH} High</span>
                        <span class="badge medium">${NUCLEI_MEDIUM} Medium</span>
                        <span class="toggle-icon">&#9660;</span>
                    </div>
                </div>
                <div class="scan-content">
                    <p>Template-based scanning for known CVEs, misconfigurations, and exposed services.</p>
HTML_ZAP_END

if [[ -d "${OUTPUT_DIR}/nuclei" ]]; then
    cat >> "$REPORT_FILE" << 'HTML_NUCLEI'
                    <a href="nuclei/" class="report-link" target="_blank">View Nuclei Reports</a>
HTML_NUCLEI
fi

cat >> "$REPORT_FILE" << HTML_TLS
                </div>
            </div>
            
            <!-- TLS/SSL -->
            <div class="scan-section">
                <div class="scan-header" onclick="toggleSection(this)">
                    <span class="scan-title">TLS/SSL Configuration</span>
                    <div class="scan-badges">
                        <span class="badge info">Analyzed</span>
                        <span class="toggle-icon">&#9660;</span>
                    </div>
                </div>
                <div class="scan-content">
                    <p>Analysis of TLS protocols, cipher suites, certificates, and known vulnerabilities (Heartbleed, POODLE, etc.).</p>
HTML_TLS

if [[ -f "${OUTPUT_DIR}/tls-report.html" ]]; then
    cat >> "$REPORT_FILE" << 'HTML_TLS_LINK'
                    <a href="tls-report.html" class="report-link" target="_blank">View TLS Report</a>
HTML_TLS_LINK
fi

cat >> "$REPORT_FILE" << HTML_HEADERS
                </div>
            </div>
            
            <!-- Security Headers -->
            <div class="scan-section">
                <div class="scan-header" onclick="toggleSection(this)">
                    <span class="scan-title">Security Headers</span>
                    <div class="scan-badges">
                        <span class="badge high">${HEADERS_FAILED} Missing</span>
                        <span class="badge medium">${HEADERS_WARNINGS} Warnings</span>
                        <span class="badge pass">${HEADERS_PASSED} OK</span>
                        <span class="toggle-icon">&#9660;</span>
                    </div>
                </div>
                <div class="scan-content">
                    <p>HTTP security header analysis including HSTS, CSP, X-Frame-Options, etc.</p>
                    <p>See raw/headers-report.json for detailed findings.</p>
                </div>
            </div>
            
            <!-- Auth Bypass -->
            <div class="scan-section">
                <div class="scan-header" onclick="toggleSection(this)">
                    <span class="scan-title">Authentication Bypass Tests</span>
                    <div class="scan-badges">
HTML_HEADERS

if [[ $AUTH_FAILED -eq 0 ]]; then
    cat >> "$REPORT_FILE" << 'HTML_AUTH_PASS'
                        <span class="badge pass">All Protected</span>
HTML_AUTH_PASS
else
    cat >> "$REPORT_FILE" << HTML_AUTH_FAIL
                        <span class="badge fail">${AUTH_FAILED} Bypasses Found</span>
HTML_AUTH_FAIL
fi

cat >> "$REPORT_FILE" << HTML_AUTH_END
                        <span class="toggle-icon">&#9660;</span>
                    </div>
                </div>
                <div class="scan-content">
                    <p>Testing protected endpoints for authentication bypass vulnerabilities using various techniques.</p>
                    <p><strong>${AUTH_PASSED}</strong> tests passed, <strong>${AUTH_FAILED}</strong> potential bypasses detected.</p>
                </div>
            </div>
            
            <!-- Rate Limiting -->
            <div class="scan-section">
                <div class="scan-header" onclick="toggleSection(this)">
                    <span class="scan-title">Rate Limiting</span>
                    <div class="scan-badges">
                        <span class="badge info">Tested</span>
                        <span class="toggle-icon">&#9660;</span>
                    </div>
                </div>
                <div class="scan-content">
                    <p>Verification that rate limiting is properly configured to prevent abuse.</p>
                    <p>See raw/rate-limit-check.json for detailed results.</p>
                </div>
            </div>
            
            <!-- Cargo Audit -->
            <div class="scan-section">
                <div class="scan-header" onclick="toggleSection(this)">
                    <span class="scan-title">Dependency Audit (Rust)</span>
                    <div class="scan-badges">
HTML_AUTH_END

if [[ $CARGO_VULNS -eq 0 ]]; then
    cat >> "$REPORT_FILE" << 'HTML_CARGO_PASS'
                        <span class="badge pass">No Vulnerabilities</span>
HTML_CARGO_PASS
else
    cat >> "$REPORT_FILE" << HTML_CARGO_FAIL
                        <span class="badge critical">${CARGO_VULNS} Vulnerabilities</span>
HTML_CARGO_FAIL
fi

cat >> "$REPORT_FILE" << HTML_FOOTER
                        <span class="toggle-icon">&#9660;</span>
                    </div>
                </div>
                <div class="scan-content">
                    <p>Scanning Rust dependencies for known security vulnerabilities using RustSec Advisory Database.</p>
                    <p><strong>${CARGO_VULNS}</strong> vulnerabilities, <strong>${CARGO_WARNINGS}</strong> warnings found.</p>
                </div>
            </div>
        </section>
        
        <footer>
            <p>Generated by Weather WMS Security Scanner</p>
            <p>Tools: OWASP ZAP, Nuclei, testssl.sh, cargo-audit</p>
        </footer>
    </div>
    
    <script>
        function toggleSection(header) {
            const section = header.parentElement;
            const content = section.querySelector('.scan-content');
            section.classList.toggle('open');
            content.classList.toggle('active');
        }
        
        // Open first section by default
        document.querySelector('.scan-section').classList.add('open');
        document.querySelector('.scan-content').classList.add('active');
    </script>
</body>
</html>
HTML_FOOTER

echo "[REPORT] Report generated: ${REPORT_FILE}"
