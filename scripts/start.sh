#!/usr/bin/env bash
#
# Weather WMS - Local Development Start Script
#
# This script sets up a complete local development environment.
# Default: docker-compose (fast, no Kubernetes overhead)
# Optional: Full Kubernetes with minikube
#
# Usage:
#   ./start.sh              # Start with docker-compose (fast)
#   ./start.sh --compose    # Start with docker-compose (same as above)
#   ./start.sh --rebuild    # Force rebuild of Docker images
#   ./start.sh --clear-cache # Clear Redis tile cache (after rendering changes)
#   ./start.sh --kubernetes # Full Kubernetes setup with minikube
#   ./start.sh --k8s        # Same as --kubernetes
#   ./start.sh --forward    # Restart port forwards for existing K8s cluster
#   ./start.sh --stop       # Stop docker-compose
#   ./start.sh --stop-k8s   # Stop K8s port forwards (cluster keeps running)
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

# Configuration
MINIKUBE_PROFILE="weather-wms"
MINIKUBE_CPUS=4
MINIKUBE_MEMORY=6000
MINIKUBE_DISK="50g"
NAMESPACE="weather-wms"
HELM_RELEASE="wms"

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
    minikube)
      echo "  macOS: brew install minikube"
      echo "  Linux: https://minikube.sigs.k8s.io/docs/start/"
      ;;
    kubectl)
      echo "  macOS: brew install kubectl"
      echo "  Linux: https://kubernetes.io/docs/tasks/tools/"
      ;;
    helm)
      echo "  macOS: brew install helm"
      echo "  Linux: https://helm.sh/docs/intro/install/"
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
  echo "  ✓ Downloader - automatically fetches new weather data"
  echo "  ✓ PostgreSQL, Redis, MinIO - data infrastructure"
  echo ""
  echo "The downloader service will automatically fetch new data."
  echo "Existing data in the database and storage is preserved."
  echo ""
  echo "Test the API directly:"
  echo "  curl \"http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities\""
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
# Kubernetes Functions
#------------------------------------------------------------------------------

check_k8s_prerequisites() {
  log_info "Checking prerequisites for Kubernetes..."

  check_command minikube
  check_command kubectl
  check_command helm
  check_command docker

  # Check Docker is running
  if ! docker info &>/dev/null; then
    log_error "Docker is not running. Please start Docker and try again."
    exit 1
  fi

  log_success "All prerequisites satisfied!"
}

start_minikube() {
  log_info "Starting minikube cluster '$MINIKUBE_PROFILE'..."

  # Check if cluster already exists
  if minikube status -p "$MINIKUBE_PROFILE" &>/dev/null; then
    log_info "Cluster already exists, starting..."
    minikube start -p "$MINIKUBE_PROFILE" --wait=all
  else
    log_info "Creating new cluster..."
    minikube start \
      -p "$MINIKUBE_PROFILE" \
      --cpus="$MINIKUBE_CPUS" \
      --memory="$MINIKUBE_MEMORY" \
      --disk-size="$MINIKUBE_DISK" \
      --driver=docker \
      --wait=all
  fi

  # Set kubectl context
  kubectl config use-context "$MINIKUBE_PROFILE"

  # Skip ingress addon - it causes webhook issues and we use port-forward anyway
  # minikube addons enable ingress -p "$MINIKUBE_PROFILE"

  log_success "Minikube cluster is running!"
}

setup_namespace() {
  log_info "Setting up namespace '$NAMESPACE'..."

  kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

  log_success "Namespace ready!"
}

deploy_dependencies() {
  log_info "Adding Helm repositories..."

  helm repo add bitnami https://charts.bitnami.com/bitnami
  helm repo update

  log_success "Helm repositories updated!"

  # Build Helm chart dependencies
  log_info "Building Helm chart dependencies..."
  cd "$PROJECT_ROOT/deploy/helm/weather-wms"
  helm dependency build
  cd "$PROJECT_ROOT"

  log_success "Helm dependencies ready!"
}

build_single_image() {
  local name="$1"
  local dockerfile="$2"
  local context="$3"
  local start_time=$(date +%s)

  echo ""
  log_info "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  log_info "Building: $name"
  log_info "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

  if docker build -t "weather-wms/${name}:latest" -f "$dockerfile" "$context" 2>&1 | while read line; do
    # Show key build steps
    if echo "$line" | grep -qE '^\[|^Step|^#[0-9]|CACHED|ERROR|error:'; then
      echo "  $line"
    fi
  done; then
    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    log_success "$name built in ${duration}s"
    return 0
  else
    log_error "Failed to build $name"
    return 1
  fi
}

