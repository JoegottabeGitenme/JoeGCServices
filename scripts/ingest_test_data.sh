#!/bin/bash

# Ingest weather data into the system
# This script:
# 1. Checks for existing data in data/ directory
# 2. Downloads sample data if none exists
# 3. Waits for database and services to be ready
# 4. Runs the ingester with the data
# 5. Verifies the ingestion was successful

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

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

# Script directory and project root
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

cd "$PROJECT_ROOT"

# Load environment variables from .env file if it exists
if [ -f .env ]; then
    log_info "Loading configuration from .env file..."
    set -a  # Automatically export all variables
    source .env
    set +a
fi

# Set defaults for ingestion flags if not set
INGEST_GFS="${INGEST_GFS:-true}"
INGEST_HRRR="${INGEST_HRRR:-true}"
INGEST_GOES="${INGEST_GOES:-true}"
INGEST_MRMS="${INGEST_MRMS:-true}"

log_info "Starting weather data ingestion..."
echo ""

#------------------------------------------------------------------------------
# Check for existing data or download sample data
#------------------------------------------------------------------------------

check_and_download_data() {
    log_info "Checking for weather data in data/ directory..."
    
    # Check for GFS data
    GFS_FILES=$(find data/gfs -type f -name "*.grib2" 2>/dev/null | head -1)
    
    # Check for HRRR data
    HRRR_FILES=$(find data/hrrr -type f -name "*.grib2" 2>/dev/null | head -1)
    
    # Check for GOES data
    GOES_FILES=$(find data/goes -type f -name "*.nc" 2>/dev/null | head -1)
    
    # Check for MRMS data
    MRMS_FILES=$(find data/mrms -type f -name "*.grib2" 2>/dev/null | head -1)
    
    # Check legacy testdata location for GFS
    LEGACY_GFS=$(find testdata -type f -name "gfs*.grib2" 2>/dev/null | head -1)
    
    # If we have any data, we're good
    if [ -n "$GFS_FILES" ] || [ -n "$HRRR_FILES" ] || [ -n "$GOES_FILES" ] || [ -n "$MRMS_FILES" ] || [ -n "$LEGACY_GFS" ]; then
        log_success "Found existing weather data"
        [ -n "$GFS_FILES" ] && log_info "  - GFS data in data/gfs/"
        [ -n "$HRRR_FILES" ] && log_info "  - HRRR data in data/hrrr/"
        [ -n "$GOES_FILES" ] && log_info "  - GOES data in data/goes/"
        [ -n "$MRMS_FILES" ] && log_info "  - MRMS data in data/mrms/"
        [ -n "$LEGACY_GFS" ] && log_info "  - Legacy GFS data in testdata/"
        return 0
    fi
    
    # No data found, download sample GFS data
    log_warn "No weather data found in data/ directory"
    log_info "Downloading sample GFS data (1-degree resolution, ~40MB)..."
    echo ""
    
    # Create data directory
    mkdir -p data/gfs
    
    # Download just one forecast hour for quick startup
    # Using 1-degree resolution for smaller file size
    DATE=$(date -u -d 'yesterday' +%Y%m%d 2>/dev/null || date -u -v-1d +%Y%m%d)
    CYCLE="00"
    GFS_BUCKET="https://noaa-gfs-bdp-pds.s3.amazonaws.com"
    
    # Download f000 (analysis) - smallest and most useful for testing
    url="${GFS_BUCKET}/gfs.${DATE}/${CYCLE}/atmos/gfs.t${CYCLE}z.pgrb2.1p00.f000"
    output_path="data/gfs/gfs_f000.grib2"
    
    log_info "Downloading GFS f000 from ${DATE}/${CYCLE}Z..."
    
    if curl -f -s -S --show-error --retry 3 --retry-delay 5 --progress-bar -o "$output_path" "$url"; then
        file_size=$(du -h "$output_path" | cut -f1)
        log_success "Downloaded: gfs_f000.grib2 ($file_size)"
    else
        log_error "Failed to download GFS data"
        log_info "You can manually download data using:"
        log_info "  ./scripts/download_gfs.sh"
        log_info "  ./scripts/download_hrrr.sh"
        log_info "  ./scripts/download_mrms.sh"
        rm -f "$output_path"
        return 1
    fi
    
    echo ""
}

# Check for data and download if needed
check_and_download_data

#------------------------------------------------------------------------------
# Wait for services to be ready
#------------------------------------------------------------------------------

log_info "Waiting for PostgreSQL to be ready..."
retries=30
while [ $retries -gt 0 ]; do
    if docker-compose exec -T postgres pg_isready -U weatherwms &>/dev/null; then
        log_success "PostgreSQL is ready"
        break
    fi
    echo -ne "\rWaiting... ($retries seconds remaining)"
    sleep 1
    retries=$((retries - 1))
