#!/bin/bash
#
# sync-database.sh - Synchronize the PostgreSQL catalog with MinIO storage
#
# This script helps identify and clean up:
# 1. Orphan DB records - database entries pointing to files that no longer exist in MinIO
# 2. Orphan MinIO objects - files in MinIO that have no corresponding database entry
#
# Usage:
#   ./scripts/sync-database.sh [--dry-run|--run|--status]
#
# Options:
#   --dry-run   Check for orphans without deleting (default)
#   --run       Delete orphan records and files
#   --status    Show current database and storage statistics
#   --cleanup   Also run the standard retention cleanup
#   --help      Show this help message
#
# Examples:
#   # Check what would be synced (safe, no changes)
#   ./scripts/sync-database.sh --dry-run
#
#   # Actually perform the sync and delete orphans
#   ./scripts/sync-database.sh --run
#
#   # Run both sync and cleanup
#   ./scripts/sync-database.sh --run --cleanup
#

set -e

# Default values
WMS_API_URL="${WMS_API_URL:-http://localhost:8080}"
DRY_RUN=true
RUN_CLEANUP=false
SHOW_STATUS=false

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

print_header() {
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}========================================${NC}"
}

print_success() {
    echo -e "${GREEN}$1${NC}"
}

print_warning() {
    echo -e "${YELLOW}$1${NC}"
}

print_error() {
    echo -e "${RED}$1${NC}"
}

show_help() {
    head -30 "$0" | tail -28 | sed 's/^# //' | sed 's/^#//'
    exit 0
}

check_api() {
    echo "Checking WMS API availability at $WMS_API_URL..."
    if ! curl -sf "$WMS_API_URL/health" > /dev/null 2>&1; then
        print_error "ERROR: WMS API is not available at $WMS_API_URL"
        echo "Make sure the service is running and WMS_API_URL is correct."
        echo ""
        echo "If running locally with docker-compose:"
        echo "  docker-compose up -d wms-api"
        echo ""
        echo "Or set WMS_API_URL to the correct endpoint:"
        echo "  WMS_API_URL=http://localhost:8080 $0"
        exit 1
    fi
    print_success "WMS API is available"
}

show_status() {
    print_header "Database & Storage Status"
    
    echo ""
    echo "Fetching storage statistics..."
    storage_stats=$(curl -sf "$WMS_API_URL/api/storage/stats" 2>/dev/null || echo '{"error": "Failed to fetch"}')
    echo "$storage_stats" | jq '.' 2>/dev/null || echo "$storage_stats"
    
    echo ""
    echo "Fetching cleanup status..."
    cleanup_stats=$(curl -sf "$WMS_API_URL/api/admin/cleanup/status" 2>/dev/null || echo '{"error": "Failed to fetch"}')
    echo "$cleanup_stats" | jq '.' 2>/dev/null || echo "$cleanup_stats"
}

run_sync_dry_run() {
    print_header "Sync Status (Dry Run)"
    
    echo ""
    echo "Checking for orphan records and files..."
    echo "(This is a dry run - no changes will be made)"
    echo ""
    
    result=$(curl -sf "$WMS_API_URL/api/admin/sync/status" 2>/dev/null)
    
    if [ $? -ne 0 ]; then
        print_error "ERROR: Failed to get sync status"
        exit 1
    fi
    
    echo "$result" | jq '.' 2>/dev/null || echo "$result"
    
    # Parse and display summary
    orphan_db=$(echo "$result" | jq -r '.orphan_db_records // 0')
    orphan_minio=$(echo "$result" | jq -r '.orphan_minio_objects // 0')
    
    echo ""
    if [ "$orphan_db" -eq 0 ] && [ "$orphan_minio" -eq 0 ]; then
        print_success "Database and storage are in sync!"
    else
        print_warning "Found orphans that need cleanup:"
        [ "$orphan_db" -gt 0 ] && print_warning "  - $orphan_db database records with missing files"
        [ "$orphan_minio" -gt 0 ] && print_warning "  - $orphan_minio MinIO files with no database entry"
        echo ""
        echo "Run with --run to clean up these orphans:"
        echo "  $0 --run"
    fi
}

