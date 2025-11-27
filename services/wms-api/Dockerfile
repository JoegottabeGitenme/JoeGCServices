# Build stage - use latest rust with bookworm base for glibc compatibility
FROM rust:bookworm AS builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services

# Build the service
RUN cargo build --release --package wms-api

# Runtime stage - same base as builder
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    netcdf-bin \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/wms-api /usr/local/bin/wms-api

# Copy config files
COPY config /app/config

# Create non-root user
RUN useradd -m -u 1000 appuser && \
    chown -R appuser:appuser /app
USER appuser

WORKDIR /app

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/wms-api"]
