# syntax=docker/dockerfile:1.4
#
# Ingester Service Dockerfile
#
# Optimizations:
# - cargo-chef for dependency caching (only rebuilds when Cargo.toml changes)
# - mold linker for 5-10x faster linking
# - BuildKit cache mounts for cargo registry/git
# - Multi-stage build for minimal runtime image
#
# Build:
#   docker build -f services/ingester/Dockerfile -t ingester .
#   docker build -f services/ingester/Dockerfile --build-arg CARGO_PROFILE=dev -t ingester:dev .

# ============================================================================
# Stage 1: Generate dependency recipe
# ============================================================================
FROM rust:bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services
COPY validation ./validation
RUN cargo chef prepare --recipe-path recipe.json

# ============================================================================
# Stage 2: Build dependencies (heavily cached)
# ============================================================================
FROM chef AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    libhdf5-dev \
    libnetcdf-dev \
    pkg-config \
    mold \
    clang \
    && rm -rf /var/lib/apt/lists/*

# Configure mold linker (5-10x faster than default ld)
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=clang
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"

# Build profile
ARG CARGO_PROFILE=release

# Copy recipe and cook dependencies (cached unless Cargo.toml changes)
COPY --from=planner /app/recipe.json recipe.json

# Use BuildKit cache mounts for cargo registry and git
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    if [ "$CARGO_PROFILE" = "dev" ]; then \
        cargo chef cook --recipe-path recipe.json; \
    else \
        cargo chef cook --release --recipe-path recipe.json; \
    fi

# ============================================================================
# Stage 3: Build application
# ============================================================================
# Copy source and build (only this layer rebuilds on code changes)
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services
COPY validation ./validation

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    if [ "$CARGO_PROFILE" = "dev" ]; then \
        cargo build --package ingester --locked && \
        cp target/debug/ingester /app/ingester-binary; \
    else \
        cargo build --release --package ingester --locked && \
        cp target/release/ingester /app/ingester-binary; \
    fi

# ============================================================================
# Stage 4: Runtime image (minimal)
# ============================================================================
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    libhdf5-103 \
    libnetcdf19 \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /app/ingester-binary /usr/local/bin/ingester

# Create non-root user
RUN useradd -m -u 1000 appuser
USER appuser

EXPOSE 8082

ENTRYPOINT ["/usr/local/bin/ingester"]
