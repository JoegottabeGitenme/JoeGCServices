#!/bin/bash
# =============================================================================
# Download Blue Lake Test Dataset
# =============================================================================
# Downloads the official OGC WMS 1.3.0 test data needed for conformance testing
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="${SCRIPT_DIR}/../test-data"

echo "============================================="
echo "  Blue Lake Test Data Downloader"
echo "============================================="
echo ""

mkdir -p "$DATA_DIR"
cd "$DATA_DIR"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "Download options:"
echo "  1) Vector data (Shapefiles) - recommended for most WMS servers"
echo "  2) Raster data (PNG worldfiles) - for raster-only servers"
echo "  3) Both"
echo ""
read -p "Select option [1/2/3]: " choice

case $choice in
    1)
        echo ""
        echo "Downloading vector data (Shapefiles)..."
        curl -L -o data-wms-1.3.0.zip \
            "https://cite.opengeospatial.org/teamengine/about/wms13/1.3.0/site/data-wms-1.3.0.zip"
        echo "Extracting..."
        unzip -o data-wms-1.3.0.zip
        echo -e "${GREEN}✓ Vector data ready in ${DATA_DIR}${NC}"
        ;;
    2)
        echo ""
        echo "Downloading raster data (PNG worldfiles)..."
        curl -L -o png-worldfiles-wms-1.3.0.zip \
            "https://cite.opengeospatial.org/teamengine/about/wms13/1.3.0/site/png-worldfiles-wms-1.3.0.zip"
        echo "Extracting..."
        unzip -o png-worldfiles-wms-1.3.0.zip
        echo -e "${GREEN}✓ Raster data ready in ${DATA_DIR}${NC}"
        ;;
    3)
        echo ""
        echo "Downloading vector data (Shapefiles)..."
        curl -L -o data-wms-1.3.0.zip \
            "https://cite.opengeospatial.org/teamengine/about/wms13/1.3.0/site/data-wms-1.3.0.zip"
        unzip -o data-wms-1.3.0.zip
        
        echo ""
        echo "Downloading raster data (PNG worldfiles)..."
        curl -L -o png-worldfiles-wms-1.3.0.zip \
            "https://cite.opengeospatial.org/teamengine/about/wms13/1.3.0/site/png-worldfiles-wms-1.3.0.zip"
        unzip -o png-worldfiles-wms-1.3.0.zip
        
        echo -e "${GREEN}✓ All data ready in ${DATA_DIR}${NC}"
        ;;
    *)
        echo "Invalid option"
        exit 1
        ;;
esac

echo ""
echo -e "${YELLOW}Next steps:${NC}"
echo "  1. Configure your WMS server to serve this data"
echo "  2. Layer names should use the 'cite:' prefix (e.g., cite:Lakes)"
echo "  3. Ensure CRS:84 is supported"
echo ""
echo "See README.md for full requirements"