load_single_image() {
  local name="$1"
  local start_time=$(date +%s)

  echo -n "  Loading $name into minikube... "

  if minikube -p "$MINIKUBE_PROFILE" image load "weather-wms/${name}:latest" 2>&1 | head -5; then
    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    echo "done (${duration}s)"
    return 0
  else
    echo "FAILED"
    return 1
  fi
}

build_k8s_images() {
  echo ""
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║              Building Docker Images                           ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""
  log_warn "First build may take 10-20 minutes (Rust compilation)"
  log_info "Subsequent builds will be much faster (cached layers)"
  echo ""

  cd "$PROJECT_ROOT"

  local failed=0

  # Build Dashboard first (fastest, good sanity check)
  build_single_image "dashboard" "web/Dockerfile" "web/" || failed=1

  if [ $failed -eq 1 ]; then
    log_error "Dashboard build failed. Check Docker is running."
    return 1
  fi

  # Build the Rust services (these take longer)
  build_single_image "wms-api" "services/wms-api/Dockerfile" "." || failed=1
  build_single_image "ingester" "services/ingester/Dockerfile" "." || failed=1
  build_single_image "downloader" "services/downloader/Dockerfile" "." || failed=1

  if [ $failed -eq 1 ]; then
    log_error "One or more image builds failed!"
    return 1
  fi

  echo ""
  log_success "All Docker images built successfully!"
  echo ""

  # Load images into minikube
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║           Loading Images into Minikube                        ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""

  load_single_image "dashboard" || failed=1
  load_single_image "wms-api" || failed=1
  load_single_image "ingester" || failed=1
  load_single_image "downloader" || failed=1

  if [ $failed -eq 1 ]; then
    log_error "Failed to load some images into minikube!"
    return 1
  fi

  echo ""
  log_success "All images loaded into minikube!"
}

preload_infra_images() {
  echo ""
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║           Pre-loading Infrastructure Images                   ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""
  log_info "Pulling images on host and loading into minikube..."
  log_info "(This works around minikube's network/DNS issues)"
  echo ""

  # Infrastructure and monitoring images
  local images=(
    "docker.io/bitnami/redis:latest"
    "docker.io/bitnami/postgresql:latest"
    "docker.io/minio/minio:latest"
    "docker.io/grafana/grafana:latest"
    "docker.io/prom/prometheus:latest"
    "docker.io/kubernetesui/dashboard:v2.7.0"
    "docker.io/kubernetesui/metrics-scraper:v1.0.8"
  )

  for img in "${images[@]}"; do
    local name=$(echo "$img" | sed 's|.*/||' | cut -d: -f1)
    echo -n "  $name: "

    # Pull
    echo -n "pulling... "
    if docker pull "$img" >/dev/null 2>&1; then
      echo -n "✓ "
    else
      echo "✗ pull failed"
      continue
    fi

    # Load into minikube
    echo -n "loading... "
    if minikube -p "$MINIKUBE_PROFILE" image load "$img" >/dev/null 2>&1; then
      echo "✓"
    else
      echo "✗ load failed"
    fi
  done
  echo ""
}

