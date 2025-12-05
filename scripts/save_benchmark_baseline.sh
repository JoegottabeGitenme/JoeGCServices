#!/bin/bash
# Save Benchmark Baseline Script
#
# Saves the current benchmark results to a named baseline for later comparison.
# Baselines are stored in benchmarks/baselines/ directory.
#
# Usage:
#   ./scripts/save_benchmark_baseline.sh <baseline_name> [description]
#
# Examples:
#   ./scripts/save_benchmark_baseline.sh initial "Initial baseline before optimizations"
#   ./scripts/save_benchmark_baseline.sh after-shm "After /dev/shm optimization"
#   ./scripts/save_benchmark_baseline.sh v1.2.0 "Release 1.2.0 baseline"
#
# To compare baselines later:
#   ./scripts/compare_benchmark_baselines.sh initial after-shm

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BASELINES_DIR="$PROJECT_ROOT/benchmarks/baselines"
CRITERION_DIR="$PROJECT_ROOT/target/criterion"

# Parse arguments
BASELINE_NAME="${1:-}"
DESCRIPTION="${2:-No description provided}"

if [ -z "$BASELINE_NAME" ]; then
    echo "Usage: $0 <baseline_name> [description]"
    echo ""
    echo "Examples:"
    echo "  $0 initial 'Initial baseline before optimizations'"
    echo "  $0 after-shm 'After /dev/shm temp file optimization'"
    echo ""
    echo "Existing baselines:"
    if [ -d "$BASELINES_DIR" ]; then
        ls -1 "$BASELINES_DIR" 2>/dev/null | sed 's/^/  /' || echo "  (none)"
    else
        echo "  (none)"
    fi
    exit 1
fi

# Validate baseline name (alphanumeric, dash, underscore only)
if [[ ! "$BASELINE_NAME" =~ ^[a-zA-Z0-9_-]+$ ]]; then
    echo "Error: Baseline name must contain only letters, numbers, dashes, and underscores"
    exit 1
fi

# Check if criterion data exists
if [ ! -d "$CRITERION_DIR" ]; then
    echo "Error: No benchmark data found at $CRITERION_DIR"
    echo "Run benchmarks first: cargo bench --package renderer --bench goes_benchmarks"
    exit 1
fi

# Create baselines directory
mkdir -p "$BASELINES_DIR"

BASELINE_DIR="$BASELINES_DIR/$BASELINE_NAME"
TIMESTAMP=$(date +"%Y-%m-%d %H:%M:%S")

# Check if baseline already exists
if [ -d "$BASELINE_DIR" ]; then
    echo "Warning: Baseline '$BASELINE_NAME' already exists."
    read -p "Overwrite? [y/N] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Aborted."
        exit 1
    fi
    rm -rf "$BASELINE_DIR"
fi

echo "Saving benchmark baseline: $BASELINE_NAME"
echo ""

# Create baseline directory
mkdir -p "$BASELINE_DIR"

# Save metadata
cat > "$BASELINE_DIR/metadata.json" << EOF
{
    "name": "$BASELINE_NAME",
    "description": "$DESCRIPTION",
    "timestamp": "$TIMESTAMP",
    "git_commit": "$(git rev-parse HEAD 2>/dev/null || echo 'unknown')",
    "git_branch": "$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'unknown')",
    "platform": "$(uname -s) $(uname -m)",
    "rust_version": "$(rustc --version 2>/dev/null || echo 'unknown')"
}
EOF

# Copy criterion data (the estimates.json files contain the actual measurements)
echo "Copying benchmark data..."

# Find and copy all estimates.json files (these contain the actual timing data)
find "$CRITERION_DIR" -name "estimates.json" | while read -r file; do
    # Get relative path from criterion dir
    rel_path="${file#$CRITERION_DIR/}"
    dest_dir="$BASELINE_DIR/data/$(dirname "$rel_path")"
    mkdir -p "$dest_dir"
    cp "$file" "$dest_dir/"
done

# Also copy benchmark.json files (contain configuration)
find "$CRITERION_DIR" -name "benchmark.json" | while read -r file; do
    rel_path="${file#$CRITERION_DIR/}"
    dest_dir="$BASELINE_DIR/data/$(dirname "$rel_path")"
    mkdir -p "$dest_dir"
    cp "$file" "$dest_dir/"
done

# Generate summary report
echo "Generating summary report..."

SUMMARY_FILE="$BASELINE_DIR/summary.txt"
cat > "$SUMMARY_FILE" << EOF
GOES Benchmark Baseline: $BASELINE_NAME
========================================
Timestamp: $TIMESTAMP
Description: $DESCRIPTION
Git Commit: $(git rev-parse --short HEAD 2>/dev/null || echo 'unknown')
Platform: $(uname -s) $(uname -m)

Key Metrics:
EOF

# Extract key metrics from estimates.json files
extract_metric() {
    local name="$1"
    local path="$2"
    local file="$BASELINE_DIR/data/$path/estimates.json"
    if [ -f "$file" ]; then
        # Extract mean time in appropriate units
        local mean_ns=$(jq -r '.mean.point_estimate' "$file" 2>/dev/null)
        if [ -n "$mean_ns" ] && [ "$mean_ns" != "null" ]; then
            # Convert to human-readable
            if (( $(echo "$mean_ns > 1000000000" | bc -l) )); then
                local time=$(echo "scale=2; $mean_ns / 1000000000" | bc)
                echo "  $name: ${time} s"
            elif (( $(echo "$mean_ns > 1000000" | bc -l) )); then
                local time=$(echo "scale=2; $mean_ns / 1000000" | bc)
                echo "  $name: ${time} ms"
            elif (( $(echo "$mean_ns > 1000" | bc -l) )); then
                local time=$(echo "scale=2; $mean_ns / 1000" | bc)
                echo "  $name: ${time} µs"
            else
                echo "  $name: ${mean_ns} ns"
            fi
        fi
    fi
}

# Add key metrics to summary
{
    echo ""
    echo "Temp File I/O (2.8MB):"
    extract_metric "  Write+Read+Delete" "temp_file_io/system_temp_write_read_delete/2.8MB_typical/new"
    extract_metric "  Memory Copy Baseline" "temp_file_io/memory_copy_baseline/2.8MB_typical/new"
    
    echo ""
    echo "Projection Transforms:"
    extract_metric "  geo_to_grid (65K)" "goes_projection/geo_to_grid/65536/new"
    
    echo ""
    echo "Resampling (256x256):"
    extract_metric "  Bilinear Only" "goes_resample/bilinear_only/full_conus_z4/new"
    extract_metric "  With Projection" "goes_resample/with_projection/full_conus_z4/new"
    
    echo ""
    echo "Full Pipeline:"
    extract_metric "  IR Tile 256x256" "goes_pipeline/ir_tile_256x256/new"
    extract_metric "  Resample Only" "goes_pipeline/resample_only_256x256/new"
    extract_metric "  Color+PNG Only" "goes_pipeline/color_and_png_only_256x256/new"
    
    echo ""
    echo "PNG Encoding:"
    extract_metric "  256x256" "goes_png/encode/256x256/new"
} >> "$SUMMARY_FILE"

# Display summary
echo ""
cat "$SUMMARY_FILE"

echo ""
echo "=========================================="
echo "Baseline saved to: $BASELINE_DIR"
echo ""
echo "To compare with another baseline later:"
echo "  ./scripts/compare_benchmark_baselines.sh $BASELINE_NAME <other_baseline>"
echo ""
