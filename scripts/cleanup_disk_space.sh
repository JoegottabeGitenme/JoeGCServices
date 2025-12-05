#!/bin/bash
# Cleanup Disk Space Script
#
# Reduces disk usage by cleaning up unnecessary build artifacts and data.
# Run this periodically to keep disk usage low.
#
# Usage:
#   ./scripts/cleanup_disk_space.sh [options]
#
# Options:
#   --dry-run    Show what would be deleted without actually deleting
#   --aggressive Clean everything including release builds

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

DRY_RUN=0
AGGRESSIVE=0

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --aggressive)
            AGGRESSIVE=1
            shift
            ;;
        --help|-h)
            cat << EOF
Cleanup Disk Space Script

Usage: $0 [options]

Options:
  --dry-run       Show what would be deleted without deleting
  --aggressive    Clean everything including release builds
  --help          Show this help

What gets cleaned:
  Normal mode:
    - Debug builds (target/debug/)
    - Old criterion benchmark data (keeps latest)
    - Cargo incremental build cache
    
  Aggressive mode (all of normal plus):
    - Release builds (target/release/)
    - All criterion data
    - Downloaded test data (if any)

EOF
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

cd "$PROJECT_ROOT"

echo "============================================================"
echo "  Disk Space Cleanup"
echo "============================================================"
echo ""

if [ $DRY_RUN -eq 1 ]; then
    echo "DRY RUN MODE - No files will be deleted"
    echo ""
fi

# Show current disk usage
echo "Current disk usage:"
du -sh target 2>/dev/null || echo "  target: (not found)"
du -sh data 2>/dev/null || echo "  data: (not found)"
du -sh benchmarks/baselines 2>/dev/null || echo "  benchmarks/baselines: (not found)"
echo ""

TOTAL_SAVED=0

# Function to clean a directory
clean_dir() {
    local dir="$1"
    local desc="$2"
    
    if [ ! -d "$dir" ]; then
        return
    fi
    
    local size=$(du -sk "$dir" 2>/dev/null | cut -f1)
    local size_mb=$((size / 1024))
    
    if [ $DRY_RUN -eq 1 ]; then
        echo "Would clean: $desc ($size_mb MB)"
    else
        echo "Cleaning: $desc ($size_mb MB)..."
        rm -rf "$dir"
        TOTAL_SAVED=$((TOTAL_SAVED + size_mb))
    fi
}

# 1. Clean debug builds (usually the largest)
clean_dir "target/debug" "Debug builds"

# 2. Clean incremental compilation cache
clean_dir "target/.rustc_info.json" "Rustc info"
find target -type d -name "incremental" -exec rm -rf {} + 2>/dev/null || true

# 3. Clean old criterion data (keep only latest baseline)
if [ -d "target/criterion" ]; then
    # Keep the latest data but clean old history
    if [ $DRY_RUN -eq 1 ]; then
        echo "Would clean: Old criterion benchmark history"
    else
        echo "Cleaning: Old criterion benchmark history..."
        # Keep only the 'new' directories and current report
        find target/criterion -type d -name "base" -exec rm -rf {} + 2>/dev/null || true
        find target/criterion -type d -name "change" -exec rm -rf {} + 2>/dev/null || true
        # Calculate saved space
        size=$(du -sk "target/criterion" 2>/dev/null | cut -f1 || echo 0)
        size_mb=$((size / 1024))
        echo "  Criterion data now: $size_mb MB"
    fi
fi

# 4. Aggressive mode cleanups
if [ $AGGRESSIVE -eq 1 ]; then
    echo ""
    echo "AGGRESSIVE MODE enabled"
    echo ""
    
    # Clean release builds
    clean_dir "target/release" "Release builds"
    
    # Clean all criterion data
    clean_dir "target/criterion" "All criterion benchmark data"
    
    # Clean bench profile
    clean_dir "target/bench" "Bench profile builds"
    
    # Clean test data if it exists
    if [ -d "data" ]; then
        echo "Data directory found. Clean it? [y/N]"
        read -r response
        if [[ "$response" =~ ^[Yy]$ ]]; then
            clean_dir "data" "Downloaded test data"
        fi
    fi
fi

echo ""
echo "============================================================"

if [ $DRY_RUN -eq 1 ]; then
    echo "DRY RUN complete - no files were deleted"
else
    echo "Cleanup complete!"
    echo "Estimated space saved: ${TOTAL_SAVED} MB"
fi

echo ""
echo "New disk usage:"
du -sh target 2>/dev/null || echo "  target: (not found)"

echo ""
echo "To rebuild:"
echo "  cargo build              # Debug build"
echo "  cargo build --release    # Release build"
echo "  cargo bench              # Benchmarks (will rebuild release)"
echo ""