deploy_dev_stack() {
  echo ""
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║           Deploying Infrastructure (Redis, PG, MinIO)         ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""

  # Redis - force tag to 'latest' which we pre-loaded
  echo -n "  [1/3] Redis............ "
  if helm upgrade --install redis bitnami/redis \
    --namespace "$NAMESPACE" \
    --set architecture=standalone \
    --set auth.enabled=false \
    --set master.persistence.enabled=false \
    --set image.registry=docker.io \
    --set image.repository=bitnami/redis \
    --set image.tag=latest \
    --set image.pullPolicy=Never \
    --wait --timeout 3m 2>&1 | grep -E 'deployed|STATUS' | head -1; then
    echo " ✓"
  else
    echo " ✗ (continuing...)"
  fi

  # PostgreSQL - force tag to 'latest'
  echo -n "  [2/3] PostgreSQL....... "
  if helm upgrade --install postgresql bitnami/postgresql \
    --namespace "$NAMESPACE" \
    --set auth.username=weatherwms \
    --set auth.password=weatherwms \
    --set auth.database=weatherwms \
    --set primary.persistence.enabled=false \
    --set image.registry=docker.io \
    --set image.repository=bitnami/postgresql \
    --set image.tag=latest \
    --set image.pullPolicy=Never \
    --wait --timeout 3m 2>&1 | grep -E 'deployed|STATUS' | head -1; then
    echo " ✓"
  else
    echo " ✗ (continuing...)"
  fi

  # MinIO - deploy directly with kubectl (bitnami chart has image tag issues)
  echo -n "  [3/3] MinIO............ "

  # Delete any existing minio resources to ensure clean state
  kubectl delete svc minio minio-console -n "$NAMESPACE" 2>/dev/null || true
  kubectl delete deployment minio -n "$NAMESPACE" 2>/dev/null || true
  sleep 1

  kubectl apply -n "$NAMESPACE" -f - >/dev/null 2>&1 <<EOF
apiVersion: apps/v1
kind: Deployment
metadata:
  name: minio
spec:
  replicas: 1
  selector:
    matchLabels:
      app: minio
  template:
    metadata:
      labels:
        app: minio
    spec:
      containers:
      - name: minio
        image: minio/minio:latest
        imagePullPolicy: Never
        args: ["server", "/data", "--console-address", ":9001"]
        env:
        - name: MINIO_ROOT_USER
          value: "minioadmin"
        - name: MINIO_ROOT_PASSWORD
          value: "minioadmin"
        ports:
        - containerPort: 9000
          name: api
        - containerPort: 9001
          name: console
        readinessProbe:
          httpGet:
            path: /minio/health/ready
            port: 9000
          initialDelaySeconds: 5
          periodSeconds: 5
        livenessProbe:
          httpGet:
            path: /minio/health/live
            port: 9000
          initialDelaySeconds: 10
          periodSeconds: 10
---
apiVersion: v1
kind: Service
metadata:
  name: minio
spec:
  selector:
    app: minio
  ports:
  - name: api
    port: 9000
    targetPort: 9000
  - name: console
    port: 9001
    targetPort: 9001
EOF
  if [ $? -eq 0 ]; then
    echo "✓"
  else
    echo "✗ (continuing...)"
  fi

  echo ""
  log_success "Infrastructure deployment initiated!"
}

deploy_monitoring() {
  echo ""
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║           Deploying Monitoring (Prometheus, Grafana)          ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""

  # Deploy Prometheus and Grafana
  echo -n "  [1/2] Prometheus....... "
  kubectl apply -n "$NAMESPACE" -f - >/dev/null 2>&1 <<'EOF'
apiVersion: v1
kind: ConfigMap
metadata:
  name: prometheus-config
data:
  prometheus.yml: |
    global:
      scrape_interval: 15s
    scrape_configs:
      - job_name: 'wms-api'
        static_configs:
          - targets: ['wms-weather-wms-api:8080']
        metrics_path: /metrics
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: prometheus
spec:
  replicas: 1
  selector:
    matchLabels:
      app: prometheus
  template:
    metadata:
      labels:
        app: prometheus
    spec:
      containers:
      - name: prometheus
        image: prom/prometheus:latest
        imagePullPolicy: Never
        args:
          - '--config.file=/etc/prometheus/prometheus.yml'
          - '--storage.tsdb.path=/prometheus'
        ports:
        - containerPort: 9090
        volumeMounts:
        - name: config
          mountPath: /etc/prometheus
      volumes:
      - name: config
        configMap:
          name: prometheus-config
---
apiVersion: v1
kind: Service
metadata:
  name: prometheus
spec:
  selector:
    app: prometheus
  ports:
  - port: 9090
    targetPort: 9090
EOF
  if [ $? -eq 0 ]; then
    echo "✓"
  else
    echo "✗"
  fi

  echo -n "  [2/2] Grafana.......... "

  # Create Grafana ConfigMaps from files
  # Dashboard provisioning config
  kubectl apply -n "$NAMESPACE" -f - >/dev/null 2>&1 <<'EOF'
apiVersion: v1
kind: ConfigMap
metadata:
  name: grafana-dashboard-provisioning
data:
  dashboard.yml: |
    apiVersion: 1
    providers:
      - name: 'WMS Dashboards'
        orgId: 1
        folder: ''
        type: file
        disableDeletion: false
        editable: true
        options:
          path: /var/lib/grafana/dashboards
EOF

  # Datasource provisioning config
  kubectl apply -n "$NAMESPACE" -f - >/dev/null 2>&1 <<'EOF'
apiVersion: v1
kind: ConfigMap
metadata:
  name: grafana-datasource-provisioning
data:
  datasource.yml: |
    apiVersion: 1
    datasources:
      - name: Prometheus
        type: prometheus
        access: proxy
        url: http://prometheus:9090
        isDefault: true
        editable: true
EOF

  # Create dashboard ConfigMap from the JSON file
  kubectl create configmap grafana-dashboards \
    --from-file=wms-performance.json="$PROJECT_ROOT/deploy/grafana/provisioning/dashboards/wms-performance.json" \
    -n "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f - >/dev/null 2>&1

  # Deploy Grafana with volume mounts
  kubectl apply -n "$NAMESPACE" -f - >/dev/null 2>&1 <<'EOF'
apiVersion: apps/v1
kind: Deployment
metadata:
  name: grafana
spec:
  replicas: 1
  selector:
    matchLabels:
      app: grafana
  template:
    metadata:
      labels:
        app: grafana
    spec:
      containers:
      - name: grafana
        image: grafana/grafana:latest
        imagePullPolicy: Never
        ports:
        - containerPort: 3000
        env:
        - name: GF_SECURITY_ADMIN_PASSWORD
          value: "admin"
        - name: GF_AUTH_ANONYMOUS_ENABLED
          value: "true"
        - name: GF_AUTH_ANONYMOUS_ORG_ROLE
          value: "Viewer"
        volumeMounts:
        - name: dashboard-provisioning
          mountPath: /etc/grafana/provisioning/dashboards
        - name: datasource-provisioning
          mountPath: /etc/grafana/provisioning/datasources
        - name: dashboards
          mountPath: /var/lib/grafana/dashboards
      volumes:
      - name: dashboard-provisioning
        configMap:
          name: grafana-dashboard-provisioning
      - name: datasource-provisioning
        configMap:
          name: grafana-datasource-provisioning
      - name: dashboards
        configMap:
          name: grafana-dashboards
---
apiVersion: v1
kind: Service
metadata:
  name: grafana
spec:
  selector:
    app: grafana
  ports:
  - port: 3000
    targetPort: 3000
EOF
  if [ $? -eq 0 ]; then
    echo "✓"
  else
    echo "✗"
  fi

  echo ""
  log_success "Monitoring stack deployed!"
}

