#!/bin/bash
# Download temporal HRRR (High-Resolution Rapid Refresh) GRIB2 data from AWS
# HRRR provides 3km resolution forecasts updated hourly with 18-hour forecast range
# Data source: https://noaa-hrrr-bdp-pds.s3.amazonaws.com/

set -e

# Configuration
HRRR_BUCKET="https://noaa-hrrr-bdp-pds.s3.amazonaws.com"
OUTPUT_DIR="${OUTPUT_DIR:-./data/hrrr}"
PRODUCT="${HRRR_PRODUCT:-wrfsfcf}"  # Surface forecast files
MAX_CYCLES="${MAX_CYCLES:-3}"  # Number of model cycles to download
FORECAST_HOURS="${FORECAST_HOURS:-0 1 2 3}"  # Forecast hours per cycle

# Colors
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
echo "HRRR Temporal Data Download"
echo "=========================================="
echo "Source: AWS S3 (noaa-hrrr-bdp-pds)"
echo "Product: $PRODUCT (Surface forecasts)"
echo "Output directory: $OUTPUT_DIR"
echo "Model cycles: Last $MAX_CYCLES"
echo "Forecast hours: $FORECAST_HOURS"
echo "=========================================="
echo ""

# Get current date and hour
current_date=$(date -u +%Y%m%d)
current_hour=$(date -u +%H)

log_info "Current UTC: ${current_date} ${current_hour}Z"
echo ""

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Calculate cycles to download
# HRRR runs every hour, so we can get recent cycles
declare -a cycles=()

for ((i=0; i<MAX_CYCLES; i++)); do
    # Calculate hours ago
    hours_ago=$((i))
    
    # Get date and hour for this offset
    if date -u -d "${hours_ago} hours ago" +%Y &>/dev/null; then
        # GNU date
        cycle_date=$(date -u -d "${hours_ago} hours ago" +%Y%m%d)
        cycle_hour=$(date -u -d "${hours_ago} hours ago" +%H)
    else
        # BSD date (macOS)
        cycle_date=$(date -u -v-${hours_ago}H +%Y%m%d)
        cycle_hour=$(date -u -v-${hours_ago}H +%H)
    fi
    
    cycles+=("${cycle_date}:${cycle_hour}")
done

log_info "Will download ${#cycles[@]} model cycles"
echo ""

total_downloaded=0
total_skipped=0
total_failed=0

# Download each cycle
for cycle in "${cycles[@]}"; do
    cycle_date=$(echo "$cycle" | cut -d: -f1)
    cycle_hour=$(echo "$cycle" | cut -d: -f2)
    
    log_info "Downloading cycle: ${cycle_date} ${cycle_hour}Z"
    
    # Create cycle directory
    cycle_dir="${OUTPUT_DIR}/${cycle_date}/${cycle_hour}Z"
    mkdir -p "$cycle_dir"
    
    cycle_downloads=0
    
    # Download each forecast hour
    for fhr in $FORECAST_HOURS; do
        # Zero-pad forecast hour
        fhr_padded=$(printf "%02d" $fhr)
        
        # Construct filename
        filename="hrrr.t${cycle_hour}z.${PRODUCT}${fhr_padded}.grib2"
        
        # Construct URL
        url="${HRRR_BUCKET}/hrrr.${cycle_date}/conus/${filename}"
        
        # Output path
        output_path="${cycle_dir}/${filename}"
        
        # Check if already exists
        if [ -f "$output_path" ]; then
            file_size=$(du -h "$output_path" | cut -f1)
            ((total_skipped++))
            continue
        fi
        
        # Download
        if curl -f -s --connect-timeout 15 --max-time 120 -o "$output_path" "$url" 2>/dev/null; then
            file_size=$(du -h "$output_path" | cut -f1)
            echo "  ✓ ${filename} ($file_size)"
            ((cycle_downloads++))
            ((total_downloaded++))
        else
            rm -f "$output_path"
            ((total_failed++))
            # Don't exit, try next file
        fi
    done
    
    if [ "$cycle_downloads" -gt 0 ]; then
        log_success "  Cycle ${cycle_hour}Z: ${cycle_downloads} files downloaded"
    else
        log_warn "  Cycle ${cycle_hour}Z: No files downloaded"
    fi
    echo ""
