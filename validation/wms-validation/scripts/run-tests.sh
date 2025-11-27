#!/bin/bash
# =============================================================================
# WMS 1.3.0 Conformance Test Runner
# =============================================================================
set -e

# Configuration
ETS_VERSION="${ETS_VERSION:-1.32}"
ETS_JAR="ets-wms13-${ETS_VERSION}-aio.jar"
ETS_URL="https://repo1.maven.org/maven2/org/opengis/cite/ets-wms13/${ETS_VERSION}/${ETS_JAR}"
RESULTS_DIR="/opt/results"
CONFIG_DIR="/opt/config"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
TEST_RUN_DIR="${RESULTS_DIR}/run_${TIMESTAMP}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}=============================================${NC}"
echo -e "${BLUE}  OGC WMS 1.3.0 Conformance Test Runner${NC}"
echo -e "${BLUE}=============================================${NC}"
echo ""

# -----------------------------------------------------------------------------
# Validate WMS URL
# -----------------------------------------------------------------------------
if [ -z "$WMS_CAPABILITIES_URL" ]; then
    echo -e "${RED}ERROR: WMS_CAPABILITIES_URL environment variable is not set${NC}"
    echo "Please set it in your .env file or pass it directly"
    exit 1
fi

echo -e "${YELLOW}Target WMS:${NC} $WMS_CAPABILITIES_URL"
echo -e "${YELLOW}ETS Version:${NC} $ETS_VERSION"
echo ""

# -----------------------------------------------------------------------------
# Wait for WMS to be available
# -----------------------------------------------------------------------------
echo -e "${BLUE}Checking WMS availability...${NC}"
MAX_RETRIES=30
RETRY_COUNT=0

