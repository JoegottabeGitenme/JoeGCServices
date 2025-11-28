#!/bin/bash
# Extract HRRR timestamps from directory structure and filenames
# HRRR has two time dimensions:
#   1. Reference Time (model cycle): When the forecast was run
#   2. Valid Time (forecast hour): When the forecast is valid for
#
# Usage: ./extract_hrrr_timestamps.sh [hrrr_data_directory]

set -e

HRRR_DIR="${1:-./data/hrrr-temporal}"

if [ ! -d "$HRRR_DIR" ]; then
    echo "Error: Directory $HRRR_DIR not found" >&2
    echo "Usage: $0 [hrrr_data_directory]" >&2
    exit 1
fi

file_count=$(find "$HRRR_DIR" -name "*.grib2" | wc -l)

if [ "$file_count" -eq 0 ]; then
    echo "Error: No .grib2 files found in $HRRR_DIR" >&2
    exit 1
fi

echo "# Extracting timestamps from $HRRR_DIR ($file_count files)" >&2
echo "# Format: REFERENCE_TIME,FORECAST_HOUR,VALID_TIME" >&2
echo "" >&2

find "$HRRR_DIR" -name "*.grib2" -type f | sort | while read grib_file; do
    
    # Extract date from path: .../YYYYMMDD/HHZ/filename
    dir_path=$(dirname "$grib_file")
    cycle_hour=$(basename "$dir_path" | sed 's/Z$//')
    cycle_date=$(basename "$(dirname "$dir_path")")
    
    # Extract forecast hour from filename: hrrr.tHHz.wrfsfcfFF.grib2
    filename=$(basename "$grib_file")
    forecast_hour=$(echo "$filename" | grep -oP "wrfsfcf\K\d+" | sed 's/^0*//')
    
    # If forecast hour is empty (edge case), set to 0
    if [ -z "$forecast_hour" ]; then
        forecast_hour=0
    fi
    
    # Parse cycle date and hour
    year=${cycle_date:0:4}
    month=${cycle_date:4:2}
    day=${cycle_date:6:2}
    hour=${cycle_hour}
    
    # Remove leading zeros for calculations
    month=$((10#$month))
    day=$((10#$day))
    hour=$((10#$hour))
    
    # Create reference time (ISO 8601)
    ref_time=$(printf "%04d-%02d-%02dT%02d:00:00Z" $year $month $day $hour)
    
    # Calculate valid time (reference time + forecast hour)
    # Use Python for date arithmetic
    valid_time=$(python3 -c "from datetime import datetime, timedelta; ref = datetime($year, $month, $day, $hour); valid = ref + timedelta(hours=$forecast_hour); print(valid.strftime('%Y-%m-%dT%H:00:00Z'))")
    
    # Output: reference_time,forecast_hour,valid_time
    echo "${ref_time},+${forecast_hour}h,${valid_time}"
done | sort -u
