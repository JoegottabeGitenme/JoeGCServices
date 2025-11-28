#!/bin/bash
# Download temporal GOES-19 satellite data from AWS S3
# GOES-19 (GOES-West) provides imagery every ~5 minutes for CONUS
# Data source: https://noaa-goes19.s3.amazonaws.com/

set -e

# Configuration
OUTPUT_DIR="${OUTPUT_DIR:-./data/goes}"
HOURS="${GOES_HOURS:-3}"  # Default: last 3 hours
MAX_FILES_PER_BAND="${MAX_FILES:-50}"  # Safety limit
S3_BUCKET="noaa-goes19"
SATELLITE="G19"

# GOES bands to download (L1b Radiances - raw sensor data)
# Band 02 (Red Visible) - 0.5 km resolution, day only
# Band 08 (Water Vapor) - 2 km resolution, 24/7
# Band 13 (Clean IR) - 2 km resolution, 24/7
declare -A BANDS=(
    ["02"]="Red Visible (0.64 µm) - Clouds, fog, day only"
    ["08"]="Water Vapor (6.19 µm) - Upper-level moisture"
    ["13"]="Clean IR (10.35 µm) - Cloud-top temps, 24/7"
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

echo "=========================================="
echo "GOES-19 Temporal Data Download"
echo "=========================================="
echo "Source: AWS S3 (${S3_BUCKET})"
echo "Product: ABI-L1b-RadC (CONUS Radiances)"
echo "Output directory: ${OUTPUT_DIR}"
echo "Time range: Last ${HOURS} hours"
echo "=========================================="
echo ""

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Check for curl
if ! command -v curl &> /dev/null; then
    log_warn "curl not found. Please install curl."
    exit 1
fi

# Get current UTC time
current_year=$(date -u +%Y)
current_doy=$(date -u +%j)
current_hour=$(date -u +%H)

log_info "Current UTC: ${current_year} Day ${current_doy} Hour ${current_hour}"
echo ""

# Build list of year/doy/hour paths to search
declare -a search_paths=()

for ((h=0; h<HOURS; h++)); do
    # Calculate time offset
    if date -u -d "${h} hours ago" +%Y &>/dev/null; then
        # GNU date
        year=$(date -u -d "${h} hours ago" +%Y)
        doy=$(date -u -d "${h} hours ago" +%j)
        hour=$(date -u -d "${h} hours ago" +%H)
    else
        # BSD date (macOS)
        year=$(date -u -v-${h}H +%Y)
        doy=$(date -u -v-${h}H +%j)
        hour=$(date -u -v-${h}H +%H)
    fi
    
    search_paths+=("${year}/${doy}/${hour}")
done

log_info "Will search ${#search_paths[@]} hour directories"
echo ""

# Product prefix
product_prefix="ABI-L1b-RadC"

# Download each band
for band in "${!BANDS[@]}"; do
    description="${BANDS[$band]}"
    log_info "Downloading Band ${band}: ${description}"
    
    # Create band directory
    band_dir="${OUTPUT_DIR}/band${band}"
    mkdir -p "$band_dir"
    
    success_count=0
    
    log_info "  Searching for ${product_prefix} Band ${band} files..."
    
    # Collect all file URLs first (newest to oldest)
    temp_file_list="/tmp/goes_files_band${band}_$$.txt"
    rm -f "$temp_file_list"
    
    for path in $(printf '%s\n' "${search_paths[@]}" | tac); do
        # Build S3 HTTP URL for listing
        s3_http_url="https://${S3_BUCKET}.s3.amazonaws.com/?list-type=2&prefix=${product_prefix}/${path}/&max-keys=100"
        
        # Get list of files for this band in this hour
        curl -s "$s3_http_url" | \
            grep -oP "(?<=<Key>)[^<]+" | \
            grep "M6C${band}_${SATELLITE}" | \
            sort -r >> "$temp_file_list"
    done
    
    # Download files from the list
    while IFS= read -r s3_key && [ "$success_count" -lt "$MAX_FILES_PER_BAND" ]; do
        filename=$(basename "$s3_key")
        
        # Check if already downloaded
        if [ -f "${band_dir}/${filename}" ]; then
            continue
        fi
        
        # Download file
        file_url="https://${S3_BUCKET}.s3.amazonaws.com/${s3_key}"
        output_file="${band_dir}/${filename}"
        
        if curl -f -s --connect-timeout 10 -o "$output_file" "$file_url" 2>/dev/null; then
            echo "  ✓ ${filename}"
            ((success_count++))
        else
            rm -f "$output_file"
        fi
    done < "$temp_file_list"
    
    rm -f "$temp_file_list"
    
    # Count final files
    file_count=$(find "$band_dir" -name "*.nc" -type f 2>/dev/null | wc -l)
    dir_size=$(du -sh "$band_dir" 2>/dev/null | cut -f1)
    
    log_success "  Band ${band}: ${file_count} files (${dir_size})"
    echo ""
done

echo ""
echo "=========================================="
echo "Download Summary"
echo "=========================================="

total_files=0

for band in "${!BANDS[@]}"; do
    band_dir="${OUTPUT_DIR}/band${band}"
    if [ -d "$band_dir" ]; then
        file_count=$(find "$band_dir" -name "*.nc" -type f 2>/dev/null | wc -l)
        dir_size=$(du -sh "$band_dir" 2>/dev/null | cut -f1)
        
        echo "  Band ${band} (${BANDS[$band]}):"
        echo "    Files: ${file_count}"
        echo "    Size: ${dir_size}"
        
        # Show temporal coverage
        oldest=$(find "$band_dir" -name "*.nc" -type f 2>/dev/null | sort | head -1 | xargs basename 2>/dev/null || echo "")
        newest=$(find "$band_dir" -name "*.nc" -type f 2>/dev/null | sort | tail -1 | xargs basename 2>/dev/null || echo "")
        
        if [ -n "$oldest" ] && [ -n "$newest" ]; then
            # Extract scan start time from filename: sYYYYDDDHHMMSSS
            oldest_time=$(echo "$oldest" | grep -oP "s\d{14}" | sed 's/s\(....\)\(...\)\(..\)\(..\)\(..\).*/\1 DOY:\2 \3:\4:\5 UTC/')
            newest_time=$(echo "$newest" | grep -oP "s\d{14}" | sed 's/s\(....\)\(...\)\(..\)\(..\)\(..\).*/\1 DOY:\2 \3:\4:\5 UTC/')
            echo "    Temporal range:"
            echo "      Oldest: ${oldest_time}"
            echo "      Newest: ${newest_time}"
        fi
        echo ""
        
        ((total_files += file_count))
    fi
done

echo "  Total files across all bands: ${total_files}"
echo ""

echo "=========================================="
echo "GOES-19 Data Properties"
echo "=========================================="
echo "Satellite: GOES-19 (GOES-West)"
echo "Product: ABI-L1b-RadC (Level 1b Radiances, CONUS)"
echo "Scan Mode: Mode 6 (CONUS)"
echo "Coverage: Continental United States"
echo "Update Frequency: ~5 minutes (Mode 6 CONUS)"
echo "Format: NetCDF-4"
echo ""
echo "Band Details:"
echo "  Band 02 (Visible): 0.5 km resolution, day only"
echo "  Band 08 (Water Vapor): 2 km resolution, 24/7"
echo "  Band 13 (Clean IR): 2 km resolution, 24/7"
echo ""
echo "Data Type: Radiances (raw sensor data)"
echo "Projection: Geostationary (Fixed Earth Grid)"
echo "Coordinates: Radians from satellite sub-point"
echo ""

if [ "$total_files" -gt 0 ]; then
    log_success "Download complete!"
    echo ""
    echo "=========================================="
    echo "Next Steps"
    echo "=========================================="
    echo "1. Ingest files into catalog:"
    echo "   for nc_file in ${OUTPUT_DIR}/band*/*.nc; do"
    echo "     cargo run --package ingester -- --test-file \"\$nc_file\""
    echo "   done"
    echo ""
    echo "2. Verify ingestion:"
    echo "   psql -h localhost -U postgres -d weather_data -c \\"
    echo "     \"SELECT reference_time, COUNT(*) FROM grib_files \\"
    echo "     WHERE dataset = 'goes19' GROUP BY reference_time \\"
    echo "     ORDER BY reference_time DESC LIMIT 10;\""
    echo ""
    echo "3. Extract timestamps and create temporal test scenarios"
    echo "   Similar to MRMS temporal tests"
    echo "=========================================="
else
    log_warn "No files downloaded. Check AWS S3 bucket availability or time range."
fi

echo ""
