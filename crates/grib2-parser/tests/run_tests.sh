#!/bin/bash
#
# Run grib2-parser tests with automatic test data generation
#
# Usage:
#   ./run_tests.sh              # Generate data if missing, then run tests
#   ./run_tests.sh --generate   # Force regenerate test data
#   ./run_tests.sh --check      # Check data availability only
#   ./run_tests.sh --clean      # Remove generated test data
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
TESTDATA_DIR="$CRATE_DIR/testdata"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }

generate_data() {
    log_info "Generating synthetic GRIB2 test data..."
    cd "$CRATE_DIR"
    cargo test --package grib2-parser --test testdata_generator generate_test_files -- --ignored --nocapture
}

check_data() {
    echo "Test data status:"
    echo "================"
    local all_present=true
    for name in gfs_sample.grib2 mrms_refl.grib2; do
        local path="$TESTDATA_DIR/$name"
        if [[ -f "$path" ]]; then
            echo -e "  ${GREEN}[OK]${NC} $name ($(du -h "$path" | cut -f1))"
        else
            echo -e "  ${YELLOW}[MISSING]${NC} $name"
            all_present=false
        fi
    done
    $all_present
}

case "${1:-}" in
    --generate)
        generate_data
        ;;
    --check)
        check_data
        ;;
    --clean)
        log_info "Cleaning test data..."
        rm -f "$TESTDATA_DIR"/*.grib2
        log_info "Done"
        ;;
    --help|-h)
        echo "Usage: $0 [--generate|--check|--clean|--help]"
        echo ""
        echo "Options:"
        echo "  (none)      Generate data if missing, then run tests"
        echo "  --generate  Force regenerate test data"
        echo "  --check     Check which files are present"
        echo "  --clean     Remove generated test data"
        echo "  --help      Show this help"
        ;;
    *)
        # Check if data exists, generate if not
        if ! check_data 2>/dev/null; then
            generate_data
        fi
        
        echo ""
        log_info "Running tests..."
        cd "$CRATE_DIR"
        cargo test --package grib2-parser -- --nocapture
        ;;
esac