done

if [ $retries -eq 0 ]; then
    log_error "PostgreSQL did not become ready in time"
    exit 1
fi

echo ""
log_info "Clearing previous ingestion data..."

# Clear old datasets from catalog (keep schema)
docker-compose exec -T postgres psql -U weatherwms -d weatherwms << SQL
DELETE FROM datasets WHERE status = 'available';
DELETE FROM layer_styles;
SQL

log_success "Cleared catalog data"

# Clear Redis tile cache
log_info "Flushing Redis tile cache..."
if docker-compose exec -T redis redis-cli FLUSHALL &>/dev/null; then
    log_success "Redis cache cleared"
else
    log_warn "Could not clear Redis cache (may not be running yet)"
fi

#------------------------------------------------------------------------------
# Ingest GFS data
#------------------------------------------------------------------------------

if [ "$INGEST_GFS" = "true" ]; then
    echo ""
    log_info "=== Ingesting GFS data ==="
    echo ""

    # Find GFS files in data/gfs/ directory first, then fall back to testdata/
    GFS_FILES=$(find data/gfs -type f -name "gfs_f*.grib2" 2>/dev/null | sort)

    if [ -z "$GFS_FILES" ]; then
        # Fall back to legacy testdata location
        GFS_FILES=$(find testdata -type f -name "gfs_f*.grib2" 2>/dev/null | sort)
        
        if [ -z "$GFS_FILES" ]; then
            # Try gfs_sample.grib2 as last resort
            if [ -f "testdata/gfs_sample.grib2" ]; then
                GFS_FILES="testdata/gfs_sample.grib2"
            fi
        fi
    fi

    if [ -z "$GFS_FILES" ]; then
        log_warn "No GFS files found, skipping GFS ingestion"
    else
        GFS_COUNT=$(echo "$GFS_FILES" | wc -l | tr -d ' ')
        CURRENT=0

        for TEST_FILE in $GFS_FILES; do
            CURRENT=$((CURRENT + 1))
            
            # Extract forecast hour from filename (e.g., gfs_f003.grib2 -> 3)
            FORECAST_HOUR=$(basename "$TEST_FILE" | sed 's/gfs_f\([0-9]*\).grib2/\1/' | sed 's/^0*//')
            
            # Handle gfs_sample.grib2 case
            if [ -z "$FORECAST_HOUR" ] || [ "$FORECAST_HOUR" = "$(basename $TEST_FILE)" ]; then
                FORECAST_HOUR=0
            fi
            
            log_info "[GFS $CURRENT/$GFS_COUNT] Ingesting $TEST_FILE (forecast hour: ${FORECAST_HOUR})"
            
            DATABASE_URL="postgresql://weatherwms:weatherwms@localhost:5432/weatherwms" \
            REDIS_URL="redis://localhost:6379" \
            S3_ENDPOINT="http://localhost:9000" \
            timeout 120 cargo run --release --package ingester -- \
                --test-file "$TEST_FILE" \
                --forecast-hour "$FORECAST_HOUR" 2>&1 | tail -10
            
            echo ""
        done
    fi
else
    echo ""
    log_info "=== Skipping GFS ingestion (INGEST_GFS=false) ==="
    echo ""
fi

#------------------------------------------------------------------------------
# Ingest HRRR data
#------------------------------------------------------------------------------

if [ "$INGEST_HRRR" = "true" ]; then
    log_info "=== Ingesting HRRR data ==="
    echo ""

    # Find HRRR files in data/hrrr directory
    HRRR_FILES=$(find data/hrrr -type f -name "hrrr.t*.wrfsfcf*.grib2" 2>/dev/null | sort)

    if [ -z "$HRRR_FILES" ]; then
        log_warn "No HRRR files found in data/hrrr/, skipping HRRR ingestion"
        log_info "  To download HRRR data, run: ./scripts/download_hrrr.sh"
    else
        HRRR_COUNT=$(echo "$HRRR_FILES" | wc -l | tr -d ' ')
        CURRENT=0
        
        for HRRR_FILE in $HRRR_FILES; do
            CURRENT=$((CURRENT + 1))
            
            # Extract forecast hour from filename (e.g., hrrr.t00z.wrfsfcf03.grib2 -> 3)
            FORECAST_HOUR=$(basename "$HRRR_FILE" | sed -n 's/.*wrfsfcf\([0-9]\+\)\.grib2/\1/p' | sed 's/^0*//')
            
            # If extraction failed, default to 0
            if [ -z "$FORECAST_HOUR" ]; then
                FORECAST_HOUR=0
            fi
            
            log_info "[HRRR $CURRENT/$HRRR_COUNT] Ingesting $(basename $HRRR_FILE) (forecast hour: ${FORECAST_HOUR})"
            
            DATABASE_URL="postgresql://weatherwms:weatherwms@localhost:5432/weatherwms" \
            REDIS_URL="redis://localhost:6379" \
            S3_ENDPOINT="http://localhost:9000" \
            timeout 180 cargo run --release --package ingester -- \
                --test-file "$HRRR_FILE" \
                --forecast-hour "$FORECAST_HOUR" 2>&1 | tail -10
            
            echo ""
        done
    fi
