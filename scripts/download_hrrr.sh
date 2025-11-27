#!/bin/bash
# Download HRRR (High-Resolution Rapid Refresh) GRIB2 data from AWS
# HRRR provides 3km resolution forecasts updated hourly
# Data source: s3://noaa-hrrr-bdp-pds

set -e

# Configuration
HRRR_BUCKET="https://noaa-hrrr-bdp-pds.s3.amazonaws.com"
OUTPUT_DIR="${OUTPUT_DIR:-./data/hrrr}"
DATE="${1:-$(date -u +%Y%m%d)}"  # Format: YYYYMMDD, default to today
CYCLE="${2:-00}"  # Model run hour (00, 06, 12, 18), default 00
FORECAST_HOURS="${3:-0 1 2 3 6}"  # Forecast hours to download, default: 0, 1, 2, 3, 6

# Parameters to download (surface level only for now)
# Using wrfsfcf files which contain 2D surface fields
PRODUCT="wrfsfcf"  # Surface forecast files

# Available products:
# wrfsfcf = 2D surface level fields
# wrfprsf = 3D pressure level fields
# wrfnatf = Native level fields
# wrfsubhf = Subhourly fields

echo "=========================================="
echo "HRRR Data Download Script"
echo "=========================================="
echo "Date: $DATE"
echo "Cycle: ${CYCLE}Z"
echo "Forecast hours: $FORECAST_HOURS"
echo "Product: $PRODUCT"
echo "Output directory: $OUTPUT_DIR"
echo "=========================================="

# Create output directory
mkdir -p "$OUTPUT_DIR/$DATE"

# Download each forecast hour
for fhr in $FORECAST_HOURS; do
    # Zero-pad forecast hour to 2 digits
    fhr_padded=$(printf "%02d" $fhr)
    
    # Construct filename
    filename="hrrr.t${CYCLE}z.${PRODUCT}${fhr_padded}.grib2"
    
    # Construct URL
    url="${HRRR_BUCKET}/hrrr.${DATE}/conus/${filename}"
    
    # Output path
    output_path="$OUTPUT_DIR/$DATE/$filename"
    
    # Check if file already exists
    if [ -f "$output_path" ]; then
        echo "✓ File already exists: $filename ($(du -h "$output_path" | cut -f1))"
        continue
    fi
    
    echo "→ Downloading: $filename"
    
    # Download with curl
    if curl -f -s -S --show-error --retry 3 --retry-delay 5 -o "$output_path" "$url"; then
        file_size=$(du -h "$output_path" | cut -f1)
        echo "✓ Downloaded: $filename ($file_size)"
    else
        echo "✗ Failed to download: $filename"
        echo "  URL: $url"
        rm -f "$output_path"  # Remove partial file
        exit 1
    fi
done

echo "=========================================="
echo "Download complete!"
echo "Files saved to: $OUTPUT_DIR/$DATE"
echo "Total files: $(ls -1 $OUTPUT_DIR/$DATE/*.grib2 2>/dev/null | wc -l)"
echo "Total size: $(du -sh $OUTPUT_DIR/$DATE | cut -f1)"
echo "=========================================="

# Optional: List downloaded files with parameters
echo ""
echo "Inspecting downloaded files..."
for grib_file in "$OUTPUT_DIR/$DATE"/*.grib2; do
    if [ -f "$grib_file" ]; then
        echo ""
        echo "File: $(basename $grib_file)"
        
        # Check if wgrib2 is available
        if command -v wgrib2 &> /dev/null; then
            echo "Grid info:"
            wgrib2 "$grib_file" -grid -s | head -5
        else
            echo "  (wgrib2 not available - install for detailed grid info)"
            echo "  File size: $(du -h "$grib_file" | cut -f1)"
        fi
    fi
done

echo ""
echo "=========================================="
echo "Next steps:"
echo "1. Ingest data: docker compose exec ingester /app/target/release/ingester ingest hrrr $OUTPUT_DIR/$DATE"
echo "2. Verify catalog: docker compose exec postgres psql -U weatherwms -d weatherwms -c \"SELECT * FROM datasets WHERE model='hrrr' LIMIT 5;\""
echo "=========================================="