while [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
    if curl -sf "$WMS_CAPABILITIES_URL" > /dev/null 2>&1; then
        echo -e "${GREEN}✓ WMS server is responding${NC}"
        break
    fi
    RETRY_COUNT=$((RETRY_COUNT + 1))
    echo "  Waiting for WMS server... (attempt $RETRY_COUNT/$MAX_RETRIES)"
    sleep 5
done

if [ $RETRY_COUNT -eq $MAX_RETRIES ]; then
    echo -e "${RED}ERROR: WMS server not responding after $MAX_RETRIES attempts${NC}"
    exit 1
fi

# -----------------------------------------------------------------------------
# Download ETS JAR if not present
# -----------------------------------------------------------------------------
echo ""
echo -e "${BLUE}Checking for ETS test suite...${NC}"

if [ ! -f "/opt/${ETS_JAR}" ]; then
    echo "Downloading ETS WMS 1.3.0 v${ETS_VERSION}..."
    curl -L -o "/opt/${ETS_JAR}" "$ETS_URL"
    if [ $? -ne 0 ]; then
        echo -e "${RED}ERROR: Failed to download ETS JAR${NC}"
        exit 1
    fi
    echo -e "${GREEN}✓ Downloaded successfully${NC}"
else
    echo -e "${GREEN}✓ ETS JAR already present${NC}"
fi

# -----------------------------------------------------------------------------
# Create test run configuration
# -----------------------------------------------------------------------------
echo ""
echo -e "${BLUE}Creating test configuration...${NC}"

mkdir -p "$TEST_RUN_DIR"

# Escape ampersands for XML
ESCAPED_URL=$(echo "$WMS_CAPABILITIES_URL" | sed 's/&/\&amp;/g')

cat > "${TEST_RUN_DIR}/test-run-props.xml" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE properties SYSTEM "http://java.sun.com/dtd/properties.dtd">
<properties version="1.0">
    <comment>WMS 1.3.0 Test Run - ${TIMESTAMP}</comment>
    <entry key="capabilities-url">${ESCAPED_URL}</entry>
</properties>
EOF

echo -e "${GREEN}✓ Configuration created${NC}"

# -----------------------------------------------------------------------------
# Run the tests
# -----------------------------------------------------------------------------
echo ""
echo -e "${BLUE}=============================================${NC}"
echo -e "${BLUE}  Running Conformance Tests${NC}"
echo -e "${BLUE}=============================================${NC}"
echo ""

cd /opt

# Run the test suite and capture output
java -jar "${ETS_JAR}" "${TEST_RUN_DIR}/test-run-props.xml" 2>&1 | tee "${TEST_RUN_DIR}/test-output.log"
TEST_EXIT_CODE=${PIPESTATUS[0]}

# -----------------------------------------------------------------------------
# Collect results
# -----------------------------------------------------------------------------
echo ""
echo -e "${BLUE}Collecting test results...${NC}"

# Find and copy TestNG results
TESTNG_DIR=$(find ~/testng -maxdepth 1 -type d -name "*" | sort -r | head -1)
if [ -d "$TESTNG_DIR" ] && [ "$TESTNG_DIR" != ~/testng ]; then
    cp -r "$TESTNG_DIR"/* "${TEST_RUN_DIR}/"
    echo -e "${GREEN}✓ Results copied to ${TEST_RUN_DIR}${NC}"
fi

# -----------------------------------------------------------------------------
# Parse and display results summary
# -----------------------------------------------------------------------------
echo ""
echo -e "${BLUE}=============================================${NC}"
echo -e "${BLUE}  Test Results Summary${NC}"
echo -e "${BLUE}=============================================${NC}"

if [ -f "${TEST_RUN_DIR}/testng-results.xml" ]; then
    # Extract test counts from TestNG XML
    TOTAL=$(grep -oP 'total="\K[0-9]+' "${TEST_RUN_DIR}/testng-results.xml" | head -1)
    PASSED=$(grep -oP 'passed="\K[0-9]+' "${TEST_RUN_DIR}/testng-results.xml" | head -1)
    FAILED=$(grep -oP 'failed="\K[0-9]+' "${TEST_RUN_DIR}/testng-results.xml" | head -1)
    SKIPPED=$(grep -oP 'skipped="\K[0-9]+' "${TEST_RUN_DIR}/testng-results.xml" | head -1)
    
    echo ""
    echo -e "  Total Tests:   ${TOTAL:-N/A}"
    echo -e "  ${GREEN}Passed:${NC}        ${PASSED:-0}"
    echo -e "  ${RED}Failed:${NC}        ${FAILED:-0}"
    echo -e "  ${YELLOW}Skipped:${NC}       ${SKIPPED:-0}"
    echo ""
    
    if [ "${FAILED:-0}" -eq 0 ] && [ "${PASSED:-0}" -gt 0 ]; then
        echo -e "${GREEN}★ ALL TESTS PASSED ★${NC}"
    elif [ "${FAILED:-0}" -gt 0 ]; then
        echo -e "${RED}✗ SOME TESTS FAILED${NC}"
        echo ""
        echo "Failed tests:"
        grep -oP 'name="\K[^"]+(?="[^>]*status="FAIL")' "${TEST_RUN_DIR}/testng-results.xml" | head -20 | while read test; do
            echo -e "  ${RED}✗${NC} $test"
        done
    fi
else
    echo -e "${YELLOW}Warning: Could not find testng-results.xml${NC}"
    echo "Check ${TEST_RUN_DIR}/test-output.log for details"
fi

echo ""
echo -e "${BLUE}Results saved to:${NC} ${TEST_RUN_DIR}"
echo ""

# -----------------------------------------------------------------------------
# Generate HTML report summary
# -----------------------------------------------------------------------------
cat > "${TEST_RUN_DIR}/summary.html" << EOF
<!DOCTYPE html>
<html>
<head>
    <title>WMS 1.3.0 Test Results - ${TIMESTAMP}</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 40px; background: #f5f5f5; }
        .container { max-width: 800px; margin: 0 auto; background: white; padding: 30px; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
        h1 { color: #333; border-bottom: 2px solid #007bff; padding-bottom: 10px; }
        .summary { display: flex; gap: 20px; margin: 20px 0; }
        .stat { flex: 1; padding: 20px; border-radius: 8px; text-align: center; }
        .stat.passed { background: #d4edda; color: #155724; }
        .stat.failed { background: #f8d7da; color: #721c24; }
        .stat.skipped { background: #fff3cd; color: #856404; }
        .stat.total { background: #cce5ff; color: #004085; }
        .stat-value { font-size: 2em; font-weight: bold; }
        .stat-label { font-size: 0.9em; text-transform: uppercase; }
        .info { background: #e9ecef; padding: 15px; border-radius: 4px; margin: 20px 0; }
        .info dt { font-weight: bold; }
        .info dd { margin: 0 0 10px 0; }
    </style>
</head>
<body>
    <div class="container">
        <h1>WMS 1.3.0 Conformance Test Results</h1>
        
        <dl class="info">
            <dt>Test Run:</dt>
            <dd>${TIMESTAMP}</dd>
            <dt>WMS Endpoint:</dt>
            <dd>${WMS_CAPABILITIES_URL}</dd>
            <dt>ETS Version:</dt>
            <dd>${ETS_VERSION}</dd>
        </dl>
        
        <div class="summary">
            <div class="stat total">
                <div class="stat-value">${TOTAL:-N/A}</div>
                <div class="stat-label">Total</div>
            </div>
            <div class="stat passed">
                <div class="stat-value">${PASSED:-0}</div>
                <div class="stat-label">Passed</div>
            </div>
            <div class="stat failed">
                <div class="stat-value">${FAILED:-0}</div>
                <div class="stat-label">Failed</div>
            </div>
            <div class="stat skipped">
                <div class="stat-value">${SKIPPED:-0}</div>
                <div class="stat-label">Skipped</div>
            </div>
        </div>
        
        <p>Full TestNG report available in <code>testng-results.xml</code></p>
    </div>
</body>
</html>
EOF

echo -e "${GREEN}✓ HTML summary generated${NC}"

exit $TEST_EXIT_CODE
