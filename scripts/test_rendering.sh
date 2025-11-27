#!/bin/bash

# Test WMS GetMap rendering and save sample images with basemap overlays
# This script generates sample requests to verify that data ingestion and rendering are working
# Filenames include the data timestamp (reference_time) for traceability

set -e

API_URL="http://localhost:8080"
OUTPUT_DIR="test_renders"
RUN_TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Create output directory
mkdir -p "$OUTPUT_DIR"

echo "=================================="
echo "WMS Rendering Test - $RUN_TIMESTAMP"
echo "=================================="
echo ""

# Check service health first
echo "Checking service health..."
if ! curl -s "$API_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities" > /dev/null 2>&1; then
    echo "ERROR: WMS service is not responding at $API_URL"
    exit 1
fi
echo "✓ WMS service is up"
echo ""

# Get data timestamps from the database for each model
# This helps us include meaningful timestamps in filenames
get_model_timestamp() {
    local model=$1
    local param=$2
    
    # Query the database for the latest reference_time for this model/parameter
    local ref_time=$(docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c \
        "SELECT to_char(reference_time, 'YYYYMMDD_HH24Z') FROM datasets WHERE model = '$model' AND parameter = '$param' AND status = 'available' ORDER BY reference_time DESC LIMIT 1;" 2>/dev/null | tr -d ' \r\n')
    
    # If no specific param found, try just the model
    if [ -z "$ref_time" ]; then
        ref_time=$(docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c \
            "SELECT to_char(reference_time, 'YYYYMMDD_HH24Z') FROM datasets WHERE model = '$model' AND status = 'available' ORDER BY reference_time DESC LIMIT 1;" 2>/dev/null | tr -d ' \r\n')
    fi
    
    echo "$ref_time"
}

# Cache timestamps for each model (avoid repeated DB queries)
echo "Fetching data timestamps from catalog..."
GFS_TIMESTAMP=$(get_model_timestamp "gfs" "TMP")
HRRR_TIMESTAMP=$(get_model_timestamp "hrrr" "TMP")
GOES_TIMESTAMP=$(get_model_timestamp "goes16" "CMI")
MRMS_TIMESTAMP=$(get_model_timestamp "mrms" "REFL")

[ -n "$GFS_TIMESTAMP" ] && echo "  GFS:  $GFS_TIMESTAMP"
[ -n "$HRRR_TIMESTAMP" ] && echo "  HRRR: $HRRR_TIMESTAMP"
[ -n "$GOES_TIMESTAMP" ] && echo "  GOES: $GOES_TIMESTAMP"
[ -n "$MRMS_TIMESTAMP" ] && echo "  MRMS: $MRMS_TIMESTAMP"
echo ""

# Function to get timestamp for a layer
get_layer_timestamp() {
    local layer=$1
    
    if [[ "$layer" == gfs_* ]]; then
        echo "$GFS_TIMESTAMP"
    elif [[ "$layer" == hrrr_* ]]; then
        echo "$HRRR_TIMESTAMP"
    elif [[ "$layer" == goes* ]]; then
        echo "$GOES_TIMESTAMP"
    elif [[ "$layer" == mrms_* ]]; then
        echo "$MRMS_TIMESTAMP"
    else
        echo ""
    fi
}

