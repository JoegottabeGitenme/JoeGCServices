# Scripts Reference

Utility scripts for managing Weather WMS located in the `scripts/` directory.

## Service Management

### start.sh

**Purpose**: Start all services with Docker Compose

```bash
./scripts/start.sh
```

**What it does**:
1. Builds Docker images (first run only)
2. Starts all 8 services
3. Initializes database schema
4. Creates MinIO bucket
5. Waits for services to be healthy

**Options**: None (configured via `.env`)

---

## Data Download Scripts

### download_gfs.sh

**Purpose**: Download GFS (Global Forecast System) data

```bash
./scripts/download_gfs.sh
```

**Downloads**:
- Latest GFS cycle (00, 06, 12, or 18 UTC)
- Forecast hours 0-120 (every 3 hours)
- File size: ~4 GB total
- Automatically triggers ingestion

**Location**: `/data/downloads/gfs/`

---

### download_hrrr.sh

**Purpose**: Download HRRR (High-Resolution Rapid Refresh) data

```bash
./scripts/download_hrrr.sh
```

**Downloads**:
- Recent HRRR cycle
- Forecast hours 0-18 (hourly)
- File size: ~3 GB total

---

### download_mrms.sh

**Purpose**: Download MRMS (radar) data

```bash
./scripts/download_mrms.sh
```

**Downloads**:
- Recent radar composites
- Reflectivity and precipitation rate
- File size: ~500 MB

---

### download_goes.sh

**Purpose**: Download GOES satellite imagery

```bash
./scripts/download_goes.sh
```

**Downloads**:
- GOES-18 channels 2 and 13
- Recent imagery (last hour)
- File size: ~500 MB

---

### download_goes_temporal.sh

**Purpose**: Download GOES data for specific time range

```bash
./scripts/download_goes_temporal.sh
```

Used for testing temporal queries. Downloads multiple time steps.

---

### download_hrrr_temporal.sh

**Purpose**: Download HRRR data for specific time range

```bash
./scripts/download_hrrr_temporal.sh
```

Used for testing temporal queries.

---

## Data Processing

### ingest_test_data.sh

**Purpose**: Trigger ingestion of downloaded data

```bash
./scripts/ingest_test_data.sh
```

**What it does**:
1. Scans `/data/downloads/` for data files
2. Triggers ingestion via API
3. Waits for completion
4. Reports success/failure

**Usage**:
```bash
# After downloading data
./scripts/download_gfs.sh
./scripts/ingest_test_data.sh
```

---

### reset_test_state.sh

**Purpose**: Reset system to clean state for testing

```bash
./scripts/reset_test_state.sh
```

**What it does**:
1. Clears L1 (in-memory) cache
2. Flushes L2 (Redis) cache
3. Optionally clears database
4. Optionally removes downloaded files

**⚠️ Warning**: Destructive operation! Use only for testing.

---

## Testing Scripts

### validate-wms.sh

**Purpose**: Validate WMS compliance

```bash
./scripts/validate-wms.sh
```

Tests WMS endpoints against OGC standards.

---

### validate-wmts.sh

**Purpose**: Validate WMTS compliance

```bash
./scripts/validate-wmts.sh
```

Tests WMTS endpoints.

---

### validate-all.sh

**Purpose**: Run all validation tests

```bash
./scripts/validate-all.sh
```

Runs both WMS and WMTS validation.

---

### run_load_test.sh

**Purpose**: Run single load test scenario

```bash
./scripts/run_load_test.sh [scenario]
```

**Example**:
```bash
./scripts/run_load_test.sh realistic
```

---

### run_all_load_tests.sh

**Purpose**: Run all load test scenarios

```bash
./scripts/run_all_load_tests.sh
```

Runs all scenarios in `validation/load-test/scenarios/`.

Results saved to `validation/load-test/results/`.

---

### run_benchmarks.sh

**Purpose**: Run performance benchmarks

```bash
./scripts/run_benchmarks.sh
```

Runs Criterion benchmarks for all crates.

---

### test_rendering.sh

**Purpose**: Test rendering pipeline

```bash
./scripts/test_rendering.sh
```

Tests tile rendering with various parameters.

---

## Profiling Scripts

### profile_flamegraph.sh

**Purpose**: Generate flamegraph for performance analysis

```bash
./scripts/profile_flamegraph.sh [service]
```

**Example**:
```bash
./scripts/profile_flamegraph.sh wms-api
```

Generates `flamegraph.svg` showing CPU usage.

**Requirements**: `cargo flamegraph` installed

---

### profile_request_pipeline.sh

**Purpose**: Profile request handling pipeline

```bash
./scripts/profile_request_pipeline.sh
```

Profiles specific request types and reports timing.

---

## Utility Scripts

### extract_goes_timestamps.sh

**Purpose**: Extract timestamps from GOES filenames

```bash
./scripts/extract_goes_timestamps.sh [directory]
```

Used for debugging temporal data issues.

---

### extract_hrrr_timestamps.sh

**Purpose**: Extract timestamps from HRRR filenames

```bash
./scripts/extract_hrrr_timestamps.sh [directory]
```

---

### update-grafana-dashboard.sh

**Purpose**: Update Grafana dashboard configuration

```bash
./scripts/update-grafana-dashboard.sh
```

Applies dashboard updates from `deploy/grafana/`.

---

## Script Conventions

All scripts follow these conventions:

1. **Exit codes**:
   - `0`: Success
   - `1`: General error
   - `2`: Invalid arguments

2. **Output**:
   - Info messages to stdout
   - Error messages to stderr
   - JSON output when appropriate

3. **Dependencies**:
   - Check for required commands
   - Fail early with clear error messages

4. **Safety**:
   - Prompt for destructive operations
   - Support `--dry-run` where applicable
   - Log actions for audit trail

## Creating Custom Scripts

Template for new scripts:

```bash
#!/bin/bash
# Description: Brief description of what this script does
#
# Usage: ./script.sh [options]

set -euo pipefail  # Exit on error, undefined vars, pipe failures

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Functions
error() {
    echo "ERROR: $*" >&2
    exit 1
}

info() {
    echo "INFO: $*"
}

# Main
main() {
    info "Starting script..."
    
    # Your code here
    
    info "Done!"
}

main "$@"
```

## Next Steps

- [Troubleshooting](./troubleshooting.md) - Problem solving
- [Development Guide](../development/README.md) - Development workflow