enable_k8s_dashboard() {
  echo ""
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║           Enabling Kubernetes Dashboard                       ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""

  # Enable the dashboard addon
  echo -n "  Enabling dashboard addon... "
  minikube addons enable dashboard -p "$MINIKUBE_PROFILE" >/dev/null 2>&1
  echo "✓"

  # Wait for dashboard pods
  sleep 3

  # Patch deployments to remove image digests (allows using pre-loaded images)
  echo -n "  Patching deployments... "
  kubectl patch deployment kubernetes-dashboard -n kubernetes-dashboard --type='json' \
    -p='[{"op": "replace", "path": "/spec/template/spec/containers/0/image", "value": "docker.io/kubernetesui/dashboard:v2.7.0"}]' >/dev/null 2>&1 || true
  kubectl patch deployment dashboard-metrics-scraper -n kubernetes-dashboard --type='json' \
    -p='[{"op": "replace", "path": "/spec/template/spec/containers/0/image", "value": "docker.io/kubernetesui/metrics-scraper:v1.0.8"}]' >/dev/null 2>&1 || true
  echo "✓"

  # Wait for pods to be ready
  echo -n "  Waiting for dashboard pods... "
  local retries=30
  while [ $retries -gt 0 ]; do
    local ready
    ready=$(kubectl get pods -n kubernetes-dashboard --no-headers 2>/dev/null | grep -c "Running" 2>/dev/null) || ready=0
    if [ "$ready" -ge 2 ]; then
      echo "✓"
      break
    fi
    sleep 2
    retries=$((retries - 1))
  done

  if [ $retries -eq 0 ]; then
    echo "timeout (may still be starting)"
  fi

  echo ""
  log_success "Kubernetes Dashboard enabled!"
}

create_config_configmaps() {
  echo ""
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║           Creating Config ConfigMaps                          ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""

  cd "$PROJECT_ROOT"

  # Create ConfigMap from config/models/*.yaml files
  echo -n "  [1/2] Models config...... "
  if kubectl create configmap "$HELM_RELEASE-weather-wms-models-config" \
    --from-file=config/models/ \
    -n "$NAMESPACE" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null 2>&1; then
    echo "✓"
  else
    echo "✗"
  fi

  # Create ConfigMap from config/styles/*.json files
  echo -n "  [2/2] Styles config...... "
  if kubectl create configmap "$HELM_RELEASE-weather-wms-styles-config" \
    --from-file=config/styles/ \
    -n "$NAMESPACE" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null 2>&1; then
    echo "✓"
  else
    echo "✗"
  fi

  echo ""
  log_success "Config ConfigMaps created from local files!"
}