done

echo ""
echo "=========================================="
echo "Download Summary"
echo "=========================================="

# Count all files
total_files=0
total_size=0

for cycle_dir in "${OUTPUT_DIR}"/*/*/*Z; do
    if [ -d "$cycle_dir" ]; then
        cycle_name=$(echo "$cycle_dir" | grep -oP "\d{8}/\d{2}Z")
        file_count=$(find "$cycle_dir" -name "*.grib2" -type f 2>/dev/null | wc -l)
        dir_size=$(du -sh "$cycle_dir" 2>/dev/null | cut -f1)
        
        if [ "$file_count" -gt 0 ]; then
            echo "  ${cycle_name}:"
            echo "    Files: ${file_count}"
            echo "    Size: ${dir_size}"
            
            # Show forecast range
            oldest_fhr=$(find "$cycle_dir" -name "*.grib2" | xargs -n1 basename | grep -oP "wrfsfcf\K\d+" | sort -n | head -1 | sed 's/^0*//')
            newest_fhr=$(find "$cycle_dir" -name "*.grib2" | xargs -n1 basename | grep -oP "wrfsfcf\K\d+" | sort -n | tail -1 | sed 's/^0*//')
            echo "    Forecast range: +${oldest_fhr}h to +${newest_fhr}h"
            echo ""
            
            ((total_files += file_count))
        fi
    fi
done

echo "  Total files: ${total_files}"
echo "  Downloaded: ${total_downloaded}"
echo "  Skipped: ${total_skipped}"
echo "  Failed: ${total_failed}"
echo ""

echo "=========================================="
echo "HRRR Data Properties"
echo "=========================================="
echo "Model: High-Resolution Rapid Refresh (HRRR)"
echo "Resolution: 3 km"
echo "Coverage: CONUS"
echo "Update Frequency: Hourly"
echo "Forecast Range: 18 hours (48 hours for 00, 06, 12, 18Z)"
echo "Format: GRIB2"
echo "Product: ${PRODUCT} (Surface forecasts)"
echo ""
echo "Temporal Dimensions:"
echo "  1. Model Cycle (Reference Time): Hourly (00Z, 01Z, ..., 23Z)"
echo "  2. Forecast Hour (Valid Time): +0h to +18h"
echo ""
echo "File Sizes: ~120-150 MB per file"
echo "Grid: Lambert Conformal"
echo "Variables: Temperature, wind, precipitation, etc."
echo ""

if [ "$total_files" -gt 0 ]; then
    log_success "Download complete!"
    echo ""
    echo "=========================================="
    echo "Next Steps"
    echo "=========================================="
    echo "1. Extract timestamps for test scenarios:"
    echo "   ./scripts/extract_hrrr_timestamps.sh ${OUTPUT_DIR}"
    echo ""
    echo "2. Ingest files into catalog:"
    echo "   for grib_file in ${OUTPUT_DIR}/*/*/*Z/*.grib2; do"
    echo "     cargo run --package ingester -- --test-file \"\$grib_file\""
    echo "   done"
    echo ""
    echo "3. Verify ingestion:"
    echo "   psql -h localhost -U postgres -d weather_data -c \\"
    echo "     \"SELECT reference_time, forecast_time, COUNT(*) FROM grib_files \\"
    echo "     WHERE dataset = 'hrrr' GROUP BY reference_time, forecast_time \\"
    echo "     ORDER BY reference_time DESC, forecast_time ASC LIMIT 20;\""
    echo ""
    echo "4. Run temporal tests (after creating scenarios)"
    echo "=========================================="
else
    log_warn "No files downloaded. Check AWS S3 bucket availability or network connection."
fi

echo ""
