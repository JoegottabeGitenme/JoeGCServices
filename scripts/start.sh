#!/usr/bin/env bash
#
# Weather WMS - Local Development Start Script
#
# This script sets up a complete local development environment using minikube.
# It's designed to be dummy-proof - just run it and everything will be set up.
#
# Usage:
#   ./start.sh              # Full setup (first time)
#   ./start.sh --quick      # Skip rebuilding images
#   ./start.sh --clean      # Delete everything and start fresh
#   ./start.sh --stop       # Stop the cluster
#   ./start.sh --status     # Show status of all components
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
MINIKUBE_MEMORY=8192
MINIKUBE_DISK="50g"
NAMESPACE="weather-wms"
HELM_RELEASE="wms"

# Script directory
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

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
    if ! command -v "$1" &> /dev/null; then
        log_error "$1 is required but not installed."
        echo "Please install $1 and try again."
        case "$1" in
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
            docker)
                echo "  https://docs.docker.com/get-docker/"
                ;;
        esac
        exit 1
    fi
}

wait_for_pods() {
    local namespace=$1
    local timeout=${2:-300}
    local selector=${3:-""}
    
    log_info "Waiting for pods in namespace '$namespace' to be ready..."
    
    local selector_arg=""
    if [ -n "$selector" ]; then
        selector_arg="-l $selector"
    fi
    
    local start_time=$(date +%s)
    while true; do
        local current_time=$(date +%s)
        local elapsed=$((current_time - start_time))
        
        if [ $elapsed -gt $timeout ]; then
            log_error "Timeout waiting for pods"
            kubectl get pods -n "$namespace" $selector_arg
            return 1
        fi
        
        local not_ready=$(kubectl get pods -n "$namespace" $selector_arg -o jsonpath='{.items[*].status.conditions[?(@.type=="Ready")].status}' 2>/dev/null | grep -c "False" || echo "0")
        local pending=$(kubectl get pods -n "$namespace" $selector_arg --field-selector=status.phase=Pending 2>/dev/null | wc -l || echo "0")
        
        if [ "$not_ready" -eq 0 ] && [ "$pending" -le 1 ]; then
            log_success "All pods are ready!"
            return 0
        fi
        
        echo -ne "\r  Waiting... ($elapsed/$timeout seconds, $not_ready not ready)    "
        sleep 5
    done
}

#------------------------------------------------------------------------------
# Main Functions
#------------------------------------------------------------------------------

check_prerequisites() {
    log_info "Checking prerequisites..."
    
    check_command minikube
    check_command kubectl
    check_command helm
    check_command docker
    
    # Check Docker is running
    if ! docker info &> /dev/null; then
        log_error "Docker is not running. Please start Docker and try again."
        exit 1
    fi
    
    log_success "All prerequisites satisfied!"
}

start_minikube() {
    log_info "Starting minikube cluster '$MINIKUBE_PROFILE'..."
    
    # Check if cluster already exists
    if minikube status -p "$MINIKUBE_PROFILE" &> /dev/null; then
        log_info "Cluster already exists, starting..."
        minikube start -p "$MINIKUBE_PROFILE"
    else
        log_info "Creating new cluster..."
        minikube start \
            -p "$MINIKUBE_PROFILE" \
            --cpus="$MINIKUBE_CPUS" \
            --memory="$MINIKUBE_MEMORY" \
            --disk-size="$MINIKUBE_DISK" \
            --driver=docker \
            --addons=ingress \
            --addons=metrics-server \
            --addons=dashboard
    fi
    
    # Set kubectl context
    kubectl config use-context "$MINIKUBE_PROFILE"
    
    log_success "Minikube cluster is running!"
}

build_images() {
    log_info "Building Docker images..."
    
    # Point docker to minikube's docker daemon
    eval $(minikube -p "$MINIKUBE_PROFILE" docker-env)
    
    cd "$PROJECT_ROOT"
    
    # Build each service
    for service in ingester renderer-worker wms-api; do
        log_info "Building $service..."
        
        # Create Dockerfile if it doesn't exist
        local dockerfile="services/$service/Dockerfile"
        if [ ! -f "$dockerfile" ]; then
            create_dockerfile "$service"
        fi
        
        docker build \
            -f "$dockerfile" \
            -t "weather-wms/$service:latest" \
            .
        
        log_success "Built $service"
    done
    
    log_success "All images built!"
}

create_dockerfile() {
    local service=$1
    local dockerfile="services/$service/Dockerfile"
    
    log_info "Creating Dockerfile for $service..."
    
    mkdir -p "$(dirname "$dockerfile")"
    
    cat > "$dockerfile" << 'EOF'
# Build stage
FROM rust:1.75-bookworm as builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services

# Build the service
ARG SERVICE_NAME
RUN cargo build --release --package ${SERVICE_NAME}

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

ARG SERVICE_NAME
COPY --from=builder /app/target/release/${SERVICE_NAME} /usr/local/bin/app

# Create non-root user
RUN useradd -m -u 1000 appuser
USER appuser

ENTRYPOINT ["/usr/local/bin/app"]
EOF

    # Update dockerfile with specific service name
    sed -i.bak "s/\${SERVICE_NAME}/$service/g" "$dockerfile"
    rm -f "${dockerfile}.bak"
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
}