# Function to test a GetMap request
test_getmap() {
    local layer=$1
    local title=$2
    local bbox=$3
    local width=${4:-512}
    local height=${5:-512}
    local style=${6:-}
    local time_param=${7:-}
    local elevation=${8:-}
    
    # Get timestamp for this layer's data
    local data_timestamp=$(get_layer_timestamp "$layer")
    
    # Build filename with timestamp
    # Format: {layer}_{timestamp}_{width}x{height}.png
    local filename
    if [ -n "$data_timestamp" ]; then
        filename="$OUTPUT_DIR/${layer//\//_}_${data_timestamp}_${width}x${height}.png"
    else
        filename="$OUTPUT_DIR/${layer//\//_}_${width}x${height}.png"
    fi
    
    echo "Testing: $title"
    echo "  Layer: $layer"
    [ -n "$data_timestamp" ] && echo "  Data Time: $data_timestamp"
    echo "  BBox: $bbox"
    echo "  Size: ${width}x${height}"
    [ -n "$style" ] && echo "  Style: $style"
    [ -n "$time_param" ] && echo "  Time: $time_param"
    [ -n "$elevation" ] && echo "  Elevation: $elevation"
    
    # Build URL
    local url="$API_URL/wms?service=WMS&request=GetMap&layers=$layer&format=image/png&transparent=true&version=1.3.0&width=$width&height=$height&crs=EPSG:4326&bbox=$bbox"
    [ -n "$style" ] && url="${url}&styles=$style" || url="${url}&styles="
    [ -n "$time_param" ] && url="${url}&time=$time_param"
    [ -n "$elevation" ] && url="${url}&elevation=$(echo $elevation | sed 's/ /%20/g')"
    
    # Make the request
    local http_code
    http_code=$(curl -s -w "%{http_code}" "$url" -o "$filename")
    
    if [ "$http_code" = "200" ] && [ -f "$filename" ] && [ -s "$filename" ]; then
        local size=$(du -h "$filename" | cut -f1)
        # Check if it's a valid PNG
        if file "$filename" | grep -q "PNG image"; then
            echo "  ✓ SUCCESS: $filename ($size)"
        else
            echo "  ✗ FAILED: Invalid image format"
            cat "$filename" | head -20
            rm -f "$filename"
        fi
    else
        echo "  ✗ FAILED: HTTP $http_code"
        [ -f "$filename" ] && cat "$filename" | head -5
        rm -f "$filename"
    fi
    echo ""
}

# Function to create a composite image with basemap
# Requires ImageMagick (convert command)
create_composite() {
    local weather_layer=$1
    local title=$2
    local bbox=$3
    local width=${4:-512}
    local height=${5:-512}
    local style=${6:-}
    
    # Get timestamp for this layer's data
    local data_timestamp=$(get_layer_timestamp "$weather_layer")
    
    local weather_file="$OUTPUT_DIR/${weather_layer}_weather.png"
    local composite_file
    if [ -n "$data_timestamp" ]; then
        composite_file="$OUTPUT_DIR/${weather_layer}_${data_timestamp}_composite.png"
    else
        composite_file="$OUTPUT_DIR/${weather_layer}_composite.png"
    fi
    
    echo "Creating composite: $title"
    [ -n "$data_timestamp" ] && echo "  Data Time: $data_timestamp"
    
    # Get weather layer
    local url="$API_URL/wms?service=WMS&request=GetMap&layers=$weather_layer&format=image/png&transparent=true&version=1.3.0&width=$width&height=$height&crs=EPSG:4326&bbox=$bbox"
    [ -n "$style" ] && url="${url}&styles=$style" || url="${url}&styles="
    
    if curl -s "$url" -o "$weather_file" && [ -s "$weather_file" ]; then
        # Check if ImageMagick is available
        if command -v convert &> /dev/null; then
            # Create a simple background (gray land, blue water representation)
            convert -size ${width}x${height} xc:'#ccd6e0' "$OUTPUT_DIR/basemap.png"
            # Overlay weather data on basemap
            convert "$OUTPUT_DIR/basemap.png" "$weather_file" -composite "$composite_file"
            rm "$OUTPUT_DIR/basemap.png"
            echo "  ✓ Composite saved: $composite_file"
        else
            echo "  (ImageMagick not installed - skipping composite)"
            mv "$weather_file" "$composite_file"
        fi
    else
        echo "  ✗ Failed to get weather layer"
    fi
    rm -f "$weather_file"
    echo ""
}

# Query available data
echo "=== CHECKING AVAILABLE DATA ==="
echo ""
echo "Querying catalog for available datasets..."
echo ""

# Get capabilities to check what layers exist
CAPS=$(curl -s "$API_URL/wms?SERVICE=WMS&REQUEST=GetCapabilities")

# Check GFS
if echo "$CAPS" | grep -q "gfs_TMP"; then
    echo "✓ GFS data available"
    GFS_AVAILABLE=1
