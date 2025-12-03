# Building from Source

Instructions for building Weather WMS from source code.

## Prerequisites

- **Rust**: 1.75 or later
- **Cargo**: Included with Rust
- **System Dependencies**: 
  - Linux: `build-essential`, `pkg-config`, `libssl-dev`
  - macOS: Xcode Command Line Tools
  - Windows: Visual Studio Build Tools

## Quick Build

```bash
# Clone repository
git clone https://github.com/JoegottabeGitenme/JoeGCServices.git
cd JoeGCServices

# Build all workspace members
cargo build --release

# Binaries will be in target/release/
ls -lh target/release/wms-api
ls -lh target/release/ingester
ls -lh target/release/downloader
ls -lh target/release/renderer-worker
```

## Build Modes

### Debug Build (Development)

```bash
cargo build

# Output: target/debug/
# - Faster compilation
# - Includes debug symbols
# - No optimizations
# - Slower runtime
```

### Release Build (Production)

```bash
cargo build --release

# Output: target/release/
# - Slower compilation
# - Optimized for performance
# - Smaller binaries
# - Fast runtime
```

### Profile-Guided Optimization (PGO)

```bash
# 1. Build instrumented binary
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" cargo build --release

# 2. Run with representative workload
./target/release/wms-api  # Generate profile data

# 3. Build optimized binary using profile
RUSTFLAGS="-Cprofile-use=/tmp/pgo-data" cargo build --release
```

## Build Individual Components

### Single Service

```bash
# Build only WMS API
cargo build --release --bin wms-api

# Build only ingester
cargo build --release --bin ingester
```

### Single Crate

```bash
# Build only grib2-parser library
cargo build --release -p grib2-parser

# Build with examples
cargo build --release -p grib2-parser --examples
```

## Build Configuration

### Optimization Levels

```toml
# Cargo.toml
[profile.release]
opt-level = 3        # Maximum optimization
lto = true           # Link-time optimization
codegen-units = 1    # Better optimization, slower build
strip = true         # Strip symbols (smaller binary)
```

### Custom Profile

```toml
# Cargo.toml
[profile.production]
inherits = "release"
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"      # Smaller binary, no stack unwinding
```

Build with custom profile:
```bash
cargo build --profile production
```

## Cross-Compilation

### Linux → Linux (different arch)

```bash
# Install target
rustup target add x86_64-unknown-linux-musl

# Build
cargo build --release --target x86_64-unknown-linux-musl

# Output: target/x86_64-unknown-linux-musl/release/
```

### macOS → Linux

```bash
# Install cross-compilation toolchain
brew install filosottile/musl-cross/musl-cross

# Add target
rustup target add x86_64-unknown-linux-musl

# Build
cargo build --release --target x86_64-unknown-linux-musl
```

### Using Docker for Reproducible Builds

```bash
# Build in Docker container
docker run --rm \
  -v "$PWD":/workspace \
  -w /workspace \
  rust:1.75 \
  cargo build --release

# Or use multi-stage Dockerfile
docker build -t weather-wms:latest -f services/wms-api/Dockerfile .
```

## Build Times

Approximate build times on modern hardware (16-core, SSD):

| Build Type | Time (clean) | Time (incremental) |
|------------|--------------|---------------------|
| Debug | 3 min | 10s |
| Release | 12 min | 45s |
| Release + LTO | 20 min | N/A |

### Speed Up Builds

#### Use Faster Linker

```bash
# Install mold (Linux)
sudo apt install mold

# Use in builds
RUSTFLAGS="-C link-arg=-fuse-ld=mold" cargo build --release
```

#### Enable Parallel Compilation

```toml
# .cargo/config.toml
[build]
jobs = 16  # Number of parallel jobs
```

#### Use sccache

```bash
# Install sccache
cargo install sccache

# Configure
export RUSTC_WRAPPER=sccache

# Build (will cache compilation artifacts)
cargo build --release
```

## Binary Sizes

Typical release binary sizes:

| Service | Size (default) | Size (stripped + LTO) |
|---------|----------------|----------------------|
| wms-api | 45 MB | 28 MB |
| ingester | 38 MB | 24 MB |
| downloader | 32 MB | 20 MB |
| renderer-worker | 35 MB | 22 MB |

### Reduce Binary Size

```toml
# Cargo.toml
[profile.release]
strip = true
lto = true
opt-level = "z"  # Optimize for size
panic = "abort"
```

## Build Artifacts

After building, you'll have:

```
target/release/
├── wms-api              # HTTP API server
├── ingester             # Ingestion binary
├── downloader           # Download binary
├── renderer-worker      # Renderer binary
├── libgrib2_parser.so   # Shared libraries (if built)
└── deps/                # Dependencies
```

## Troubleshooting

### Compilation Errors

**Error: "linker `cc` not found"**

```bash
# Ubuntu/Debian
sudo apt-get install build-essential

# macOS
xcode-select --install

# Windows
# Install Visual Studio Build Tools
```

**Error: "could not find native library `ssl`"**

```bash
# Ubuntu/Debian
sudo apt-get install pkg-config libssl-dev

# macOS (should be included)
# Windows - use vcpkg
```

### Out of Memory

```bash
# Reduce parallel jobs
cargo build --release -j 4

# Or increase swap space
sudo fallocate -l 8G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
```

### Slow Builds

```bash
# Check what's taking time
cargo build --release --timings

# Open target/cargo-timings/cargo-timing.html
```

## CI/CD Build

Example GitHub Actions workflow:

```yaml
name: Build
on: [push, pull_request]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
    - uses: actions/checkout@v4
    
    - name: Install Rust
      uses: actions-rs/toolchain@v1
      with:
        toolchain: stable
        override: true
    
    - name: Cache cargo
      uses: actions/cache@v3
      with:
        path: |
          ~/.cargo/registry
          ~/.cargo/git
          target
        key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
    
    - name: Build
      run: cargo build --release --workspace
    
    - name: Upload artifacts
      uses: actions/upload-artifact@v3
      with:
        name: binaries
        path: target/release/wms-api
```

## Next Steps

- [Testing](./testing.md) - Run tests after building
- [Benchmarking](./benchmarking.md) - Performance testing
- [Deployment](../deployment/README.md) - Deploy built binaries
