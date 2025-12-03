# Documentation Plan for Weather-WMS

## Overview

This document outlines the plan to create comprehensive, deployable documentation for the Weather-WMS system.

## Documentation Framework

**Recommended Tool: [mdBook](https://rust-lang.github.io/mdBook/)**

Why mdBook:
- Native Rust tooling (fits the project)
- Generates static HTML with navigation
- Built-in search functionality
- GitHub Pages compatible
- Markdown-based (easy to maintain)
- Supports code syntax highlighting

Alternative: MkDocs with Material theme (Python-based, more features)

## Documentation Structure

```
docs/
├── book.toml                 # mdBook configuration
├── src/
│   ├── SUMMARY.md           # Table of contents (defines navigation)
│   ├── introduction.md      # Project overview
│   │
│   ├── getting-started/
│   │   ├── README.md        # Section intro
│   │   ├── prerequisites.md # System requirements
│   │   ├── installation.md  # Docker/K8s setup
│   │   ├── quickstart.md    # 5-minute getting started
│   │   └── configuration.md # Environment variables
│   │
│   ├── architecture/
│   │   ├── README.md        # Architecture overview
│   │   ├── system-design.md # High-level design
│   │   ├── data-flow.md     # Request/ingestion flows
│   │   └── caching.md       # L1/L2 cache strategy
│   │
│   ├── services/
│   │   ├── README.md        # Services overview
│   │   ├── wms-api.md       # WMS API service
│   │   ├── ingester.md      # Ingestion service
│   │   ├── downloader.md    # Download service
│   │   └── renderer-worker.md # Render worker
│   │
│   ├── crates/
│   │   ├── README.md        # Crates overview
│   │   ├── grib2-parser.md  # GRIB2 parsing
│   │   ├── netcdf-parser.md # NetCDF/GOES parsing
│   │   ├── projection.md    # CRS transformations
│   │   ├── renderer.md      # Image rendering
│   │   ├── storage.md       # Storage abstractions
│   │   ├── wms-common.md    # Shared types
│   │   └── wms-protocol.md  # OGC protocol impl
│   │
│   ├── api-reference/
│   │   ├── README.md        # API overview
│   │   ├── wms.md           # WMS endpoints
│   │   ├── wmts.md          # WMTS endpoints
│   │   ├── rest-api.md      # REST API endpoints
│   │   └── examples.md      # curl/code examples
│   │
│   ├── data-sources/
│   │   ├── README.md        # Data sources overview
│   │   ├── gfs.md           # GFS model
│   │   ├── hrrr.md          # HRRR model
│   │   ├── mrms.md          # MRMS radar
│   │   └── goes.md          # GOES satellite
│   │
│   ├── configuration/
│   │   ├── README.md        # Config overview
│   │   ├── models.md        # Model YAML configs
│   │   ├── styles.md        # Style JSON configs
│   │   ├── parameters.md    # GRIB2 parameter tables
│   │   └── environment.md   # Environment variables
│   │
│   ├── deployment/
│   │   ├── README.md        # Deployment overview
│   │   ├── docker-compose.md # Local development
│   │   ├── kubernetes.md    # K8s deployment
│   │   ├── helm.md          # Helm chart reference
│   │   └── monitoring.md    # Grafana/Prometheus
│   │
│   ├── development/
│   │   ├── README.md        # Dev guide overview
│   │   ├── building.md      # Build instructions
│   │   ├── testing.md       # Running tests
│   │   ├── benchmarking.md  # Performance testing
│   │   └── contributing.md  # Contribution guide
│   │
│   └── reference/
│       ├── README.md        # Reference overview
│       ├── scripts.md       # Script documentation
│       ├── troubleshooting.md # Common issues
│       └── glossary.md      # Terms and acronyms
```

## Deployment Options

### Option 1: GitHub Pages (Recommended)

```yaml
# .github/workflows/docs.yml
name: Deploy Documentation

on:
  push:
    branches: [main]
    paths: ['docs/**']

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install mdBook
        run: |
          curl -sSL https://github.com/rust-lang/mdBook/releases/download/v0.4.36/mdbook-v0.4.36-x86_64-unknown-linux-gnu.tar.gz | tar -xz
          chmod +x mdbook
      - name: Build docs
        run: cd docs && ../mdbook build
      - name: Deploy to GitHub Pages
        uses: peaceiris/actions-gh-pages@v3
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: ./docs/book
```

### Option 2: Self-hosted (alongside web dashboard)

Add to existing web service or deploy as separate container:

```dockerfile
# docs/Dockerfile
FROM rust:1.75-slim as builder
RUN cargo install mdbook
WORKDIR /docs
COPY . .
RUN mdbook build

FROM nginx:alpine
COPY --from=builder /docs/book /usr/share/nginx/html
EXPOSE 80
```

### Option 3: Integrate with existing web dashboard

Serve docs from the Python web server at `/docs/` path.

## Implementation Phases

### Phase 1: Setup (Day 1)
- [ ] Install mdBook
- [ ] Create `docs/book.toml` configuration
- [ ] Create `docs/src/SUMMARY.md` structure
- [ ] Create placeholder files for all sections
- [ ] Set up GitHub Actions workflow

### Phase 2: Core Documentation (Days 2-3)
- [ ] Write introduction and getting-started guide
- [ ] Document architecture and system design
- [ ] Document all 4 services
- [ ] Document all 7 crates

### Phase 3: API & Configuration (Days 4-5)
- [ ] Write WMS/WMTS API reference with examples
- [ ] Document all configuration files
- [ ] Document environment variables
- [ ] Add curl examples and code snippets

### Phase 4: Operations (Day 6)
- [ ] Write deployment guides
- [ ] Document monitoring setup
- [ ] Create troubleshooting guide
- [ ] Document all scripts

### Phase 5: Polish (Day 7)
- [ ] Add diagrams (Mermaid supported in mdBook)
- [ ] Review and edit all content
- [ ] Add search keywords
- [ ] Test all links

## Content Guidelines

1. **Each page should include:**
   - Brief overview (1-2 sentences)
   - Key concepts/features (bullet list)
   - Configuration options (if applicable)
   - Code/command examples
   - Links to related sections

2. **Code examples should be:**
   - Copy-pasteable
   - Include expected output where helpful
   - Use realistic values

3. **Diagrams should use Mermaid:**
   ```mermaid
   graph LR
     A[Client] --> B[WMS API]
     B --> C[Cache]
     B --> D[Storage]
   ```

## Files to Create

See the detailed task list in `DOCUMENTATION_TASKS.md` for the specific content to write for each file.
