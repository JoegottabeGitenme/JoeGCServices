#!/bin/bash
# Download sample GOES-16 data from AWS S3
# GOES-16 is GOES-East, GOES-18 is GOES-West

set -e

DATA_DIR="${1:-data/goes}"
mkdir -p "$DATA_DIR"

echo "=== Downloading GOES-16 Sample Data ==="
echo "Data will be saved to: $DATA_DIR"

# Get the current UTC date for finding recent data
YEAR=$(date -u +%Y)
DOY=$(date -u +%j)  # Day of year

# GOES-16 ABI products on AWS:
# - ABI-L1b-RadC: Level 1b Radiances (CONUS)
# - ABI-L1b-RadF: Level 1b Radiances (Full Disk)
# - ABI-L1b-RadM: Level 1b Radiances (Mesoscale)
# - ABI-L2-CMIPF: Cloud and Moisture Imagery (Full Disk) - what we want
# - ABI-L2-CMIPC: Cloud and Moisture Imagery (CONUS) - smaller, faster
# - ABI-L2-MCMIPF: Multi-band CMI (Full Disk)
# - ABI-L2-MCMIPC: Multi-band CMI (CONUS)

# For simplicity, let's get CONUS Cloud and Moisture Imagery (CMI)
# Band 02 = Visible (Red)
# Band 08 = Water Vapor
# Band 13 = Clean IR Longwave

echo ""
echo "Listing available GOES-16 CONUS CMI products..."

# Try to list recent files (without credentials for public bucket)
# S3 bucket: s3://noaa-goes16/

# Use AWS CLI if available, otherwise use curl
if command -v aws &> /dev/null; then
    echo "Using AWS CLI..."
    
    # List recent CONUS CMI files
    echo "Looking for recent CONUS CMI Band 02 (Visible)..."
    LATEST_VIS=$(aws s3 ls --no-sign-request s3://noaa-goes16/ABI-L2-CMIPC/${YEAR}/${DOY}/ 2>/dev/null | grep "M6C02" | tail -1 | awk '{print $4}')
    
    if [ -n "$LATEST_VIS" ]; then
        echo "Found: $LATEST_VIS"
        aws s3 cp --no-sign-request "s3://noaa-goes16/ABI-L2-CMIPC/${YEAR}/${DOY}/${LATEST_VIS}" "$DATA_DIR/"
    else
        echo "No visible band found for today, trying yesterday..."
        DOY_PREV=$(printf "%03d" $((10#$DOY - 1)))
        LATEST_VIS=$(aws s3 ls --no-sign-request s3://noaa-goes16/ABI-L2-CMIPC/${YEAR}/${DOY_PREV}/ 2>/dev/null | grep "M6C02" | tail -1 | awk '{print $4}')
        if [ -n "$LATEST_VIS" ]; then
            echo "Found: $LATEST_VIS"
            aws s3 cp --no-sign-request "s3://noaa-goes16/ABI-L2-CMIPC/${YEAR}/${DOY_PREV}/${LATEST_VIS}" "$DATA_DIR/"
        fi
    fi
    
    echo ""
    echo "Looking for recent CONUS CMI Band 08 (Water Vapor)..."
    LATEST_WV=$(aws s3 ls --no-sign-request s3://noaa-goes16/ABI-L2-CMIPC/${YEAR}/${DOY}/ 2>/dev/null | grep "M6C08" | tail -1 | awk '{print $4}')
    
    if [ -n "$LATEST_WV" ]; then
        echo "Found: $LATEST_WV"
        aws s3 cp --no-sign-request "s3://noaa-goes16/ABI-L2-CMIPC/${YEAR}/${DOY}/${LATEST_WV}" "$DATA_DIR/"
    fi
    
    echo ""
    echo "Looking for recent CONUS CMI Band 13 (IR)..."
    LATEST_IR=$(aws s3 ls --no-sign-request s3://noaa-goes16/ABI-L2-CMIPC/${YEAR}/${DOY}/ 2>/dev/null | grep "M6C13" | tail -1 | awk '{print $4}')
    
    if [ -n "$LATEST_IR" ]; then
        echo "Found: $LATEST_IR"
        aws s3 cp --no-sign-request "s3://noaa-goes16/ABI-L2-CMIPC/${YEAR}/${DOY}/${LATEST_IR}" "$DATA_DIR/"
    fi
    
else
    echo "AWS CLI not found. Please install it or download manually from:"
    echo "https://noaa-goes16.s3.amazonaws.com/index.html"
    echo ""
    echo "Navigate to: ABI-L2-CMIPC/${YEAR}/${DOY}/"
    echo "Download files matching: *M6C02*.nc (Visible), *M6C08*.nc (Water Vapor), *M6C13*.nc (IR)"
fi

echo ""
echo "=== Download Complete ==="
echo "Files in $DATA_DIR:"
ls -la "$DATA_DIR"/*.nc 2>/dev/null || echo "No .nc files found"

echo ""
echo "=== GOES ABI Band Reference ==="
echo "Band 01: Blue (0.47 µm) - Aerosols"
echo "Band 02: Red (0.64 µm) - Visible clouds/fog"
echo "Band 03: Veggie (0.87 µm) - Vegetation"
echo "Band 04: Cirrus (1.38 µm) - Thin cirrus"
echo "Band 05: Snow/Ice (1.61 µm) - Snow/ice discrimination"
echo "Band 06: Cloud Particle (2.25 µm) - Cloud particle size"
echo "Band 07: Shortwave IR (3.9 µm) - Fog, fire hot spots"
echo "Band 08: Upper Water Vapor (6.19 µm) - Upper-level moisture"
echo "Band 09: Mid Water Vapor (6.95 µm) - Mid-level moisture"
echo "Band 10: Lower Water Vapor (7.34 µm) - Lower-level moisture"
echo "Band 11: Cloud-Top Phase (8.5 µm) - Ice vs water clouds"
echo "Band 12: Ozone (9.61 µm) - Ozone patterns"
echo "Band 13: Clean IR (10.35 µm) - Cloud-top temps, clean window"
echo "Band 14: IR (11.2 µm) - Cloud-top temps"
echo "Band 15: Dirty IR (12.3 µm) - Low-level moisture"
echo "Band 16: CO2 (13.3 µm) - Cloud heights"
