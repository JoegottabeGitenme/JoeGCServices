#!/usr/bin/env bash
#
# validate-configs.sh - Validate YAML configuration files against JSON schemas
#
# This script validates all configuration files in the project against their
# corresponding JSON schemas. It uses a temporary npm environment to avoid
# polluting global npm packages.
#
# Usage:
#   ./scripts/validate-configs.sh           # Validate all config files
#   ./scripts/validate-configs.sh --verbose # Validate with detailed errors
#   ./scripts/validate-configs.sh --help    # Show help
#
# Exit codes:
#   0 - All validations passed
#   1 - One or more validations failed
#   2 - Missing dependencies
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SCHEMA_DIR="$PROJECT_ROOT/schemas"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Counters
PASSED=0
FAILED=0
SKIPPED=0

# Options
VERBOSE=false

usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Validate YAML configuration files against JSON schemas."
    echo ""
    echo "Options:"
    echo "  --verbose, -v    Show detailed validation output"
    echo "  --help, -h       Show this help message"
    echo ""
    echo "Requirements:"
    echo "  - Node.js and npm"
    echo ""
    echo "Examples:"
    echo "  $0               # Validate all configs"
    echo "  $0 --verbose     # Validate with detailed output"
    echo ""
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --verbose|-v|--fix)
            VERBOSE=true
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

# Check for Node.js
if ! command -v node &> /dev/null; then
    echo -e "${RED}Error: Node.js is required but not installed.${NC}"
    echo "Please install Node.js: https://nodejs.org/"
    exit 2
fi

# Check for npm
if ! command -v npm &> /dev/null; then
    echo -e "${RED}Error: npm is required but not installed.${NC}"
    exit 2
fi

# Create temp directory for npm packages and converted files
TEMP_DIR=$(mktemp -d)
trap "rm -rf $TEMP_DIR" EXIT

# Setup local npm environment
echo -e "${YELLOW}Setting up validation environment...${NC}"
cd "$TEMP_DIR"
npm init -y > /dev/null 2>&1
npm install --silent ajv ajv-formats js-yaml 2>/dev/null

# Create the validation script
cat > "$TEMP_DIR/validate.js" << 'VALIDATE_SCRIPT'
const Ajv = require('ajv');
const addFormats = require('ajv-formats');
const yaml = require('js-yaml');
const fs = require('fs');
const path = require('path');

const ajv = new Ajv({ allErrors: true, strict: false });
addFormats(ajv);

const [,, schemaPath, yamlPath, verbose] = process.argv;

try {
    // Load schema
    const schemaContent = fs.readFileSync(schemaPath, 'utf8');
    const schema = JSON.parse(schemaContent);
    
    // Load and parse YAML
    const yamlContent = fs.readFileSync(yamlPath, 'utf8');
    const data = yaml.load(yamlContent);
    
    // Validate
    const validate = ajv.compile(schema);
    const valid = validate(data);
    
    if (valid) {
        console.log('VALID');
        process.exit(0);
    } else {
        console.log('INVALID');
        if (verbose === '--verbose') {
            validate.errors.forEach(err => {
                console.log(`  - ${err.instancePath || '/'}: ${err.message}`);
                if (err.params) {
                    const params = Object.entries(err.params)
                        .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
                        .join(', ');
                    if (params) console.log(`    (${params})`);
                }
            });
        }
        process.exit(1);
    }
} catch (err) {
    console.log('ERROR');
    console.log(`  ${err.message}`);
    process.exit(2);
}
VALIDATE_SCRIPT

cd "$PROJECT_ROOT"

# Function to validate a single file
validate_file() {
    local schema="$1"
    local yaml_file="$2"
    local rel_path="${yaml_file#$PROJECT_ROOT/}"
    
    local verbose_flag=""
    if $VERBOSE; then
        verbose_flag="--verbose"
    fi
    
    local output
    output=$(node "$TEMP_DIR/validate.js" "$schema" "$yaml_file" $verbose_flag 2>&1)
    local exit_code=$?
    
    local status
    status=$(echo "$output" | head -1)
    
    case "$status" in
        "VALID")
            echo -e "${GREEN}PASS${NC} $rel_path"
            ((PASSED++))
            return 0
            ;;
        "INVALID")
            echo -e "${RED}FAIL${NC} $rel_path"
            if $VERBOSE; then
                echo "$output" | tail -n +2
            fi
            ((FAILED++))
            return 1
            ;;
        "ERROR")
            echo -e "${RED}ERROR${NC} $rel_path"
            echo "$output" | tail -n +2
            ((FAILED++))
            return 1
            ;;
        *)
            echo -e "${RED}FAIL${NC} $rel_path (unexpected output)"
            if $VERBOSE; then
                echo "$output"
            fi
            ((FAILED++))
            return 1
            ;;
    esac
}

