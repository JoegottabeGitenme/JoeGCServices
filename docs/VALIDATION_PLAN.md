# OGC Compliance Validation Implementation Plan

## Executive Summary

This plan outlines the implementation of automated OGC WMS/WMTS compliance validation as part of the Weather WMS system. The validation suite will:

1. **Run automatically on system startup** to verify service compliance
2. **Display validation status in the web UI** with real-time results
3. **Execute as a pre-commit hook** to prevent non-compliant commits
4. **Support both quick sanity checks and full OGC TEAM Engine tests**

---

## Current State Analysis

### Existing Validation Infrastructure

The `validation/` folder already contains:
- **OGC TEAM Engine Docker setup** - Full WMS 1.3.0 conformance testing
- **Test runner scripts** - Automated test execution
- **Results parsing** - TestNG XML results with HTML summary generation

### Gaps to Address

1. **No integration with startup workflow** - Validation is manual only
2. **No web UI integration** - No visibility into compliance status
3. **No pre-commit hooks** - Compliance isn't enforced before commits
4. **No lightweight quick-check** - Only full TEAM Engine tests exist (slow)
5. **No WMTS validation** - Only WMS is covered

---

## Implementation Plan

### Phase 1: Quick Validation Scripts (Lightweight)
**Estimated Time: 2 hours**
**Priority: High**

Create fast, lightweight validation scripts that can run in seconds (vs. minutes for full OGC tests).

#### 1.1 Create `scripts/validate-wms.sh`

A quick WMS compliance checker that validates:
- GetCapabilities returns valid XML with required elements
- GetCapabilities schema validates against WMS 1.3.0 XSD
- GetMap returns valid PNG images
- GetFeatureInfo returns valid responses
- All advertised layers are queryable
- Required CRS support (EPSG:4326, EPSG:3857, CRS:84)
- Required formats (image/png)

Quick tests (< 30 seconds):
- Capabilities XML structure validation
- Required WMS operations present (GetCapabilities, GetMap, GetFeatureInfo)
- All layers have required elements (Name, Title, CRS, BoundingBox)
- GetMap returns image/png for each layer
- GetFeatureInfo returns valid JSON/XML for each queryable layer
- Exception handling (invalid parameters return proper XML exceptions)

#### 1.2 Create `scripts/validate-wmts.sh`

A quick WMTS compliance checker that validates:
- WMTS GetCapabilities returns valid XML
- Required operations present
- TileMatrixSet properly defined
- REST endpoints functional
- KVP endpoints functional

#### 1.3 Create `scripts/validate-all.sh`

Combined script that runs both WMS and WMTS validation with summary output.

---

### Phase 2: API Validation Endpoint
**Estimated Time: 1.5 hours**
**Priority: High**

Add a `/api/validation` endpoint to the WMS API service that returns current compliance status.

#### 2.1 Validation Status Endpoint

**Endpoint:** `GET /api/validation/status`

**Response:**
```json
{
  "timestamp": "2025-11-26T16:30:00Z",
  "wms": {
    "status": "compliant",
    "version": "1.3.0",
    "checks": {
      "capabilities": { "status": "pass", "message": "Valid WMS 1.3.0 capabilities" },
      "getmap": { "status": "pass", "message": "All 5 layers render correctly" },
      "getfeatureinfo": { "status": "pass", "message": "All 5 layers queryable" },
      "exceptions": { "status": "pass", "message": "Proper exception handling" }
    },
    "layers_tested": 5,
    "layers_passed": 5
  },
  "wmts": {
    "status": "compliant",
    "version": "1.0.0",
    "checks": {
      "capabilities": { "status": "pass", "message": "Valid WMTS 1.0.0 capabilities" },
      "gettile_rest": { "status": "pass", "message": "REST tiles working" },
      "gettile_kvp": { "status": "pass", "message": "KVP tiles working" }
    }
  },
  "overall_status": "compliant",
  "last_full_test": "2025-11-26T12:00:00Z",
  "full_test_results": {
    "total": 45,
    "passed": 45,
    "failed": 0,
    "skipped": 0
  }
}
```

#### 2.2 Run Validation Endpoint

**Endpoint:** `POST /api/validation/run`

Triggers a quick validation run and returns results.

#### 2.3 Detailed Results Endpoint

**Endpoint:** `GET /api/validation/results/{run_id}`

Returns detailed results from a specific validation run.

---

### Phase 3: Web UI Integration
**Estimated Time: 2 hours**
**Priority: High**

Add a validation status panel to the web dashboard.

#### 3.1 UI Components

