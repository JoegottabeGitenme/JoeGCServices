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
#   ./start.sh --kubernetes # Full Kubernetes setup with minikube
#   ./start.sh --k8s        # Same as --kubernetes
#   ./start.sh --stop       # Stop docker-compose
#   ./start.sh --clean      # Delete everything and start fresh
#   ./start.sh --status     # Show status
#   ./start.sh --help       # Show this help message
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
    if ! docker info &> /dev/null; then
        log_error "Docker is not running. Please start Docker and try again."
        exit 1
    fi
    
    log_success "All prerequisites satisfied!"
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
    
    docker-compose up -d
    
    # Wait for services to be ready
    log_info "Waiting for services to be ready..."
    local retries=30
    while [ $retries -gt 0 ]; do
        if docker-compose exec -T postgres pg_isready -U weatherwms &>/dev/null && \
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
    echo "  PostgreSQL:  localhost:5432  (weatherwms/weatherwms)"
    echo "  Redis:       localhost:6379  (no auth)"
    echo "  MinIO:       localhost:9000  (minioadmin/minioadmin)"
    echo "  MinIO UI:    localhost:9001  (minioadmin/minioadmin)"
}

show_compose_access_info() {
    echo ""
    log_success "=== Quick Start ==="
    echo ""
    echo "Run the WMS API in another terminal:"
    echo "  cargo run --bin wms-api"
    echo ""
    echo "Or with debug logging:"
    echo "  RUST_LOG=debug cargo run --bin wms-api"
    echo ""
    echo "Test it:"
    echo "  curl \"http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities\""
    echo ""
    log_success "=== Service Credentials ==="
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
    echo "Run tests:"
    echo "  cargo test"
    echo ""
    echo "View service logs:"
    echo "  docker-compose logs -f postgresql"
    echo "  docker-compose logs -f redis"
    echo "  docker-compose logs -f minio"
    echo ""
    echo "Stop services:"
    echo "  ./start.sh --stop"
    echo ""
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
    
    # Enable only essential addons (dashboard often has DNS issues)
    minikube addons enable ingress -p "$MINIKUBE_PROFILE" || true
    minikube addons enable metrics-server -p "$MINIKUBE_PROFILE" || true
    
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
}

deploy_dev_stack() {
    log_info "Deploying development stack..."
    
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

wait_for_pods() {
    local namespace=$1
    local timeout=${2:-300}
    
    log_info "Waiting for pods in namespace '$namespace' to be ready..."
    
    local start_time=$(date +%s)
    while true; do
        local current_time=$(date +%s)
        local elapsed=$((current_time - start_time))
        
        if [ $elapsed -gt $timeout ]; then
            log_error "Timeout waiting for pods"
            kubectl get pods -n "$namespace"
            return 1
        fi
        
        local not_ready=$(kubectl get pods -n "$namespace" -o jsonpath='{.items[*].status.conditions[?(@.type=="Ready")].status}' 2>/dev/null | grep -c "False" || echo "0")
        
        if [ "$not_ready" -eq 0 ]; then
            log_success "All pods are ready!"
            return 0
        fi
        
        echo -ne "\rWaiting... ($elapsed/$timeout seconds, $not_ready not ready)    "
        sleep 5
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
    log_success "=== Kubernetes Cluster Running ==="
    echo ""
    
    local minikube_ip=$(minikube ip -p "$MINIKUBE_PROFILE")
    
    echo "Minikube IP: $minikube_ip"
    echo ""
    
    log_success "=== View Resources ==="
    echo ""
    echo "List all resources:"
    echo "  kubectl get all -n $NAMESPACE"
    echo ""
    echo "Watch pods live:"
    echo "  kubectl get pods -n $NAMESPACE -w"
    echo ""
    echo "View pod logs:"
    echo "  kubectl logs -n $NAMESPACE <pod-name> -f"
    echo ""
    echo "See MONITORING.md for 100+ more kubectl commands"
    echo ""
    
    log_success "=== Access Services ==="
    echo ""
    echo "PostgreSQL (port-forward in another terminal):"
    echo "  kubectl port-forward -n $NAMESPACE svc/postgresql 5432:5432"
    echo "  psql -h localhost -U weatherwms -d weatherwms"
    echo ""
    echo "Redis:"
    echo "  kubectl port-forward -n $NAMESPACE svc/redis-master 6379:6379"
    echo "  redis-cli -h localhost"
    echo ""
    echo "MinIO:"
    echo "  kubectl port-forward -n $NAMESPACE svc/minio 9000:9000"
    echo "  kubectl port-forward -n $NAMESPACE svc/minio 9001:9001"
    echo "  Access UI: http://localhost:9001 (minioadmin/minioadmin)"
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
    
    case "${1:-}" in
        --kubernetes|--k8s)
            check_k8s_prerequisites
            start_minikube
            setup_namespace
            deploy_dependencies
            deploy_dev_stack
            wait_for_pods "$NAMESPACE" 300
            show_k8s_status
            show_k8s_access_info
            ;;
        --compose|"")
            check_compose_prerequisites
            start_compose
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
            if minikube status -p "$MINIKUBE_PROFILE" &> /dev/null; then
                stop_k8s
            fi
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
        --status)
            if minikube status -p "$MINIKUBE_PROFILE" &> /dev/null; then
                show_k8s_status
            else
                show_compose_status
            fi
            ;;
        --help|-h)
            echo "Usage: $0 [option]"
            echo ""
            echo "Options:"
            echo "  (none)         Start with docker-compose (RECOMMENDED - fast)"
            echo "  --compose      Start with docker-compose"
            echo "  --kubernetes   Full Kubernetes setup with minikube (slower)"
            echo "  --k8s          Same as --kubernetes"
            echo "  --stop         Stop docker-compose or minikube"
            echo "  --clean        Delete everything and start fresh"
            echo "  --status       Show status of services"
            echo "  --help         Show this help message"
            echo ""
            echo "RECOMMENDED WORKFLOW:"
            echo "  1. Run: ./start.sh          (starts docker-compose)"
            echo "  2. In another terminal:"
            echo "     cargo run --bin wms-api"
            echo "  3. Test:"
            echo "     curl 'http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities'"
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
