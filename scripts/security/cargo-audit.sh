#!/bin/bash
# =============================================================================
# Cargo Audit - Rust Dependency CVE Scanner
# =============================================================================
# Scans Cargo.lock for known security vulnerabilities in dependencies
# using the RustSec Advisory Database.
# =============================================================================

set -euo pipefail

PROJECT_ROOT="$1"
OUTPUT_DIR="$2"

RESULTS_JSON="${OUTPUT_DIR}/raw/cargo-audit.json"

echo "[CARGO] Scanning Rust dependencies for vulnerabilities..."

cd "$PROJECT_ROOT"

# Check if cargo-audit is installed
if command -v cargo-audit &> /dev/null; then
    echo "[CARGO] Using installed cargo-audit"
    cargo audit --json > "$RESULTS_JSON" 2>&1 || true
elif command -v cargo &> /dev/null; then
    echo "[CARGO] cargo-audit not found, attempting to install..."
    cargo install cargo-audit --locked 2>/dev/null || {
        echo "[CARGO] Failed to install cargo-audit, skipping..."
        echo '{"vulnerabilities": {"count": 0, "list": []}, "warnings": {"count": 0, "list": []}, "error": "cargo-audit not installed"}' > "$RESULTS_JSON"
        exit 0
    }
    cargo audit --json > "$RESULTS_JSON" 2>&1 || true
else
    echo "[CARGO] Cargo not found, skipping dependency audit"
    echo '{"vulnerabilities": {"count": 0, "list": []}, "warnings": {"count": 0, "list": []}, "error": "cargo not installed"}' > "$RESULTS_JSON"
    exit 0
fi

# Parse results for summary
if [[ -f "$RESULTS_JSON" ]]; then
    VULN_COUNT=$(jq -r '.vulnerabilities.count // 0' "$RESULTS_JSON" 2>/dev/null || echo "0")
    WARN_COUNT=$(jq -r '.warnings.count // 0' "$RESULTS_JSON" 2>/dev/null || echo "0")
    
    echo "[CARGO] Found ${VULN_COUNT} vulnerabilities, ${WARN_COUNT} warnings"
    echo "[CARGO] Results saved to ${RESULTS_JSON}"
else
    echo "[CARGO] No results generated"
fi
