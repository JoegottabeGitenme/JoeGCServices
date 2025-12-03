# Documentation Index

This document provides an overview of all documentation in the weather-wms project.

## 📚 Getting Started

**Start here:** Read one of these first depending on your role:

- **[QUICKREF.md](QUICKREF.md)** ⚡ - One-page reference with common commands
  - Best for: Quick lookups, copy-paste commands
  - Time: 5 minutes

- **[DEVELOPMENT.md](DEVELOPMENT.md)** 🛠️ - Complete development guide
  - Best for: Setting up development environment
  - Includes: Local development, testing, debugging, troubleshooting
  - Time: 20 minutes

- **[README.md](README.md)** 📖 - Project overview
  - Best for: Understanding the project purpose and architecture
  - Includes: Features, architecture diagram, supported standards
  - Time: 15 minutes

## 🎯 By Use Case

### I want to...

**...start developing immediately**
1. Read [QUICKREF.md](QUICKREF.md) (5 min)
2. Run: `docker-compose up && cargo run --bin wms-api`
3. Reference [DEVELOPMENT.md](DEVELOPMENT.md) as needed

**...deploy to Kubernetes**
1. Read [DEVELOPMENT.md - Kubernetes Deployment section](DEVELOPMENT.md#kubernetes-deployment)
2. Run: `./scripts/start.sh`
3. Monitor with: [MONITORING.md](MONITORING.md)

**...monitor running services**
- Use [MONITORING.md](MONITORING.md) - 100+ kubectl examples
- Common: `kubectl logs -n weather-wms <pod> -f`

**...contribute code**
1. Read [AGENTS.md](AGENTS.md) - Code style guidelines
2. See [DEVELOPMENT.md - Contributing](DEVELOPMENT.md#contributing)
3. Run: `cargo fmt && cargo clippy && cargo test`

**...debug a problem**
- See [DEVELOPMENT.md - Troubleshooting](DEVELOPMENT.md#troubleshooting)
- See [MONITORING.md - Troubleshooting Common Issues](MONITORING.md#troubleshooting-common-issues)
- Use: `kubectl logs` commands from [MONITORING.md](MONITORING.md)

**...set up git**
- Use [.gitignore](../.gitignore) - Already configured
- See [AGENTS.md](AGENTS.md) - Code style before committing

## 📋 Documentation Files

### [QUICKREF.md](QUICKREF.md) - Quick Reference Card
- **Size:** ~120 lines
- **Purpose:** Copy-paste commands and quick lookups
- **Includes:**
  - Common build/test commands
  - Running services locally vs Kubernetes
  - Service credentials
  - Common issues & solutions
  - File structure overview
  
### [DEVELOPMENT.md](DEVELOPMENT.md) - Development Guide
- **Size:** ~180 lines
- **Purpose:** Complete development workflow guide
- **Includes:**
  - Prerequisites and setup
  - Local development with docker-compose
  - Kubernetes deployment
  - Common development tasks
  - Configuration (PostgreSQL, Redis, MinIO)
  - Troubleshooting
  - Contributing guidelines

### [MONITORING.md](MONITORING.md) - Kubernetes Monitoring
- **Size:** ~250 lines
- **Purpose:** kubectl reference and monitoring guide
- **Includes:**
  - 100+ real kubectl commands
  - Status checks and live watching
  - Viewing logs and debugging
  - Service access and port-forwarding
  - Troubleshooting common issues
  - Metrics and performance monitoring
  - YAML export and backup
  
### [AGENTS.md](AGENTS.md) - Code Style & Build Commands
- **Size:** ~60 lines
- **Purpose:** Build, test, and code style guidelines
- **Includes:**
  - Cargo build/test/lint commands
  - Code style guidelines (imports, formatting, naming, error handling)
  - Type system conventions
  - Documentation standards
  - Testing patterns
  - Async/concurrency guidelines

### [README.md](README.md) - Project Overview
- **Size:** ~250 lines
- **Purpose:** Project description, features, and architecture
- **Includes:**
  - Project overview
  - Architecture diagram
  - Quick start guide
  - Project structure
  - Supported data sources
  - WMS/WMTS capabilities
  - Style configuration
  - Configuration reference
  - Development info
  - OGC CITE testing info

### [docker-compose.yml](../docker-compose.yml) - Local Dev Stack
- **Purpose:** Local development without Kubernetes
- **Includes:**
  - PostgreSQL database
  - Redis cache
  - MinIO object storage
  - Health checks for all services

### [.gitignore](../.gitignore) - Git Configuration
- **Size:** ~70 lines
- **Purpose:** Ignore build artifacts and development files
- **Includes:**
  - Rust build artifacts
  - IDE/editor files
  - OS files
  - Docker and Kubernetes files
  - Logs and temporary files
  - Python and Node artifacts

## 🔗 Documentation Relationships

```
README.md (Project Overview)
    ↓
QUICKREF.md (Quick Start) OR DEVELOPMENT.md (Full Guide)
    ↓
Choose your path:
├─→ Local Dev: docker-compose.yml + cargo commands
├─→ Kubernetes: ./scripts/start.sh + MONITORING.md
└─→ Code Changes: AGENTS.md + contributing section
```

## 🛠️ Development Workflow

1. **Setup**: Read [DEVELOPMENT.md](DEVELOPMENT.md)
2. **Daily work**: Reference [QUICKREF.md](QUICKREF.md)
3. **Before commit**: Follow [AGENTS.md](AGENTS.md)
4. **Monitor services**: Use [MONITORING.md](MONITORING.md)
5. **Troubleshoot**: Check [DEVELOPMENT.md](DEVELOPMENT.md) troubleshooting

## 📊 Project Structure

```
weather-wms/
├── crates/                  # Shared libraries
│   ├── wms-common/         # Core types
│   ├── wms-protocol/       # OGC standards
│   ├── grib2-parser/       # Data parsing
│   ├── renderer/           # Image rendering
│   ├── projection/         # CRS transforms
│   ├── storage/            # DB/cache/S3
│   └── netcdf-parser/      # Data parsing
├── services/               # Deployable services
│   ├── wms-api/           # HTTP server
│   ├── ingester/          # Data import
│   └── renderer-worker/   # Tile rendering
├── deploy/                 # Kubernetes manifests
├── scripts/                # Automation
├── docs/                   # Documentation (this folder)
├── Cargo.toml             # Workspace config
├── docker-compose.yml     # Local dev stack
└── README.md              # Project overview
```

## 🚀 Quick Start Examples

### Local Development (Recommended)
```bash
# Terminal 1: Start services
docker-compose up

# Terminal 2: Run API
cargo run --bin wms-api

# Terminal 3: Run tests
cargo test
```

### Kubernetes Deployment
```bash
./scripts/start.sh
kubectl get pods -n weather-wms -w
kubectl logs -n weather-wms <pod-name> -f
```

### Before Committing Code
```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

## 📞 Where to Find Answers

| Question | Answer Location |
|----------|-----------------|
| How do I build/test? | [QUICKREF.md](QUICKREF.md) or [AGENTS.md](AGENTS.md) |
| How do I set up locally? | [DEVELOPMENT.md](DEVELOPMENT.md) |
| How do I use Kubernetes? | [DEVELOPMENT.md](DEVELOPMENT.md) + [MONITORING.md](MONITORING.md) |
| What code style do I follow? | [AGENTS.md](AGENTS.md) |
| How do I monitor services? | [MONITORING.md](MONITORING.md) |
| What's the project about? | [README.md](README.md) |
| I need a quick command | [QUICKREF.md](QUICKREF.md) |
| Something's broken | [DEVELOPMENT.md](DEVELOPMENT.md) or [MONITORING.md](MONITORING.md) |

## 📝 Notes

- **All commands assume** you're in the project root directory
- **Kubernetes commands** require: minikube, kubectl, helm
- **Local development** requires: Docker, Docker Compose, Rust
- **File sizes** are approximate lines of content
- **Read time** estimates assume moderate reading speed with code examples

## Last Updated

This documentation index was created to provide a comprehensive guide to the weather-wms project. All files are actively maintained and represent the current project state.

For the latest changes, see individual documentation files.
