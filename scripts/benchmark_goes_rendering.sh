#!/bin/bash
# GOES Rendering Pipeline Benchmark Script
#
# This script runs the GOES rendering benchmarks and generates a summary report.
# It tests the key performance bottlenecks identified in the performance analysis:
#
# 1. Temp file I/O (PRIMARY bottleneck - NetCDF parsing requires temp files)
# 2. Geostationary projection transforms (per-pixel coordinate conversion)
# 3. Grid resampling (bilinear interpolation)
# 4. Color mapping and PNG encoding
#
# Usage:
#   ./scripts/benchmark_goes_rendering.sh [options]
#
# Options:
#   --quick           Run fewer iterations (faster, less accurate)
#   --io-only         Only run I/O benchmarks
#   --full            Run all benchmarks with extra iterations
#   --save            Save raw criterion output to file
#   --baseline NAME   Save results as a named baseline after running
#   --compare NAME    Compare results with a saved baseline after running

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
REPORT_DIR="$PROJECT_ROOT/target/benchmark-reports"
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
REPORT_FILE="$REPORT_DIR/goes_benchmark_$TIMESTAMP.txt"

# Parse arguments
QUICK=""
IO_ONLY=""
SAVE_OUTPUT=""
BENCHMARK_FILTER=""
SAVE_BASELINE=""
COMPARE_BASELINE=""

show_help() {
    cat << EOF
GOES Rendering Pipeline Benchmark Script

Usage: $0 [options]

Options:
  --quick           Run fewer iterations (faster, less accurate)
  --io-only         Only run I/O benchmarks (temp file overhead)
  --full            Run all benchmarks with extra iterations
  --save            Save raw criterion output to file
  --baseline NAME   Save results as a named baseline after running
  --compare NAME    Compare results with a saved baseline after running
  --help            Show this help message

Examples:
  $0                          # Run all benchmarks
  $0 --io-only                # Run only I/O benchmarks
  $0 --baseline before-opt    # Run and save as 'before-opt' baseline
  $0 --compare initial        # Run and compare with 'initial' baseline
  $0 --quick --compare initial # Quick run and compare

Baseline Management:
  ./scripts/save_benchmark_baseline.sh NAME "description"
  ./scripts/compare_benchmark_baselines.sh NAME1 NAME2

EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --help|-h)
            show_help
            ;;
        --quick)
            QUICK="--quick"
            shift
            ;;
        --io-only)
            BENCHMARK_FILTER="temp_file_io|netcdf_io"
            shift
            ;;
        --full)
            # Run with more samples
            export CRITERION_SAMPLE_SIZE=200
            shift
            ;;
        --save)
            SAVE_OUTPUT="1"
            mkdir -p "$REPORT_DIR"
            shift
            ;;
        --baseline)
            SAVE_BASELINE="$2"
            shift 2
            ;;
        --compare)
            COMPARE_BASELINE="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

cd "$PROJECT_ROOT"

echo "============================================================"
echo "  GOES Rendering Pipeline Benchmarks"
echo "============================================================"
echo ""
echo "This benchmark suite tests the GOES satellite tile rendering"
echo "pipeline to identify performance bottlenecks."
echo ""
echo "Key areas tested:"
echo "  1. Temp File I/O   - NetCDF library requires temp files (PRIMARY BOTTLENECK)"
echo "  2. Projection      - Geostationary to Mercator transforms"
echo "  3. Resampling      - Grid interpolation for tile generation"
echo "  4. Color/PNG       - Color mapping and PNG encoding"
echo ""
echo "Starting benchmarks..."
echo ""

# Build first to separate compile time from benchmark time
echo "Building benchmarks..."
cargo build --release --package renderer --bench goes_benchmarks 2>&1 | grep -E "(Compiling|Finished)" || true
echo ""

