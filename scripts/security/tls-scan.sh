#!/bin/bash
# =============================================================================
# TLS/SSL Configuration Scanner using testssl.sh
# =============================================================================
# Analyzes TLS configuration including protocols, ciphers, certificates,
# and known vulnerabilities (Heartbleed, POODLE, BEAST, etc.)
# =============================================================================

set -euo pipefail

TARGET="$1"
OUTPUT_DIR="$2"

# Extract domain from URL
DOMAIN=$(echo "$TARGET" | sed -E 's|https?://([^/]+).*|\1|')

echo "[TLS] Scanning ${DOMAIN}..."

# Run testssl.sh via Docker
docker run --rm \
    -v "${OUTPUT_DIR}:/output" \
    drwetter/testssl.sh \
    --quiet \
    --warnings batch \
    --severity LOW \
    --htmlfile "/output/tls-report.html" \
    --jsonfile-pretty "/output/raw/tls-report.json" \
    "$DOMAIN" 2>&1 | tee "${OUTPUT_DIR}/raw/tls-scan.log" || true

echo "[TLS] Results saved to ${OUTPUT_DIR}/tls-report.html"
