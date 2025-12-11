#!/bin/bash
# Test script for netcdf-parser crate
#
# Usage:
#   ./test.sh              # Run all tests
#   ./test.sh --verbose    # Run with verbose output
#   ./test.sh --release    # Run tests in release mode
#   ./test.sh --nocapture  # Show println! output
#
# Requirements:
#   - libnetcdf-dev and libhdf5-dev installed
#   - Rust toolchain

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Parse arguments
CARGO_ARGS=""
TEST_ARGS=""

for arg in "$@"; do
    case $arg in
        --verbose|-v)
            CARGO_ARGS="$CARGO_ARGS -v"
            ;;
        --release)
            CARGO_ARGS="$CARGO_ARGS --release"
            ;;
        --nocapture)
            TEST_ARGS="$TEST_ARGS --nocapture"
            ;;
        --help|-h)
            echo "Usage: $0 [--verbose] [--release] [--nocapture]"
            echo ""
            echo "Options:"
            echo "  --verbose, -v    Verbose cargo output"
            echo "  --release        Build and test in release mode"
            echo "  --nocapture      Show test output (println!)"
            echo "  --help, -h       Show this help"
            exit 0
            ;;
        *)
            # Pass unknown args to cargo test
            TEST_ARGS="$TEST_ARGS $arg"
            ;;
    esac
done

echo "=== netcdf-parser Test Suite ==="
echo ""

# Check dependencies
echo "Checking dependencies..."
if ! pkg-config --exists netcdf 2>/dev/null; then
    echo "WARNING: libnetcdf not found. Install with: apt install libnetcdf-dev"
fi
if ! pkg-config --exists hdf5 2>/dev/null; then
    echo "WARNING: libhdf5 not found. Install with: apt install libhdf5-dev"
fi

# Run tests
echo ""
echo "Running tests..."
cargo test $CARGO_ARGS -p netcdf-parser -- $TEST_ARGS

echo ""
echo "=== All tests passed! ==="
