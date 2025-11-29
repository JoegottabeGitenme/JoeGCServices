#!/bin/bash
# Run all renderer benchmarks with optional baseline comparison
#
# Usage:
#   ./scripts/run_benchmarks.sh              # Run all benchmarks
#   ./scripts/run_benchmarks.sh save         # Save as new baseline
#   ./scripts/run_benchmarks.sh compare      # Compare with saved baseline
#   ./scripts/run_benchmarks.sh gradient     # Run only gradient benchmarks
#   ./scripts/run_benchmarks.sh barbs        # Run only barb benchmarks
#   ./scripts/run_benchmarks.sh contour      # Run only contour benchmarks

set -e

BASELINE_NAME="main"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

ACTION=${1:-"run"}
FILTER=${2:-""}

echo "=== Weather WMS Renderer Benchmarks ==="
echo "Project root: $PROJECT_ROOT"
echo ""

# Build first to ensure we have the latest code
echo "Building renderer in release mode..."
cargo build --release --package renderer 2>/dev/null

case "$ACTION" in
    save)
        echo "Running benchmarks and saving as baseline '$BASELINE_NAME'..."
        cargo bench --package renderer -- --save-baseline "$BASELINE_NAME"
        echo ""
        echo "Baseline saved. Use './scripts/run_benchmarks.sh compare' to compare future runs."
        ;;
    
    compare)
        if [ ! -d "target/criterion/$BASELINE_NAME" ]; then
            echo "ERROR: No baseline found. Run './scripts/run_benchmarks.sh save' first."
            exit 1
        fi
        echo "Running benchmarks and comparing with baseline '$BASELINE_NAME'..."
        cargo bench --package renderer -- --baseline "$BASELINE_NAME"
        ;;
    
    run)
        if [ -n "$FILTER" ]; then
            echo "Running benchmarks matching: $FILTER"
            cargo bench --package renderer -- "$FILTER"
        else
            echo "Running all benchmarks..."
            cargo bench --package renderer
        fi
        ;;
    
    gradient|barbs|contour)
        echo "Running $ACTION benchmarks..."
        cargo bench --package renderer --bench "${ACTION}_benchmarks" 2>/dev/null || \
        cargo bench --package renderer --bench "render_benchmarks" -- "$ACTION"
        ;;
    
    quick)
        echo "Running quick benchmark subset..."
        cargo bench --package renderer -- --quick
        ;;
    
    *)
        # Treat as a filter pattern
        echo "Running benchmarks matching: $ACTION"
        cargo bench --package renderer -- "$ACTION"
        ;;
esac

echo ""
echo "=== Benchmark Complete ==="
echo "HTML report: target/criterion/report/index.html"
echo ""
echo "Tips:"
echo "  - Open the HTML report for detailed graphs and analysis"
echo "  - Use './scripts/run_benchmarks.sh save' before making changes"
echo "  - Use './scripts/run_benchmarks.sh compare' after changes to see impact"
