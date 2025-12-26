# Build stage
FROM rust:bookworm AS builder

WORKDIR /app

# Copy only dependency files first (for better layer caching)
COPY Cargo.toml Cargo.lock ./
COPY crates/grib2-parser/Cargo.toml crates/grib2-parser/Cargo.toml
COPY crates/grid-processor/Cargo.toml crates/grid-processor/Cargo.toml
COPY crates/ingestion/Cargo.toml crates/ingestion/Cargo.toml
COPY crates/netcdf-parser/Cargo.toml crates/netcdf-parser/Cargo.toml
COPY crates/projection/Cargo.toml crates/projection/Cargo.toml
COPY crates/renderer/Cargo.toml crates/renderer/Cargo.toml
COPY crates/storage/Cargo.toml crates/storage/Cargo.toml
COPY crates/test-utils/Cargo.toml crates/test-utils/Cargo.toml
COPY crates/wms-common/Cargo.toml crates/wms-common/Cargo.toml
COPY crates/wms-protocol/Cargo.toml crates/wms-protocol/Cargo.toml
COPY services/wms-api/Cargo.toml services/wms-api/Cargo.toml
COPY services/ingester/Cargo.toml services/ingester/Cargo.toml
COPY services/downloader/Cargo.toml services/downloader/Cargo.toml
COPY validation/load-test/Cargo.toml validation/load-test/Cargo.toml

# Create dummy source files to build dependencies (cached layer)
RUN mkdir -p crates/grib2-parser/src && echo "fn main() {}" > crates/grib2-parser/src/lib.rs && \
    mkdir -p crates/grid-processor/src && echo "fn main() {}" > crates/grid-processor/src/lib.rs && \
    mkdir -p crates/ingestion/src && echo "fn main() {}" > crates/ingestion/src/lib.rs && \
    mkdir -p crates/netcdf-parser/src && echo "fn main() {}" > crates/netcdf-parser/src/lib.rs && \
    mkdir -p crates/projection/src && echo "fn main() {}" > crates/projection/src/lib.rs && \
    mkdir -p crates/renderer/src && echo "fn main() {}" > crates/renderer/src/lib.rs && \
    mkdir -p crates/storage/src && echo "fn main() {}" > crates/storage/src/lib.rs && \
    mkdir -p crates/test-utils/src && echo "fn main() {}" > crates/test-utils/src/lib.rs && \
    mkdir -p crates/wms-common/src && echo "fn main() {}" > crates/wms-common/src/lib.rs && \
    mkdir -p crates/wms-protocol/src && echo "fn main() {}" > crates/wms-protocol/src/lib.rs && \
    mkdir -p services/wms-api/src && echo "fn main() {}" > services/wms-api/src/main.rs && \
    mkdir -p services/ingester/src && echo "fn main() {}" > services/ingester/src/main.rs && \
    mkdir -p services/downloader/src && echo "fn main() {}" > services/downloader/src/main.rs && \
    mkdir -p validation/load-test/src && echo "fn main() {}" > validation/load-test/src/lib.rs

# Build profile: use CARGO_PROFILE env var (default: release for smaller images)
# Set CARGO_PROFILE=dev for faster builds during development
ARG CARGO_PROFILE=release

# Pre-build dependencies (this layer is cached unless Cargo.toml changes)
RUN if [ "$CARGO_PROFILE" = "dev" ]; then \
        cargo build --package downloader; \
    else \
        cargo build --release --package downloader; \
    fi || true

# Now copy the actual source code
COPY crates ./crates
COPY services ./services
COPY validation ./validation

# Touch the main files to ensure they're rebuilt
RUN find services/downloader/src -name "*.rs" -exec touch {} \; && \
    find crates -name "*.rs" -exec touch {} \;

# Build the service with the selected profile
RUN if [ "$CARGO_PROFILE" = "dev" ]; then \
        cargo build --package downloader && \
        cp target/debug/downloader /app/downloader-binary; \
    else \
        cargo build --release --package downloader && \
        cp target/release/downloader /app/downloader-binary; \
    fi

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/downloader-binary /usr/local/bin/downloader

# Create non-root user
RUN useradd -m -u 1000 appuser

# Create directories for downloads and state
RUN mkdir -p /data/downloads /data/downloader /tmp/weather-downloads && \
    chown -R appuser:appuser /data /tmp/weather-downloads

USER appuser

ENTRYPOINT ["/usr/local/bin/downloader"]
