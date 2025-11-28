#!/bin/bash
# Extract GOES-19 timestamps from filenames and convert to ISO 8601 format
# Usage: ./extract_goes_timestamps.sh [band_directory]

set -e

BAND_DIR="${1:-./data/goes/band08}"

if [ ! -d "$BAND_DIR" ]; then
    echo "Error: Directory $BAND_DIR not found" >&2
    echo "Usage: $0 [band_directory]" >&2
    echo "Example: $0 ./data/goes/band13" >&2
    exit 1
fi

file_count=$(find "$BAND_DIR" -name "*.nc" | wc -l)

if [ "$file_count" -eq 0 ]; then
    echo "Error: No .nc files found in $BAND_DIR" >&2
    exit 1
fi

echo "# Extracting timestamps from $BAND_DIR ($file_count files)" >&2
echo "# Scan start times in ISO 8601 format" >&2
echo "" >&2

for file in "$BAND_DIR"/*.nc; do
  filename=$(basename "$file")
  
  # Extract scan start time: sYYYYDDDHHMMSSS
  scan_time=$(echo "$filename" | grep -oP "s\d{14}")
  
  if [ -n "$scan_time" ]; then
    year=${scan_time:1:4}
    doy=${scan_time:5:3}
    hour=${scan_time:8:2}
    minute=${scan_time:10:2}
    second=${scan_time:12:2}
    
    # Remove leading zeros to avoid octal interpretation
    doy=$((10#$doy))
    hour=$((10#$hour))
    minute=$((10#$minute))
    second=$((10#$second))
    
    # Convert day-of-year to ISO 8601 date
    python3 -c "from datetime import datetime, timedelta; base = datetime($year, 1, 1); dt = base + timedelta(days=$doy-1, hours=$hour, minutes=$minute, seconds=$second); print(dt.strftime('%Y-%m-%dT%H:%M:%SZ'))"
  fi
done | sort -u
