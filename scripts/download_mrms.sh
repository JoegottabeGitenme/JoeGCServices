#!/bin/bash
# Download MRMS (Multi-Radar Multi-Sensor) GRIB2 data from NCEP
# MRMS provides ~1km resolution radar and precipitation data updated every 2 minutes
# Data source: https://mrms.ncep.noaa.gov/2D/

set -e

# Configuration
MRMS_BASE_URL="https://mrms.ncep.noaa.gov/2D"
OUTPUT_DIR="${OUTPUT_DIR:-./data/mrms}"

# Products to download (key=directory, value=description)
declare -A PRODUCTS=(
    ["MergedReflectivityComposite"]="Composite Radar Reflectivity (dBZ)"
    ["PrecipRate"]="Instantaneous Precipitation Rate (mm/hr)"
    ["MultiSensor_QPE_01H_Pass2"]="1-Hour Quantitative Precipitation Estimate (mm)"
)

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

echo "=========================================="
echo "MRMS Data Download Script"
echo "=========================================="
echo "Source: ${MRMS_BASE_URL}"
echo "Output directory: ${OUTPUT_DIR}"
echo "=========================================="

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Download each product
for product in "${!PRODUCTS[@]}"; do
    description="${PRODUCTS[$product]}"
    log_info "Downloading: ${description}"
    
    # Construct the latest file URL
    latest_url="${MRMS_BASE_URL}/${product}/MRMS_${product}.latest.grib2.gz"
    output_file="${OUTPUT_DIR}/${product}_latest.grib2.gz"
    
    # Download the file
    if curl -f -s -S --show-error --retry 3 --retry-delay 5 -o "$output_file" "$latest_url"; then
        file_size=$(du -h "$output_file" | cut -f1)
        log_success "  Downloaded: ${product}_latest.grib2.gz (${file_size})"
        
        # Decompress
        gunzip -f "$output_file" 2>/dev/null || true
        log_info "  Decompressed to: ${product}_latest.grib2"
    else
        log_error "  Failed to download: ${product}"
        rm -f "$output_file"
    fi
done

echo ""
echo "=========================================="
echo "Download Summary"
echo "=========================================="

# List downloaded files
for grib_file in "$OUTPUT_DIR"/*.grib2; do
    if [ -f "$grib_file" ]; then
        filename=$(basename "$grib_file")
        file_size=$(du -h "$grib_file" | cut -f1)
        
        # Get grid info if wgrib2 is available
        if command -v wgrib2 &> /dev/null; then
            grid_info=$(wgrib2 "$grib_file" -grid 2>&1 | grep -E "lat-lon|Lambert" | head -1 || echo "Unknown grid")
            param_info=$(wgrib2 "$grib_file" -s 2>&1 | head -1 | cut -d: -f3-4 || echo "Unknown param")
            echo "  ${filename}"
            echo "    Size: ${file_size}"
            echo "    Grid: ${grid_info}"
            echo "    Param: ${param_info}"
        else
            echo "  ${filename} (${file_size})"
        fi
        echo ""
    fi
done

echo "=========================================="
echo "MRMS Data Properties"
echo "=========================================="
echo "Grid: 7000 x 3500 points (lat-lon)"
echo "Resolution: 0.01 degrees (~1 km)"
echo "Coverage: CONUS (20°N to 55°N, 130°W to 60°W)"
echo "Update frequency: Every 2 minutes"
echo ""
echo "Bounding Box (for ingestion):"
echo "  min_lon: -130.0 (230.0 - 360)"
echo "  min_lat: 20.0"
echo "  max_lon: -60.0 (300.0 - 360)"
echo "  max_lat: 55.0"
echo ""
echo "=========================================="
echo "Next steps:"
echo "1. Ingest data: cargo run --package ingester -- --test-file data/mrms/MergedReflectivityComposite_latest.grib2"
echo "2. Or manually add MRMS config to ingester"
echo "=========================================="
