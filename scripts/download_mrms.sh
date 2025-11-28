#!/bin/bash
# Download MRMS (Multi-Radar Multi-Sensor) GRIB2 data from AWS S3
# MRMS provides ~1km resolution radar and precipitation data updated every 2 minutes
# Data source: https://noaa-mrms-pds.s3.amazonaws.com/

set -e

# Configuration
AWS_S3_BASE="https://noaa-mrms-pds.s3.amazonaws.com/CONUS"
OUTPUT_DIR="${OUTPUT_DIR:-./data/mrms}"

# Number of hours to download (default: 24)
HOURS="${MRMS_HOURS:-24}"

# Maximum files to list from S3 (determines how far back we can go)
MAX_S3_LIST="${MAX_S3_LIST:-1000}"

# Products to download
# Only MergedReflectivityQC_00.50 is reliably available on AWS S3
declare -A PRODUCTS=(
    ["MergedReflectivityQC_00.50"]="Composite Radar Reflectivity (dBZ)"
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
echo "Source: AWS S3 (${AWS_S3_BASE})"
echo "Output directory: ${OUTPUT_DIR}"
echo "Time range: Last ${HOURS} hours"
echo "=========================================="

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Calculate cutoff time
cutoff_timestamp=$(date -u -d "${HOURS} hours ago" +"%Y%m%d-%H%M%S" 2>/dev/null || \
                   date -u -v-${HOURS}H +"%Y%m%d-%H%M%S")

log_info "Downloading files newer than ${cutoff_timestamp}"
echo ""

# Get current date (files are organized by YYYYMMDD)
current_date=$(date -u +"%Y%m%d")

# Download each product
for product in "${!PRODUCTS[@]}"; do
    description="${PRODUCTS[$product]}"
    log_info "Downloading: ${description} (${product})"
    
    # Create product directory
    product_dir="${OUTPUT_DIR}/${product}"
    mkdir -p "$product_dir"
    
    success_count=0
    fail_count=0
    skip_count=0
    
    # List available files from S3 for today, sorted newest-first
    log_info "  Listing available files from AWS S3..."
    
    # Use a pipeline to download files directly without building large arrays
    curl -s "https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS/${product}/${current_date}/&max-keys=${MAX_S3_LIST}" | \
        grep -oP "(?<=<Key>)CONUS/${product}/${current_date}/MRMS[^<]+" | \
        sort -r | \
        while IFS= read -r s3_key; do
            # Extract timestamp from filename
            filename=$(basename "$s3_key")
            timestamp=$(echo "$filename" | grep -oP "\d{8}-\d{6}")
            
            if [ -z "$timestamp" ]; then
                continue
            fi
            
            # Check if file is within our time range
            if [[ "$timestamp" < "$cutoff_timestamp" ]]; then
                # Files are sorted newest-first, so we can stop here
                break
            fi
            
            # Check if already exists (decompressed)
            decompressed_file="${product_dir}/${filename%.gz}"
            if [ -f "$decompressed_file" ]; then
                continue
            fi
            
            # Download and decompress
            file_url="https://noaa-mrms-pds.s3.amazonaws.com/${s3_key}"
            temp_file="${product_dir}/${filename}"
            
            if curl -f -s --retry 2 --retry-delay 1 --connect-timeout 10 -o "$temp_file" "$file_url" 2>/dev/null; then
                if gunzip -f "$temp_file" 2>/dev/null; then
                    echo "  ✓ ${filename}"
                else
                    rm -f "$temp_file"
                fi
            else
                rm -f "$temp_file"
            fi
        done
    
    # Count final files
    file_count=$(find "$product_dir" -name "*.grib2" -type f 2>/dev/null | wc -l)
    dir_size=$(du -sh "$product_dir" 2>/dev/null | cut -f1)
    
    log_success "  ${product}: ${file_count} files (${dir_size})"
    echo ""
done

echo ""
echo "=========================================="
echo "Download Summary"
echo "=========================================="

# Count total files across all products
total_files=0

for product in "${!PRODUCTS[@]}"; do
    product_dir="${OUTPUT_DIR}/${product}"
    if [ -d "$product_dir" ]; then
        file_count=$(find "$product_dir" -name "*.grib2" -type f 2>/dev/null | wc -l)
        dir_size=$(du -sh "$product_dir" 2>/dev/null | cut -f1)
        
        echo "  ${product}:"
        echo "    Files: ${file_count}"
        echo "    Size: ${dir_size}"
        
        # Show temporal coverage (oldest and newest files)
        oldest=$(find "$product_dir" -name "*.grib2" -type f 2>/dev/null | sort | head -1 | xargs basename 2>/dev/null || echo "")
        newest=$(find "$product_dir" -name "*.grib2" -type f 2>/dev/null | sort | tail -1 | xargs basename 2>/dev/null || echo "")
        
        if [ -n "$oldest" ] && [ -n "$newest" ]; then
            oldest_time=$(echo "$oldest" | grep -oP "\d{8}-\d{6}" | sed 's/\(....\)\(..\)\(..\)-\(..\)\(..\)\(..\)/\1-\2-\3 \4:\5:\6/')
            newest_time=$(echo "$newest" | grep -oP "\d{8}-\d{6}" | sed 's/\(....\)\(..\)\(..\)-\(..\)\(..\)\(..\)/\1-\2-\3 \4:\5:\6/')
            echo "    Temporal range:"
            echo "      Oldest: ${oldest_time} UTC"
            echo "      Newest: ${newest_time} UTC"
        fi
        
        # Show sample file info if wgrib2 is available
        sample_file=$(find "$product_dir" -name "*.grib2" -type f 2>/dev/null | head -1)
        if [ -f "$sample_file" ] && command -v wgrib2 &> /dev/null; then
            param_info=$(wgrib2 "$sample_file" -s 2>&1 | head -1 | cut -d: -f3-5 || echo "Unknown param")
            echo "    Sample parameter: ${param_info}"
        fi
        echo ""
        
        ((total_files += file_count))
    fi
done

echo "  Total files: ${total_files}"
echo ""

echo "=========================================="
echo "MRMS Data Properties"
echo "=========================================="
echo "Grid: 7000 x 3500 points (lat-lon)"
echo "Resolution: 0.01 degrees (~1 km)"
echo "Coverage: CONUS (20°N to 55°N, 130°W to 60°W)"
echo "Update frequency: Every ~2 minutes"
echo ""
echo "Bounding Box (for WMS layer config):"
echo "  min_lon: -130.0"
echo "  min_lat: 20.0"
echo "  max_lon: -60.0"
echo "  max_lat: 55.0"
echo ""

if [ "$total_files" -gt 0 ]; then
    log_success "Download complete!"
    echo ""
    echo "=========================================="
    echo "Next Steps"
    echo "=========================================="
    echo "1. Ingest files into catalog:"
    echo "   for grib_file in ${OUTPUT_DIR}/*/*.grib2; do"
    echo "     cargo run --package ingester -- --test-file \"\$grib_file\""
    echo "   done"
    echo ""
    echo "2. Verify ingestion:"
    echo "   psql -h localhost -U postgres -d weather_data -c \"SELECT * FROM grib_files ORDER BY reference_time DESC LIMIT 10;\""
    echo ""
    echo "3. Test temporal requests with load test tool:"
    echo "   cargo run --package load-test -- run --scenario validation/load-test/scenarios/temporal_animation.yaml"
    echo "=========================================="
else
    log_warn "No files downloaded. Check AWS S3 bucket availability or adjust time range."
fi

echo ""