**Validation Status Panel (Sidebar Section):**
```
┌─────────────────────────────────────┐
│ OGC Compliance Status               │
├─────────────────────────────────────┤
│ WMS 1.3.0:  ● Compliant            │
│   ✓ GetCapabilities                 │
│   ✓ GetMap (5/5 layers)             │
│   ✓ GetFeatureInfo (5/5 layers)     │
│   ✓ Exception Handling              │
├─────────────────────────────────────┤
│ WMTS 1.0.0: ● Compliant            │
│   ✓ GetCapabilities                 │
│   ✓ REST Tiles                      │
│   ✓ KVP Tiles                       │
├─────────────────────────────────────┤
│ Last checked: 2 minutes ago         │
│ [Run Validation] [Full OGC Test]    │
└─────────────────────────────────────┘
```

**Status Indicators:**
- 🟢 Green: Compliant
- 🟡 Yellow: Partial/Warning
- 🔴 Red: Non-compliant
- ⚪ Gray: Not tested

#### 3.2 JavaScript Integration

Add to `web/app.js`:
- `checkValidationStatus()` - Fetch validation status on load
- `runQuickValidation()` - Trigger validation on button click
- `updateValidationUI()` - Update UI with results
- Auto-refresh every 5 minutes

#### 3.3 CSS Styling

Add validation panel styles to match existing dashboard design.

---

### Phase 4: Startup Integration
**Estimated Time: 1 hour**
**Priority: Medium**

Integrate validation into the system startup workflow.

#### 4.1 Modify `scripts/start.sh`

Add validation step after services are healthy:

```bash
# After services start:
echo "Running compliance validation..."
./scripts/validate-all.sh --quick

if [ $? -ne 0 ]; then
    echo "⚠️  WARNING: Some compliance checks failed"
    echo "Run './scripts/validate-all.sh --verbose' for details"
fi
```

#### 4.2 Add Health Check with Validation

Modify Docker healthcheck to include basic compliance:

```yaml
healthcheck:
  test: ["CMD", "curl", "-sf", "http://localhost:8080/api/validation/quick"]
  interval: 60s
  timeout: 30s
```

---

### Phase 5: Pre-commit Hook
**Estimated Time: 1.5 hours**
**Priority: High**

Create a pre-commit hook that validates WMS compliance before allowing commits.

#### 5.1 Create `.git/hooks/pre-commit`

```bash
#!/bin/bash
# Weather WMS Pre-commit Hook
# Ensures code changes don't break OGC compliance

set -e

echo "🔍 Running pre-commit validation..."

# 1. Run cargo checks
echo "  Checking code formatting..."
cargo fmt --check || {
    echo "❌ Code is not formatted. Run 'cargo fmt' first."
    exit 1
}

echo "  Running clippy..."
cargo clippy -- -D warnings || {
    echo "❌ Clippy found issues. Fix them before committing."
    exit 1
}

echo "  Running unit tests..."
cargo test --quiet || {
    echo "❌ Unit tests failed."
    exit 1
}

# 2. Run WMS validation (only if services are running)
if curl -sf "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities" > /dev/null 2>&1; then
    echo "  Running WMS compliance checks..."
    ./scripts/validate-wms.sh --quick || {
        echo "❌ WMS compliance check failed."
        echo "   Your changes may have broken OGC compliance."
        exit 1
    }
else
    echo "  ⚠️  WMS service not running, skipping compliance check"
fi

echo "✅ All pre-commit checks passed!"
```

#### 5.2 Create Hook Installation Script

`scripts/install-hooks.sh`:
```bash
#!/bin/bash
# Install git hooks for Weather WMS development

HOOKS_DIR="$(git rev-parse --show-toplevel)/.git/hooks"
cp scripts/hooks/pre-commit "$HOOKS_DIR/pre-commit"
chmod +x "$HOOKS_DIR/pre-commit"
echo "✅ Git hooks installed"
```

#### 5.3 Add to DEVELOPMENT.md

Document the pre-commit hook setup for developers.

---

### Phase 6: Full OGC TEAM Engine Integration
**Estimated Time: 1.5 hours**
**Priority: Medium**

Enhance the existing TEAM Engine setup for better integration.

#### 6.1 Update `validation/wms-validation/docker-compose.yml`

Add network configuration to connect with main services.

#### 6.2 Create `scripts/validate-ogc-full.sh`

Wrapper script to run full OGC TEAM Engine tests:
```bash
#!/bin/bash
# Run full OGC conformance tests

cd validation/wms-validation
./validate.sh "http://host.docker.internal:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0"

# Parse results and return exit code
```

#### 6.3 Store Results in API

After full tests complete, store results accessible via API:
- `/api/validation/full-results` - Latest full test results
- `/api/validation/full-history` - Historical test results

---

### Phase 7: CI/CD Integration (Optional)
**Estimated Time: 1 hour**
**Priority: Low**

Add GitHub Actions workflow for automated testing.

#### 7.1 Create `.github/workflows/compliance.yml`