deploy_weather_wms() {
  echo ""
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║           Deploying Weather WMS Application                   ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""

  cd "$PROJECT_ROOT"

  # Create ConfigMaps from local config files first
  create_config_configmaps

  log_info "Installing Helm chart..."

  # Deploy the weather-wms Helm chart
  # - Disable subcharts (we deployed infra separately)
  # - Use explicit URLs pointing to our standalone services
  # - Use Never pull policy since images are pre-loaded
  # - Disable ingress (we use port-forward for local dev)
  if helm upgrade --install "$HELM_RELEASE" deploy/helm/weather-wms \
    --namespace "$NAMESPACE" \
    -f deploy/helm/weather-wms/values-dev.yaml \
    --set redis.enabled=false \
    --set postgresql.enabled=false \
    --set minio.enabled=false \
    --set config.redis.url="redis://redis-master:6379" \
    --set config.database.url="postgresql://weatherwms:weatherwms@postgresql:5432/weatherwms" \
    --set config.s3.endpoint="http://minio:9000" \
    --set api.image.pullPolicy=Never \
    --set ingester.image.pullPolicy=Never \
    --set renderer.image.pullPolicy=Never \
    --set dashboard.image.pullPolicy=Never \
    --set downloader.image.pullPolicy=Never \
    --set api.ingress.enabled=false \
    --wait --timeout 10m 2>&1 | grep -E 'STATUS|REVISION|deployed|failed|error' | head -5; then
    echo ""
    log_success "Weather WMS application deployed!"
  else
    log_error "Failed to deploy Weather WMS!"
    echo ""
    log_info "Debug: Check pod status with:"
    echo "  kubectl get pods -n $NAMESPACE"
    echo "  kubectl describe pods -n $NAMESPACE"
    return 1
  fi
}

setup_port_forwards() {
  echo ""
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║           Setting up Port Forwards                            ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""

  # Kill any existing port forwards
  pkill -f "kubectl port-forward" 2>/dev/null || true
  sleep 1

  # Create a directory for port-forward logs
  local pf_log_dir="/tmp/weather-wms-port-forwards"
  mkdir -p "$pf_log_dir"

  log_info "Starting port forwards..."
  echo ""

  # Get actual service names
  local api_svc=$(kubectl get svc -n "$NAMESPACE" -o name 2>/dev/null | grep -E 'api$' | head -1)
  local dashboard_svc=$(kubectl get svc -n "$NAMESPACE" -o name 2>/dev/null | grep -E 'wms.*dashboard$' | head -1)

  # Port forward WMS API (8080)
  if [ -n "$api_svc" ]; then
    nohup kubectl port-forward -n "$NAMESPACE" "$api_svc" 8080:8080 \
      >"$pf_log_dir/api.log" 2>&1 &
    echo $! >"$pf_log_dir/api.pid"
    echo "    ✓ WMS API:          http://localhost:8080"
  else
    echo "    ✗ WMS API service not found"
  fi

  # Port forward Weather Dashboard (8000)
  if [ -n "$dashboard_svc" ]; then
    nohup kubectl port-forward -n "$NAMESPACE" "$dashboard_svc" 8000:8000 \
      >"$pf_log_dir/dashboard.log" 2>&1 &
    echo $! >"$pf_log_dir/dashboard.pid"
    echo "    ✓ Weather Dashboard: http://localhost:8000"
  else
    echo "    ✗ Weather Dashboard service not found"
  fi

  # Port forward MinIO API (9000) and Console (9001)
  if kubectl get svc minio -n "$NAMESPACE" &>/dev/null; then
    nohup kubectl port-forward -n "$NAMESPACE" svc/minio 9000:9000 9001:9001 \
      >"$pf_log_dir/minio.log" 2>&1 &
    echo $! >"$pf_log_dir/minio.pid"
    echo "    ✓ MinIO Console:    http://localhost:9001  (minioadmin/minioadmin)"
  fi

  # Port forward Grafana (3000)
  if kubectl get svc grafana -n "$NAMESPACE" &>/dev/null; then
    nohup kubectl port-forward -n "$NAMESPACE" svc/grafana 3000:3000 \
      >"$pf_log_dir/grafana.log" 2>&1 &
    echo $! >"$pf_log_dir/grafana.pid"
    echo "    ✓ Grafana:          http://localhost:3000  (admin/admin)"
  fi

  # Port forward Prometheus (9090)
  if kubectl get svc prometheus -n "$NAMESPACE" &>/dev/null; then
    nohup kubectl port-forward -n "$NAMESPACE" svc/prometheus 9090:9090 \
      >"$pf_log_dir/prometheus.log" 2>&1 &
    echo $! >"$pf_log_dir/prometheus.pid"
    echo "    ✓ Prometheus:       http://localhost:9090"
  fi

  # Start kubectl proxy for K8s Dashboard and API access (8001)
  if kubectl get svc kubernetes-dashboard -n kubernetes-dashboard &>/dev/null; then
    nohup kubectl proxy --port=8001 \
      >"$pf_log_dir/k8s-dashboard.log" 2>&1 &
    echo $! >"$pf_log_dir/k8s-dashboard.pid"
    echo "    ✓ K8s Dashboard:    http://localhost:8001/api/v1/namespaces/kubernetes-dashboard/services/http:kubernetes-dashboard:/proxy/"
  fi

  echo ""

  # Wait for port forwards to establish
  log_info "Waiting for port forwards to establish..."
  local retries=10
  while [ $retries -gt 0 ]; do
    sleep 1
    if curl -s --max-time 2 "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities" &>/dev/null; then
      break
    fi
    retries=$((retries - 1))
  done

  if [ $retries -gt 0 ]; then
    log_success "Port forwards are active and services are responding!"
  else
    log_warn "Port forwards started but API may still be initializing..."
    log_info "Check logs: cat $pf_log_dir/*.log"
  fi

  echo ""
  echo "  Port forward logs: $pf_log_dir/"
  echo "  To stop port forwards: pkill -f 'kubectl port-forward'"
}