else
    log_info "=== Skipping HRRR ingestion (INGEST_HRRR=false) ==="
    echo ""
fi

#------------------------------------------------------------------------------
# Ingest GOES data
#------------------------------------------------------------------------------

if [ "$INGEST_GOES" = "true" ]; then
    log_info "=== Ingesting GOES satellite data ==="
    echo ""

    # Find GOES NetCDF files in data/goes directory
    GOES_FILES=$(find data/goes -type f -name "*.nc" 2>/dev/null | sort)

    if [ -z "$GOES_FILES" ]; then
        log_warn "No GOES files found in data/goes/, skipping GOES ingestion"
        log_info "  To download GOES data, run: ./scripts/download_goes.sh"
    else
        GOES_COUNT=$(echo "$GOES_FILES" | wc -l | tr -d ' ')
        CURRENT=0
        
        for GOES_FILE in $GOES_FILES; do
            CURRENT=$((CURRENT + 1))
            
            # Determine satellite from filename or path
            if echo "$GOES_FILE" | grep -qi "G18\|goes18\|goes-18"; then
                MODEL="goes18"
            else
                MODEL="goes16"
            fi
            
            log_info "[GOES $CURRENT/$GOES_COUNT] Ingesting $(basename $GOES_FILE) as $MODEL"
            
            DATABASE_URL="postgresql://weatherwms:weatherwms@localhost:5432/weatherwms" \
            REDIS_URL="redis://localhost:6379" \
            S3_ENDPOINT="http://localhost:9000" \
            timeout 180 cargo run --release --package ingester -- \
                --test-file "$GOES_FILE" \
                --model "$MODEL" 2>&1 | tail -10
            
            echo ""
        done
    fi
else
    log_info "=== Skipping GOES ingestion (INGEST_GOES=false) ==="
    echo ""
fi

#------------------------------------------------------------------------------
# Download and Ingest MRMS data
#------------------------------------------------------------------------------

if [ "$INGEST_MRMS" = "true" ]; then
    log_info "=== Downloading and Ingesting MRMS radar data ==="
    echo ""

    # Create MRMS directory if it doesn't exist
    mkdir -p data/mrms

    # Check if we already have MRMS data
    EXISTING_MRMS=$(find data/mrms -type f -name "*.grib2" 2>/dev/null | head -1)

    if [ -z "$EXISTING_MRMS" ]; then
        # Download fresh MRMS data from NCEP
        MRMS_BASE_URL="https://mrms.ncep.noaa.gov/2D"
        MRMS_PRODUCTS=(
            "MergedReflectivityComposite"
            "PrecipRate"
            "MultiSensor_QPE_01H_Pass2"
        )

        for product in "${MRMS_PRODUCTS[@]}"; do
            log_info "Downloading MRMS ${product}..."
            
            latest_url="${MRMS_BASE_URL}/${product}/MRMS_${product}.latest.grib2.gz"
            output_file="data/mrms/${product}_latest.grib2.gz"
            
            if curl -f -s -S --retry 3 --retry-delay 2 --connect-timeout 10 -o "$output_file" "$latest_url" 2>/dev/null; then
                # Decompress
                gunzip -f "$output_file" 2>/dev/null || true
                grib_file="data/mrms/${product}_latest.grib2"
                
                if [ -f "$grib_file" ]; then
                    file_size=$(du -h "$grib_file" | cut -f1)
                    log_success "  Downloaded: ${product}_latest.grib2 (${file_size})"
                fi
            else
                log_warn "  Could not download ${product} (server may be unavailable)"
                rm -f "$output_file"
            fi
        done
    else
        log_info "Using existing MRMS data in data/mrms/"
    fi

    # Find and ingest MRMS files
    MRMS_FILES=$(find data/mrms -type f -name "*.grib2" 2>/dev/null | sort)

    if [ -z "$MRMS_FILES" ]; then
        log_warn "No MRMS files found in data/mrms/, skipping MRMS ingestion"
    else
        MRMS_COUNT=$(echo "$MRMS_FILES" | wc -l | tr -d ' ')
        CURRENT=0
        
        for MRMS_FILE in $MRMS_FILES; do
            CURRENT=$((CURRENT + 1))
            
            log_info "[MRMS $CURRENT/$MRMS_COUNT] Ingesting $(basename $MRMS_FILE)"
            
            DATABASE_URL="postgresql://weatherwms:weatherwms@localhost:5432/weatherwms" \
            REDIS_URL="redis://localhost:6379" \
            S3_ENDPOINT="http://localhost:9000" \
            timeout 120 cargo run --release --package ingester -- \
                --test-file "$MRMS_FILE" 2>&1 | tail -10
            
            echo ""
        done
    fi
