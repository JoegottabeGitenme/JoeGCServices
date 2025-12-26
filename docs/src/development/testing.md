# Testing

Comprehensive testing strategy for Weather WMS covering unit tests, integration tests, and validation.

## Test Organization

```
crates/
├── grib2-parser/tests/
│   ├── parse_gfs.rs         # GFS GRIB2 parsing tests
│   ├── parse_mrms.rs        # MRMS parsing tests
│   ├── mrms_bounds.rs       # Bounds extraction tests
│   ├── sections_unit_tests.rs # Section parsing unit tests
│   └── common/mod.rs        # Shared test utilities
├── grid-processor/tests/
│   ├── zarr_roundtrip.rs    # Zarr read/write roundtrip tests
│   └── testdata_integration.rs # Integration with test data
├── renderer/tests/
│   ├── gradient_tests.rs    # Color gradient tests
│   ├── contour_tests.rs     # Contour generation tests
│   ├── barbs_tests.rs       # Wind barb rendering tests
│   ├── numbers_tests.rs     # Numeric label tests
│   ├── style_tests.rs       # Style configuration tests
│   └── png_tests.rs         # PNG encoding tests
├── wms-common/tests/
│   └── bbox_tests.rs        # Bounding box tests
├── test-utils/              # Shared test utilities crate
│   └── src/
│       ├── lib.rs           # Macros: require_test_file!, assert_approx_eq!
│       ├── fixtures.rs      # Common test fixtures
│       ├── generators.rs    # Grid data generators
│       └── paths.rs         # Test file path helpers
services/
├── ingester/tests/
│   └── server_tests.rs      # Ingester API tests
validation/
└── load-test/scenarios/     # Load testing scenarios
    ├── gfs.yaml             # GFS-focused load test
    ├── hrrr.yaml            # HRRR-focused load test
    ├── goes.yaml            # GOES satellite load test
    ├── mrms.yaml            # MRMS radar load test
    └── mixed.yaml           # Multi-source realistic traffic
web/
├── wms-compliance.html      # WMS 1.3.0 compliance tests
├── wms-compliance.js        # WMS test implementation
├── wmts-compliance.html     # WMTS 1.0.0 compliance tests
└── wmts-compliance.js       # WMTS test implementation
```

## Running Tests

### All Tests

```bash
# Run all workspace tests
cargo test --workspace

# With output
cargo test --workspace -- --nocapture

# Run in parallel (default)
cargo test --workspace -- --test-threads=8
```

### Specific Tests

```bash
# Single crate
cargo test -p grib2-parser

# Single test file
cargo test --test integration

# Single test function
cargo test test_parse_gfs_file

# Tests matching pattern
cargo test grib2
```

### Test Types

```bash
# Unit tests only (in src/)
cargo test --lib

# Integration tests only (in tests/)
cargo test --test '*'

# Doc tests only
cargo test --doc

# Criterion benchmarks (in crates/renderer/benches/)
cargo bench -p renderer
```

## Test Categories

### Unit Tests

Located in source files:

```rust
// crates/grib2-parser/src/lib.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_indicator() {
        let data = b"GRIB\x00\x00\x00\x02\x00\x00\x00\x00\x00\x00\x00\x64";
        let indicator = Indicator::parse(data).unwrap();
        
        assert_eq!(indicator.discipline, 0);
        assert_eq!(indicator.edition, 2);
        assert_eq!(indicator.total_length, 100);
    }

    #[test]
    fn test_invalid_magic() {
        let data = b"BADM\x00\x00\x00\x02";
        let result = Indicator::parse(data);
        
        assert!(result.is_err());
    }
}
```

### Integration Tests

Located in `tests/` directory:

```rust
// services/wms-api/tests/api_tests.rs

use axum_test::TestServer;

#[tokio::test]
async fn test_health_endpoint() {
    let app = create_test_app().await;
    let server = TestServer::new(app).unwrap();
    
    let response = server.get("/health").await;
    
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<serde_json::Value>().get("status"), Some("ok"));
}

#[tokio::test]
async fn test_wms_getcapabilities() {
    let server = create_test_server().await;
    
    let response = server
        .get("/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0")
        .await;
    
    assert_eq!(response.status(), 200);
    assert!(response.text().contains("WMS_Capabilities"));
}
```