else
    echo "✗ GFS data NOT available"
    GFS_AVAILABLE=0
fi

# Check HRRR
if echo "$CAPS" | grep -q "hrrr_TMP"; then
    echo "✓ HRRR data available"
    HRRR_AVAILABLE=1
else
    echo "✗ HRRR data NOT available"
    HRRR_AVAILABLE=0
fi

# Check GOES
if echo "$CAPS" | grep -q "goes16_CMI"; then
    echo "✓ GOES-16 data available"
    GOES_AVAILABLE=1
else
    echo "✗ GOES-16 data NOT available"
    GOES_AVAILABLE=0
fi

# Check MRMS
if echo "$CAPS" | grep -q "mrms_REFL"; then
    echo "✓ MRMS data available"
    MRMS_AVAILABLE=1
else
    echo "✗ MRMS data NOT available"
    MRMS_AVAILABLE=0
fi

echo ""
echo "=================================="
echo ""

# ============================================================================
# GFS Tests
# ============================================================================
if [ "$GFS_AVAILABLE" = "1" ]; then
    echo "=== GFS MODEL (Global, 0.25° Resolution) ==="
    echo ""
    
    echo "--- Pressure (PRMSL) ---"
    test_getmap "gfs_PRMSL" "GFS Pressure (Global)" "-180,-90,180,90" 1024 512 "atmospheric"
    test_getmap "gfs_PRMSL" "GFS Pressure (North Atlantic)" "-80,20,-20,60" 512 384 "atmospheric"
    
    echo "--- Temperature (TMP) ---"
    # Use an existing level from database (1000 mb instead of 2m above ground)
    test_getmap "gfs_TMP" "GFS Temperature (Global, 1000mb)" "-180,-90,180,90" 1024 512 "temperature" "" "1000 mb"
    test_getmap "gfs_TMP" "GFS Temperature (North America, 1000mb)" "-130,20,-60,55" 512 384 "temperature" "" "1000 mb"
    test_getmap "gfs_TMP" "GFS Temperature (Europe, 500mb)" "-15,35,45,70" 512 384 "temperature" "" "500 mb"
    
    echo "--- Wind Components ---"
    test_getmap "gfs_UGRD" "GFS U-Wind (Global, 1000mb)" "-180,-90,180,90" 1024 512 "" "" "1000 mb"
    test_getmap "gfs_VGRD" "GFS V-Wind (Global, 1000mb)" "-180,-90,180,90" 1024 512 "" "" "1000 mb"
    
    echo "--- Wind Barbs ---"
    test_getmap "gfs_WIND_BARBS" "GFS Wind Barbs (Global)" "-180,-90,180,90" 1024 512 "" "" "1000 mb"
    test_getmap "gfs_WIND_BARBS" "GFS Wind Barbs (North Atlantic)" "-80,20,-20,60" 512 384 "" "" "1000 mb"
fi

# ============================================================================
# HRRR Tests
# ============================================================================
if [ "$HRRR_AVAILABLE" = "1" ]; then
    echo ""
    echo "=== HRRR MODEL (CONUS, 3km Resolution) ==="
    echo ""
    
    echo "--- Temperature ---"
    test_getmap "hrrr_TMP" "HRRR Temperature (CONUS)" "-125,21,-60,48" 768 384 "temperature"
    test_getmap "hrrr_TMP" "HRRR Temperature (Southeast)" "-95,25,-75,38" 512 384 "temperature"
    test_getmap "hrrr_TMP" "HRRR Temperature (Great Plains)" "-110,30,-90,48" 512 384 "temperature"
    
    echo "--- Wind Barbs ---"
    test_getmap "hrrr_WIND_BARBS" "HRRR Wind Barbs (CONUS)" "-125,21,-60,48" 768 384
    test_getmap "hrrr_WIND_BARBS" "HRRR Wind Barbs (California)" "-125,32,-114,42" 512 512
fi

