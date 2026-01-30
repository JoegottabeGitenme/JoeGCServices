#!/usr/bin/env bash
#
# Weather WMS - Local Development Start Script
#
# This script sets up a complete local development environment using docker-compose.
#
# Usage:
#   ./start.sh              # Start with docker-compose (fast)
#   ./start.sh --compose    # Start with docker-compose (same as above)
#   ./start.sh --dev        # Development mode: debug builds (MUCH faster compilation)
#   ./start.sh --rebuild    # Force rebuild of Docker images (release)
#   ./start.sh --rebuild-dev # Force rebuild with debug profile (faster)
#   ./start.sh --clear-cache # Clear Redis tile cache (after rendering changes)
#   ./start.sh --stop       # Stop docker-compose
#   ./start.sh --clean      # Delete everything and start fresh
#   ./start.sh --status     # Show status
#   ./start.sh --help       # Show this help message
#
# On startup, the system will:
#   1. Start all Docker containers (PostgreSQL, Redis, MinIO, WMS API, Dashboard, Downloader)
#   2. Wait for services to be ready
#   3. Display dashboard at http://localhost:8000
#
# The downloader service will automatically fetch new weather data.
# Existing data in the system will be preserved.
# To manually ingest test data, run: ./scripts/ingest_test_data.sh
#

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

#------------------------------------------------------------------------------
# Load Environment Configuration
#------------------------------------------------------------------------------

load_env_file() {
  if [ -f "$PROJECT_ROOT/.env" ]; then
    log_info "Loading configuration from .env file"
    # Export all variables from .env
    set -a
    source "$PROJECT_ROOT/.env"
    set +a
    log_success "Environment configuration loaded"
  else
    log_info "No .env file found, using defaults from .env.example"
    log_info "Create .env from .env.example to customize settings:"
    log_info "  cp .env.example .env"
  fi
  
  # Enable CITE test data for local development (OGC WMS compliance testing)
  # This adds cite:Lakes, cite:Ponds, etc. layers for CITE test suite
  export ENABLE_CITE_DATA="${ENABLE_CITE_DATA:-true}"
  if [ "$ENABLE_CITE_DATA" = "true" ]; then
    log_info "CITE test data enabled (cite:Lakes, cite:Ponds, etc.)"
  fi
}

#------------------------------------------------------------------------------
# Helper Functions
#------------------------------------------------------------------------------

log_info() {
  echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
  echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
  echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
  echo -e "${RED}[ERROR]${NC} $1"
}

check_command() {
  if ! command -v "$1" &>/dev/null; then
    log_error "$1 is required but not installed."
    echo "Please install $1 and try again."
    case "$1" in
    docker)
      echo "  https://docs.docker.com/get-docker/"
      ;;
    docker-compose)
      echo "  https://docs.docker.com/compose/install/"
      ;;
    esac
    exit 1
  fi
}

#------------------------------------------------------------------------------
# Docker Compose Functions
#------------------------------------------------------------------------------

check_compose_prerequisites() {
  log_info "Checking prerequisites for docker-compose..."

  check_command docker
  check_command docker-compose

  # Check Docker is running
  if ! docker info &>/dev/null; then
    log_error "Docker is not running. Please start Docker and try again."
    exit 1
  fi

  log_success "All prerequisites satisfied!"
}

rebuild_images_if_needed() {
  log_info "Checking if Docker images need rebuilding..."

  cd "$PROJECT_ROOT"

  # Check if images exist
  local need_rebuild=false

  if ! docker images weather-wms-wms-api | grep -q weather-wms-wms-api; then
    log_info "WMS API image not found, will build"
    need_rebuild=true
  else
    # Check if source code is newer than image
    local image_time=$(docker inspect -f '{{ .Created }}' weather-wms-wms-api:latest 2>/dev/null || echo "1970-01-01T00:00:00Z")
    local image_epoch=$(date -d "$image_time" +%s 2>/dev/null || echo 0)

    # Find newest Rust source file
    local newest_src=$(find crates/ services/ -name "*.rs" -o -name "Cargo.toml" 2>/dev/null | xargs ls -t 2>/dev/null | head -1)
    if [ -n "$newest_src" ]; then
      local src_epoch=$(stat -c %Y "$newest_src" 2>/dev/null || echo 0)

      if [ $src_epoch -gt $image_epoch ]; then
        log_info "Source code has changed since last build, will rebuild"
        need_rebuild=true
      fi
    fi
  fi

  if [ "$need_rebuild" = true ]; then
    log_info "Rebuilding Docker images..."
    docker-compose build
    log_success "Docker images rebuilt!"
  else
    log_info "Docker images are up to date"
  fi
}