```yaml
name: OGC Compliance

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  quick-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Start services
        run: docker-compose up -d
      - name: Wait for healthy
        run: ./scripts/wait-for-healthy.sh
      - name: Run quick validation
        run: ./scripts/validate-all.sh --quick
      - name: Stop services
        run: docker-compose down

  full-compliance:
    runs-on: ubuntu-latest
    if: github.event_name == 'push' && github.ref == 'refs/heads/main'
    steps:
      - uses: actions/checkout@v4
      - name: Run full OGC tests
        run: ./scripts/validate-ogc-full.sh
      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: ogc-compliance-results
          path: validation/wms-validation/results/
```

---

## File Structure

```
weather-wms/
├── scripts/
│   ├── validate-wms.sh          # Quick WMS validation
│   ├── validate-wmts.sh         # Quick WMTS validation
│   ├── validate-all.sh          # Combined validation
│   ├── validate-ogc-full.sh     # Full OGC TEAM Engine tests
│   ├── install-hooks.sh         # Git hooks installer
│   ├── hooks/
│   │   └── pre-commit           # Pre-commit hook
│   └── start.sh                 # (modified) Add validation step
│
├── services/wms-api/src/
│   ├── handlers.rs              # (modified) Add validation endpoints
│   └── validation.rs            # NEW: Validation logic
│
├── web/
│   ├── app.js                   # (modified) Add validation UI
│   ├── index.html               # (modified) Add validation panel
│   └── style.css                # (modified) Add validation styles
│
├── validation/
│   ├── wms-validation/          # Existing OGC TEAM Engine setup
│   └── wmts-validation/         # NEW: WMTS validation (if needed)
│
└── .github/workflows/
    └── compliance.yml           # CI/CD workflow
```

---

## Implementation Order

| Phase | Description | Priority | Est. Time | Dependencies |
|-------|-------------|----------|-----------|--------------|
| 1 | Quick Validation Scripts | High | 2 hours | None |
| 2 | API Validation Endpoint | High | 1.5 hours | Phase 1 |
| 3 | Web UI Integration | High | 2 hours | Phase 2 |
| 4 | Startup Integration | Medium | 1 hour | Phase 1 |
| 5 | Pre-commit Hook | High | 1.5 hours | Phase 1 |
| 6 | Full OGC Integration | Medium | 1.5 hours | Phase 1 |
| 7 | CI/CD Integration | Low | 1 hour | Phase 1-6 |

**Total Estimated Time: ~10.5 hours**

---

## Quick Validation Checks (Phase 1 Detail)

### WMS Checks

| Check | Description | Pass Criteria |
|-------|-------------|---------------|
| `capabilities_valid` | GetCapabilities returns valid XML | HTTP 200, valid XML, has WMS_Capabilities root |
| `capabilities_version` | Correct WMS version | version="1.3.0" attribute present |
| `capabilities_service` | Service metadata present | Service/Name, Service/Title exist |
| `capabilities_operations` | Required operations | GetCapabilities, GetMap, GetFeatureInfo in Request |
| `capabilities_layers` | Layers properly defined | Each layer has Name, Title, CRS, BoundingBox |
| `getmap_png` | GetMap returns PNG | HTTP 200, Content-Type: image/png |
| `getmap_all_layers` | All layers render | Each layer returns valid PNG |
| `getfeatureinfo_json` | GFI returns JSON | HTTP 200, valid JSON response |
| `getfeatureinfo_all_layers` | All queryable layers work | Each queryable layer returns data |
| `exception_invalid_layer` | Invalid layer exception | Returns ServiceException XML |
| `exception_invalid_crs` | Invalid CRS exception | Returns ServiceException XML |
| `crs_4326` | EPSG:4326 support | Renders correctly in EPSG:4326 |
| `crs_3857` | EPSG:3857 support | Renders correctly in EPSG:3857 |

### WMTS Checks

| Check | Description | Pass Criteria |
|-------|-------------|---------------|
| `capabilities_valid` | GetCapabilities returns valid XML | HTTP 200, valid WMTS XML |
| `capabilities_tilematrixset` | TileMatrixSet defined | WebMercatorQuad present |
| `gettile_rest` | REST tiles work | HTTP 200 for tile requests |
| `gettile_kvp` | KVP tiles work | HTTP 200 for KVP tile requests |
| `tile_format` | Correct format | Content-Type: image/png |

---

## Success Criteria

1. **Quick validation completes in < 30 seconds**
2. **Web UI shows real-time compliance status**
3. **Pre-commit hook prevents non-compliant commits**
4. **Full OGC tests can be run on-demand**
5. **Results are stored and accessible via API**
6. **Integration with startup workflow is seamless**

---

## Next Steps

To implement this plan:

1. Create the quick validation scripts (`validate-wms.sh`, `validate-wmts.sh`, `validate-all.sh`)
2. Add the `/api/validation` endpoints to the WMS API
3. Update the web UI with the validation status panel
4. Modify `start.sh` to include validation
5. Create and install the pre-commit hook
6. Enhance the TEAM Engine integration
