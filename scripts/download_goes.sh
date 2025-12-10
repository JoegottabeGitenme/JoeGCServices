#!/bin/bash
# Download GOES satellite data from AWS S3
# Supports both GOES-16 (East) and GOES-18 (West)
# Data source: https://noaa-goes16.s3.amazonaws.com/ and https://noaa-goes18.s3.amazonaws.com/

set -e

# Configuration - can be overridden via environment variables
SATELLITE="${GOES_SATELLITE:-18}"         # 16 or 18 (default: 18 = GOES-West)
OUTPUT_DIR="${OUTPUT_DIR:-./data/goes}"
HOURS="${GOES_HOURS:-1}"                  # Hours of historical data to download
MAX_FILES="${GOES_MAX_FILES:-50}"         # Safety limit per band
PRODUCT="${GOES_PRODUCT:-ABI-L2-CMIPC}"   # CONUS Cloud and Moisture Imagery (default)
BANDS="${GOES_BANDS:-01 02 08 13}"        # Blue, Red, Water Vapor, Clean IR

# Determine S3 bucket based on satellite
if [ "$SATELLITE" = "16" ]; then
    S3_BUCKET="noaa-goes16"
    SAT_CODE="G16"
    SAT_NAME="GOES-16 (East)"
elif [ "$SATELLITE" = "18" ]; then
    S3_BUCKET="noaa-goes18"
    SAT_CODE="G18"
    SAT_NAME="GOES-18 (West)"
else
    echo "Error: Invalid satellite number. Use 16 or 18."
    exit 1
fi

# Band descriptions
declare -A BAND_DESC=(
    ["01"]="Blue Visible (0.47 µm) - Aerosols"
    ["02"]="Red Visible (0.64 µm) - Clouds/fog"
    ["03"]="Veggie (0.87 µm) - Vegetation"
    ["08"]="Upper Water Vapor (6.19 µm)"
    ["09"]="Mid Water Vapor (6.95 µm)"
    ["10"]="Lower Water Vapor (7.34 µm)"
    ["13"]="Clean IR (10.35 µm) - Cloud temps"
    ["14"]="IR (11.2 µm) - Cloud temps"
)

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

echo "=========================================="
echo "${SAT_NAME} Data Download"
echo "=========================================="
echo "Source: AWS S3 (${S3_BUCKET})"
echo "Product: ${PRODUCT}"
echo "Output directory: ${OUTPUT_DIR}"
echo "Time range: Last ${HOURS} hour(s)"
echo "Bands: ${BANDS}"
echo "Max files per band: ${MAX_FILES}"
echo "=========================================="
echo ""

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Check for required tools
if ! command -v curl &> /dev/null; then
    log_error "curl is required but not found. Please install curl."
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
    if date -u -d "${h} hours ago" +%Y &>/dev/null 2>&1; then
        # GNU date (Linux)
        year=$(date -u -d "${h} hours ago" +%Y)
        doy=$(date -u -d "${h} hours ago" +%j)
        hour=$(date -u -d "${h} hours ago" +%H)
    elif date -u -v-${h}H +%Y &>/dev/null 2>&1; then
        # BSD date (macOS)
        year=$(date -u -v-${h}H +%Y)
        doy=$(date -u -v-${h}H +%j)
        hour=$(date -u -v-${h}H +%H)
    else
        log_warn "Could not calculate time offset. Using current time only."
        year=$current_year
        doy=$current_doy
        hour=$current_hour
    fi
    
    search_paths+=("${year}/${doy}/${hour}")
done

log_info "Will search ${#search_paths[@]} hour directory(ies)"
echo ""

total_files=0
total_size=0

