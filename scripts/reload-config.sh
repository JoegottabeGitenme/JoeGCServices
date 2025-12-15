#!/bin/bash
# Hot reload configuration files without restarting the wms-api service
#
# Usage:
#   ./scripts/reload-config.sh              # Full reload (configs + clear caches)
#   ./scripts/reload-config.sh layers       # Reload layer configs only
#   ./scripts/reload-config.sh cache        # Clear caches only
#   ./scripts/reload-config.sh styles       # Just reminder that styles are auto-reloaded
#
# Endpoints called:
#   POST /api/config/reload         - Full reload (layers + clear all caches)
#   POST /api/config/reload/layers  - Reload layer configs only (no cache clear)
#   POST /api/cache/clear           - Clear in-memory caches only

set -e

# Configuration
# Default to port 8080 (direct API), use 3000 for proxy mode
WMS_API_URL="${WMS_API_URL:-http://localhost:8080}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to check if wms-api is reachable
check_api() {
    if ! curl -s --fail "${WMS_API_URL}/health" > /dev/null 2>&1; then
        print_error "wms-api is not reachable at ${WMS_API_URL}"
        print_info "Make sure the service is running or set WMS_API_URL environment variable"
        exit 1
    fi
}

# Full reload: configs + caches
reload_full() {
    print_info "Performing full configuration reload..."
    print_info "This will reload layer configs and clear all caches"
    echo
    
    response=$(curl -s -X POST "${WMS_API_URL}/api/config/reload")
    
    if echo "$response" | grep -q '"success":true'; then
        print_success "Full reload completed!"
        echo
        echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
    else
        print_error "Reload failed"
        echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
        exit 1
    fi
}

# Reload layer configs only
reload_layers() {
    print_info "Reloading layer configurations from config/layers/*.yaml..."
    echo
    
    response=$(curl -s -X POST "${WMS_API_URL}/api/config/reload/layers")
    
    if echo "$response" | grep -q '"success":true'; then
        print_success "Layer configs reloaded!"
        echo
        echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
    else
        print_error "Reload failed"
        echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
        exit 1
    fi
}

# Clear caches only
clear_caches() {
    print_info "Clearing all in-memory caches..."
    echo
    
    response=$(curl -s -X POST "${WMS_API_URL}/api/cache/clear")
    
    if echo "$response" | grep -q '"success":true'; then
        print_success "Caches cleared!"
        echo
        echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
    else
        print_error "Cache clear failed"
        echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
        exit 1
    fi
}

# Info about styles
styles_info() {
    print_info "Style files (config/styles/*.json) are loaded fresh on each render request."
    print_info "No reload needed - just save your changes and request a new tile!"
    echo
    print_warning "If you want to see changes immediately, clear the tile cache:"
    echo "  ./scripts/reload-config.sh cache"
}

# Show usage
usage() {
    echo "Usage: $0 [command]"
    echo
    echo "Commands:"
    echo "  (none)     Full reload - reload layer configs and clear all caches"
    echo "  full       Same as above"
    echo "  layers     Reload layer configs only (config/layers/*.yaml)"
    echo "  cache      Clear in-memory caches only (L1, GRIB, Grid, Chunk)"
    echo "  styles     Show info about style config reloading"
    echo "  help       Show this help message"
    echo
    echo "Environment Variables:"
    echo "  WMS_API_URL  URL of the wms-api service (default: http://localhost:3000)"
    echo
    echo "Examples:"
    echo "  # After editing config/layers/gfs.yaml"
    echo "  $0 layers"
    echo
    echo "  # After editing config/styles/temperature.json"
    echo "  $0 cache    # Clear cache to see changes"
    echo
    echo "  # After any config change, full reset"
    echo "  $0"
}

# Main
main() {
    local command="${1:-full}"
    
    case "$command" in
        full|"")
            check_api
            reload_full
            ;;
        layers)
            check_api
            reload_layers
            ;;
        cache)
            check_api
            clear_caches
            ;;
        styles)
            styles_info
            ;;
        help|--help|-h)
            usage
            ;;
        *)
            print_error "Unknown command: $command"
            echo
            usage
            exit 1
            ;;
    esac
}

main "$@"
