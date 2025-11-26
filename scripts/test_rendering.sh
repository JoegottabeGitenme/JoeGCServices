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

# Test requests for different parameters and regions
echo "=== PRESSURE (PRMSL) ==="
echo ""
echo "1. Global view of pressure"
test_getmap "gfs_PRMSL" "GFS Pressure (Global)" "-180,-90,180,90" 512 256

echo "2. North Atlantic region"
test_getmap "gfs_PRMSL" "GFS Pressure (North Atlantic)" "-80,20,-20,60" 512 384

echo ""
echo "=== TEMPERATURE (TMP) ==="
echo ""
echo "3. Global view of temperature"
test_getmap "gfs_TMP" "GFS Temperature (Global)" "-180,-90,180,90" 512 256

echo "4. North America region"
test_getmap "gfs_TMP" "GFS Temperature (North America)" "-130,20,-60,50" 512 384

echo "5. Europe region"
test_getmap "gfs_TMP" "GFS Temperature (Europe)" "-10,35,40,70" 512 384

echo ""
echo "=== U-WIND COMPONENT (UGRD) ==="
echo ""
echo "6. Global view of U-wind"
test_getmap "gfs_UGRD" "GFS U-Wind (Global)" "-180,-90,180,90" 512 256

echo "7. Pacific region"
test_getmap "gfs_UGRD" "GFS U-Wind (Pacific)" "120,-60,-80,60" 512 384

echo ""
echo "=== V-WIND COMPONENT (VGRD) ==="
echo ""
echo "8. Global view of V-wind"
test_getmap "gfs_VGRD" "GFS V-Wind (Global)" "-180,-90,180,90" 512 256

echo "9. Tropical region"
test_getmap "gfs_VGRD" "GFS V-Wind (Tropical)" "-180,-30,180,30" 512 256

echo ""
echo "=== WIND BARBS (COMPOSITE) ==="
echo ""
echo "10. Global wind barbs"
test_getmap "gfs_WIND_BARBS" "GFS Wind Barbs (Global)" "-180,-90,180,90" 512 256

echo "11. North Atlantic wind barbs"
test_getmap "gfs_WIND_BARBS" "GFS Wind Barbs (North Atlantic)" "-80,20,-20,60" 512 384

echo "12. North America wind barbs"
test_getmap "gfs_WIND_BARBS" "GFS Wind Barbs (North America)" "-130,20,-60,50" 512 384

echo "13. High resolution wind barbs (Caribbean region)"
test_getmap "gfs_WIND_BARBS" "GFS Wind Barbs (High Res)" "-85,15,-60,30" 1024 1024

echo ""
echo "=== HIGH RESOLUTION TESTS ==="
echo ""
echo "14. Temperature - High resolution"
test_getmap "gfs_TMP" "GFS Temperature (High Res)" "-100,25,-95,35" 1024 1024

echo "15. Pressure - High resolution"
test_getmap "gfs_PRMSL" "GFS Pressure (High Res)" "-75,35,-70,42" 1024 1024

# Summary
echo "=================================="
echo "Test Complete!"
echo "Generated images are in: $OUTPUT_DIR"
echo ""
echo "Parameters tested:"
echo "  ✓ PRMSL      - Pressure at Mean Sea Level"
echo "  ✓ TMP        - Temperature at 2m"
echo "  ✓ UGRD       - U-Wind Component at 10m"
echo "  ✓ VGRD       - V-Wind Component at 10m"
echo "  ✓ WIND_BARBS - Wind Barbs (composite layer)"
echo ""
echo "View the images to verify:"
echo "  - Proper color gradient rendering (for scalar parameters)"
echo "  - Wind barb representation with correct direction and magnitude"
echo "  - Data is from ingested GRIB2"
echo "  - No placeholder gray images"
echo "  - Different patterns for each parameter"
echo ""
echo "For wind barbs specifically, check:"
echo "  - Barbs point FROM the correct direction (meteorological convention)"
echo "  - Barb count represents wind speed correctly (50kt pennant, 10kt barb, 5kt barb)"
echo "  - Calm winds (< 3 knots) show as circles"
echo "  - Grid spacing is appropriate (not too crowded, not too sparse)"
echo "=================================="

echo ""
echo "Generated files:"
ls -lh "$OUTPUT_DIR" || true