start_compose() {
  log_info "Starting weather-wms stack with docker-compose..."

  cd "$PROJECT_ROOT"

  # Check if already running
  if docker-compose ps 2>/dev/null | grep -q "Up"; then
    log_warn "Stack is already running!"
    log_info "Run './start.sh --stop' to stop it"
    show_compose_access_info
    return
  fi

  # Rebuild images if source code changed
  rebuild_images_if_needed

  docker-compose up -d

  # Wait for services to be ready
  log_info "Waiting for services to be ready..."
  local retries=30
  while [ $retries -gt 0 ]; do
    if docker-compose exec -T postgres pg_isready -U weatherwms &>/dev/null &&
      docker-compose exec -T redis redis-cli ping &>/dev/null 2>&1; then
      log_success "All services are ready!"
      break
    fi
    echo -ne "\rWaiting... ($retries seconds remaining)"
    sleep 1
    retries=$((retries - 1))
  done

  if [ $retries -eq 0 ]; then
    log_warn "Services may not be fully ready yet. Check with: docker-compose ps"
  fi

  echo ""
  show_compose_access_info
}

stop_compose() {
  log_info "Stopping docker-compose stack..."

  cd "$PROJECT_ROOT"

  if docker-compose ps 2>/dev/null | grep -q "Up"; then
    docker-compose down
    log_success "Stack stopped!"
  else
    log_info "Stack is not running"
  fi
}

show_compose_status() {
  log_info "=== Docker Compose Stack Status ==="
  echo ""

  cd "$PROJECT_ROOT"
  docker-compose ps

  echo ""
  log_info "Service URLs:"
  echo "  Web Dashboard: http://localhost:8000  ✓"
  echo "  WMS API:       http://localhost:8080  ✓"
  echo "  EDR API:       http://localhost:8083  ✓"
  echo "  PostgreSQL:    localhost:5432         (weatherwms/weatherwms)"
  echo "  Redis:         localhost:6379         (no auth)"
  echo "  MinIO API:     localhost:9000         (minioadmin/minioadmin)"
  echo "  MinIO UI:      localhost:9001         (minioadmin/minioadmin)"
}

show_compose_access_info() {
  echo ""
  log_success "=== Quick Start ==="
  echo ""
  echo "All services are running! Open your browser:"
  echo "  http://localhost:8000  (Web Dashboard)"
  echo ""
  echo "Services running:"
  echo "  ✓ WMS API - serves weather map tiles"
  echo "  ✓ EDR API - OGC Environmental Data Retrieval"
  echo "  ✓ Downloader - automatically fetches new weather data"
  echo "  ✓ PostgreSQL, Redis, MinIO - data infrastructure"
  if [ "${ENABLE_CITE_DATA:-true}" = "true" ]; then
    echo "  ✓ CITE Test Data - OGC compliance test layers (cite:Lakes, etc.)"
  fi
  echo ""
  echo "The downloader service will automatically fetch new data."
  echo "Existing data in the database and storage is preserved."
  echo ""
  echo "Test the APIs directly:"
  echo "  curl \"http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities\""
  echo "  curl \"http://localhost:8083/edr/collections\""
  echo ""
  log_success "=== Service URLs & Credentials ==="
  echo ""
  echo "Web Dashboard (Interactive WMS Testing):"
  echo "  URL:  http://localhost:8000  ✓ Running"
  echo ""
  echo "WMS API:"
  echo "  URL:  http://localhost:8080  ✓ Running"
  echo ""
  echo "PostgreSQL:"
  echo "  User: weatherwms"
  echo "  Pass: weatherwms"
  echo "  DB:   weatherwms"
  echo "  Host: localhost:5432"
  echo ""
  echo "Redis:"
  echo "  Host: localhost:6379"
  echo "  No authentication"
  echo ""
  echo "MinIO (Object Storage):"
  echo "  User: minioadmin"
  echo "  Pass: minioadmin"
  echo "  API:  localhost:9000"
  echo "  UI:   localhost:9001"
  echo ""
  log_success "=== Other Commands ==="
  echo ""
  echo "View service logs:"
  echo "  docker-compose logs -f wms-api"
  echo "  docker-compose logs -f downloader"
  echo "  docker-compose logs -f web-dashboard"
  echo ""
  echo "Stop services:"
  echo "  ./start.sh --stop"
  echo ""
  echo "Manually ingest test data (optional):"
  echo "  ./scripts/ingest_test_data.sh"
  echo ""
  echo "Download specific data:"
  echo "  ./scripts/download_gfs.sh"
  echo "  ./scripts/download_hrrr.sh"
  echo "  ./scripts/download_goes.sh"
  echo "  ./scripts/download_mrms.sh"
  echo ""
}