# Function to validate files matching a pattern against a schema
validate_pattern() {
    local schema="$1"
    local pattern="$2"
    local description="$3"
    
    echo -e "\n${BLUE}Validating $description${NC}"
    echo "Schema: $(basename "$schema")"
    echo "Pattern: $pattern"
    echo "---"
    
    local files
    files=$(find "$PROJECT_ROOT" -path "$PROJECT_ROOT/$pattern" -type f 2>/dev/null | sort)
    
    if [[ -z "$files" ]]; then
        echo -e "${YELLOW}SKIP${NC} No files found matching pattern"
        ((SKIPPED++))
        return 0
    fi
    
    local result=0
    while IFS= read -r file; do
        validate_file "$schema" "$file" || result=1
    done <<< "$files"
    
    return $result
}

echo "=============================================="
echo "  Config File Schema Validation"
echo "=============================================="
echo ""
echo "Project: $PROJECT_ROOT"
echo "Schemas: $SCHEMA_DIR"

# Validate weather model configs
validate_pattern \
    "$SCHEMA_DIR/weather-model.schema.json" \
    "config/models/*.yaml" \
    "Weather Model Configs"

# Validate WMS layer configs
validate_pattern \
    "$SCHEMA_DIR/wms-layer.schema.json" \
    "config/layers/*.yaml" \
    "WMS Layer Configs"

# Validate EDR collection configs (excluding locations.yaml)
echo -e "\n${BLUE}Validating EDR Collection Configs${NC}"
echo "Schema: edr-collection.schema.json"
echo "---"
for file in "$PROJECT_ROOT"/config/edr/*.yaml; do
    if [[ "$(basename "$file")" != "locations.yaml" ]]; then
        validate_file "$SCHEMA_DIR/edr-collection.schema.json" "$file" || true
    fi
done

# Validate EDR locations config
validate_pattern \
    "$SCHEMA_DIR/edr-locations.schema.json" \
    "config/edr/locations.yaml" \
    "EDR Locations Config"

# Validate ingestion config
validate_pattern \
    "$SCHEMA_DIR/ingestion.schema.json" \
    "config/ingestion.yaml" \
    "Ingestion Config"

# Validate WMS/WMTS load test scenarios (non-EDR)
echo -e "\n${BLUE}Validating WMS Load Test Scenarios${NC}"
echo "Schema: load-test-scenario.schema.json"
echo "---"
for file in "$PROJECT_ROOT"/validation/load-test/scenarios/*.yaml; do
    if [[ "$(basename "$file")" != *"-edr"* ]]; then
        validate_file "$SCHEMA_DIR/load-test-scenario.schema.json" "$file" || true
    fi
done

# Validate EDR load test scenarios
echo -e "\n${BLUE}Validating EDR Load Test Scenarios${NC}"
echo "Schema: edr-load-test-scenario.schema.json"
echo "---"
for file in "$PROJECT_ROOT"/validation/load-test/scenarios/*-edr*.yaml; do
    if [[ -f "$file" ]]; then
        validate_file "$SCHEMA_DIR/edr-load-test-scenario.schema.json" "$file" || true
    fi
done

# Validate Helm values
validate_pattern \
    "$SCHEMA_DIR/helm-values.schema.json" \
    "deploy/helm/weather-wms/values.yaml" \
    "Helm Values"

validate_pattern \
    "$SCHEMA_DIR/helm-values.schema.json" \
    "deploy/helm/weather-wms/values-*.yaml" \
    "Helm Values (Environment Overrides)"

# Summary
echo ""
echo "=============================================="
echo "  Summary"
echo "=============================================="
echo -e "  ${GREEN}Passed:${NC}  $PASSED"
echo -e "  ${RED}Failed:${NC}  $FAILED"
echo -e "  ${YELLOW}Skipped:${NC} $SKIPPED"
echo "=============================================="

if [[ $FAILED -gt 0 ]]; then
    echo ""
    echo -e "${RED}Validation failed!${NC}"
    echo "Run with --verbose for detailed error messages."
    exit 1
else
    echo ""
    echo -e "${GREEN}All validations passed!${NC}"
    exit 0
fi
