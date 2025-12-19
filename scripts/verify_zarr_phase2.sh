#!/bin/bash

# =============================================================================
# Phase 2 Zarr Verification Script
# =============================================================================
#
# This script verifies that the Phase 2 Zarr ingestion changes are working:
#   1. ZarrWriter can write grid data correctly
#   2. ZarrGridProcessor can read the data back
#   3. Catalog correctly stores zarr_metadata
#   4. Full roundtrip from GRIB2 → Zarr → GridProcessor works
#
# Usage:
#   ./scripts/verify_zarr_phase2.sh [--unit-only] [--integration-only] [--help]
#
# Options:
#   --unit-only        Run only unit tests (no database required)
#   --integration-only Run only integration tests (requires running services)
#   --help             Show this help message

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[FAIL]${NC} $1"
}

log_section() {
    echo ""
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN} $1${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
}

# Script directory and project root
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

cd "$PROJECT_ROOT"

# Parse arguments
RUN_UNIT=true
RUN_INTEGRATION=true

while [[ $# -gt 0 ]]; do
    case $1 in
        --unit-only)
            RUN_INTEGRATION=false
            shift
            ;;
        --integration-only)
            RUN_UNIT=false
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [options]"
            echo ""
            echo "Options:"
            echo "  --unit-only        Run only unit tests (no database required)"
            echo "  --integration-only Run only integration tests (requires running services)"
            echo "  --help             Show this help message"
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Track test results
PASSED=0
FAILED=0
SKIPPED=0

run_test() {
    local name="$1"
    local cmd="$2"
    
    echo -n "  Testing: $name... "
    
    if eval "$cmd" > /tmp/test_output.txt 2>&1; then
        log_success "OK"
        PASSED=$((PASSED + 1))
        return 0
    else
        log_error "FAILED"
        cat /tmp/test_output.txt | head -20
        FAILED=$((FAILED + 1))
        return 1
    fi
}

skip_test() {
    local name="$1"
    local reason="$2"
    echo -e "  Testing: $name... ${YELLOW}SKIPPED${NC} ($reason)"
    SKIPPED=$((SKIPPED + 1))
}

# =============================================================================
# UNIT TESTS (no external dependencies)
# =============================================================================

if [ "$RUN_UNIT" = "true" ]; then
    log_section "Unit Tests: Grid Processor Core"

    log_info "Running grid-processor unit tests..."
    run_test "ZarrWriter basic write" \
        "cargo test --package grid-processor test_zarr_writer_simple --quiet"
    
    run_test "ZarrWriter with compression" \
        "cargo test --package grid-processor test_zarr_writer_with_compression --quiet"
    
    run_test "ZarrMetadata serialization" \
        "cargo test --package grid-processor test_zarr_metadata_serialization --quiet"
    
    run_test "Chunk cache operations" \
        "cargo test --package grid-processor test_cache_insert_and_get --quiet"
    
    run_test "BoundingBox operations" \
        "cargo test --package grid-processor test_bbox --quiet"
    
    log_section "Unit Tests: Zarr Roundtrip"
    
    log_info "Running Zarr roundtrip integration tests..."
    run_test "Full grid roundtrip" \
        "cargo test --package grid-processor test_zarr_roundtrip_full_grid --quiet"
    
    run_test "Partial region read" \
        "cargo test --package grid-processor test_zarr_partial_read --quiet"
    
    run_test "Point value read" \
        "cargo test --package grid-processor test_zarr_read_point --quiet"
    
    run_test "Chunk cache efficiency" \
        "cargo test --package grid-processor test_chunk_cache_efficiency --quiet"

    log_section "Unit Tests: Storage Catalog"
    
    log_info "Running storage unit tests..."
    run_test "Cache key format" \
        "cargo test --package storage test_cache_key_format --quiet"
    
    run_test "Storage paths" \
        "cargo test --package storage test_storage_paths --quiet"
    
    log_section "Unit Tests: Ingester Config"
    
    log_info "Running ingester unit tests..."
    run_test "Config env var expansion" \
        "cargo test --package ingester test_expand_env_vars --quiet"
fi

# =============================================================================
# INTEGRATION TESTS (require running services)
# =============================================================================