else
    log_info "=== Skipping MRMS ingestion (INGEST_MRMS=false) ==="
    echo ""
fi

#------------------------------------------------------------------------------
# Verify ingestion
#------------------------------------------------------------------------------

echo ""
log_info "Verifying ingestion..."

DATASET_COUNT=$(docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c \
    "SELECT COUNT(*) FROM datasets WHERE status = 'available';" | tr -d ' ')

if [ -z "$DATASET_COUNT" ] || [ "$DATASET_COUNT" = "0" ]; then
    log_error "No datasets found after ingestion!"
    exit 1
fi

log_success "Ingestion verified: $DATASET_COUNT datasets registered"

# Show what was ingested
echo ""
log_info "Ingested datasets:"
docker-compose exec -T postgres psql -U weatherwms -d weatherwms << SQL
SELECT 
    model,
    parameter,
    level,
    COUNT(*) as count,
    MIN(reference_time) as first_time,
    MAX(reference_time) as last_time
FROM datasets
WHERE status = 'available'
GROUP BY model, parameter, level
ORDER BY model, parameter;
SQL

echo ""

# Verify MinIO has the file
log_info "Verifying storage in MinIO..."

if docker exec weather-wms-minio-1 bash -c 'export AWS_ACCESS_KEY_ID=minioadmin && export AWS_SECRET_ACCESS_KEY=minioadmin && /usr/bin/mc alias set local http://localhost:9000 minioadmin minioadmin 2>/dev/null && /usr/bin/mc ls --recursive local/weather-data | head -5' &>/dev/null; then
    log_success "Data confirmed in MinIO"
else
    log_warn "Could not verify MinIO storage (file may be present)"
fi

echo ""
log_success "============================================"
log_success "Data ingestion completed successfully!"
log_success "============================================"
echo ""

# Count datasets by model
GFS_DATASET_COUNT=$(docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c \
    "SELECT COUNT(*) FROM datasets WHERE model = 'gfs' AND status = 'available';" | tr -d ' ')

HRRR_DATASET_COUNT=$(docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c \
    "SELECT COUNT(*) FROM datasets WHERE model = 'hrrr' AND status = 'available';" | tr -d ' ')

GOES16_DATASET_COUNT=$(docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c \
    "SELECT COUNT(*) FROM datasets WHERE model = 'goes16' AND status = 'available';" | tr -d ' ')

GOES18_DATASET_COUNT=$(docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c \
    "SELECT COUNT(*) FROM datasets WHERE model = 'goes18' AND status = 'available';" | tr -d ' ')

MRMS_DATASET_COUNT=$(docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c \
    "SELECT COUNT(*) FROM datasets WHERE model = 'mrms' AND status = 'available';" | tr -d ' ')

log_info "System is ready with:"
log_info "  Total: $DATASET_COUNT datasets in catalog"
log_info "    - GFS: ${GFS_DATASET_COUNT:-0} datasets (global coverage)"
log_info "    - HRRR: ${HRRR_DATASET_COUNT:-0} datasets (CONUS only)"
log_info "    - GOES-16: ${GOES16_DATASET_COUNT:-0} datasets (GOES-East satellite)"
log_info "    - GOES-18: ${GOES18_DATASET_COUNT:-0} datasets (GOES-West satellite)"
log_info "    - MRMS: ${MRMS_DATASET_COUNT:-0} datasets (CONUS radar/precip)"
log_info "  Data files in MinIO storage"
log_info "  WMS service ready to render"
echo ""
log_info "To download more data, use:"
log_info "  ./scripts/download_gfs.sh   # GFS global forecast (~40MB/file)"
log_info "  ./scripts/download_hrrr.sh  # HRRR high-res CONUS"
log_info "  ./scripts/download_mrms.sh  # MRMS radar data"
log_info "  ./scripts/download_goes.sh  # GOES satellite imagery"
echo ""
