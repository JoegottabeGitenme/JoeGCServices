#!/bin/bash

# =============================================================================
# Zarr Roundtrip Verification Script
# =============================================================================
#
# This script creates synthetic grid data, writes it as Zarr, reads it back,
# and verifies the values match. This tests the core Zarr infrastructure
# without needing actual GRIB2 files.
#
# Usage:
#   ./scripts/verify_zarr_roundtrip.sh

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_error() { echo -e "${RED}[FAIL]${NC} $1"; }

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

cd "$PROJECT_ROOT"

echo ""
echo "=============================================="
echo " Zarr Roundtrip Verification"
echo "=============================================="
echo ""

log_info "Building test binary..."
cargo build --package grid-processor --tests --quiet 2>/dev/null

log_info "Running Zarr roundtrip tests..."
echo ""

# Run specific tests with verbose output
cargo test --package grid-processor \
    test_zarr_roundtrip_full_grid \
    test_zarr_partial_read \
    test_zarr_read_point \
    test_chunk_cache_efficiency \
    -- --nocapture 2>&1 | while read line; do
    
    if echo "$line" | grep -q "^test.*ok$"; then
        echo -e "${GREEN}✓${NC} $line"
    elif echo "$line" | grep -q "^test.*FAILED$"; then
        echo -e "${RED}✗${NC} $line"
    elif echo "$line" | grep -q "passed\|failed"; then
        echo "$line"
    elif echo "$line" | grep -q "^  "; then
        # Test output
        echo "$line"
    fi
done

echo ""
log_success "Zarr roundtrip verification complete!"
echo ""
