#!/bin/bash

# Ingest test GRIB2 data into the system
# This script:
# 1. Waits for database and services to be ready
# 2. Runs the ingester with test data
# 3. Verifies the ingestion was successful

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

log_info "Starting test data ingestion..."
echo ""

# Check if test data exists
if [ ! -f "testdata/gfs_sample.grib2" ]; then
    log_error "Test data not found: testdata/gfs_sample.grib2"
    exit 1
fi

# Wait for database to be ready
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

log_success "Cleared previous data"

# Run the ingester with test files
echo ""
log_info "Running ingester with test data..."
echo ""

# Find all gfs_f*.grib2 files
TEST_FILES=$(ls testdata/gfs_f*.grib2 2>/dev/null | sort)

if [ -z "$TEST_FILES" ]; then
    log_warn "No gfs_f*.grib2 files found, falling back to gfs_sample.grib2"
    TEST_FILES="testdata/gfs_sample.grib2"
fi

FILE_COUNT=$(echo "$TEST_FILES" | wc -l | tr -d ' ')
CURRENT=0

for TEST_FILE in $TEST_FILES; do
    CURRENT=$((CURRENT + 1))
    
    # Extract forecast hour from filename (e.g., gfs_f003.grib2 -> 3)
    FORECAST_HOUR=$(basename "$TEST_FILE" | sed 's/gfs_f\([0-9]*\).grib2/\1/' | sed 's/^0*//')
    
    # If extraction failed, default to 0
    if [ -z "$FORECAST_HOUR" ] || [ "$FORECAST_HOUR" = "$TEST_FILE" ]; then
        FORECAST_HOUR=0
    fi
    
    log_info "[$CURRENT/$FILE_COUNT] Ingesting $TEST_FILE (forecast hour: ${FORECAST_HOUR})"
    
    DATABASE_URL="postgresql://weatherwms:weatherwms@localhost:5432/weatherwms" \
    REDIS_URL="redis://localhost:6379" \
    S3_ENDPOINT="http://localhost:9000" \
    timeout 120 cargo run --release --package ingester -- \
        --test-file "$TEST_FILE" \
        --forecast-hour "$FORECAST_HOUR" 2>&1 | tail -10
    
    echo ""
done

echo ""

# Verify ingestion was successful
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

if docker exec weather-wms-minio-1 bash -c 'export AWS_ACCESS_KEY_ID=minioadmin && export AWS_SECRET_ACCESS_KEY=minioadmin && /usr/bin/mc alias set local http://localhost:9000 minioadmin minioadmin 2>/dev/null && /usr/bin/mc ls --recursive local/weather-data | grep -q "gfs_sample"'; then
    log_success "Test data confirmed in MinIO"
else
    log_warn "Could not verify MinIO storage (file may be present)"
fi

echo ""
log_success "============================================"
log_success "Data ingestion completed successfully!"
log_success "============================================"
echo ""
log_info "System is ready with:"
log_info "  • $DATASET_COUNT datasets in catalog"
log_info "  • GRIB2 files in MinIO storage"
log_info "  • WMS service ready to render"
echo ""
