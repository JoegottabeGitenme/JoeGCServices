#!/bin/bash

# Test WMS GetMap rendering and save sample images
# This script generates sample requests to verify that data ingestion and rendering are working

set -e

API_URL="http://localhost:8080"
OUTPUT_DIR="test_renders"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Create output directory
mkdir -p "$OUTPUT_DIR"

echo "=================================="
echo "WMS Rendering Test - $TIMESTAMP"
echo "=================================="
echo ""

# Function to test a GetMap request
test_getmap() {
    local layer=$1
    local title=$2
    local bbox=$3
    local width=${4:-512}
    local height=${5:-512}
    
    local filename="$OUTPUT_DIR/${layer//\//_}_${width}x${height}.png"
    
    echo "Testing: $title"
    echo "  Layer: $layer"
    echo "  BBox: $bbox"
    echo "  Size: ${width}x${height}"
    
    # Make the request
    if curl -s \
        "$API_URL/wms?service=WMS&request=GetMap&layers=$layer&styles=&format=image/png&transparent=true&version=1.3.0&width=$width&height=$height&crs=EPSG:4326&bbox=$bbox" \
        -o "$filename"; then
        
        # Check if file exists and has content
        if [ -f "$filename" ] && [ -s "$filename" ]; then
            local size=$(du -h "$filename" | cut -f1)
            echo "  ✓ Saved to: $filename ($size)"
        else
            echo "  ✗ Failed: Empty response"
            rm -f "$filename"
        fi
    else
        echo "  ✗ Failed: Request error"
    fi
    echo ""
}

# Test requests for different regions and zoom levels
echo "1. Global view of temperature"
test_getmap "gfs_TMP" "GFS Temperature (Global)" "-180,-90,180,90" 512 256

echo "2. North America region"
test_getmap "gfs_TMP" "GFS Temperature (North America)" "-130,20,-60,50" 512 384

echo "3. Europe region"
test_getmap "gfs_TMP" "GFS Temperature (Europe)" "-10,35,40,70" 512 384

echo "4. Tropical region"
test_getmap "gfs_TMP" "GFS Temperature (Tropical)" "-180,-30,180,30" 512 256

echo "5. High resolution test"
test_getmap "gfs_TMP" "GFS Temperature (High Res)" "-100,25,-95,35" 1024 1024

# Summary
echo "=================================="
echo "Test Complete!"
echo "Generated images are in: $OUTPUT_DIR"
echo ""
echo "View the images to verify:"
echo "  - Proper color gradient rendering"
echo "  - Data is from ingested GRIB2"
echo "  - No placeholder gray images"
echo "=================================="

ls -lh "$OUTPUT_DIR" || true
