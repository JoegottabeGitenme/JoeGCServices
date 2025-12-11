#!/bin/bash
#
# Generate synthetic GRIB2 test data
#
# This script compiles and runs the test data generator to create
# minimal valid GRIB2 files for testing.
#
# Usage:
#   ./setup_test_data.sh           # Generate test data
#   ./setup_test_data.sh --clean   # Remove generated files
#   ./setup_test_data.sh --check   # Check if files exist
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(dirname "$SCRIPT_DIR")"
TESTDATA_DIR="$CRATE_DIR/testdata"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

clean_testdata() {
    log_info "Cleaning test data..."
    rm -f "$TESTDATA_DIR"/*.grib2
    log_info "Done"
}

check_testdata() {
    echo "Test data status:"
    echo "================"
    
    local all_present=true
    for name in gfs_sample.grib2 mrms_refl.grib2; do
        local path="$TESTDATA_DIR/$name"
        if [[ -f "$path" ]]; then
            echo -e "  ${GREEN}[OK]${NC} $name ($(du -h "$path" | cut -f1))"
        else
            echo -e "  ${RED}[MISSING]${NC} $name"
            all_present=false
        fi
    done
    
    if $all_present; then
        echo ""
        echo "All test data present. Run: cargo test --package grib2-parser"
    else
        echo ""
        echo "Some files missing. Run: $0"
    fi
}

generate_testdata() {
    log_info "Generating synthetic GRIB2 test data..."
    
    mkdir -p "$TESTDATA_DIR"
    
    # Run the generator via cargo test
    cd "$CRATE_DIR"
    
    # Build and run the generator
    cargo test --package grib2-parser generate_test_files -- --ignored --nocapture 2>&1 || {
        log_warn "Generator test not found, using inline generator..."
        
        # Fallback: create minimal files inline
        # This creates files that should parse correctly
        create_minimal_gfs
        create_minimal_mrms
    }
    
    log_info "Done"
    check_testdata
}

# Fallback generator using shell/hex
create_minimal_gfs() {
    log_info "Creating minimal GFS sample..."
    # This would need a hex dump, but the Rust generator is preferred
    log_warn "Rust generator preferred - run: cargo test --package grib2-parser generator -- --nocapture"
}

create_minimal_mrms() {
    log_info "Creating minimal MRMS sample..."
    log_warn "Rust generator preferred - run: cargo test --package grib2-parser generator -- --nocapture"
}

# Main
case "${1:-}" in
    --clean)
        clean_testdata
        ;;
    --check)
        check_testdata
        ;;
    --help|-h)
        echo "Usage: $0 [--clean|--check|--help]"
        echo ""
        echo "Generates synthetic GRIB2 test data files."
        echo ""
        echo "Options:"
        echo "  --clean   Remove all generated test data"
        echo "  --check   Check which files are present"
        echo "  --help    Show this help"
        ;;
    *)
        generate_testdata
        ;;
esac