run_k8s_data_ingestion() {
  log_info "Running data ingestion in Kubernetes..."

  # Ensure port forwards are active for data services
  echo "  Ensuring port forwards are active..."

  # Check and start PostgreSQL port-forward if needed
  if ! netstat -tlnp 2>/dev/null | grep -q ":5432" && ! ss -tlnp 2>/dev/null | grep -q ":5432"; then
    kubectl port-forward -n "$NAMESPACE" svc/postgresql 5432:5432 &>/dev/null &
    sleep 1
  fi

  # Check and start Redis port-forward if needed
  if ! netstat -tlnp 2>/dev/null | grep -q ":6379" && ! ss -tlnp 2>/dev/null | grep -q ":6379"; then
    kubectl port-forward -n "$NAMESPACE" svc/redis-master 6379:6379 &>/dev/null &
    sleep 1
  fi

  # Check and start MinIO port-forward if needed
  if ! netstat -tlnp 2>/dev/null | grep -q ":9000" && ! ss -tlnp 2>/dev/null | grep -q ":9000"; then
    kubectl port-forward -n "$NAMESPACE" svc/minio 9000:9000 &>/dev/null &
    sleep 1
  fi

  # Ensure MinIO bucket exists
  echo "  Ensuring MinIO bucket exists..."
  kubectl exec -n "$NAMESPACE" deployment/minio -- mc alias set local http://localhost:9000 minioadmin minioadmin &>/dev/null || true
  kubectl exec -n "$NAMESPACE" deployment/minio -- mc mb --ignore-existing local/weather-data &>/dev/null || true

  # Wait for API to be ready
  local retries=30
  while [ $retries -gt 0 ]; do
    if curl -s "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities" &>/dev/null; then
      break
    fi
    echo -ne "\rWaiting for API to be ready... ($retries seconds remaining)"
    sleep 1
    retries=$((retries - 1))
  done
  echo ""

  if [ $retries -eq 0 ]; then
    log_warn "API may not be fully ready yet. Skipping data ingestion."
    return
  fi

  # Run the ingestion script (it will use localhost URLs via port-forward)
  if bash scripts/ingest_test_data.sh; then
    log_success "Data ingestion completed!"
  else
    log_warn "Data ingestion had issues, but services are still running"
  fi
}