run_sync() {
    print_header "Running Sync (Deleting Orphans)"
    
    echo ""
    print_warning "This will DELETE orphan records and files!"
    echo ""
    
    # First show what will be deleted
    echo "Checking what will be deleted..."
    status_result=$(curl -sf "$WMS_API_URL/api/admin/sync/status" 2>/dev/null)
    orphan_db=$(echo "$status_result" | jq -r '.orphan_db_records // 0')
    orphan_minio=$(echo "$status_result" | jq -r '.orphan_minio_objects // 0')
    
    if [ "$orphan_db" -eq 0 ] && [ "$orphan_minio" -eq 0 ]; then
        print_success "Nothing to sync - database and storage are already in sync!"
        return 0
    fi
    
    echo "Will delete:"
    [ "$orphan_db" -gt 0 ] && echo "  - $orphan_db database records (missing from MinIO)"
    [ "$orphan_minio" -gt 0 ] && echo "  - $orphan_minio MinIO objects (missing from database)"
    echo ""
    
    # Confirm unless FORCE is set
    if [ -z "$FORCE" ]; then
        read -p "Continue? [y/N] " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            echo "Aborted."
            exit 0
        fi
    fi
    
    echo ""
    echo "Running sync..."
    result=$(curl -sf -X POST "$WMS_API_URL/api/admin/sync/run" 2>/dev/null)
    
    if [ $? -ne 0 ]; then
        print_error "ERROR: Sync failed"
        exit 1
    fi
    
    echo "$result" | jq '.' 2>/dev/null || echo "$result"
    
    # Parse and display summary
    deleted_db=$(echo "$result" | jq -r '.orphan_db_deleted // 0')
    deleted_minio=$(echo "$result" | jq -r '.orphan_minio_deleted // 0')
    errors=$(echo "$result" | jq -r '.errors | length // 0')
    
    echo ""
    if [ "$errors" -eq 0 ]; then
        print_success "Sync completed successfully!"
        echo "  - Deleted $deleted_db orphan database records"
        echo "  - Deleted $deleted_minio orphan MinIO objects"
    else
        print_warning "Sync completed with $errors errors"
        echo "  - Deleted $deleted_db orphan database records"
        echo "  - Deleted $deleted_minio orphan MinIO objects"
    fi
}

run_cleanup() {
    print_header "Running Retention Cleanup"
    
    echo ""
    echo "Running cleanup to remove expired data..."
    result=$(curl -sf -X POST "$WMS_API_URL/api/admin/cleanup/run" 2>/dev/null)
    
    if [ $? -ne 0 ]; then
        print_error "ERROR: Cleanup failed"
        exit 1
    fi
    
    echo "$result" | jq '.' 2>/dev/null || echo "$result"
    
    # Parse and display summary
    marked=$(echo "$result" | jq -r '.marked_expired // 0')
    files=$(echo "$result" | jq -r '.files_deleted // 0')
    records=$(echo "$result" | jq -r '.records_deleted // 0')
    
    echo ""
    print_success "Cleanup completed!"
    echo "  - Marked $marked datasets as expired"
    echo "  - Deleted $files files from storage"
    echo "  - Deleted $records database records"
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --run)
            DRY_RUN=false
            shift
            ;;
        --status)
            SHOW_STATUS=true
            shift
            ;;
        --cleanup)
            RUN_CLEANUP=true
            shift
            ;;
        --force)
            FORCE=1
            shift
            ;;
        --help|-h)
            show_help
            ;;
        *)
            print_error "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Check for required tools
if ! command -v curl &> /dev/null; then
    print_error "ERROR: curl is required but not installed"
    exit 1
fi

if ! command -v jq &> /dev/null; then
    print_warning "WARNING: jq not installed - output may be less readable"
fi

# Main execution
echo ""
check_api
echo ""

if [ "$SHOW_STATUS" = true ]; then
    show_status
    echo ""
fi

if [ "$DRY_RUN" = true ]; then
    run_sync_dry_run
else
    run_sync
fi

if [ "$RUN_CLEANUP" = true ]; then
    echo ""
    run_cleanup
fi

echo ""
print_success "Done!"
