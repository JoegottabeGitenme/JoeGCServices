# WMS/WMTS Verification Test Plan

## Overview

This document outlines a comprehensive verification test that "hammers" the WMS/WMTS server to ensure **every advertised layer and capability is functional**. Unlike the stress tests in `LOAD_TEST_HAMMERING_PLAN.md` which focus on finding breaking points, this verification test focuses on **coverage and correctness**.

### Goals

1. **100% Layer Coverage** - Fetch at least one tile from every advertised layer
2. **Style Verification** - Test each layer with all its supported styles
3. **Temporal Coverage** - Verify time dimension works for temporal layers
4. **Protocol Compliance** - Test both WMS and WMTS endpoints
5. **Error Handling** - Verify proper error responses for edge cases
6. **Fast Feedback** - Complete in reasonable time (target: < 5 minutes)

---

## Implementation Approach

### Primary: Dynamic Verification Script (Implemented)

The main verification tool is `scripts/verify_all_capabilities.sh` which:
1. Fetches GetCapabilities from both WMS and WMTS
2. **Dynamically parses all advertised layers** (no hardcoding!)
3. Requests one tile from each layer with appropriate style
4. Reports success/failure for each
5. Generates a JSON report for CI/CD integration

### Secondary: Load Test Scenarios

For stress testing with specific layer configurations, use the YAML scenarios in
`validation/load-test/scenarios/`. These contain static layer lists and are useful
for benchmarking specific configurations, but should be updated when layers change.

---

## Quick Start

```bash
# Run the dynamic verification script (recommended)
./scripts/verify_all_capabilities.sh http://localhost:8080

# Or run load test scenarios for benchmarking
cargo run --release -p load-test -- run --scenario scenarios/verify_all_layers.yaml
```

---

## Dynamic Verification Script

The primary verification tool is `scripts/verify_all_capabilities.sh`. This script:

1. **Fetches GetCapabilities** from both WMS and WMTS endpoints
2. **Parses layer names dynamically** - no hardcoded layer lists!
3. **Maps layers to appropriate styles** based on naming conventions
4. **Tests each layer** with WMTS, WMS, and XYZ endpoints
5. **Generates a JSON report** with pass/fail results

### How Layer Discovery Works

The script extracts layer names from WMS GetCapabilities XML:
```bash
# Extract layers matching the pattern: {model}_{parameter}
grep -oP '(?<=<Name>)[^<]+(?=</Name>)' capabilities.xml | \
    grep -E '^(gfs|hrrr|mrms|goes)[0-9]*_'
```

### Style Mapping

Styles are automatically selected based on layer name patterns:

| Pattern | Style |
|---------|-------|
| `*_TMP`, `*_TEMP` | `temperature` |
| `*_WIND_BARBS` | `wind_barbs` |
| `*_PRMSL`, `*_PRES` | `atmospheric` |
| `*_RH`, `*_PWAT` | `humidity` |
| `*_CAPE` | `cape` |
| `*_TCDC`, `*_LCDC`, `*_MCDC`, `*_HCDC` | `cloud` |
| `*_VIS` | `visibility` |
| `*_GUST` | `wind` |
| `*_REFC`, `*_REFL`, `*_RETOP` | `reflectivity` |
| `*_PRECIP_RATE` | `precip_rate` |
| `*_APCP`, `*_QPE*` | `precipitation` |
| `*_LTNG` | `lightning` |
| `*_MXUPHL`, `*_HLCY` | `helicity` |
| `goes*_CMI_C01`, `goes*_CMI_C02`, `goes*_CMI_C03` | `goes_visible` |
| `goes*_CMI_C*` (other) | `goes_ir` |
| (default) | `default` |

---

## Static Load Test Scenarios

For benchmarking specific configurations, use YAML scenarios. These require manual
updates when layers change, but are useful for reproducible load testing.

---

## Running Verification

### Dynamic Verification (Recommended)

```bash
# Make script executable (first time only)
chmod +x scripts/verify_all_capabilities.sh

# Run verification - discovers layers automatically
./scripts/verify_all_capabilities.sh http://localhost:8080
```

### Load Test Scenarios (For Benchmarking)

```bash
# Run the static verification scenario
cargo run --release -p load-test -- run --scenario scenarios/verify_all_layers.yaml --output table

# Stress test all endpoints
cargo run --release -p load-test -- run --scenario scenarios/hammer_all_endpoints.yaml
```

### As CI/CD Pre-Deploy Check

```bash
# In CI/CD pipeline - fails if any layer is broken
./scripts/verify_all_capabilities.sh http://staging:8080 || {
    echo "Verification failed! Blocking deployment."
    exit 1
}
```

### Example Output

```
==============================================
WMS/WMTS Verification Test
Base URL: http://localhost:8080
Timestamp: 20251205_120000
==============================================

--- Service Health Checks ---
v PASS: Health Check
v PASS: Ready Check
v PASS: Metrics Endpoint

--- Fetching Capabilities Documents ---
Fetching WMS GetCapabilities...
v PASS: WMS GetCapabilities
Fetching WMTS GetCapabilities...
v PASS: WMTS GetCapabilities

--- Discovering Layers from WMS GetCapabilities ---
Discovered 46 layers from WMS GetCapabilities

--- Testing All Discovered Layers (WMTS) ---

Model: GFS
v PASS: WMTS gfs_TMP (temperature)
v PASS: WMTS gfs_DPT (default)
...

==============================================
VERIFICATION SUMMARY
==============================================
Layers Discovered: 46
Total Tests:       78
Passed:            78 (100.0%)
Failed:            0

VERIFICATION PASSED: All 78 tests passed!
```

---

## Success Criteria

| Check | Criteria |
|-------|----------|
| All layers render | 100% of advertised layers return HTTP 200 + image/png |
| All styles work | Every layer/style combination renders correctly |
| WMS GetCapabilities | Returns valid XML with all layers |
| WMTS GetCapabilities | Returns valid XML with tile matrix sets |
| GetFeatureInfo | Returns valid JSON/HTML responses |
| Error handling | Invalid requests return proper error responses |

---

## Implementation Steps

1. **Create scenario files** - Add `verify_all_layers.yaml` and `verify_all_styles.yaml`
2. **Create shell script** - Add `scripts/verify_all_capabilities.sh`
3. **Optional: Add Rust verify command** - Enhance load-test tool with `verify` subcommand
4. **Integration** - Add to CI/CD pipeline as gate check
5. **Documentation** - Update README with verification instructions

---

## Files Created

| File | Purpose |
|------|---------|
| `scripts/verify_all_capabilities.sh` | **Primary tool** - Dynamic verification script |
| `validation/load-test/scenarios/verify_all_layers.yaml` | Static layer coverage test |
| `validation/load-test/scenarios/verify_all_styles.yaml` | Static style verification test |
| `validation/load-test/scenarios/hammer_all_endpoints.yaml` | Aggressive stress test |

---

## Future Enhancements

1. [ ] Add `verify` subcommand to load-test Rust tool for better performance
2. [ ] Parse styles from GetCapabilities instead of using naming conventions
3. [ ] Add temporal dimension testing (TIME parameter)
4. [ ] Integrate with GitHub Actions workflow
5. [ ] Add Grafana dashboard for verification results