run_data_ingestion() {
  log_info "Ingesting test weather data..."
  echo ""

  cd "$PROJECT_ROOT"

  # Run the ingestion script
  if bash scripts/ingest_test_data.sh; then
    log_success "Data ingestion completed successfully!"
  else
    log_error "Data ingestion failed!"
    return 1
  fi
}

run_test_rendering() {
  log_info "Running test rendering to verify ingestion and rendering..."
  echo ""

  cd "$PROJECT_ROOT"

  # Check if API is ready
  local retries=10
  while [ $retries -gt 0 ]; do
    if curl -s "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities" &>/dev/null; then
      break
    fi
    echo -ne "\rWaiting for API to be ready... ($retries seconds remaining)"
    sleep 1
    retries=$((retries - 1))
  done

  if [ $retries -eq 0 ]; then
    log_warn "API may not be fully ready yet. Skipping test rendering."
    return
  fi

  echo ""

  # Run the test script
  if bash scripts/test_rendering.sh; then
    log_success "Test rendering completed!"
    echo ""
    log_info "Sample images saved to: test_renders/"
    echo "  Verify the images contain colored temperature data, not gray placeholders"
  else
    log_warn "Test rendering had issues, but services are still running"
  fi
}

clear_tile_cache() {
  log_info "Clearing Redis tile cache..."

  cd "$PROJECT_ROOT"

  # Check if Redis is running
  if ! docker-compose ps redis 2>/dev/null | grep -q "Up"; then
    log_warn "Redis is not running. Start services first."
    return 1
  fi

  # Flush all cached tiles
  if docker-compose exec -T redis redis-cli FLUSHALL &>/dev/null; then
    log_success "Redis tile cache cleared!"

    # Get cache stats after clearing
    local key_count=$(docker-compose exec -T redis redis-cli DBSIZE 2>/dev/null | tr -d '\r')
    log_info "Cache keys remaining: ${key_count:-0}"
  else
    log_error "Failed to clear Redis cache"
    return 1
  fi
}

#------------------------------------------------------------------------------
# Main
#------------------------------------------------------------------------------

