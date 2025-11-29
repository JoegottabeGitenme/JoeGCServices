#!/bin/bash
# Generate CPU flamegraph of the WMS API server under load.
#
# Prerequisites:
#   - Linux with perf installed: sudo apt install linux-tools-common linux-tools-generic
#   - Rust flamegraph tools: cargo install flamegraph
#   - stackcollapse-perf.pl and flamegraph.pl (from https://github.com/brendangregg/FlameGraph)
#
# Usage:
#   ./scripts/profile_flamegraph.sh              # Default 30s with quick load test
#   ./scripts/profile_flamegraph.sh 60           # 60 second profile
#   ./scripts/profile_flamegraph.sh 30 stress    # 30s with stress scenario
#   ./scripts/profile_flamegraph.sh bench        # Profile criterion benchmarks

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

DURATION=${1:-30}
MODE=${2:-"quick"}
OUTPUT_DIR="$PROJECT_ROOT/profiling"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

mkdir -p "$OUTPUT_DIR"

echo "=== WMS API Flamegraph Profiling ==="
echo "Duration: ${DURATION}s"
echo "Mode: $MODE"
echo "Output: $OUTPUT_DIR"
echo ""

# Check for required tools
if ! command -v perf &> /dev/null; then
    echo "ERROR: 'perf' not found. Install with: sudo apt install linux-tools-common linux-tools-generic"
    exit 1
fi

cleanup() {
    echo ""
    echo "Cleaning up..."
    [ -n "$WMS_PID" ] && kill "$WMS_PID" 2>/dev/null || true
    [ -n "$LOAD_PID" ] && kill "$LOAD_PID" 2>/dev/null || true
    rm -f "$OUTPUT_DIR/perf.data" "$OUTPUT_DIR/perf.data.old"
}
trap cleanup EXIT

case "$MODE" in
    bench)
        # Profile the benchmark binary itself
        echo "Building benchmark binary with debug symbols..."
        RUSTFLAGS="-C debuginfo=2" cargo build --release --package renderer --bench render_benchmarks
        
        echo "Profiling benchmarks..."
        BENCH_BIN=$(find target/release/deps -name "render_benchmarks-*" -type f -executable | head -1)
        
        if [ -z "$BENCH_BIN" ]; then
            echo "ERROR: Could not find benchmark binary"
            exit 1
        fi
        
        if command -v flamegraph &> /dev/null; then
            flamegraph -o "$OUTPUT_DIR/flamegraph_bench_$TIMESTAMP.svg" -- "$BENCH_BIN" --bench
        else
            sudo perf record -g --call-graph dwarf -o "$OUTPUT_DIR/perf.data" -- "$BENCH_BIN" --bench --profile-time 10
            sudo perf script -i "$OUTPUT_DIR/perf.data" > "$OUTPUT_DIR/perf_script.txt"
            echo "perf data saved. Use 'perf report -i $OUTPUT_DIR/perf.data' to analyze"
        fi
        ;;
        
    *)
        # Profile the WMS API server under load
        echo "Building WMS API with debug symbols..."
        RUSTFLAGS="-C debuginfo=2" cargo build --release --package wms-api
        
        echo "Starting WMS API server..."
        ./target/release/wms-api &
        WMS_PID=$!
        sleep 3
        
        # Verify server is running
        if ! kill -0 $WMS_PID 2>/dev/null; then
            echo "ERROR: WMS API failed to start"
            exit 1
        fi
        
        # Check if server is responding
        if ! curl -s --max-time 5 http://localhost:8080/health > /dev/null 2>&1; then
            echo "WARNING: Server health check failed, continuing anyway..."
        else
            echo "Server is healthy"
        fi
        
        echo ""
        echo "Starting load test in background..."
        if [ -f "$SCRIPT_DIR/run_load_test.sh" ]; then
            timeout $((DURATION + 10))s "$SCRIPT_DIR/run_load_test.sh" "$MODE" --duration "$DURATION" &
            LOAD_PID=$!
        else
            echo "WARNING: Load test script not found, using curl loop"
            for i in $(seq 1 $((DURATION * 10))); do
                curl -s "http://localhost:8080/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=gfs_TMP&STYLE=temperature&FORMAT=image/png&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=5&TILEROW=10&TILECOL=8" > /dev/null &
            done &
            LOAD_PID=$!
        fi
        sleep 2
        
        echo "Recording performance data for ${DURATION}s..."
        
        if command -v flamegraph &> /dev/null; then
            # Use cargo flamegraph if available (easier)
            sudo flamegraph -p $WMS_PID -o "$OUTPUT_DIR/flamegraph_wms_$TIMESTAMP.svg" -- sleep "$DURATION"
            FLAMEGRAPH_FILE="$OUTPUT_DIR/flamegraph_wms_$TIMESTAMP.svg"
        else
            # Use perf directly
            sudo perf record -g --call-graph dwarf -p $WMS_PID -o "$OUTPUT_DIR/perf.data" -- sleep "$DURATION"
            
            echo "Generating flamegraph..."
            if command -v stackcollapse-perf.pl &> /dev/null; then
                sudo perf script -i "$OUTPUT_DIR/perf.data" | stackcollapse-perf.pl | flamegraph.pl > "$OUTPUT_DIR/flamegraph_wms_$TIMESTAMP.svg"
                FLAMEGRAPH_FILE="$OUTPUT_DIR/flamegraph_wms_$TIMESTAMP.svg"
            else
                echo "WARNING: stackcollapse-perf.pl not found"
                echo "Install FlameGraph tools from: https://github.com/brendangregg/FlameGraph"
                echo "Or use: sudo perf report -i $OUTPUT_DIR/perf.data"
                FLAMEGRAPH_FILE=""
            fi
        fi
        
        echo "Stopping load test..."
        kill $LOAD_PID 2>/dev/null || true
        wait $LOAD_PID 2>/dev/null || true
        ;;
esac

echo ""
echo "=== Profiling Complete ==="
if [ -n "$FLAMEGRAPH_FILE" ] && [ -f "$FLAMEGRAPH_FILE" ]; then
    echo "Flamegraph: $FLAMEGRAPH_FILE"
    echo ""
    echo "Open in browser:"
    echo "  xdg-open $FLAMEGRAPH_FILE"
    echo "  # or: open $FLAMEGRAPH_FILE (macOS)"
fi

echo ""
echo "Additional analysis commands:"
echo "  perf report -i $OUTPUT_DIR/perf.data                    # Interactive report"
echo "  perf report -i $OUTPUT_DIR/perf.data --sort comm,dso,symbol  # By function"
echo "  perf annotate -i $OUTPUT_DIR/perf.data                  # Source annotation"
