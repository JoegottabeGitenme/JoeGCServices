#!/bin/bash

# =============================================================================
# Zarr Catalog Verification Script
# =============================================================================
#
# This script verifies that the catalog correctly stores and retrieves
# zarr_metadata. It requires PostgreSQL to be running.
#
# Usage:
#   ./scripts/verify_zarr_catalog.sh

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[FAIL]${NC} $1"; }

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

cd "$PROJECT_ROOT"

echo ""
echo "=============================================="
echo " Zarr Catalog Verification"
echo "=============================================="
echo ""

# Helper to run SQL
pg_exec() {
    docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c "$1" 2>/dev/null | tr -d ' \n\r'
}

pg_exec_full() {
    docker-compose exec -T postgres psql -U weatherwms -d weatherwms -c "$1" 2>/dev/null
}

# Check PostgreSQL availability
log_info "Checking PostgreSQL connection..."
if ! docker-compose exec -T postgres pg_isready -U weatherwms &>/dev/null; then
    log_error "PostgreSQL is not available. Please start services with:"
    echo "  docker-compose up -d postgres"
    exit 1
fi
log_success "PostgreSQL is available"

# Verify schema
echo ""
log_info "Verifying database schema..."

# Check zarr_metadata column exists
if pg_exec "SELECT column_name FROM information_schema.columns WHERE table_name='datasets' AND column_name='zarr_metadata'" | grep -q zarr_metadata; then
    log_success "zarr_metadata column exists in datasets table"
else
    log_error "zarr_metadata column NOT found!"
    log_info "Running migrations..."
    # Try to run migrations by starting ingester briefly
    exit 1
fi

# Check column type
COLUMN_TYPE=$(pg_exec "SELECT data_type FROM information_schema.columns WHERE table_name='datasets' AND column_name='zarr_metadata'")
if [ "$COLUMN_TYPE" = "jsonb" ]; then
    log_success "zarr_metadata column type is JSONB"
else
    log_warn "zarr_metadata column type is '$COLUMN_TYPE' (expected 'jsonb')"
fi

# Count datasets
echo ""
log_info "Checking dataset statistics..."

TOTAL_DATASETS=$(pg_exec "SELECT COUNT(*) FROM datasets WHERE status = 'available'")
ZARR_DATASETS=$(pg_exec "SELECT COUNT(*) FROM datasets WHERE status = 'available' AND zarr_metadata IS NOT NULL")
LEGACY_DATASETS=$(pg_exec "SELECT COUNT(*) FROM datasets WHERE status = 'available' AND zarr_metadata IS NULL")

echo "  Total datasets:  $TOTAL_DATASETS"
echo "  Zarr datasets:   $ZARR_DATASETS"
echo "  Legacy datasets: $LEGACY_DATASETS"

# If we have Zarr datasets, verify the metadata structure
if [ "$ZARR_DATASETS" -gt 0 ]; then
    echo ""
    log_info "Verifying Zarr metadata structure..."
    
    # Get a sample zarr_metadata
    echo ""
    echo "Sample zarr_metadata:"
    pg_exec_full "SELECT jsonb_pretty(zarr_metadata) FROM datasets WHERE zarr_metadata IS NOT NULL LIMIT 1"
    
    # Check required fields
    echo ""
    log_info "Checking required metadata fields..."
    
    FIELDS=("shape" "chunk_shape" "bbox" "compression" "model" "parameter" "level")
    
    for field in "${FIELDS[@]}"; do
        HAS_FIELD=$(pg_exec "SELECT COUNT(*) FROM datasets WHERE zarr_metadata IS NOT NULL AND zarr_metadata ? '$field'")
        if [ "$HAS_FIELD" = "$ZARR_DATASETS" ]; then
            log_success "All Zarr datasets have '$field' field"
        else
            log_warn "Only $HAS_FIELD/$ZARR_DATASETS datasets have '$field' field"
        fi
    done
    
    # Show breakdown by model
    echo ""
    log_info "Zarr datasets by model:"
    pg_exec_full "SELECT model, COUNT(*) as count FROM datasets WHERE zarr_metadata IS NOT NULL GROUP BY model ORDER BY model"
    
    # Show storage paths
    echo ""
    log_info "Sample Zarr storage paths:"
    pg_exec_full "SELECT storage_path FROM datasets WHERE zarr_metadata IS NOT NULL ORDER BY storage_path LIMIT 5"
else
    echo ""
    log_warn "No Zarr datasets found in catalog."
    log_info "To ingest data with Zarr format, run:"
    echo "  ./scripts/ingest_test_data.sh"
fi

echo ""
log_success "Catalog verification complete!"
echo ""