main() {
  echo ""
  echo "╔═══════════════════════════════════════════════════════════════╗"
  echo "║           Weather WMS - Local Development Setup               ║"
  echo "╚═══════════════════════════════════════════════════════════════╝"
  echo ""

  # Load environment configuration first (before any commands)
  load_env_file
  echo ""

  case "${1:-}" in
  --compose | "")
    check_compose_prerequisites
    start_compose
    ;;
  --dev)
    log_info "Starting in DEVELOPMENT mode (debug builds - faster compilation)..."
    export CARGO_PROFILE=dev
    check_compose_prerequisites
    cd "$PROJECT_ROOT"
    # Force rebuild with dev profile
    log_info "Building with debug profile (this is MUCH faster than release)..."
    docker-compose build
    docker-compose up -d
    log_success "Development stack started with debug builds!"
    show_compose_access_info
    log_warn "Note: Debug builds are slower at runtime but compile much faster."
    log_warn "Use './start.sh' (without --dev) for production-like performance."
    ;;
  --rebuild)
    log_info "Forcing rebuild of Docker images (release profile)..."
    cd "$PROJECT_ROOT"
    export CARGO_PROFILE=release
    docker-compose build
    log_success "Docker images rebuilt (release)!"
    log_info "Run './start.sh' to start with rebuilt images"
    ;;
  --rebuild-dev)
    log_info "Forcing rebuild of Docker images (dev/debug profile - FAST)..."
    cd "$PROJECT_ROOT"
    export CARGO_PROFILE=dev
    docker-compose build
    log_success "Docker images rebuilt (debug)!"
    log_info "Run './start.sh --dev' to start with debug images"
    ;;
  --stop)
    log_info "Stopping services..."
    if [ -f "$PROJECT_ROOT/docker-compose.yml" ]; then
      cd "$PROJECT_ROOT"
      docker-compose down || true
      log_success "Docker-compose stack stopped!"
    fi
    ;;
  --clean)
    log_info "Cleaning up..."
    cd "$PROJECT_ROOT"
    docker-compose down -v || true
    log_success "Docker-compose cleaned!"
    ;;
  --clear-cache)
    check_compose_prerequisites
    clear_tile_cache
    ;;
  --status)
    show_compose_status
    ;;
  --help | -h)
    echo "Usage: $0 [option]"
    echo ""
    echo "Options:"
    echo "  (none)         Start with docker-compose (release builds)"
    echo "  --compose      Start with docker-compose (same as above)"
    echo "  --dev          FAST: Start with debug builds (much faster compilation!)"
    echo "  --rebuild      Force rebuild (release profile)"
    echo "  --rebuild-dev  Force rebuild (debug profile - FAST)"
    echo "  --clear-cache  Clear Redis tile cache (useful after rendering changes)"
    echo "  --stop         Stop docker-compose"
    echo "  --clean        Delete everything and start fresh"
    echo "  --status       Show status of services"
    echo "  --help         Show this help message"
    echo ""
    echo "DEVELOPMENT WORKFLOW (FAST):"
    echo "  1. First time or after major changes:"
    echo "     ./start.sh --dev"
    echo "     (Debug builds compile 3-5x faster than release)"
    echo ""
    echo "  2. After code changes:"
    echo "     ./start.sh --rebuild-dev && ./start.sh --dev"
    echo ""
    echo "  3. For production-like performance testing:"
    echo "     ./start.sh --rebuild && ./start.sh"
    echo ""
    echo "BUILD TIME COMPARISON:"
    echo "  --rebuild-dev  ~1-2 min (debug, incremental)"
    echo "  --rebuild      ~5-10 min (release, optimized)"
    echo ""
    echo "NOTE: Debug builds are slower at runtime but compile MUCH faster."
    echo "      Use release builds only when testing performance."
    echo ""
  echo "Services automatically started:"
  echo "  - PostgreSQL (localhost:5432)"
  echo "  - Redis (localhost:6379)"
  echo "  - MinIO S3 (localhost:9000 + UI at 9001)"
  echo "  - WMS API (localhost:8080)"
  echo "  - Web Dashboard (localhost:8000)"
  echo ""
  echo "OGC COMPLIANCE TESTING:"
  echo "  CITE test data is enabled by default for local development."
  echo "  This adds cite:Lakes, cite:Ponds, etc. layers for WMS compliance testing."
  echo "  To run OGC compliance tests:"
  echo "    cd validation/ogc-compliance && ./run_wms_compliance.sh"
  echo ""
  echo "  To disable CITE data: export ENABLE_CITE_DATA=false"
  echo ""
  ;;
  *)
    log_error "Unknown option: $1"
    echo "Run './start.sh --help' for usage information"
    exit 1
    ;;
  esac
}

main "$@"