### Doc Tests

Embedded in documentation:

```rust
/// Parses a GRIB2 file.
///
/// # Example
///
/// ```
/// use grib2_parser::parse_file;
///
/// let messages = parse_file("test.grib2")?;
/// assert!(!messages.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn parse_file(path: &str) -> Result<Vec<Message>> {
    // Implementation
}
```

## Test Data

### Small Test Files

```bash
# Create test data directory
mkdir -p crates/grib2-parser/tests/data

# Download sample files
curl -o crates/grib2-parser/tests/data/sample.grib2 \
  https://example.com/sample.grib2
```

### Ignored Tests (Requires Test Data)

```rust
#[test]
#[ignore]  // Run with: cargo test -- --ignored
fn test_real_gfs_file() {
    let path = "tests/data/gfs.t00z.pgrb2.0p25.f000";
    if !Path::new(path).exists() {
        return; // Skip if file not available
    }
    
    let messages = parse_file(path).unwrap();
    assert_eq!(messages.len(), 586);
}
```

Run ignored tests:
```bash
cargo test -- --ignored
```

## Mocking

### Mock External Dependencies

```rust
// Use trait for mockability
#[async_trait]
pub trait Storage {
    async fn get(&self, key: &str) -> Result<Vec<u8>>;
    async fn put(&self, key: &str, data: &[u8]) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;

    mock! {
        Storage {}
        
        #[async_trait]
        impl Storage for Storage {
            async fn get(&self, key: &str) -> Result<Vec<u8>>;
            async fn put(&self, key: &str, data: &[u8]) -> Result<()>;
        }
    }

    #[tokio::test]
    async fn test_with_mock_storage() {
        let mut mock = MockStorage::new();
        mock.expect_get()
            .times(1)
            .returning(|_| Ok(vec![1, 2, 3]));
        
        let result = fetch_data(&mock, "test_key").await;
        assert_eq!(result.unwrap(), vec![1, 2, 3]);
    }
}
```

## Test Coverage

### Generate Coverage Report

```bash
# Install tarpaulin
cargo install cargo-tarpaulin

# Generate coverage
cargo tarpaulin --workspace --out Html --output-dir coverage

# Open report
open coverage/index.html
```

### CI Coverage

```yaml
# .github/workflows/coverage.yml
- name: Coverage
  run: |
    cargo install cargo-tarpaulin
    cargo tarpaulin --workspace --out Xml
    
- name: Upload to Codecov
  uses: codecov/codecov-action@v3
  with:
    files: ./cobertura.xml
```

## Load Testing

### Using validation/load-test

```bash
cd validation/load-test

# Run default scenario (realistic)
cargo run --release -- scenarios/realistic.yaml

# Run specific scenario
cargo run --release -- scenarios/cache_test.yaml

# Run with custom parameters
cargo run --release -- \
  --concurrent 100 \
  --duration 300 \
  --scenario scenarios/realistic.yaml
```

### Load Test Scenarios

Available scenarios in `validation/load-test/scenarios/`:

| Scenario | Description | Use Case |
|----------|-------------|----------|
| `gfs.yaml` | GFS model layers only | Baseline GFS performance |
| `hrrr.yaml` | HRRR model layers only | High-res CONUS testing |
| `goes.yaml` | GOES satellite imagery | Satellite rendering perf |
| `mrms.yaml` | MRMS radar products | Radar tile performance |
| `mixed.yaml` | All data sources combined | Realistic multi-product traffic |

```yaml
# scenarios/mixed.yaml - Realistic multi-source traffic
name: mixed
description: |
  Mixed data source load test - combines layers from all available data types.
  Tests realistic multi-product access patterns.

base_url: http://localhost:8080
duration_secs: 60
concurrency: 20
warmup_secs: 5

layers:
  # GFS (weight determines request probability)
  - name: gfs_TMP
    style: temperature
    weight: 2.0
  - name: gfs_WIND_BARBS
    style: default
    weight: 1.5

  # HRRR
  - name: hrrr_TMP
    style: temperature
    weight: 1.5

  # MRMS Radar
  - name: mrms_REFL
    style: default
    weight: 2.0

  # GOES-18 Satellite
  - name: goes18_CMI_C02
    style: default
    weight: 1.5

