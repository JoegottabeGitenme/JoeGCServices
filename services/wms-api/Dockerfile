# Build stage
FROM rust:latest AS builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services

# Build the service
RUN cargo build --release --package wms-api

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/wms-api /usr/local/bin/wms-api

# Create non-root user
RUN useradd -m -u 1000 appuser
USER appuser

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/wms-api"]
