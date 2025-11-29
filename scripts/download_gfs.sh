#!/bin/bash
# Download GFS (Global Forecast System) GRIB2 data from AWS
# Uses the 1-degree resolution product which contains essential parameters
# (temperature, wind, pressure, humidity) in a smaller file size (~40MB each)
# Data source: s3://noaa-gfs-bdp-pds

set -e

# Configuration
GFS_BUCKET="https://noaa-gfs-bdp-pds.s3.amazonaws.com"
OUTPUT_DIR="${OUTPUT_DIR:-./data/gfs}"
DATE="${1:-$(date -u -d 'yesterday' +%Y%m%d)}"  # Format: YYYYMMDD, default to yesterday
CYCLE="${2:-00}"  # Model run hour (00, 06, 12, 18), default 00

# Read forecast hours from environment variable or use default
if [ -n "$GFS_FORECAST_HOURS" ]; then
    # Convert comma-separated to space-separated
    FORECAST_HOURS=$(echo "$GFS_FORECAST_HOURS" | tr ',' ' ')
else
    FORECAST_HOURS="${3:-0 3 6 12 24}"  # Forecast hours to download
fi

# Max files limit (can be overridden by environment variable)
MAX_FILES="${GFS_MAX_FILES:-999}"  # Default: no limit

# Product type: 1-degree resolution (~40MB per file)
# Contains: TMP, UGRD, VGRD, PRMSL, RH, HGT, and more
# Alternative resolutions:
#   pgrb2.0p25 = 0.25 degree (~300MB) - full resolution
#   pgrb2.0p50 = 0.50 degree (~150MB)
#   pgrb2.1p00 = 1.00 degree (~40MB) - used here
PRODUCT="pgrb2.1p00"

echo "=========================================="
echo "GFS Data Download Script"
echo "=========================================="
echo "Date: $DATE"
echo "Cycle: ${CYCLE}Z"
echo "Forecast hours: $FORECAST_HOURS"
echo "Product: $PRODUCT (1-degree resolution)"
echo "Output directory: $OUTPUT_DIR"
echo "=========================================="

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Download each forecast hour
FILES_DOWNLOADED=0

for fhr in $FORECAST_HOURS; do
    # Check if we've reached the max file limit
    if [ $FILES_DOWNLOADED -ge $MAX_FILES ]; then
        echo "Reached maximum file limit ($MAX_FILES files)"
        break
    fi
    
    # Zero-pad forecast hour to 3 digits
    fhr_padded=$(printf "%03d" $fhr)
    
    # Construct filename
    filename="gfs_f${fhr_padded}.grib2"
    
    # Construct URL
    url="${GFS_BUCKET}/gfs.${DATE}/${CYCLE}/atmos/gfs.t${CYCLE}z.${PRODUCT}.f${fhr_padded}"
    
    # Output path
    output_path="$OUTPUT_DIR/$filename"
    
    # Check if file already exists and is complete
    if [ -f "$output_path" ]; then
        SIZE=$(stat -c%s "$output_path" 2>/dev/null || stat -f%z "$output_path" 2>/dev/null)
        if [ "$SIZE" -gt 10000000 ]; then  # At least 10MB
            echo "File already exists: $filename ($(numfmt --to=iec-i --suffix=B $SIZE 2>/dev/null || echo "${SIZE} bytes"))"
            FILES_DOWNLOADED=$((FILES_DOWNLOADED + 1))
            continue
        else
            echo "File incomplete, re-downloading: $filename"
            rm -f "$output_path"
        fi
    fi
    
    echo "Downloading: $filename"
    
    # Download with curl
    if curl -f -s -S --show-error --retry 3 --retry-delay 5 -o "$output_path" "$url"; then
        file_size=$(du -h "$output_path" | cut -f1)
        echo "Downloaded: $filename ($file_size)"
        FILES_DOWNLOADED=$((FILES_DOWNLOADED + 1))
    else
        echo "Failed to download: $filename"
        echo "  URL: $url"
        rm -f "$output_path"  # Remove partial file
    fi
done

echo "=========================================="
echo "Download complete!"
echo "Files saved to: $OUTPUT_DIR"
echo "Total files: $(ls -1 $OUTPUT_DIR/gfs_f*.grib2 2>/dev/null | wc -l)"
echo "Total size: $(du -sh $OUTPUT_DIR 2>/dev/null | cut -f1)"
echo "=========================================="

# List downloaded files
echo ""
echo "Downloaded files:"
ls -lh "$OUTPUT_DIR"/gfs_f*.grib2 2>/dev/null | awk '{print "  " $9 " - " $5}' || echo "  No files downloaded"

echo ""
echo "=========================================="
echo "Available parameters include:"
echo "  TMP  - Temperature"
echo "  UGRD - U-component of wind"
echo "  VGRD - V-component of wind"
echo "  PRMSL - Pressure at mean sea level"
echo "  RH   - Relative humidity"
echo "  HGT  - Geopotential height"
echo "=========================================="
echo ""
echo "To verify contents, run:"
echo "  wgrib2 $OUTPUT_DIR/gfs_f000.grib2 -s | head -20"
echo "=========================================="
