# Shared builder base image for all Rust services
# This image contains compiled dependencies that can be reused across services
#
# Build this image first:
#   docker build -f Dockerfile.builder -t weather-builder:latest .
#
# Or let docker compose build it automatically (see docker-compose.yml)

FROM rust:bookworm AS chef

# Install cargo-chef for better dependency caching
RUN cargo install cargo-chef --locked

# Install mold linker for faster linking (5-10x faster than default ld)
RUN apt-get update && apt-get install -y \
    mold \
    clang \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# ============================================================================
# Stage 1: Generate recipe (dependency list)
# ============================================================================
FROM chef AS planner

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services
COPY validation ./validation

# Generate recipe.json (list of all dependencies)
RUN cargo chef prepare --recipe-path recipe.json

# ============================================================================
# Stage 2: Build dependencies (cached layer)
# ============================================================================
FROM chef AS builder

# Install build dependencies for netcdf-parser
RUN apt-get update && apt-get install -y \
    libhdf5-dev \
    libnetcdf-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Configure mold as the default linker
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=clang
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"

# Copy recipe from planner
COPY --from=planner /app/recipe.json recipe.json

# Build profile argument
ARG CARGO_PROFILE=release

# Cook dependencies (this is the cached layer)
# Using --release or not based on CARGO_PROFILE
RUN if [ "$CARGO_PROFILE" = "dev" ]; then \
        cargo chef cook --recipe-path recipe.json; \
    else \
        cargo chef cook --release --recipe-path recipe.json; \
    fi

# ============================================================================
# This image now has all dependencies compiled.
# Individual service Dockerfiles can use this as their base.
# ============================================================================
