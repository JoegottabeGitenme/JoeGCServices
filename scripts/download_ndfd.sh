#!/bin/bash
# Download NDFD (National Digital Forecast Database) GRIB2 data from NWS
# NDFD provides official NWS gridded forecasts at 2.5km resolution for CONUS
# Data source: https://tgftp.nws.noaa.gov/SL.us008001/ST.opnl/DF.gr2/DC.ndfd/
#
# NDFD is continuously updated (no run cycles like HRRR/GFS).
# Each file contains multiple forecast valid times for Days 1-3.

set -e

# Configuration
NWS_BASE_URL="https://tgftp.nws.noaa.gov"
NDFD_PATH="SL.us008001/ST.opnl/DF.gr2/DC.ndfd/AR.conus"
OUTPUT_DIR="${OUTPUT_DIR:-./data/ndfd}"

# Forecast period to download (001-003 = Days 1-3, 004-007 = Days 4-7)
FORECAST_PERIOD="${NDFD_FORECAST_PERIOD:-VP.001-003}"

# Core Weather Parameters (8 parameters)
# These match config/models/ndfd.yaml
declare -A PRODUCTS=(
    ["temp"]="Temperature (2m)"
    ["td"]="Dew Point Temperature (2m)"
    ["wspd"]="Wind Speed (10m)"
    ["wdir"]="Wind Direction (10m)"
    ["wgust"]="Wind Gust (surface)"
    ["rhm"]="Relative Humidity (2m)"
    ["sky"]="Sky Cover (cloud %)"
    ["pop12"]="12-Hour Probability of Precipitation"
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
echo "NDFD Data Download Script"
echo "=========================================="
echo "Source: NWS Telecommunications Gateway"
echo "URL: ${NWS_BASE_URL}/${NDFD_PATH}/${FORECAST_PERIOD}/"
echo "Output directory: ${OUTPUT_DIR}"
echo "Parameters: ${#PRODUCTS[@]} (core weather set)"
echo "=========================================="
echo ""

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Record download time
download_time=$(date -u +"%Y%m%d_%H%M%S")

log_info "Downloading NDFD files (${FORECAST_PERIOD})..."
echo ""

success_count=0
fail_count=0
total_size=0

# Download each product
for product in "${!PRODUCTS[@]}"; do
    description="${PRODUCTS[$product]}"
    filename="ds.${product}.bin"
    url="${NWS_BASE_URL}/${NDFD_PATH}/${FORECAST_PERIOD}/${filename}"
    output_file="${OUTPUT_DIR}/${filename}"
    
    log_info "Downloading: ${description} (${product})"
    log_info "  URL: ${url}"
    
    if curl -f -s --retry 3 --retry-delay 2 --connect-timeout 30 -o "$output_file" "$url"; then
        file_size=$(stat -f%z "$output_file" 2>/dev/null || stat -c%s "$output_file" 2>/dev/null || echo "0")
        file_size_mb=$(echo "scale=2; $file_size / 1048576" | bc 2>/dev/null || echo "?")
        log_success "  Downloaded: ${file_size_mb} MB"
        ((success_count++))
        ((total_size += file_size))
    else
        log_error "  Failed to download ${filename}"
        rm -f "$output_file"
        ((fail_count++))
    fi
    echo ""
done

echo "=========================================="
echo "Download Summary"
echo "=========================================="
echo "  Successful: ${success_count}"
echo "  Failed: ${fail_count}"
total_size_mb=$(echo "scale=2; $total_size / 1048576" | bc 2>/dev/null || echo "?")
echo "  Total size: ${total_size_mb} MB"
echo ""

# List downloaded files
echo "Downloaded files:"
ls -lh "$OUTPUT_DIR"/*.bin 2>/dev/null || echo "  No files found"
echo ""

echo "=========================================="
echo "NDFD Data Properties"
echo "=========================================="
echo "Grid: CONUS 2.5km Lambert Conformal"
echo "Dimensions: 2145 x 1377 points"
echo "Resolution: 2.5 km (~2539.703 m)"
echo "Coverage: CONUS (approximately 20N to 55N, 130W to 60W)"
echo "Update frequency: Continuous (as forecasters update)"
echo ""
echo "Parameters downloaded:"
for product in "${!PRODUCTS[@]}"; do
    echo "  - ${product}: ${PRODUCTS[$product]}"
done
echo ""

if [ "$success_count" -gt 0 ]; then
    log_success "Download complete!"
    echo ""
    echo "=========================================="
    echo "Estimated Disk Usage"
    echo "=========================================="
    echo "  Raw data: ~${total_size_mb} MB"
    echo "  After Zarr compression: ~$((total_size / 3 / 1048576)) MB (estimated)"
    echo "  With 2 versions retained: ~$((total_size * 2 / 3 / 1048576)) MB (estimated)"
    echo ""
    echo "=========================================="
    echo "Next Steps"
    echo "=========================================="
    echo "1. Test parsing NDFD files:"
    echo "   NDFD_TEST_FILE=${OUTPUT_DIR}/ds.temp.bin cargo test -p grib2-parser --test parse_ndfd"
    echo ""
    echo "2. Ingest files into the catalog:"
    echo "   cargo run --package ingester -- ingest ${OUTPUT_DIR}"
    echo ""
    echo "3. Or use the downloader service:"
    echo "   cargo run --package downloader -- --model ndfd"
    echo "=========================================="
else
    log_error "No files downloaded. Check network connectivity and NWS server availability."
    echo ""
    echo "You can manually verify the data source:"
    echo "  curl -I ${NWS_BASE_URL}/${NDFD_PATH}/${FORECAST_PERIOD}/ds.temp.bin"
fi

echo ""
