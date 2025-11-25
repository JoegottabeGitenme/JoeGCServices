# Build stage
FROM rust:1.75-bookworm as builder

WORKDIR /app

# Create a dummy project to cache dependencies
RUN cargo new --bin dummy
WORKDIR /app/dummy

# Copy workspace Cargo files
COPY Cargo.toml Cargo.lock /app/

# Copy all crate Cargo.toml files to cache dependencies
COPY crates/wms-common/Cargo.toml /app/crates/wms-common/
COPY crates/grib2-parser/Cargo.toml /app/crates/grib2-parser/
COPY crates/netcdf-parser/Cargo.toml /app/crates/netcdf-parser/
COPY crates/projection/Cargo.toml /app/crates/projection/
COPY crates/renderer/Cargo.toml /app/crates/renderer/
COPY crates/wms-protocol/Cargo.toml /app/crates/wms-protocol/
COPY crates/storage/Cargo.toml /app/crates/storage/
COPY services/ingester/Cargo.toml /app/services/ingester/
COPY services/renderer-worker/Cargo.toml /app/services/renderer-worker/
COPY services/wms-api/Cargo.toml /app/services/wms-api/

# Create dummy lib.rs files
RUN mkdir -p /app/crates/wms-common/src && echo "pub fn dummy() {}" > /app/crates/wms-common/src/lib.rs
RUN mkdir -p /app/crates/grib2-parser/src && echo "pub fn dummy() {}" > /app/crates/grib2-parser/src/lib.rs
RUN mkdir -p /app/crates/netcdf-parser/src && echo "pub fn dummy() {}" > /app/crates/netcdf-parser/src/lib.rs
RUN mkdir -p /app/crates/projection/src && echo "pub fn dummy() {}" > /app/crates/projection/src/lib.rs
RUN mkdir -p /app/crates/renderer/src && echo "pub fn dummy() {}" > /app/crates/renderer/src/lib.rs
RUN mkdir -p /app/crates/wms-protocol/src && echo "pub fn dummy() {}" > /app/crates/wms-protocol/src/lib.rs
RUN mkdir -p /app/crates/storage/src && echo "pub fn dummy() {}" > /app/crates/storage/src/lib.rs
RUN mkdir -p /app/services/ingester/src && echo "fn main() {}" > /app/services/ingester/src/main.rs
RUN mkdir -p /app/services/renderer-worker/src && echo "fn main() {}" > /app/services/renderer-worker/src/main.rs
RUN mkdir -p /app/services/wms-api/src && echo "fn main() {}" > /app/services/wms-api/src/main.rs

# Build dependencies only
WORKDIR /app
RUN cargo build --release --package ingester 2>/dev/null || true

# Now copy actual source code
COPY crates /app/crates
COPY services /app/services

# Touch all source files to invalidate cache
RUN find /app -name "*.rs" -exec touch {} \;

# Build the actual service
RUN cargo build --release --package ingester

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ingester /usr/local/bin/ingester

# Create non-root user
RUN useradd -m -u 1000 appuser
USER appuser

ENTRYPOINT ["/usr/local/bin/ingester"]