# ============================================================================
# GOES-16 Tests
# ============================================================================
if [ "$GOES_AVAILABLE" = "1" ]; then
    echo ""
    echo "=== GOES-16 SATELLITE ==="
    echo ""
    
    echo "--- Visible Band (C02) ---"
    test_getmap "goes16_CMI_C02" "GOES-16 Visible (CONUS)" "-140,15,-55,55" 768 384 "default"
    test_getmap "goes16_CMI_C02" "GOES-16 Visible (Southeast)" "-95,25,-75,38" 512 384 "default"
    
    echo "--- Infrared Band (C13) ---"
    test_getmap "goes16_CMI_C13" "GOES-16 IR (CONUS)" "-140,15,-55,55" 768 384 "default"
    test_getmap "goes16_CMI_C13" "GOES-16 IR (Gulf of Mexico)" "-100,18,-80,32" 512 384 "default"
fi

# ============================================================================
# MRMS Tests
# ============================================================================
if [ "$MRMS_AVAILABLE" = "1" ]; then
    echo ""
    echo "=== MRMS RADAR ==="
    echo ""
    
    echo "--- Reflectivity ---"
    test_getmap "mrms_REFL" "MRMS Reflectivity (CONUS)" "-130,20,-60,55" 768 384 "reflectivity"
    test_getmap "mrms_REFL" "MRMS Reflectivity (Central US)" "-105,30,-85,45" 512 384 "reflectivity"
    
    echo "--- Precipitation ---"
    test_getmap "mrms_PRECIP_RATE" "MRMS Precip Rate (CONUS)" "-130,20,-60,55" 768 384 "precip_rate"
    test_getmap "mrms_QPE_01H" "MRMS QPE 1hr (CONUS)" "-130,20,-60,55" 768 384 "precipitation"
fi

# ============================================================================
# Create HTML viewer for easy comparison
# ============================================================================
echo ""
echo "=== CREATING HTML VIEWER ==="
echo ""

