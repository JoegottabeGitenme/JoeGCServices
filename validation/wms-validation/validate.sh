#!/bin/bash
# =============================================================================
# WMS Validation - Quick Test Script
# Run this from the host machine to execute conformance tests
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Load environment variables if .env exists
if [ -f .env ]; then
    export $(grep -v '^#' .env | xargs)
fi

# Show usage if no URL provided
if [ -z "$WMS_CAPABILITIES_URL" ] && [ -z "$1" ]; then
    echo "Usage: ./validate.sh [WMS_CAPABILITIES_URL]"
    echo ""
    echo "Examples:"
    echo "  ./validate.sh 'http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0'"
    echo ""
    echo "Or set WMS_CAPABILITIES_URL in .env file and run:"
    echo "  ./validate.sh"
    echo ""
    exit 1
fi

# Use argument if provided, otherwise use env var
if [ -n "$1" ]; then
    export WMS_CAPABILITIES_URL="$1"
fi

echo "============================================="
echo "  WMS 1.3.0 Conformance Validation"
echo "============================================="
echo ""
echo "Target: $WMS_CAPABILITIES_URL"
echo ""

# Run the test container
docker compose --profile test run --rm test-runner

# Show results location
echo ""
echo "Results are saved in: ./results/"
echo ""