tile_selection:
  type: random
  zoom_range: [4, 12]
  bbox:
    min_lon: -130.0
    min_lat: 20.0
    max_lon: -60.0
    max_lat: 55.0
```

## OGC Compliance Testing

### Web-Based Compliance Test Suites

Interactive OGC compliance test suites are available through the web dashboard:

#### WMS 1.3.0 Compliance Tests

```
http://localhost:8000/wms-compliance.html
```

Tests include:
- **GetCapabilities**: Version negotiation, XML structure, layer metadata
- **GetMap**: BBOX handling, CRS axis order, FORMAT validation, STYLES parameter
- **GetFeatureInfo**: Coordinate handling, INFO_FORMAT support
- **Exceptions**: ServiceException XML format, error codes

#### WMTS 1.0.0 Compliance Tests

```
http://localhost:8000/wmts-compliance.html
```

Tests include:
- **GetCapabilities**: Contents structure, TileMatrixSet definitions
- **GetTile (KVP)**: Parameter validation, error responses
- **GetTile (RESTful)**: URL path parsing, format handling
- **TileMatrix**: Scale denominators, tile bounds

### Features

Both compliance test suites support:

- **External endpoint testing**: Point at any WMS/WMTS server for comparison
- **Per-layer validation**: Tests run against each layer in capabilities
- **Visual tile preview**: See rendered tiles alongside test results
- **Reference links**: Direct links to OGC specification sections
- **Batch execution**: Run all tests with a single click

### Capabilities Caching Test

Test that GetCapabilities responses are properly cached:

```bash
./scripts/test_capabilities_cache.sh
```

This validates that:
- Repeated requests return cached responses quickly
- Cache is invalidated after configuration changes
- Response times improve after initial request

## Performance Testing

### Criterion Benchmarks

The `renderer` crate includes Criterion benchmarks for performance-critical code:

```bash
# Run all renderer benchmarks
cargo bench -p renderer

# Run specific benchmark group
cargo bench -p renderer -- goes_pipeline
cargo bench -p renderer -- contour
cargo bench -p renderer -- barbs
```

Benchmark files in `crates/renderer/benches/`:

| File | What it benchmarks |
|------|-------------------|
| `render_benchmarks.rs` | Full tile rendering pipeline |
| `goes_benchmarks.rs` | GOES satellite tile generation |
| `contour_benchmarks.rs` | Contour line generation |
| `barbs_benchmarks.rs` | Wind barb rendering |

Results are saved to `target/criterion/` and can be compared across runs:

```bash
# Save baseline
cargo bench -p renderer -- --save-baseline before

# Make changes, then compare
cargo bench -p renderer -- --baseline before
```

### Quick Smoke Test

```bash
# Test single endpoint
curl -w "@curl-format.txt" -o /dev/null -s \
  "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=gfs_TMP_2m&STYLES=temperature&CRS=EPSG:3857&BBOX=-20037508,-20037508,20037508,20037508&WIDTH=256&HEIGHT=256&FORMAT=image/png"

# curl-format.txt:
#     time_namelookup:  %{time_namelookup}s\n
#        time_connect:  %{time_connect}s\n
#     time_appconnect:  %{time_appconnect}s\n
#    time_pretransfer:  %{time_pretransfer}s\n
#       time_redirect:  %{time_redirect}s\n
#  time_starttransfer:  %{time_starttransfer}s\n
#                     ----------\n
#          time_total:  %{time_total}s\n
```

### Apache Bench

```bash
# 1000 requests, 10 concurrent
ab -n 1000 -c 10 \
  "http://localhost:8080/tiles/gfs_TMP_2m/temperature/4/3/5.png"
```

## Continuous Integration

The project uses GitHub Actions for CI/CD with multiple workflows.

### CI Workflow (`.github/workflows/ci.yml`)

Runs on every push and pull request:

| Job | Description |
|-----|-------------|
| **Lint** | Format check (`cargo fmt`) and Clippy lints |
| **Test** | Build and run all workspace tests with result summary |
| **Coverage** | Generate per-crate coverage report with tarpaulin |
| **Docs** | Build rustdoc with `-D warnings` |

```yaml
# Example: Test job summary shows pass/fail counts
## Test Results
| Status | Count |
|--------|-------|
| Passed | 142 |
| Failed | 0 |
| Ignored | 3 |
```

Coverage reports are uploaded as artifacts and show per-crate breakdown:

```
## Code Coverage
**Overall: 73.2%**