wait_for_pods() {
  local namespace=$1
  local timeout=${2:-300}

  echo ""
  log_info "╔═══════════════════════════════════════════════════════════════╗"
  log_info "║           Waiting for Pods to be Ready                        ║"
  log_info "╚═══════════════════════════════════════════════════════════════╝"
  echo ""

  local start_time=$(date +%s)
  local last_status=""

  while true; do
    local current_time=$(date +%s)
    local elapsed=$((current_time - start_time))

    if [ $elapsed -gt $timeout ]; then
      echo ""
      log_error "Timeout waiting for pods after ${timeout}s"
      echo ""
      kubectl get pods -n "$namespace"
      return 1
    fi

    # Get pod status summary
    local pod_status=$(kubectl get pods -n "$namespace" --no-headers 2>/dev/null | awk '{print $1": "$3}' | tr '\n' ' ')
    local total=$(kubectl get pods -n "$namespace" --no-headers 2>/dev/null | wc -l)
    local ready
    ready=$(kubectl get pods -n "$namespace" --no-headers 2>/dev/null | grep -c "Running" 2>/dev/null) || ready=0

    # Only print if status changed
    if [ "$pod_status" != "$last_status" ]; then
      echo ""
      echo "  [${elapsed}s] Pods: $ready/$total ready"
      kubectl get pods -n "$namespace" --no-headers 2>/dev/null | while read line; do
        local name=$(echo "$line" | awk '{print $1}')
        local ready_col=$(echo "$line" | awk '{print $2}')
        local status=$(echo "$line" | awk '{print $3}')
        case "$status" in
        Running)
          echo "    ✓ $name ($status)"
          ;;
        Pending | ContainerCreating | PodInitializing)
          echo "    ⏳ $name ($status)"
          ;;
        *)
          echo "    ✗ $name ($status)"
          ;;
        esac
      done
      last_status="$pod_status"
    else
      echo -ne "."
    fi

    # Check if all pods are ready
    local not_ready
    not_ready=$(kubectl get pods -n "$namespace" -o jsonpath='{.items[*].status.conditions[?(@.type=="Ready")].status}' 2>/dev/null | grep -c "False" 2>/dev/null) || not_ready=0
    local pending
    pending=$(kubectl get pods -n "$namespace" --no-headers 2>/dev/null | grep -c "Pending\|ContainerCreating\|PodInitializing" 2>/dev/null) || pending=0

    if [ "$not_ready" -eq 0 ] && [ "$pending" -eq 0 ] && [ "$total" -gt 0 ]; then
      echo ""
      log_success "All $total pods are ready!"
      return 0
    fi

    sleep 3
  done
}

show_k8s_status() {
  echo ""
  log_info "=== Cluster Status ==="
  echo ""

  echo "Minikube:"
  minikube status -p "$MINIKUBE_PROFILE" || true

  echo ""
  echo "Pods in $NAMESPACE:"
  kubectl get pods -n "$NAMESPACE" -o wide || true

  echo ""
  echo "Services in $NAMESPACE:"
  kubectl get svc -n "$NAMESPACE" || true
}

show_k8s_access_info() {
  echo ""
  echo ""
  log_success "╔═══════════════════════════════════════════════════════════════╗"
  log_success "║           Weather WMS is Ready!                               ║"
  log_success "╚═══════════════════════════════════════════════════════════════╝"
  echo ""
  echo "  Open these URLs in your browser:"
  echo ""
  echo "    🌐 Weather Dashboard:  http://localhost:8000"
  echo "    🗺️  WMS API:            http://localhost:8080"
  echo ""
  echo "    📊 Grafana:            http://localhost:3000   (admin/admin)"
  echo "    📈 Prometheus:         http://localhost:9090"
  echo "    📦 MinIO Console:      http://localhost:9001   (minioadmin/minioadmin)"
  echo "    ☸️  K8s Dashboard:      http://localhost:8001/.../proxy/ (via kubectl proxy)"
  echo ""
  echo "  ─────────────────────────────────────────────────────────────────"
  echo ""
  echo "  Quick test commands:"
  echo ""
  echo "    # Get WMS capabilities"
  echo "    curl \"http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities\""
  echo ""
  echo "    # View pods"
  echo "    kubectl get pods -n $NAMESPACE"
  echo ""
  echo "    # View logs"
  echo "    kubectl logs -n $NAMESPACE -l app.kubernetes.io/component=api -f"
  echo ""
  echo "  ─────────────────────────────────────────────────────────────────"
  echo ""
  echo "  To stop:"
  echo "    ./scripts/start.sh --stop-k8s    # Stop port forwards"
  echo "    minikube stop -p $MINIKUBE_PROFILE       # Stop cluster"
  echo "    minikube delete -p $MINIKUBE_PROFILE     # Delete cluster"
  echo ""

  log_success "=== Additional Services ==="
  echo ""
  echo "PostgreSQL:"
  echo "  kubectl port-forward -n $NAMESPACE svc/postgresql 5432:5432"
  echo "  psql -h localhost -U weatherwms -d weatherwms"
  echo ""
  echo "Redis:"
  echo "  kubectl port-forward -n $NAMESPACE svc/redis-master 6379:6379"
  echo "  redis-cli -h localhost"
  echo ""
}

stop_k8s() {
  log_info "Stopping minikube cluster..."
  minikube stop -p "$MINIKUBE_PROFILE"
  log_success "Cluster stopped!"
}