if [ "$RUN_INTEGRATION" = "true" ]; then
    log_section "Integration Tests: Database"
    
    # Check if PostgreSQL is available
    if docker-compose exec -T postgres pg_isready -U weatherwms &>/dev/null; then
        log_info "PostgreSQL is available, running database tests..."
        
        run_test "Database connection" \
            "docker-compose exec -T postgres psql -U weatherwms -d weatherwms -c 'SELECT 1' > /dev/null"
        
        run_test "datasets table exists" \
            "docker-compose exec -T postgres psql -U weatherwms -d weatherwms -c 'SELECT COUNT(*) FROM datasets' > /dev/null"
        
        run_test "zarr_metadata column exists" \
            "docker-compose exec -T postgres psql -U weatherwms -d weatherwms -c \"SELECT column_name FROM information_schema.columns WHERE table_name='datasets' AND column_name='zarr_metadata'\" | grep -q zarr_metadata"
        
        # Check if there are any datasets with zarr_metadata
        ZARR_DATASET_COUNT=$(docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c "SELECT COUNT(*) FROM datasets WHERE zarr_metadata IS NOT NULL" | tr -d ' \n\r')
        
        if [ "$ZARR_DATASET_COUNT" -gt 0 ]; then
            log_success "Found $ZARR_DATASET_COUNT datasets with zarr_metadata"
            
            run_test "zarr_metadata has valid JSON" \
                "docker-compose exec -T postgres psql -U weatherwms -d weatherwms -t -c \"SELECT zarr_metadata->>'shape' FROM datasets WHERE zarr_metadata IS NOT NULL LIMIT 1\" | grep -q '\['"
        else
            skip_test "zarr_metadata validation" "No Zarr datasets ingested yet"
        fi
    else
        skip_test "Database tests" "PostgreSQL not available"
    fi
    
    log_section "Integration Tests: Object Storage"
    
    # Check if MinIO is available
    if docker-compose exec -T minio curl -s http://localhost:9000/minio/health/live &>/dev/null; then
        log_info "MinIO is available, running storage tests..."
        
        run_test "MinIO health check" \
            "docker-compose exec -T minio curl -s http://localhost:9000/minio/health/live"
        
        # Check for Zarr files in grids/ directory
        ZARR_FILES=$(docker exec weather-wms-minio-1 bash -c 'export AWS_ACCESS_KEY_ID=minioadmin && export AWS_SECRET_ACCESS_KEY=minioadmin && /usr/bin/mc alias set local http://localhost:9000 minioadmin minioadmin 2>/dev/null && /usr/bin/mc ls --recursive local/weather-data/grids/ 2>/dev/null | wc -l' 2>/dev/null || echo "0")
        
        if [ "$ZARR_FILES" -gt 0 ]; then
            log_success "Found $ZARR_FILES files in grids/ directory"
            
            run_test "Zarr metadata file exists" \
                "docker exec weather-wms-minio-1 bash -c 'export AWS_ACCESS_KEY_ID=minioadmin && export AWS_SECRET_ACCESS_KEY=minioadmin && /usr/bin/mc alias set local http://localhost:9000 minioadmin minioadmin 2>/dev/null && /usr/bin/mc ls --recursive local/weather-data/grids/ 2>/dev/null | grep -q zarr.json'"
        else
            skip_test "Zarr storage verification" "No Zarr files in grids/ yet"
        fi
    else
        skip_test "MinIO tests" "MinIO not available"
    fi
fi

# =============================================================================
# SUMMARY
# =============================================================================

log_section "Test Summary"

TOTAL=$((PASSED + FAILED + SKIPPED))

echo -e "  ${GREEN}Passed:${NC}  $PASSED"
echo -e "  ${RED}Failed:${NC}  $FAILED"
echo -e "  ${YELLOW}Skipped:${NC} $SKIPPED"
echo -e "  ─────────────"
echo -e "  Total:   $TOTAL"
echo ""

if [ $FAILED -gt 0 ]; then
    log_error "Some tests failed! Please check the output above."
    exit 1
elif [ $PASSED -eq 0 ]; then
    log_warn "No tests were run."
    exit 0
else
    log_success "All tests passed!"
    exit 0
fi
