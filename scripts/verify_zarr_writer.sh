#!/bin/bash

# =============================================================================
# Zarr Writer Verification Script
# =============================================================================
#
# This script tests the ZarrWriter by running unit tests and verifying builds.
#
# Usage:
#   ./scripts/verify_zarr_writer.sh

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
echo " Zarr Writer Verification"
echo "=============================================="
echo ""

PASSED=0
FAILED=0

run_test() {
    local name="$1"
    local cmd="$2"
    
    echo -n "  $name... "
    if timeout 60 bash -c "$cmd" > /tmp/test_output.txt 2>&1; then
        echo -e "${GREEN}OK${NC}"
        PASSED=$((PASSED + 1))
    else
        echo -e "${RED}FAILED${NC}"
        cat /tmp/test_output.txt | head -10
        FAILED=$((FAILED + 1))
    fi
}

log_info "Running ZarrWriter unit tests..."
run_test "ZarrWriter simple write" \
    "cargo test --package grid-processor test_zarr_writer_simple --quiet"

run_test "ZarrWriter with compression" \
    "cargo test --package grid-processor test_zarr_writer_with_compression --quiet"

run_test "ZarrMetadata serialization" \
    "cargo test --package grid-processor test_zarr_metadata_serialization --quiet"

echo ""
log_info "Running Zarr read/write roundtrip tests..."
run_test "Full grid roundtrip" \
    "cargo test --package grid-processor test_zarr_roundtrip_full_grid --quiet"

run_test "Partial region read" \
    "cargo test --package grid-processor test_zarr_partial_read --quiet"

run_test "Point value read" \
    "cargo test --package grid-processor test_zarr_read_point --quiet"

echo ""
log_info "Verifying package builds..."
run_test "grid-processor builds" \
    "cargo build --package grid-processor --quiet"

run_test "ingester builds (uses ZarrWriter)" \
    "cargo build --package ingester --quiet"

echo ""
echo "=============================================="
echo " Summary"
echo "=============================================="
echo ""
echo -e "  ${GREEN}Passed:${NC} $PASSED"
echo -e "  ${RED}Failed:${NC} $FAILED"
echo ""

if [ $FAILED -gt 0 ]; then
    log_error "Some tests failed!"
    exit 1
else
    log_success "All Zarr Writer tests passed!"
fi

echo ""
echo "Verified:"
echo "  - ZarrWriter can create Zarr V3 arrays"
echo "  - Compression (Blosc+Zstd) works"
echo "  - Metadata serialization works"
echo "  - Full read/write roundtrip works"
echo "  - grid-processor package builds"
echo "  - ingester package builds with ZarrWriter"
echo ""
