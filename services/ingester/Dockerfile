# Build stage - use bookworm base for glibc compatibility
FROM rust:bookworm AS builder

# Install build dependencies for netcdf-parser (needs libhdf5-dev and libnetcdf-dev)
RUN apt-get update && apt-get install -y \
    libhdf5-dev \
    libnetcdf-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services
COPY validation ./validation

# Build the service
RUN cargo build --release --package ingester

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    libhdf5-103 \
    libnetcdf19 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ingester /usr/local/bin/ingester

# Create non-root user
RUN useradd -m -u 1000 appuser
USER appuser

EXPOSE 8082

ENTRYPOINT ["/usr/local/bin/ingester"]