deploy_helm_chart() {
    log_info "Deploying Weather WMS Helm chart..."
    
    cd "$PROJECT_ROOT/deploy/helm"
    
    # Update dependencies
    helm dependency update weather-wms
    
    # Install/upgrade the chart
    helm upgrade --install "$HELM_RELEASE" weather-wms \
        --namespace "$NAMESPACE" \
        --create-namespace \
        --values weather-wms/values.yaml \
        --set api.image.pullPolicy=Never \
        --set renderer.image.pullPolicy=Never \
        --set ingester.image.pullPolicy=Never \
        --wait \
        --timeout 10m
    
    log_success "Helm chart deployed!"
}

deploy_dev_stack() {
    log_info "Deploying development stack (standalone components)..."
    
    # Deploy MinIO, PostgreSQL, and Redis using Helm directly for dev simplicity
    
    # Redis
    log_info "Deploying Redis..."
    helm upgrade --install redis bitnami/redis \
        --namespace "$NAMESPACE" \
        --set architecture=standalone \
        --set auth.enabled=false \
        --set master.persistence.enabled=false \
        --wait --timeout 5m || true
    
    # PostgreSQL
    log_info "Deploying PostgreSQL..."
    helm upgrade --install postgresql bitnami/postgresql \
        --namespace "$NAMESPACE" \
        --set auth.username=weatherwms \
        --set auth.password=weatherwms \
        --set auth.database=weatherwms \
        --set primary.persistence.enabled=false \
        --wait --timeout 5m || true
    
    # MinIO
    log_info "Deploying MinIO..."
    helm upgrade --install minio bitnami/minio \
        --namespace "$NAMESPACE" \
        --set auth.rootUser=minioadmin \
        --set auth.rootPassword=minioadmin \
        --set persistence.enabled=false \
        --set defaultBuckets=weather-data \
        --wait --timeout 5m || true
    
    log_success "Development stack deployed!"
}

show_status() {
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
    
    echo ""
    echo "Ingress:"
    kubectl get ingress -n "$NAMESPACE" || true
}

show_access_info() {
    echo ""
    log_success "=== Access Information ==="
    echo ""
    
    # Get minikube IP
    local minikube_ip=$(minikube ip -p "$MINIKUBE_PROFILE")
    
    echo "Minikube IP: $minikube_ip"
    echo ""
    echo "To access services:"
    echo ""
    echo "  WMS API (via port-forward):"
    echo "    kubectl port-forward -n $NAMESPACE svc/${HELM_RELEASE}-weather-wms-api 8080:8080"
    echo "    Then open: http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities"
    echo ""
    echo "  MinIO Console:"
    echo "    kubectl port-forward -n $NAMESPACE svc/minio 9001:9001"
    echo "    Then open: http://localhost:9001 (minioadmin/minioadmin)"
    echo ""
    echo "  PostgreSQL:"
    echo "    kubectl port-forward -n $NAMESPACE svc/postgresql 5432:5432"
    echo "    psql -h localhost -U weatherwms -d weatherwms"
    echo ""
    echo "  Redis:"
    echo "    kubectl port-forward -n $NAMESPACE svc/redis-master 6379:6379"
    echo "    redis-cli -h localhost"
    echo ""
    echo "  Kubernetes Dashboard:"
    echo "    minikube dashboard -p $MINIKUBE_PROFILE"
    echo ""
    echo "To view logs:"
    echo "    kubectl logs -n $NAMESPACE -l app.kubernetes.io/component=api -f"
    echo "    kubectl logs -n $NAMESPACE -l app.kubernetes.io/component=renderer -f"
    echo "    kubectl logs -n $NAMESPACE -l app.kubernetes.io/component=ingester -f"
    echo ""
}

stop_cluster() {
    log_info "Stopping minikube cluster..."
    minikube stop -p "$MINIKUBE_PROFILE"
    log_success "Cluster stopped!"
}

clean_all() {
    log_warn "This will delete the entire minikube cluster and all data!"
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
# Development Helpers
#------------------------------------------------------------------------------

dev_build_and_deploy() {
    log_info "Building and deploying services for development..."
    
    # Point to minikube docker
    eval $(minikube -p "$MINIKUBE_PROFILE" docker-env)
    
    # Build images
    build_images
    
    # Restart deployments to pick up new images
    kubectl rollout restart deployment -n "$NAMESPACE" -l app.kubernetes.io/name=weather-wms || true
    
    log_success "Services rebuilt and redeployed!"
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
    
    case "${1:-}" in
        --quick)
            check_prerequisites
            start_minikube
            setup_namespace
            deploy_dev_stack
            wait_for_pods "$NAMESPACE" 300
            show_status
            show_access_info
            ;;
        --clean)
            clean_all
            ;;
        --stop)
            stop_cluster
            ;;
        --status)
            show_status
            ;;
        --rebuild)
            dev_build_and_deploy
            ;;
        --help|-h)
            echo "Usage: $0 [option]"
            echo ""
            echo "Options:"
            echo "  (none)      Full setup - creates cluster, builds images, deploys everything"
            echo "  --quick     Quick start - skip image building"
            echo "  --clean     Delete everything and start fresh"
            echo "  --stop      Stop the cluster (preserves data)"
            echo "  --status    Show status of all components"
            echo "  --rebuild   Rebuild images and redeploy"
            echo "  --help      Show this help message"
            echo ""
            ;;
        *)
            # Full setup
            check_prerequisites
            start_minikube
            setup_namespace
            deploy_dependencies
            build_images
            deploy_dev_stack
            wait_for_pods "$NAMESPACE" 300
            show_status
            show_access_info
            ;;
    esac
}

main "$@"