cat > "$OUTPUT_DIR/index.html" << 'HTMLEOF'
<!DOCTYPE html>
<html>
<head>
    <title>WMS Rendering Test Results</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; background: #1a1a2e; color: #eee; }
        h1 { color: #00d4ff; }
        h2 { color: #ff6b6b; border-bottom: 1px solid #444; padding-bottom: 10px; }
        .data-times { 
            background: #16213e; 
            padding: 15px; 
            border-radius: 8px; 
            margin-bottom: 20px;
            display: inline-block;
        }
        .data-times h3 { margin: 0 0 10px 0; color: #00d4ff; }
        .data-times ul { margin: 0; padding-left: 20px; }
        .data-times li { color: #aaa; margin: 5px 0; }
        .data-times .timestamp { color: #4caf50; font-family: monospace; }
        .gallery { display: flex; flex-wrap: wrap; gap: 20px; }
        .image-card { 
            background: #16213e; 
            border-radius: 8px; 
            padding: 15px; 
            max-width: 550px;
            box-shadow: 0 4px 6px rgba(0,0,0,0.3);
        }
        .image-card img { 
            max-width: 100%; 
            height: auto; 
            border: 1px solid #444;
            border-radius: 4px;
        }
        .image-card h3 { margin: 10px 0 5px 0; color: #00d4ff; font-size: 14px; }
        .image-card p { margin: 0; color: #888; font-size: 12px; }
        .image-card .data-time { color: #4caf50; font-family: monospace; font-size: 11px; margin-top: 5px; }
        .status-ok { color: #4caf50; }
        .status-fail { color: #f44336; }
    </style>
</head>
<body>
    <h1>WMS Rendering Test Results</h1>
    <p>Generated: TIMESTAMP_PLACEHOLDER</p>
    
    <div class="data-times">
        <h3>Data Reference Times</h3>
        <ul>
            <li>GFS: <span class="timestamp">GFS_TIME_PLACEHOLDER</span></li>
            <li>HRRR: <span class="timestamp">HRRR_TIME_PLACEHOLDER</span></li>
            <li>GOES: <span class="timestamp">GOES_TIME_PLACEHOLDER</span></li>
            <li>MRMS: <span class="timestamp">MRMS_TIME_PLACEHOLDER</span></li>
        </ul>
    </div>
    
    <h2>GFS - Global Forecast System</h2>
    <div class="gallery" id="gfs-gallery"></div>
    
    <h2>HRRR - High Resolution Rapid Refresh</h2>
    <div class="gallery" id="hrrr-gallery"></div>
    
    <h2>GOES-16 Satellite</h2>
    <div class="gallery" id="goes-gallery"></div>
    
    <h2>MRMS Radar</h2>
    <div class="gallery" id="mrms-gallery"></div>

    <script>
        // Auto-populate galleries with images
        const images = IMAGES_PLACEHOLDER;
        
        // Extract timestamp from filename (format: layer_YYYYMMDD_HHZ_WxH.png)
        function extractTimestamp(filename) {
            const match = filename.match(/_(\d{8}_\d{2}Z)_/);
            return match ? match[1] : null;
        }
        
        function addImage(gallery, src, title) {
            const card = document.createElement('div');
            card.className = 'image-card';
            const timestamp = extractTimestamp(src);
            const timeHtml = timestamp ? `<p class="data-time">Data: ${timestamp}</p>` : '';
            card.innerHTML = `
                <img src="${src}" alt="${title}" onerror="this.parentElement.innerHTML='<p class=status-fail>Image not found: ${src}</p>'">
                <h3>${title}</h3>
                <p>${src}</p>
                ${timeHtml}
            `;
            document.getElementById(gallery).appendChild(card);
        }
        
        images.forEach(img => {
            if (img.src.includes('gfs_')) addImage('gfs-gallery', img.src, img.title);
            else if (img.src.includes('hrrr_')) addImage('hrrr-gallery', img.src, img.title);
            else if (img.src.includes('goes')) addImage('goes-gallery', img.src, img.title);
            else if (img.src.includes('mrms_')) addImage('mrms-gallery', img.src, img.title);
        });
    </script>
</body>
</html>
HTMLEOF

# Build image list
IMAGE_JSON="["
for f in "$OUTPUT_DIR"/*.png; do
    [ -f "$f" ] || continue
    fname=$(basename "$f")
    title=$(echo "$fname" | sed 's/.png$//' | sed 's/_/ /g')
    IMAGE_JSON="$IMAGE_JSON{\"src\":\"$fname\",\"title\":\"$title\"},"
done
IMAGE_JSON="${IMAGE_JSON%,}]"

# Replace placeholders
sed -i "s/TIMESTAMP_PLACEHOLDER/$(date)/g" "$OUTPUT_DIR/index.html"
sed -i "s/IMAGES_PLACEHOLDER/$IMAGE_JSON/g" "$OUTPUT_DIR/index.html"
sed -i "s/GFS_TIME_PLACEHOLDER/${GFS_TIMESTAMP:-N\/A}/g" "$OUTPUT_DIR/index.html"
sed -i "s/HRRR_TIME_PLACEHOLDER/${HRRR_TIMESTAMP:-N\/A}/g" "$OUTPUT_DIR/index.html"
sed -i "s/GOES_TIME_PLACEHOLDER/${GOES_TIMESTAMP:-N\/A}/g" "$OUTPUT_DIR/index.html"
sed -i "s/MRMS_TIME_PLACEHOLDER/${MRMS_TIMESTAMP:-N\/A}/g" "$OUTPUT_DIR/index.html"

echo "✓ Created HTML viewer: $OUTPUT_DIR/index.html"

# ============================================================================
# Summary
# ============================================================================
echo ""
echo "=================================="
echo "Test Complete!"
echo "=================================="
echo ""
echo "Generated images are in: $OUTPUT_DIR"
echo ""
echo "To view results:"
echo "  1. Open $OUTPUT_DIR/index.html in a browser"
echo "  2. Or view individual PNGs in the directory"
echo ""

# Count successful renders
TOTAL=$(ls -1 "$OUTPUT_DIR"/*.png 2>/dev/null | wc -l)
echo "Total images generated: $TOTAL"
echo ""

echo "Generated files:"
ls -lh "$OUTPUT_DIR"/*.png 2>/dev/null || echo "(No PNG files generated)"
echo ""
