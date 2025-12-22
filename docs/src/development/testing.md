# Testing

Comprehensive testing strategy for Weather WMS covering unit tests, integration tests, and validation.

## Test Organization

```
crates/
├── grib2-parser/
│   ├── src/
│   │   └── lib.rs           # #[cfg(test)] mod tests
│   └── tests/
│       ├── data/            # Test data files
│       └── integration.rs   # Integration tests
services/
├── wms-api/
│   ├── src/
│   │   └── handlers/        # Handler modules with tests
│   └── tests/
│       └── api_tests.rs
validation/
└── load-test/               # Load testing scenarios
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

# Benchmarks
cargo test --benches
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

```yaml
# scenarios/realistic.yaml
name: "Realistic User Traffic"
duration: 300  # seconds
concurrent_users: 50

requests:
  - name: "GetMap Temperature"
    weight: 40
    endpoint: "/wms"
    params:
      SERVICE: WMS
      VERSION: "1.3.0"
      REQUEST: GetMap
      LAYERS: gfs_TMP_2m
      STYLES: temperature
      CRS: EPSG:3857
      BBOX: "-20037508,-20037508,20037508,20037508"
      WIDTH: 256
      HEIGHT: 256
      FORMAT: image/png
  
  - name: "GetCapabilities"
    weight: 10
    endpoint: "/wms"
    params:
      SERVICE: WMS
      REQUEST: GetCapabilities
      VERSION: "1.3.0"
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

### GitHub Actions

```yaml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    
    services:
      postgres:
        image: postgres:15
        env:
          POSTGRES_PASSWORD: postgres
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
      
      redis:
        image: redis:7
        options: >-
          --health-cmd "redis-cli ping"
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
    
    steps:
    - uses: actions/checkout@v4
    
    - name: Install Rust
      uses: actions-rs/toolchain@v1
      with:
        toolchain: stable
        override: true
    
    - name: Run tests
      run: cargo test --workspace
      env:
        DATABASE_URL: postgresql://postgres:postgres@localhost:5432/test
        REDIS_URL: redis://localhost:6379
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
