#!/bin/bash
# Run all renderer benchmarks with optional baseline comparison
#
# Usage:
#   ./scripts/run_benchmarks.sh              # Run all benchmarks
#   ./scripts/run_benchmarks.sh save         # Save current results as baseline (copies to baseline dir)
#   ./scripts/run_benchmarks.sh compare      # Compare with saved baseline
#   ./scripts/run_benchmarks.sh gradient     # Run only gradient benchmarks
#   ./scripts/run_benchmarks.sh barbs        # Run only barb benchmarks
#   ./scripts/run_benchmarks.sh contour      # Run only contour benchmarks
#
# Note: Criterion 0.5 automatically compares with previous runs stored in target/criterion.
# The 'save' and 'compare' commands provide manual baseline management.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BASELINE_DIR="$PROJECT_ROOT/target/criterion-baseline"
CRITERION_DIR="$PROJECT_ROOT/target/criterion"

cd "$PROJECT_ROOT"

ACTION=${1:-"run"}

echo "=== Weather WMS Renderer Benchmarks ==="
echo "Project root: $PROJECT_ROOT"
echo ""

# Build first to ensure we have the latest code
echo "Building renderer in release mode..."
cargo build --release --package renderer 2>/dev/null

case "$ACTION" in
    save)
        echo "Running benchmarks and saving as baseline..."
        cargo bench --package renderer
        
        # Copy results to baseline directory
        if [ -d "$CRITERION_DIR" ]; then
            rm -rf "$BASELINE_DIR"
            cp -r "$CRITERION_DIR" "$BASELINE_DIR"
            echo ""
            echo "Baseline saved to: $BASELINE_DIR"
            echo "Use './scripts/run_benchmarks.sh compare' after making changes."
        else
            echo "ERROR: No criterion results found to save."
            exit 1
        fi
        ;;
    
    compare)
        if [ ! -d "$BASELINE_DIR" ]; then
            echo "ERROR: No baseline found at $BASELINE_DIR"
            echo "Run './scripts/run_benchmarks.sh save' first to create a baseline."
            exit 1
        fi
        
        echo "Running benchmarks and comparing with saved baseline..."
        cargo bench --package renderer
        
        echo ""
        echo "=== Comparison with Baseline ==="
        echo ""
        
        # Compare key metrics between baseline and current
        compare_benchmark() {
            local name=$1
            local baseline_file="$BASELINE_DIR/$name/new/estimates.json"
            local current_file="$CRITERION_DIR/$name/new/estimates.json"
            
            if [ -f "$baseline_file" ] && [ -f "$current_file" ]; then
                local baseline_mean=$(jq -r '.mean.point_estimate' "$baseline_file" 2>/dev/null)
                local current_mean=$(jq -r '.mean.point_estimate' "$current_file" 2>/dev/null)
                
                if [ -n "$baseline_mean" ] && [ -n "$current_mean" ] && [ "$baseline_mean" != "null" ] && [ "$current_mean" != "null" ]; then
                    local change=$(echo "scale=2; (($current_mean - $baseline_mean) / $baseline_mean) * 100" | bc 2>/dev/null)
                    local baseline_ms=$(echo "scale=3; $baseline_mean / 1000000" | bc 2>/dev/null)
                    local current_ms=$(echo "scale=3; $current_mean / 1000000" | bc 2>/dev/null)
                    
                    if [ -n "$change" ]; then
                        local indicator="="
                        if (( $(echo "$change < -5" | bc -l) )); then
                            indicator="improved"
                        elif (( $(echo "$change > 5" | bc -l) )); then
                            indicator="REGRESSED"
                        fi
                        printf "  %-50s %8s ms -> %8s ms  (%+.1f%% %s)\n" "$name" "$baseline_ms" "$current_ms" "$change" "$indicator"
                    fi
                fi
            fi
        }
        
        # Find all benchmarks and compare
        if command -v jq &> /dev/null; then
            for dir in "$CRITERION_DIR"/*/; do
                if [ -d "$dir" ]; then
                    group=$(basename "$dir")
                    if [ "$group" != "report" ]; then
                        echo "$group:"
                        for bench_dir in "$dir"*/; do
                            if [ -d "$bench_dir/new" ]; then
                                bench_name=$(basename "$bench_dir")
                                compare_benchmark "$group/$bench_name"
                            fi
                            # Check for nested benchmarks
                            for nested_dir in "$bench_dir"*/; do
                                if [ -d "$nested_dir/new" ]; then
                                    nested_name=$(basename "$nested_dir")
                                    compare_benchmark "$group/$bench_name/$nested_name"
                                fi
                            done
                        done
                        echo ""
                    fi
                fi
            done
        else
            echo "Note: Install 'jq' for detailed comparison output."
            echo "Comparison data available in HTML report."
        fi
        ;;
    
    run)
        echo "Running all benchmarks..."
        cargo bench --package renderer
        ;;
    
    gradient)
        echo "Running gradient/render benchmarks..."
        cargo bench --package renderer --bench render_benchmarks
        ;;
    
    barbs)
        echo "Running barb benchmarks..."
        cargo bench --package renderer --bench barbs_benchmarks
        ;;
    
    contour)
        echo "Running contour benchmarks..."
        cargo bench --package renderer --bench contour_benchmarks
        ;;
    
    quick)
        echo "Running quick benchmark (temperature only)..."
        cargo bench --package renderer -- "temperature"
        ;;
    
    list)
        echo "Available benchmark groups:"
        echo "  - render_benchmarks (gradient rendering, PNG encoding)"
        echo "  - barbs_benchmarks (wind barb rendering)"
        echo "  - contour_benchmarks (contour/isoline generation)"
        echo ""
        echo "Commands:"
        echo "  ./scripts/run_benchmarks.sh           # Run all benchmarks"
        echo "  ./scripts/run_benchmarks.sh save      # Save baseline"
        echo "  ./scripts/run_benchmarks.sh compare   # Compare with baseline"
        echo "  ./scripts/run_benchmarks.sh gradient  # Run gradient benchmarks"
        echo "  ./scripts/run_benchmarks.sh barbs     # Run barb benchmarks"
        echo "  ./scripts/run_benchmarks.sh contour   # Run contour benchmarks"
        echo "  ./scripts/run_benchmarks.sh quick     # Run quick subset"
        ;;
    
    *)
        # Treat as a filter pattern
        echo "Running benchmarks matching: $ACTION"
        cargo bench --package renderer -- "$ACTION"
        ;;
esac

echo ""
echo "=== Benchmark Complete ==="
echo "HTML report: file://$CRITERION_DIR/report/index.html"
echo ""
echo "Tips:"
echo "  - Open the HTML report for detailed graphs and analysis"
echo "  - Use './scripts/run_benchmarks.sh save' before making changes"
echo "  - Use './scripts/run_benchmarks.sh compare' after changes to see impact"
echo "  - Criterion automatically compares with the previous run"