# Download each band
for band in $BANDS; do
    # Zero-pad band to 2 digits
    band_padded=$(printf "%02d" $((10#$band)))
    
    description="${BAND_DESC[$band_padded]:-Band $band_padded}"
    log_info "Downloading Band ${band_padded}: ${description}"
    
    success_count=0
    
    # Collect all file URLs first (newest to oldest)
    temp_file_list="/tmp/goes_files_band${band_padded}_$$.txt"
    rm -f "$temp_file_list"
    
    for path in $(printf '%s\n' "${search_paths[@]}" | tac); do
        # Build S3 HTTP URL for listing
        s3_http_url="https://${S3_BUCKET}.s3.amazonaws.com/?list-type=2&prefix=${PRODUCT}/${path}/&max-keys=200"
        
        # Get list of files for this band in this hour
        curl -s "$s3_http_url" 2>/dev/null | \
            grep -oP "(?<=<Key>)[^<]+" 2>/dev/null | \
            grep "M6C${band_padded}_${SAT_CODE}" 2>/dev/null | \
            sort -r >> "$temp_file_list" || true
    done
    
    if [ ! -s "$temp_file_list" ]; then
        log_warn "  No files found for Band ${band_padded}"
        rm -f "$temp_file_list"
        continue
    fi
    
    # Download files from the list
    while IFS= read -r s3_key && [ "$success_count" -lt "$MAX_FILES" ]; do
        filename=$(basename "$s3_key")
        
        # Generate output filename with model prefix for ingester
        output_filename="goes${SATELLITE}_${filename}"
        output_file="${OUTPUT_DIR}/${output_filename}"
        
        # Check if already downloaded
        if [ -f "$output_file" ]; then
            continue
        fi
        
        # Download file
        file_url="https://${S3_BUCKET}.s3.amazonaws.com/${s3_key}"
        
        if curl -f -s --connect-timeout 30 -o "$output_file" "$file_url" 2>/dev/null; then
            file_size=$(stat -f%z "$output_file" 2>/dev/null || stat -c%s "$output_file" 2>/dev/null || echo 0)
            file_size_mb=$(echo "scale=2; $file_size / 1048576" | bc 2>/dev/null || echo "?")
            echo "  ✓ ${output_filename} (${file_size_mb} MB)"
            ((success_count++))
            ((total_files++))
            ((total_size += file_size))
        else
            rm -f "$output_file"
            log_warn "  Failed to download: ${filename}"
        fi
    done < "$temp_file_list"
    
    rm -f "$temp_file_list"
    
    if [ "$success_count" -gt 0 ]; then
        log_success "  Band ${band_padded}: Downloaded ${success_count} file(s)"
    fi
    echo ""
done

echo ""
echo "=========================================="
echo "Download Summary"
echo "=========================================="
echo "Total files: ${total_files}"
total_size_mb=$(echo "scale=2; $total_size / 1048576" | bc 2>/dev/null || echo "?")
echo "Total size: ${total_size_mb} MB"
echo ""

# List downloaded files
echo "Files in ${OUTPUT_DIR}:"
ls -la "${OUTPUT_DIR}"/goes${SATELLITE}_*.nc 2>/dev/null | head -20 || echo "  No files found"
echo ""

if [ "$total_files" -gt 0 ]; then
    log_success "Download complete!"
    echo ""
    echo "=========================================="
    echo "Next Steps - Trigger Ingestion"
    echo "=========================================="
    echo ""
    echo "Option 1: Use the downloader API to trigger ingestion:"
    echo "  for f in ${OUTPUT_DIR}/goes${SATELLITE}_*.nc; do"
    echo "    curl -X POST 'http://localhost:8081/ingest' \\"
    echo "      -H 'Content-Type: application/json' \\"
    echo "      -d \"{\\\"file_path\\\": \\\"\$f\\\"}\""
    echo "  done"
    echo ""
    echo "Option 2: Copy files to downloader's data directory:"
    echo "  docker cp ${OUTPUT_DIR}/. weather-wms-downloader-1:/data/downloads/"
    echo "  # Then trigger via admin API:"
    echo "  for f in ${OUTPUT_DIR}/goes${SATELLITE}_*.nc; do"
    echo "    curl -X POST 'http://localhost:8080/api/admin/ingest' \\"
    echo "      -H 'Content-Type: application/json' \\"
    echo "      -d \"{\\\"file_path\\\": \\\"/data/downloads/\$(basename \$f)\\\"}\""
    echo "  done"
    echo ""
    echo "=========================================="
    echo "GOES ABI Band Reference"
    echo "=========================================="
    echo "Band 01: Blue (0.47 µm) - Aerosols"
    echo "Band 02: Red (0.64 µm) - Visible clouds/fog"
    echo "Band 03: Veggie (0.87 µm) - Vegetation"
    echo "Band 04: Cirrus (1.38 µm) - Thin cirrus"
    echo "Band 05: Snow/Ice (1.61 µm) - Snow/ice"
    echo "Band 06: Cloud Particle (2.25 µm) - Cloud particle size"
    echo "Band 07: Shortwave IR (3.9 µm) - Fog, fire hot spots"
    echo "Band 08: Upper Water Vapor (6.19 µm) - Upper moisture"
    echo "Band 09: Mid Water Vapor (6.95 µm) - Mid moisture"
    echo "Band 10: Lower Water Vapor (7.34 µm) - Lower moisture"
    echo "Band 11: Cloud-Top Phase (8.5 µm) - Ice vs water"
    echo "Band 12: Ozone (9.61 µm) - Ozone patterns"
    echo "Band 13: Clean IR (10.35 µm) - Cloud temps"
    echo "Band 14: IR (11.2 µm) - Cloud temps"
    echo "Band 15: Dirty IR (12.3 µm) - Low moisture"
    echo "Band 16: CO2 (13.3 µm) - Cloud heights"
    echo ""
else
    log_warn "No files downloaded. Check AWS S3 bucket availability or parameters."
    echo ""
    echo "Troubleshooting:"
    echo "  - Ensure internet connectivity"
    echo "  - Try increasing GOES_HOURS (current: ${HOURS})"
    echo "  - Try different bands: GOES_BANDS='02 13'"
    echo "  - Check S3 bucket manually: https://${S3_BUCKET}.s3.amazonaws.com/"
fi