clean_k8s() {
  log_warn "This will delete the entire minikube cluster!"
  read -p "Are you sure? (y/N) " -n 1 -r
  echo
  if [[ $REPLY =~ ^[Yy]$ ]]; then
    log_info "Deleting minikube cluster..."
    minikube delete -p "$MINIKUBE_PROFILE" || true
    log_success "Cluster deleted!"
  else
    log_info "Cancelled."
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
  --kubernetes | --k8s)
    check_k8s_prerequisites
    start_minikube
    setup_namespace
    build_k8s_images
    preload_infra_images
    enable_k8s_dashboard
    deploy_dependencies
    deploy_dev_stack
    deploy_monitoring
    deploy_weather_wms
    wait_for_pods "$NAMESPACE" 300
    setup_port_forwards
    show_k8s_status
    show_k8s_access_info
    ;;
  --compose | "")
    check_compose_prerequisites
    start_compose
    ;;
  --rebuild)
    log_info "Forcing rebuild of Docker images..."
    cd "$PROJECT_ROOT"
    docker-compose build
    log_success "Docker images rebuilt!"
    log_info "Run './start.sh' to start with rebuilt images"
    ;;
  --stop)
    log_info "Stopping services..."
    # Try docker-compose first
    if [ -f "$PROJECT_ROOT/docker-compose.yml" ]; then
      cd "$PROJECT_ROOT"
      docker-compose down || true
      log_success "Docker-compose stack stopped!"
    fi
    # Also stop minikube if running
    if minikube status -p "$MINIKUBE_PROFILE" &>/dev/null; then
      stop_k8s
    fi
    ;;
  --stop-k8s)
    log_info "Stopping Kubernetes port forwards..."
    pkill -f "kubectl port-forward.*$NAMESPACE" 2>/dev/null || true
    rm -rf /tmp/weather-wms-port-forwards 2>/dev/null || true
    log_success "Port forwards stopped!"
    echo ""
    echo "  Cluster is still running. To stop it:"
    echo "    minikube stop -p $MINIKUBE_PROFILE"
    echo ""
    echo "  To restart port forwards:"
    echo "    ./scripts/start.sh --forward"
    ;;
  --forward)
    log_info "Starting port forwards for existing cluster..."
    if ! minikube status -p "$MINIKUBE_PROFILE" &>/dev/null; then
      log_error "Minikube cluster '$MINIKUBE_PROFILE' is not running!"
      echo "  Start it with: ./scripts/start.sh --kubernetes"
      exit 1
    fi
    setup_port_forwards
    show_k8s_access_info
    ;;
  --clean)
    log_info "Cleaning up..."
    # Clean docker-compose
    cd "$PROJECT_ROOT"
    docker-compose down -v || true
    log_success "Docker-compose cleaned!"
    # Clean minikube
    clean_k8s
    ;;
  --clear-cache)
    check_compose_prerequisites
    clear_tile_cache
    ;;
  --status)
    if minikube status -p "$MINIKUBE_PROFILE" &>/dev/null; then
      show_k8s_status
    else
      show_compose_status
    fi
    ;;
  --help | -h)
    echo "Usage: $0 [option]"
    echo ""
    echo "Options:"
    echo "  (none)         Start with docker-compose (RECOMMENDED - fast)"
    echo "  --compose      Start with docker-compose"
    echo "  --rebuild      Force rebuild of Docker images (use after code changes)"
    echo "  --clear-cache  Clear Redis tile cache (useful after rendering changes)"
    echo "  --kubernetes   Full Kubernetes setup with minikube (slower)"
    echo "  --k8s          Same as --kubernetes"
    echo "  --stop         Stop docker-compose or minikube"
    echo "  --clean        Delete everything and start fresh"
    echo "  --status       Show status of services"
    echo "  --help         Show this help message"
    echo ""
    echo "NOTE: Images are automatically rebuilt if source code changed since last build"
    echo ""
    echo "RECOMMENDED WORKFLOW:"
    echo "  1. Run: ./start.sh"
    echo "     Starts all services (database, cache, storage, API, dashboard)"
    echo ""
    echo "  2. Open your browser:"
    echo "     http://localhost:8000  (Web Dashboard)"
    echo ""
    echo "  3. Upload layers and test interactively!"
    echo ""
    echo "Services automatically started:"
    echo "  - PostgreSQL (localhost:5432)"
    echo "  - Redis (localhost:6379)"
    echo "  - MinIO S3 (localhost:9000 + UI at 9001)"
    echo "  - WMS API (localhost:8080)"
    echo "  - Web Dashboard (localhost:8000)"
    echo ""
    echo "For full Kubernetes setup:"
    echo "  ./start.sh --kubernetes"
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
