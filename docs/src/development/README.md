# Development Guide

Guide for developers contributing to Weather WMS or building custom extensions.

## Quick Start for Developers

```bash
# Clone repository
git clone https://github.com/JoegottabeGitenme/JoeGCServices.git
cd JoeGCServices

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build all services
cargo build --release

# Run tests
cargo test --workspace

# Run a single service
cargo run --bin wms-api

# Or use Docker Compose for full stack
./scripts/start.sh
```

## Development Environment

### Prerequisites

- **Rust**: 1.75+ (install via [rustup](https://rustup.rs/))
- **Cargo**: Included with Rust
- **Docker**: For running dependencies (PostgreSQL, Redis, MinIO)
- **wgrib2**: Optional, for inspecting GRIB2 files
- **ncdump**: Optional, for inspecting NetCDF files

### Recommended Tools

```bash
# Rust development tools
cargo install cargo-watch    # Auto-rebuild on file changes
cargo install cargo-edit      # Manage dependencies
cargo install cargo-audit     # Security audits
cargo install cargo-tarpaulin # Code coverage

# Code formatting
rustup component add rustfmt

# Linting
rustup component add clippy
```

## Project Structure

```
weather-wms/
├── crates/              # Reusable libraries
│   ├── grib2-parser/    # GRIB2 parsing
│   ├── netcdf-parser/   # NetCDF parsing
│   ├── projection/      # CRS transformations
│   ├── renderer/        # Weather visualization
│   ├── storage/         # Storage abstractions
│   ├── wms-common/      # Shared types
│   └── wms-protocol/    # OGC protocols
├── services/            # Microservices
│   ├── wms-api/         # HTTP API server
│   ├── ingester/        # Data ingestion
│   ├── downloader/      # Data download
├── validation/          # Testing and validation
│   └── load-test/       # Load testing suite
├── config/              # Configuration files
│   ├── models/          # Model definitions
│   ├── styles/          # Visualization styles
│   └── parameters/      # GRIB2 parameter tables
├── scripts/             # Utility scripts
└── docs/                # Documentation (mdBook)
```

## Workflow

### 1. Development Loop

```bash
# Terminal 1: Auto-rebuild on changes
cargo watch -x 'build --bin wms-api'

# Terminal 2: Run tests on changes
cargo watch -x test

# Terminal 3: Run service
cargo run --bin wms-api
```

### 2. Code Quality

```bash
# Format code
cargo fmt --all

# Run linter
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Check for security issues
cargo audit
```

### 3. Testing

```bash
# Run all tests
cargo test --workspace

# Run tests for specific crate
cargo test -p grib2-parser

# Run with output
cargo test -- --nocapture

# Run ignored tests (integration tests with test data)
cargo test -- --ignored
```

## Development Sections

- [Building](./building.md) - Build instructions and optimization
- [Testing](./testing.md) - Testing strategy and running tests
- [Benchmarking](./benchmarking.md) - Performance testing and profiling
- [Contributing](./contributing.md) - Contribution guidelines

## Common Tasks

### Adding a New Parameter

1. Add to parameter table (`config/parameters/grib2_ncep.yaml`)
2. Add style configuration (`config/styles/new_param.json`)
3. Update ingester to handle parameter
4. Test with real data

### Adding a New Data Source

1. Create model config (`config/models/new_model.yaml`)
2. Implement parser in ingester
3. Add download script (`scripts/download_new_model.sh`)
4. Test ingestion pipeline
5. Add documentation

### Optimizing Performance

1. Profile with `cargo flamegraph`
2. Identify bottlenecks
3. Add benchmarks in `benches/`
4. Implement optimization
5. Verify with benchmarks

## Debugging

### Enable Debug Logging

```bash
# Environment variable
RUST_LOG=debug cargo run --bin wms-api

# Or in .env
RUST_LOG=debug
RUST_BACKTRACE=1
```

### Debug Specific Modules

```bash
# Only log from specific module
RUST_LOG=wms_api::handlers=debug cargo run --bin wms-api

# Multiple modules
RUST_LOG=wms_api::handlers=debug,storage=trace cargo run --bin wms-api
```

### Using Rust Debugger

```bash
# Install lldb (macOS) or gdb (Linux)
# Then use with cargo
rust-lldb target/debug/wms-api

# Or with VSCode + rust-analyzer
# Add breakpoints and press F5
```

## Code Style

### Rust Conventions

- Use `rustfmt` for formatting (automatic with `cargo fmt`)
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Write documentation comments for public APIs
- Keep functions small and focused
- Prefer explicit error handling with `Result`

### Example

```rust
/// Parses a GRIB2 file and extracts all messages.
///
/// # Arguments
///
/// * `path` - Path to GRIB2 file
///
/// # Returns
///
/// Vector of parsed messages
///
/// # Errors
///
/// Returns error if file cannot be read or parsed
///
/// # Example
///
/// ```
/// use grib2_parser::parse_file;
///
/// let messages = parse_file("gfs.grib2")?;
/// println!("Found {} messages", messages.len());
/// ```
pub fn parse_file(path: &Path) -> Result<Vec<Grib2Message>, Grib2Error> {
    let data = std::fs::read(path)?;
    parse_bytes(&data)
}
```

## Getting Help

- **GitHub Issues**: Report bugs or request features
- **Discussions**: Ask questions or share ideas
- **Documentation**: This documentation site
- **Code Comments**: Inline documentation in source code

## Next Steps

- [Building](./building.md) - Build from source
- [Testing](./testing.md) - Run tests
- [Contributing](./contributing.md) - Submit changes