| Crate | Coverage | Status |
|-------|----------|--------|
| renderer | 82.1% | :white_check_mark: |
| grib2-parser | 76.3% | :warning: |
| projection | 68.5% | :warning: |
```

### Benchmarks Workflow (`.github/workflows/benchmarks.yml`)

Runs when performance-critical crates change:

- **Trigger paths**: `crates/renderer/**`, `crates/grib2-parser/**`, `crates/projection/**`, `crates/wms-common/**`
- **Smart detection**: Only runs benchmarks for changed crates
- **Baseline comparison**: Compares against previous runs stored in `gh-pages` branch
- **PR comments**: Automatically posts benchmark comparison on PRs

```
## Benchmark Results (vs Previous)
| Benchmark | Current | Previous | Change |
|-----------|---------|----------|--------|
| goes_pipeline/ir_tile_256x256 | 12.34 ms | 12.56 ms | 🟢 -1.8% |
| gradient/interpolate_f32_256 | 45.2 µs | 44.8 µs | 🟡 +0.9% |
```

Legend:
- 🟢 **Faster** (>5% improvement)
- 🟡 **Similar** (within ±5%)
- 🔴 **Slower** (>5% regression)

Manual trigger: `workflow_dispatch` with `run_all: true` to run all benchmarks

### Pre-commit Hooks

Local pre-commit hooks catch issues before they reach CI:

```bash
# Install the hooks (run once)
git config core.hooksPath .githooks
```

The pre-commit hook (`.githooks/pre-commit`) runs:

1. **Format check** (`cargo fmt --check`)
2. **Trailing whitespace** detection
3. **Cargo check** for compilation errors

If any check fails, the commit is blocked with a clear error message.

## Test Utilities (`test-utils` crate)

The `test-utils` crate provides shared testing infrastructure:

### Macros

```rust
use test_utils::{require_test_file, assert_approx_eq, assert_coords_approx_eq};

#[test]
fn test_with_data_file() {
    // Skips test with message if file not found (useful for CI without large test data)
    let path = require_test_file!("gfs_sample.grib2");
    
    // Approximate floating-point comparison
    assert_approx_eq!(computed_value, expected_value, 0.001);
    
    // Coordinate pair comparison
    assert_coords_approx_eq!((lon, lat), (expected_lon, expected_lat), 0.0001);
}
```

### Grid Data Generators

```rust
use test_utils::generators;

// Generate test grid data for rendering tests
let grid = generators::create_gradient_grid(256, 256, 0.0, 100.0);
let wind_u = generators::create_wind_u_grid(256, 256);
let wind_v = generators::create_wind_v_grid(256, 256);
```

### Test File Paths

```rust
use test_utils::paths;

// Finds test files in standard locations (crate testdata/, workspace testdata/, TEST_DATA_DIR)
let path = paths::find_test_file("sample.grib2");
```

## Test Best Practices

1. **Keep tests fast**: Unit tests should run in <1s total
2. **Use meaningful names**: `test_parse_gfs_temperature` not `test1`
3. **Test edge cases**: Empty input, maximum values, invalid data
4. **One assertion per test**: Makes failures clearer
5. **Don't test implementation**: Test behavior, not internals
6. **Use fixtures**: Share test data setup
7. **Clean up**: Remove test files, reset state

## Debugging Failed Tests

```bash
# Run single test with output
cargo test test_name -- --nocapture

# Run with backtrace
RUST_BACKTRACE=1 cargo test test_name

# Run with debug logging
RUST_LOG=debug cargo test test_name

# Run in debugger
rust-lldb target/debug/deps/integration_test-<hash>
```

## Next Steps

- [Benchmarking](./benchmarking.md) - Performance testing
- [Contributing](./contributing.md) - Submit changes
- [Building](./building.md) - Build optimized binaries
