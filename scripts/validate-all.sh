#!/bin/bash
# Combined WMS + WMTS Validation
# Runs all OGC compliance checks

set -e

# Configuration
BASE_URL="${BASE_URL:-http://localhost:8080}"
VERBOSE="${VERBOSE:-0}"
QUICK="${QUICK:-0}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --verbose|-v)
            VERBOSE=1
            export VERBOSE=1
            shift
            ;;
        --quick|-q)
            QUICK=1
            export QUICK=1
            shift
            ;;
        --url)
            BASE_URL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--verbose] [--quick] [--url BASE_URL]"
            exit 1
            ;;
    esac
done

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Start validation
echo ""
echo "╔═══════════════════════════════════════════════════════╗"
echo "║                                                       ║"
echo "║       Weather WMS/WMTS Compliance Validation         ║"
echo "║                                                       ║"
echo "╚═══════════════════════════════════════════════════════╝"
echo ""
echo "  Base URL: $BASE_URL"
if [ "$QUICK" -eq 1 ]; then
    echo "  Mode: Quick (essential checks only)"
else
    echo "  Mode: Full (all checks)"
fi
echo ""

# Check if service is reachable
echo -ne "${BLUE}→${NC} Checking service availability... "
if curl -sf "${BASE_URL}/wms?SERVICE=WMS&REQUEST=GetCapabilities" > /dev/null 2>&1; then
    echo -e "${GREEN}OK${NC}"
else
    echo -e "${RED}FAILED${NC}"
    echo ""
    echo "Error: Cannot reach WMS service at $BASE_URL"
    echo "Make sure the services are running with: docker-compose up -d"
    exit 1
fi

echo ""

# Track overall status
WMS_PASSED=0
WMTS_PASSED=0

# Run WMS validation
echo "═══════════════════════════════════════════════════════"
echo "  Running WMS 1.3.0 Validation"
echo "═══════════════════════════════════════════════════════"
echo ""

export WMS_URL="${BASE_URL}/wms"

if "$SCRIPT_DIR/validate-wms.sh"; then
    WMS_PASSED=1
fi

echo ""

# Run WMTS validation
echo "═══════════════════════════════════════════════════════"
echo "  Running WMTS 1.0.0 Validation"
echo "═══════════════════════════════════════════════════════"
echo ""

export WMTS_URL="${BASE_URL}/wmts"

if "$SCRIPT_DIR/validate-wmts.sh"; then
    WMTS_PASSED=1
fi

echo ""

# Overall summary
echo "╔═══════════════════════════════════════════════════════╗"
echo "║                                                       ║"
echo "║                   Overall Summary                     ║"
echo "║                                                       ║"
echo "╚═══════════════════════════════════════════════════════╝"
echo ""

if [ "$WMS_PASSED" -eq 1 ]; then
    echo -e "  WMS 1.3.0:   ${GREEN}✓ COMPLIANT${NC}"
else
    echo -e "  WMS 1.3.0:   ${RED}✗ NON-COMPLIANT${NC}"
fi

if [ "$WMTS_PASSED" -eq 1 ]; then
    echo -e "  WMTS 1.0.0:  ${GREEN}✓ COMPLIANT${NC}"
else
    echo -e "  WMTS 1.0.0:  ${RED}✗ NON-COMPLIANT${NC}"
fi

echo ""

# Final verdict
if [ "$WMS_PASSED" -eq 1 ] && [ "$WMTS_PASSED" -eq 1 ]; then
    echo "╔═══════════════════════════════════════════════════════╗"
    echo "║                                                       ║"
    echo -e "║          ${GREEN}✓ ALL VALIDATIONS PASSED${NC}                  ║"
    echo "║                                                       ║"
    echo "╚═══════════════════════════════════════════════════════╝"
    echo ""
    exit 0
else
    echo "╔═══════════════════════════════════════════════════════╗"
    echo "║                                                       ║"
    echo -e "║          ${RED}✗ SOME VALIDATIONS FAILED${NC}                 ║"
    echo "║                                                       ║"
    echo "╚═══════════════════════════════════════════════════════╝"
    echo ""
    echo "Run with --verbose flag for detailed error information"
    echo ""
    exit 1
fi
