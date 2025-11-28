# AGENTS.md

## Build, Lint, and Test Commands

**Build all crates:**
```bash
cargo build
```

**Run all tests:**
```bash
cargo test
```

**Run tests for a specific crate:**
```bash
cargo test --package wms-common
cargo test --package grib2-parser
# etc. for any crate in crates/ or services/
```

**Run a single test by name:**
```bash
cargo test test_name -- --exact
```

**Run tests with output:**
```bash
cargo test -- --nocapture
```

**Build a specific service:**
```bash
cargo build --package wms-api
```

**Check code without building:**
```bash
cargo check
```

**Format code:**
```bash
cargo fmt
```

**Run clippy linter:**
```bash
cargo clippy -- -D warnings
```

## Code Style Guidelines

### Imports
- Organize imports in three groups (separated by blank lines): std library, external crates, local modules
- Use `use` statements for public APIs, not glob imports in library code
- Prefer aliasing with `as` for disambiguation

### Formatting
- Use 4-space indentation (configured by Cargo default)
- Run `cargo fmt` before committing
- Wrap lines at reasonable lengths for readability

### Type System
- Prefer explicit type annotations in function signatures
- Use type aliases for complex types (e.g., `WmsResult<T> = Result<T, WmsError>`)
- Leverage Rust's type system to prevent errors at compile time

### Naming Conventions
- **Crate names:** lowercase with hyphens (e.g., `wms-common`, `grib2-parser`)
- **Module/function names:** snake_case
- **Type/trait names:** PascalCase
- **Constants:** SCREAMING_SNAKE_CASE

### Error Handling
- Always return `Result<T, E>` from fallible operations
- Use custom error types with `thiserror` derive macro for domain-specific errors
- Implement `From` impls for automatic error conversion
- Map WMS errors to OGC exception codes and HTTP status codes in `WmsError`
- Use `WmsResult<T>` alias for cleaner function signatures

### Documentation
- Add doc comments (`///`) for all public items
- Use module-level doc comments (`//!`) explaining module purpose
- Include examples in doc comments where helpful

### Testing
- Write unit tests in the same file as code using `#[cfg(test)]` modules
- Name test functions descriptively (e.g., `test_invalid_bbox_returns_error`)
- Use `tokio::test` for async test functions
- Test both success and error paths

### Async/Concurrency
- Use `tokio` as the async runtime (configured workspace-wide)
- Mark async functions with `#[tokio::main]` or `#[tokio::test]`
- Use `Arc` for shared state across async tasks
- Prefer `tracing` over println! for logging

### Dependencies
- Keep workspace dependencies synchronized in `Cargo.toml [workspace.dependencies]`
- Use workspace inheritance for common crates (tokio, serde, axum, etc.)
- Avoid duplicate dependency versions across the workspace

## Performance Testing and Load Tests

**Run a quick smoke test:**
```bash
./scripts/run_load_test.sh
# or
./scripts/run_load_test.sh quick
```

**Run specific test scenarios:**
```bash
# Test cache-miss performance
./scripts/run_load_test.sh cold_cache

# Test cache-hit performance (requires cache warming first)
./scripts/run_load_test.sh warm_cache

# High concurrency stress test
./scripts/run_load_test.sh stress

# Compare different layer types
./scripts/run_load_test.sh layer_comparison
```

**Save results for tracking over time:**
```bash
# Save as JSON
./scripts/run_load_test.sh cold_cache --save --output json

# Append to CSV for historical tracking
./scripts/run_load_test.sh cold_cache --save --output csv
```

**Reset cache before testing:**
```bash
./scripts/run_load_test.sh cold_cache --reset-cache
```

**Run custom scenario:**
```bash
./scripts/run_load_test.sh --scenario my_custom_test.yaml
```

**Direct use of load-test tool:**
```bash
# Build the tool
cargo build --package load-test --release

# Run a scenario
./target/release/load-test run --scenario validation/load-test/scenarios/quick.yaml

# Quick test with defaults
./target/release/load-test quick --layer gfs_TMP --requests 100

# List available scenarios
./target/release/load-test list
```

**Reset system state for consistent benchmarking:**
```bash
# Flush Redis cache and reset metrics
./scripts/reset_test_state.sh

# Also restart WMS API to clear in-memory state
./scripts/reset_test_state.sh --restart
```

**Create custom test scenarios:**

Create a YAML file in `validation/load-test/scenarios/`:
```yaml
name: my_test
description: Custom test scenario
base_url: http://localhost:8080
duration_secs: 60
concurrency: 20
warmup_secs: 5
layers:
  - name: gfs_TMP
    style: temperature
    weight: 1.0
  - name: gfs_WIND_BARBS
    weight: 0.5
tile_selection:
  type: random
  zoom_range: [4, 8]
  bbox:
    min_lon: -130
    min_lat: 20
    max_lon: -60
    max_lat: 55
```