# Run benchmarks
if [ -n "$BENCHMARK_FILTER" ]; then
    echo "Running filtered benchmarks: $BENCHMARK_FILTER"
    echo ""
    
    if [ -n "$SAVE_OUTPUT" ]; then
        cargo bench --package renderer --bench goes_benchmarks $QUICK -- "$BENCHMARK_FILTER" 2>&1 | tee "$REPORT_FILE"
    else
        cargo bench --package renderer --bench goes_benchmarks $QUICK -- "$BENCHMARK_FILTER" 2>&1
    fi
else
    if [ -n "$SAVE_OUTPUT" ]; then
        cargo bench --package renderer --bench goes_benchmarks $QUICK 2>&1 | tee "$REPORT_FILE"
    else
        cargo bench --package renderer --bench goes_benchmarks $QUICK 2>&1
    fi
fi

echo ""
echo "============================================================"
echo "  Benchmark Summary"
echo "============================================================"
echo ""

# Generate summary (parse criterion output if saved)
if [ -n "$SAVE_OUTPUT" ] && [ -f "$REPORT_FILE" ]; then
    echo "Raw output saved to: $REPORT_FILE"
    echo ""
    
    # Extract key metrics
    echo "Key Results:"
    echo ""
    
    echo "TEMP FILE I/O (2.8MB typical GOES file):"
    grep -E "temp_file_io.*2.8MB" "$REPORT_FILE" | grep "time:" | head -4 || echo "  (run with --save to see results)"
    echo ""
    
    echo "PROJECTION TRANSFORMS (256x256 tile = 65K transforms):"
    grep -E "geo_to_grid/65536" "$REPORT_FILE" | grep "time:" | head -1 || echo "  (run with --save to see results)"
    echo ""
    
    echo "FULL PIPELINE (256x256 IR tile):"
    grep -E "ir_tile_256x256" "$REPORT_FILE" | grep "time:" | head -1 || echo "  (run with --save to see results)"
fi

echo ""
echo "============================================================"
echo "  Performance Analysis Notes"
echo "============================================================"
echo ""
echo "BOTTLENECK PRIORITY (based on GOES_RENDERING_PERFORMANCE_ANALYSIS.md):"
echo ""
echo "1. TEMP FILE I/O (~65% of cache miss time)"
echo "   - NetCDF library requires writing to disk, reading back, then deleting"
echo "   - For 2.8MB file: expect 10-50ms on HDD, 2-10ms on SSD"
echo "   - Optimization: Use /dev/shm on Linux (memory-backed filesystem)"
echo ""
echo "2. PROJECTION TRANSFORMS (~0.2% but scales with tiles)"
echo "   - 18 trig operations per pixel"
echo "   - For 256x256 tile: ~65K transforms = 1-2ms"
echo "   - Optimization: Pre-compute lookup tables, SIMD vectorization"
echo ""
echo "3. COLOR/PNG ENCODING (~14% of pipeline)"
echo "   - Already well optimized"
echo "   - ~1ms for 256x256 tile"
echo ""
echo "For detailed analysis, see: docs/GOES_RENDERING_PERFORMANCE_ANALYSIS.md"
echo ""

# Print HTML report location if it exists
if [ -d "$PROJECT_ROOT/target/criterion" ]; then
    echo "Detailed HTML reports available at:"
    echo "  file://$PROJECT_ROOT/target/criterion/report/index.html"
    echo ""
fi

# Save baseline if requested
if [ -n "$SAVE_BASELINE" ]; then
    echo ""
    echo "============================================================"
    echo "  Saving Baseline: $SAVE_BASELINE"
    echo "============================================================"
    "$SCRIPT_DIR/save_benchmark_baseline.sh" "$SAVE_BASELINE" "Benchmark run $TIMESTAMP"
fi

# Compare with baseline if requested
if [ -n "$COMPARE_BASELINE" ]; then
    echo ""
    "$SCRIPT_DIR/compare_benchmark_baselines.sh" "$COMPARE_BASELINE" --current
fi

echo "Done!"
