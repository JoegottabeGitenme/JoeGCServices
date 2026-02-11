#!/usr/bin/env bash
#
# Weather WMS - Remote Production Deployment
# =============================================================================
# Deploys weather-wms services to a remote server.
#
# Architecture:
#   - Gateway (/opt/gateway) handles nginx, Cloudflare tunnel, TLS, and routing
#   - This script deploys only the weather services stack (/opt/weather-wms)
#   - Services join the shared-services Docker network for gateway access
#   - Firewall configuration (UFW)
#   - Auto-generated secure passwords
#
# Usage:
#   ./scripts/deploy-remote.sh              # Full deployment
#   ./scripts/deploy-remote.sh --update     # Update config, restart services
#   ./scripts/deploy-remote.sh --rebuild    # Rebuild images and redeploy
#   ./scripts/deploy-remote.sh --ingest-viirs # Ingest VIIRS light pollution data
#   ./scripts/deploy-remote.sh --status     # Check deployment status
#   ./scripts/deploy-remote.sh --logs [svc] # View remote logs
#   ./scripts/deploy-remote.sh --ssh        # SSH to remote server
#   ./scripts/deploy-remote.sh --help       # Show help/
#
# Prerequisites:
#   - .env.nuc file configured (copy from .env.nuc.example)
#   - SSH key access to remote server
#   - Gateway deployed at /opt/gateway on the remote server
#   - shared-services Docker network created on the remote server
#   - Docker installed on remote server
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ENV_FILE="$PROJECT_ROOT/.env.nuc"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

log_info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
log_warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error()   { echo -e "${RED}[ERROR]${NC} $1"; }
log_step()    { echo -e "${CYAN}[STEP]${NC} $1"; }

# =============================================================================
# HELPER FUNCTIONS
# =============================================================================

show_help() {
  cat << 'EOF'
Weather WMS - Remote Production Deployment

Usage:
  ./scripts/deploy-remote.sh [command]

Commands:
  (none)        Full deployment (first time or complete redeploy)
  --update      Update config files and restart services (no image rebuild)
  --rebuild     Rebuild images locally and redeploy
  --ingest-viirs  Ingest VIIRS light pollution data (one-time static dataset)
  --status      Check deployment status on remote
  --logs [svc]  View logs (optional: specify service name)
  --ssh         SSH to remote server
  --help        Show this help

Prerequisites:
  1. Copy .env.nuc.example to .env.nuc and fill in your values
  2. Set up SSH key access to your server
  3. Ensure Docker is installed on the remote server
  4. Deploy the gateway at /opt/gateway (handles nginx + Cloudflare tunnel)
  5. Create the shared-services Docker network: docker network create shared-services

Example:
  # First deployment
  cp .env.nuc.example .env.nuc
  # Edit .env.nuc with your settings
  ./scripts/deploy-remote.sh

  # Later updates
  ./scripts/deploy-remote.sh --update      # Config changes only
  ./scripts/deploy-remote.sh --rebuild     # Code changes

EOF
}

generate_password() {
  openssl rand -base64 32 | tr -dc 'a-zA-Z0-9' | head -c 32
}

ssh_cmd() {
  ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=accept-new "$REMOTE_HOST" "$@"
}

scp_cmd() {
  scp -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=accept-new "$@"
}

rsync_cmd() {
  rsync -avz --progress -e "ssh -i $SSH_KEY_PATH -o StrictHostKeyChecking=accept-new" "$@"
}

# =============================================================================
# PHASE 1: LOAD & VALIDATE CONFIGURATION
# =============================================================================

load_config() {
  log_step "Phase 1: Loading configuration..."
  
  if [[ ! -f "$ENV_FILE" ]]; then
    log_error "Configuration file not found: $ENV_FILE"
    echo ""
    echo "Please create it from the example:"
    echo "  cp .env.nuc.example .env.nuc"
    echo "  # Edit .env.nuc with your settings"
    echo ""
    exit 1
  fi
  
  # Source the config
  set -a
  source "$ENV_FILE"
  set +a
  
  # Validate required fields
  local missing=()
  [[ -z "${REMOTE_HOST:-}" ]] && missing+=("REMOTE_HOST")
  [[ -z "${DOMAIN:-}" ]] && missing+=("DOMAIN")
  [[ -z "${SSH_KEY_PATH:-}" ]] && missing+=("SSH_KEY_PATH")

  if [[ ${#missing[@]} -gt 0 ]]; then
    log_error "Missing required configuration:"
    for field in "${missing[@]}"; do
      echo "  - $field"
    done
    echo ""
    echo "Please edit $ENV_FILE and fill in the required values."
    exit 1
  fi
  
  # Expand SSH key path
  SSH_KEY_PATH="${SSH_KEY_PATH/#\~/$HOME}"
  
  if [[ ! -f "$SSH_KEY_PATH" ]]; then
    log_error "SSH key not found: $SSH_KEY_PATH"
    echo ""
    echo "Please create an SSH key or update SSH_KEY_PATH in $ENV_FILE"
    echo "  ssh-keygen -t ed25519 -f ~/.ssh/id_nuc"
    echo "  ssh-copy-id -i ~/.ssh/id_nuc.pub $REMOTE_HOST"
    exit 1
  fi
  
  log_success "Configuration loaded from $ENV_FILE"
}

# =============================================================================
# PHASE 2: GENERATE SECRETS
# =============================================================================

generate_secrets() {
  log_step "Phase 2: Generating secrets..."
  
  local updated=false
  
  # Generate passwords for blank fields
  if [[ -z "${ADMIN_PASSWORD:-}" ]]; then
    ADMIN_PASSWORD=$(generate_password)
    log_info "Generated ADMIN_PASSWORD"
    updated=true
  fi
  
  if [[ -z "${POSTGRES_PASSWORD:-}" ]]; then
    POSTGRES_PASSWORD=$(generate_password)
    log_info "Generated POSTGRES_PASSWORD"
    updated=true
  fi
  
  if [[ -z "${REDIS_PASSWORD:-}" ]]; then
    REDIS_PASSWORD=$(generate_password)
    log_info "Generated REDIS_PASSWORD"
    updated=true
  fi
  
  if [[ -z "${S3_ACCESS_KEY:-}" ]]; then
    S3_ACCESS_KEY=$(generate_password | head -c 20)
    log_info "Generated S3_ACCESS_KEY"
    updated=true
  fi
  
  if [[ -z "${S3_SECRET_KEY:-}" ]]; then
    S3_SECRET_KEY=$(generate_password)
    log_info "Generated S3_SECRET_KEY"
    updated=true
  fi
  
  if [[ -z "${GRAFANA_ADMIN_PASSWORD:-}" ]]; then
    GRAFANA_ADMIN_PASSWORD=$(generate_password)
    log_info "Generated GRAFANA_ADMIN_PASSWORD"
    updated=true
  fi
  
  # Save generated passwords back to .env.nuc
  if [[ "$updated" == "true" ]]; then
    log_info "Saving generated passwords to $ENV_FILE..."
    
    # Update the file with generated values
    sed -i "s|^ADMIN_PASSWORD=.*|ADMIN_PASSWORD=$ADMIN_PASSWORD|" "$ENV_FILE"
    sed -i "s|^POSTGRES_PASSWORD=.*|POSTGRES_PASSWORD=$POSTGRES_PASSWORD|" "$ENV_FILE"
    sed -i "s|^REDIS_PASSWORD=.*|REDIS_PASSWORD=$REDIS_PASSWORD|" "$ENV_FILE"
    sed -i "s|^S3_ACCESS_KEY=.*|S3_ACCESS_KEY=$S3_ACCESS_KEY|" "$ENV_FILE"
    sed -i "s|^S3_SECRET_KEY=.*|S3_SECRET_KEY=$S3_SECRET_KEY|" "$ENV_FILE"
    sed -i "s|^GRAFANA_ADMIN_PASSWORD=.*|GRAFANA_ADMIN_PASSWORD=$GRAFANA_ADMIN_PASSWORD|" "$ENV_FILE"
    
    log_success "Passwords saved to $ENV_FILE (keep this file safe!)"
  else
    log_info "All passwords already set"
  fi
}

# =============================================================================
# PHASE 3: VALIDATE REMOTE SERVER
# =============================================================================

validate_remote() {
  log_step "Phase 3: Validating remote server..."
  
  # Test SSH connection
  log_info "Testing SSH connection to $REMOTE_HOST..."
  if ! ssh_cmd "echo 'SSH connection successful'" 2>/dev/null; then
    log_error "Cannot connect to $REMOTE_HOST"
    echo ""
    echo "Please ensure:"
    echo "  1. The server is running and accessible"
    echo "  2. Your SSH key is authorized on the server"
    echo "  3. Run: ssh-copy-id -i $SSH_KEY_PATH $REMOTE_HOST"
    exit 1
  fi
  log_success "SSH connection successful"
  
  # Check Docker
  log_info "Checking Docker installation..."
  if ! ssh_cmd "docker --version" 2>/dev/null; then
    log_error "Docker is not installed on $REMOTE_HOST"
    echo ""
    echo "Please install Docker on the remote server:"
    echo "  curl -fsSL https://get.docker.com | sh"
    echo "  sudo usermod -aG docker \$USER"
    exit 1
  fi
  local docker_version=$(ssh_cmd "docker --version" 2>/dev/null)
  log_success "Docker installed: $docker_version"
  
  # Check user is in docker group (can run docker without sudo)
  log_info "Checking Docker permissions..."
  if ! ssh_cmd "docker ps" 2>/dev/null; then
    log_error "User cannot access Docker without sudo"
    echo ""
    echo "Please add your user to the docker group on the remote server:"
    echo "  sudo usermod -aG docker \$USER"
    echo ""
    echo "Then log out and back in (or reboot), and re-run this script."
    exit 1
  fi
  log_success "Docker permissions OK"
  
  # Check Docker Compose
  log_info "Checking Docker Compose..."
  local compose_cmd=""
  if ssh_cmd "docker compose version" 2>/dev/null; then
    compose_cmd="docker compose"
    local compose_version=$(ssh_cmd "docker compose version" 2>/dev/null)
    log_success "Docker Compose (plugin): $compose_version"
  elif ssh_cmd "docker-compose --version" 2>/dev/null; then
    compose_cmd="docker-compose"
    local compose_version=$(ssh_cmd "docker-compose --version" 2>/dev/null)
    log_success "Docker Compose (standalone): $compose_version"
  else
    log_error "Docker Compose is not installed"
    echo ""
    echo "Please install Docker Compose on the remote server"
    exit 1
  fi
  
  # Store compose command for later use
  COMPOSE_CMD="$compose_cmd"
  
  # Check rsync is installed on remote
  log_info "Checking rsync..."
  if ! ssh_cmd "which rsync" &>/dev/null; then
    log_error "rsync is not installed on $REMOTE_HOST"
    echo ""
    echo "Please install rsync on the remote server:"
    echo "  sudo apt install -y rsync"
    exit 1
  fi
  log_success "rsync is installed"
  
  # Check jq for better JSON parsing (optional but recommended)
  log_info "Checking jq..."
  if ssh_cmd "which jq" &>/dev/null; then
    log_success "jq is installed (recommended for health checks)"
  else
    log_warn "jq not installed - health checks will use fallback grep method"
    log_info "  For more reliable health checks, install jq: sudo apt install -y jq"
  fi
  
  # Check disk space
  log_info "Checking disk space..."
  local disk_free=$(ssh_cmd "df -h $REMOTE_DIR 2>/dev/null | tail -1 | awk '{print \$4}'" || echo "unknown")
  log_info "Available disk space: $disk_free"
}

# =============================================================================
# PHASE 4: SETUP REMOTE SERVER
# =============================================================================

setup_remote() {
  log_step "Phase 4: Setting up remote server..."
  
  # Create deployment directory
  log_info "Creating deployment directory: $REMOTE_DIR"
  ssh_cmd "sudo mkdir -p $REMOTE_DIR && sudo chown \$USER:\$USER $REMOTE_DIR"
  
  # Ensure shared-services network exists
  log_info "Ensuring shared-services Docker network exists..."
  ssh_cmd "docker network create shared-services 2>/dev/null || true"
  
  # Configure firewall (UFW)
  # Note: On Debian, ufw is in /usr/sbin which may not be in PATH for non-root users
  log_info "Configuring firewall..."
  if ssh_cmd "which ufw >/dev/null 2>&1 || test -x /usr/sbin/ufw" &>/dev/null; then
    ssh_cmd "sudo /usr/sbin/ufw default deny incoming 2>/dev/null || true"
    ssh_cmd "sudo /usr/sbin/ufw default allow outgoing 2>/dev/null || true"
    ssh_cmd "sudo /usr/sbin/ufw allow 22/tcp 2>/dev/null || true"   # SSH
    ssh_cmd "sudo /usr/sbin/ufw allow 80/tcp 2>/dev/null || true"   # HTTP
    ssh_cmd "sudo /usr/sbin/ufw allow 443/tcp 2>/dev/null || true"  # HTTPS
    ssh_cmd "sudo /usr/sbin/ufw --force enable 2>/dev/null || true"
    log_success "Firewall configured (ports 22, 80, 443 open)"
  else
    log_warn "UFW not installed - skipping firewall configuration"
    log_warn "Please manually configure your firewall to allow ports 22, 80, 443"
  fi
}

# =============================================================================
# PHASE 5: BUILD IMAGES LOCALLY
# =============================================================================

build_images() {
  log_step "Phase 5: Building Docker images locally..."
  
  cd "$PROJECT_ROOT"
  
  # Set release profile
  export CARGO_PROFILE=release
  
  log_info "Building application images (this may take 10-20 minutes for first build)..."
  docker compose build
  
  # Note: nginx is managed by the gateway (/opt/gateway), not this stack
  
  log_success "All images built successfully"
}

# =============================================================================
# PHASE 6: TRANSFER IMAGES
# =============================================================================

transfer_images() {
  log_step "Phase 6: Transferring images to remote server..."
  
  # Only transfer custom-built images
  # Base images (postgres, redis, minio, grafana, etc.) will be pulled on remote
  # Note: nginx is managed by the gateway (/opt/gateway), not this stack
  local images=(
    "weather-wms-wms-api:latest"
    "weather-wms-edr-api:latest"
    "weather-wms-ingester:latest"
    "weather-wms-downloader:latest"
  )
  
  local archive="/tmp/weather-wms-images.tar.gz"
  
  log_info "Saving ${#images[@]} custom images to archive..."
  docker save "${images[@]}" | gzip > "$archive"
  
  local size=$(du -h "$archive" | cut -f1)
  log_info "Archive size: $size"
  
  log_info "Transferring to remote server (this may take a while)..."
  scp_cmd "$archive" "$REMOTE_HOST:/tmp/"
  
  log_info "Loading images on remote server..."
  ssh_cmd "gunzip -c /tmp/weather-wms-images.tar.gz | docker load"
  
  # Cleanup
  rm -f "$archive"
  ssh_cmd "rm -f /tmp/weather-wms-images.tar.gz"
  
  log_info "Pulling base images on remote (postgres, redis, minio, etc.)..."
  ssh_cmd "docker pull python:3.11-slim" || log_warn "Failed to pull python:3.11-slim"
  ssh_cmd "docker pull postgis/postgis:16-3.4" || log_warn "Failed to pull postgis"
  ssh_cmd "docker pull redis:7-bookworm" || log_warn "Failed to pull redis"
  ssh_cmd "docker pull minio/minio:latest" || log_warn "Failed to pull minio"
  ssh_cmd "docker pull grafana/grafana:latest" || log_warn "Failed to pull grafana"
  ssh_cmd "docker pull grafana/loki:latest" || log_warn "Failed to pull loki"
  ssh_cmd "docker pull grafana/promtail:latest" || log_warn "Failed to pull promtail"
  ssh_cmd "docker pull prom/prometheus:latest" || log_warn "Failed to pull prometheus"
  ssh_cmd "docker pull minio/mc:latest" || log_warn "Failed to pull minio client"
  
  log_success "Images transferred and base images pulled"
}

# =============================================================================
# PHASE 7: SYNC FILES
# =============================================================================

sync_files() {
  log_step "Phase 7: Syncing deployment files..."
  
  cd "$PROJECT_ROOT"
  
  # Create remote directory structure
  ssh_cmd "mkdir -p $REMOTE_DIR/deploy/production"
  ssh_cmd "mkdir -p $REMOTE_DIR/deploy/grafana/provisioning"
  ssh_cmd "mkdir -p $REMOTE_DIR/deploy/prometheus"
  ssh_cmd "mkdir -p $REMOTE_DIR/deploy/loki"
  ssh_cmd "mkdir -p $REMOTE_DIR/deploy/promtail"
  ssh_cmd "mkdir -p $REMOTE_DIR/config"
  ssh_cmd "mkdir -p $REMOTE_DIR/web"
  
  # Sync files
  log_info "Syncing docker-compose files..."
  scp_cmd docker-compose.yml "$REMOTE_HOST:$REMOTE_DIR/"
  scp_cmd deploy/production/docker-compose.prod.yml "$REMOTE_HOST:$REMOTE_DIR/deploy/production/"
  
  log_info "Syncing configuration..."
  rsync_cmd config/ "$REMOTE_HOST:$REMOTE_DIR/config/"
  rsync_cmd web/ "$REMOTE_HOST:$REMOTE_DIR/web/"
  rsync_cmd deploy/grafana/ "$REMOTE_HOST:$REMOTE_DIR/deploy/grafana/"
  rsync_cmd deploy/prometheus/ "$REMOTE_HOST:$REMOTE_DIR/deploy/prometheus/"
  rsync_cmd deploy/loki/ "$REMOTE_HOST:$REMOTE_DIR/deploy/loki/"
  rsync_cmd deploy/promtail/ "$REMOTE_HOST:$REMOTE_DIR/deploy/promtail/"
  
  # Note: nginx, TLS certs, and .htpasswd are managed by the gateway (/opt/gateway)
  
  # Generate .env file for remote
  log_info "Generating remote .env..."
  cat > /tmp/.env.remote << EOF
# Generated by deploy-remote.sh on $(date)
# DO NOT EDIT - regenerated on each deployment

# Domain
DOMAIN=$DOMAIN

# PostgreSQL
POSTGRES_USER=${POSTGRES_USER:-weatherwms}
POSTGRES_PASSWORD=$POSTGRES_PASSWORD
POSTGRES_DB=${POSTGRES_DB:-weatherwms}

# Redis
REDIS_PASSWORD=$REDIS_PASSWORD

# MinIO
S3_ACCESS_KEY=$S3_ACCESS_KEY
S3_SECRET_KEY=$S3_SECRET_KEY
S3_BUCKET=${S3_BUCKET:-weather-data}
S3_REGION=${S3_REGION:-us-east-1}

# Grafana
GRAFANA_ADMIN_PASSWORD=$GRAFANA_ADMIN_PASSWORD

# Performance
TOKIO_WORKER_THREADS=${TOKIO_WORKER_THREADS:-16}
DATABASE_POOL_SIZE=${DATABASE_POOL_SIZE:-50}
ENABLE_L1_CACHE=${ENABLE_L1_CACHE:-true}
TILE_CACHE_SIZE_MB=${TILE_CACHE_SIZE_MB:-8192}
TILE_CACHE_TTL_SECS=${TILE_CACHE_TTL_SECS:-600}
CHUNK_CACHE_SIZE_MB=${CHUNK_CACHE_SIZE_MB:-8192}
ENABLE_PREFETCH=${ENABLE_PREFETCH:-true}
PREFETCH_RINGS=${PREFETCH_RINGS:-2}
PREFETCH_MIN_ZOOM=${PREFETCH_MIN_ZOOM:-3}
PREFETCH_MAX_ZOOM=${PREFETCH_MAX_ZOOM:-12}
ENABLE_CACHE_WARMING=${ENABLE_CACHE_WARMING:-true}
CACHE_WARMING_MAX_ZOOM=${CACHE_WARMING_MAX_ZOOM:-4}
CACHE_WARMING_HOURS=${CACHE_WARMING_HOURS:-0}
CACHE_WARMING_LAYERS=${CACHE_WARMING_LAYERS:-gfs_TMP:temperature}
CACHE_WARMING_CONCURRENCY=${CACHE_WARMING_CONCURRENCY:-8}
ENABLE_MEMORY_PRESSURE=${ENABLE_MEMORY_PRESSURE:-true}
MEMORY_LIMIT_MB=${MEMORY_LIMIT_MB:-28000}
MEMORY_PRESSURE_THRESHOLD=${MEMORY_PRESSURE_THRESHOLD:-0.80}
MEMORY_PRESSURE_TARGET=${MEMORY_PRESSURE_TARGET:-0.70}

# Logging
RUST_LOG=${RUST_LOG:-info}
RUST_BACKTRACE=${RUST_BACKTRACE:-0}

# Monitoring
PROMETHEUS_RETENTION_DAYS=${PROMETHEUS_RETENTION_DAYS:-30}

# Note: Cloudflare tunnel/DDNS config is in /opt/gateway/.env
EOF
  scp_cmd /tmp/.env.remote "$REMOTE_HOST:$REMOTE_DIR/.env"
  rm /tmp/.env.remote
  
  log_success "Files synced"
}

# =============================================================================
# PHASE 8: SETUP TLS
# =============================================================================

setup_tls() {
  log_step "Phase 8: Setting up TLS (Cloudflare Origin Certificate)..."
  
  # TLS certs are now managed by the gateway at /opt/gateway/ssl/
  local cert_dir="$PROJECT_ROOT/deploy/production/nginx/ssl"
  local gateway_ssl_dir="/opt/gateway/ssl"
  
  if [[ ! -f "$cert_dir/origin.cert" ]] || [[ ! -f "$cert_dir/origin.key" ]]; then
    log_error "Cloudflare origin certificate files not found!"
    echo ""
    echo "Please create the SSL directory and add your Cloudflare origin certificates:"
    echo "  mkdir -p $cert_dir"
    echo "  # Copy your origin.cert (certificate) to: $cert_dir/origin.cert"
    echo "  # Copy your origin.key (private key) to: $cert_dir/origin.key"
    echo ""
    echo "To create an origin certificate:"
    echo "  1. Go to Cloudflare Dashboard -> SSL/TLS -> Origin Server"
    echo "  2. Click 'Create Certificate'"
    echo "  3. Save the certificate as origin.cert and private key as origin.key"
    exit 1
  fi
  
  log_info "Found Cloudflare origin certificate files"
  
  # Upload certs to the gateway's ssl directory
  log_info "Uploading certificates to gateway ($gateway_ssl_dir)..."
  ssh_cmd "mkdir -p $gateway_ssl_dir"
  scp_cmd "$cert_dir/origin.cert" "$REMOTE_HOST:$gateway_ssl_dir/origin.cert"
  scp_cmd "$cert_dir/origin.key" "$REMOTE_HOST:$gateway_ssl_dir/origin.key"
  
  # Set proper permissions
  ssh_cmd "chmod 600 $gateway_ssl_dir/origin.key"
  ssh_cmd "chmod 644 $gateway_ssl_dir/origin.cert"
  
  log_success "Cloudflare origin certificates installed in gateway"
  log_info "Make sure Cloudflare SSL/TLS mode is set to 'Full (Strict)'"
}

# =============================================================================
# PHASE 9: START SERVICES
# =============================================================================

start_services() {
  log_step "Phase 9: Starting services..."
  
  cd "$PROJECT_ROOT"
  
  # Ensure shared-services network exists
  log_info "Ensuring shared-services network exists..."
  ssh_cmd "docker network create shared-services 2>/dev/null || true"
  
  log_info "Starting all services..."
  ssh_cmd "cd $REMOTE_DIR && $COMPOSE_CMD -f docker-compose.yml -f deploy/production/docker-compose.prod.yml up -d"
  
  # Wait for services to be healthy
  log_info "Waiting for services to be healthy (this may take a few minutes)..."
  local retries=60
  while [[ $retries -gt 0 ]]; do
    # Use jq if available, fall back to grep
    local healthy
    if ssh_cmd "which jq >/dev/null 2>&1"; then
      healthy=$(ssh_cmd "cd $REMOTE_DIR && $COMPOSE_CMD -f docker-compose.yml -f deploy/production/docker-compose.prod.yml ps --format json 2>/dev/null | jq -s '[.[] | select(.Health == \"healthy\")] | length'" 2>/dev/null || echo "0")
    else
      healthy=$(ssh_cmd "cd $REMOTE_DIR && $COMPOSE_CMD -f docker-compose.yml -f deploy/production/docker-compose.prod.yml ps --format json 2>/dev/null | grep -c 'healthy'" 2>/dev/null || echo "0")
    fi
    local total=$(ssh_cmd "cd $REMOTE_DIR && $COMPOSE_CMD -f docker-compose.yml -f deploy/production/docker-compose.prod.yml ps -q 2>/dev/null | wc -l" 2>/dev/null || echo "0")
    
    echo -ne "\r  Services healthy: $healthy (waiting... ${retries}s remaining)"
    
    # Check if WMS API is responding via the gateway
    if ssh_cmd "curl -s -o /dev/null -w '%{http_code}' http://localhost:80/wms?SERVICE=WMS\\&REQUEST=GetCapabilities 2>/dev/null" | grep -q "200"; then
      echo ""
      log_success "Services are healthy and responding"
      break
    fi
    
    sleep 5
    retries=$((retries - 5))
  done
  
  if [[ $retries -le 0 ]]; then
    log_warn "Timeout waiting for services - they may still be starting"
    log_info "Check status with: ./scripts/deploy-remote.sh --status"
  fi
  
  # Restart gateway nginx to refresh upstream DNS for new/restarted services
  log_info "Restarting gateway nginx to refresh upstream DNS..."
  ssh_cmd "docker restart gateway-nginx" || log_warn "Could not restart gateway-nginx (is the gateway running?)"
}

# =============================================================================
# PHASE 10: VERIFY SECURITY
# =============================================================================

verify_security() {
  log_step "Phase 10: Verifying security..."
  
  local remote_ip=$(echo "$REMOTE_HOST" | cut -d@ -f2)
  local errors=0
  
  # Test public endpoints
  log_info "Testing public endpoints..."
  
  if ssh_cmd "curl -s -o /dev/null -w '%{http_code}' 'http://localhost:80/wms?SERVICE=WMS&REQUEST=GetCapabilities'" | grep -q "301\|200"; then
    log_success "  /wms endpoint accessible"
  else
    log_warn "  /wms endpoint may not be ready yet"
  fi
  
  # Test that internal ports are NOT accessible from outside
  log_info "Verifying internal ports are blocked..."
  for port in 5432 6379 9000 8082; do
    if timeout 2 nc -z "$remote_ip" "$port" 2>/dev/null; then
      log_error "  Port $port is accessible from outside (should be blocked)"
      errors=$((errors + 1))
    else
      log_success "  Port $port is blocked (good)"
    fi
  done
  
  if [[ $errors -gt 0 ]]; then
    log_warn "Some security checks failed - please review firewall settings"
  else
    log_success "Security verification passed"
  fi
}

# =============================================================================
# PHASE 11: PRINT SUMMARY
# =============================================================================

print_summary() {
  echo ""
  echo -e "${GREEN}╔═══════════════════════════════════════════════════════════════╗${NC}"
  echo -e "${GREEN}║           Deployment Complete!                                ║${NC}"
  echo -e "${GREEN}╚═══════════════════════════════════════════════════════════════╝${NC}"
  echo ""
  echo -e "${CYAN}Public Endpoints (no auth required):${NC}"
  echo "  WMS API:       https://$DOMAIN/wms"
  echo "  WMTS API:      https://$DOMAIN/wmts"
  echo "  EDR API:       https://$DOMAIN/edr"
  echo "  FolkWeather:   https://$DOMAIN/app/"
  echo ""
  echo -e "${CYAN}Admin Endpoints (basic auth required):${NC}"
  echo "  Dashboard:     https://$DOMAIN/"
  echo "  Grafana:       https://$DOMAIN/grafana/"
  echo "  MinIO:         https://$DOMAIN/minio/"
  echo "  Downloader:    https://$DOMAIN/downloader/"
  echo ""
  echo -e "${CYAN}Architecture:${NC}"
  echo "  Gateway:       /opt/gateway        (nginx + Cloudflare tunnel)"
  echo "  Weather WMS:   /opt/weather-wms    (weather services)"
  echo "  FolkWeather:   /opt/folkweather-app (PWA)"
  echo ""
  echo -e "${CYAN}Credentials:${NC}"
  echo "  Admin User:     ${ADMIN_USER:-admin}"
  echo "  Admin Password: $ADMIN_PASSWORD"
  echo ""
  echo "  (All credentials saved in $ENV_FILE)"
  echo ""
  echo -e "${CYAN}Useful Commands:${NC}"
  echo "  View logs:      ./scripts/deploy-remote.sh --logs"
  echo "  View status:    ./scripts/deploy-remote.sh --status"
  echo "  SSH to server:  ./scripts/deploy-remote.sh --ssh"
  echo "  Update config:  ./scripts/deploy-remote.sh --update"
  echo "  Full rebuild:   ./scripts/deploy-remote.sh --rebuild"
  echo ""
}

# =============================================================================
# ADDITIONAL COMMANDS
# =============================================================================

show_status() {
  load_config
  echo ""
  log_info "Deployment Status for $REMOTE_HOST"
  
  echo ""
  echo -e "${CYAN}--- Gateway (/opt/gateway) ---${NC}"
  ssh_cmd "cd /opt/gateway && docker compose ps --format 'table {{.Name}}\t{{.Status}}' 2>/dev/null" || echo "  Not deployed"
  
  echo ""
  echo -e "${CYAN}--- Weather WMS (/opt/weather-wms) ---${NC}"
  ssh_cmd "cd $REMOTE_DIR && docker compose -f docker-compose.yml -f deploy/production/docker-compose.prod.yml ps --format 'table {{.Name}}\t{{.Status}}'"
  
  echo ""
  echo -e "${CYAN}--- FolkWeather App (/opt/folkweather-app) ---${NC}"
  ssh_cmd "cd /opt/folkweather-app && docker compose -f docker-compose.prod.yml ps --format 'table {{.Name}}\t{{.Status}}' 2>/dev/null" || echo "  Not deployed"
  
  echo ""
  log_info "Disk Usage:"
  ssh_cmd "df -h $REMOTE_DIR | tail -1"
  echo ""
  log_info "Memory Usage:"
  ssh_cmd "free -h | head -2"
}

show_logs() {
  load_config
  local service="${1:-}"
  if [[ -n "$service" ]]; then
    ssh_cmd "cd $REMOTE_DIR && docker compose -f docker-compose.yml -f deploy/production/docker-compose.prod.yml logs -f $service"
  else
    ssh_cmd "cd $REMOTE_DIR && docker compose -f docker-compose.yml -f deploy/production/docker-compose.prod.yml logs -f"
  fi
}

do_ssh() {
  load_config
  ssh -i "$SSH_KEY_PATH" "$REMOTE_HOST"
}

do_update() {
  load_config
  generate_secrets
  validate_remote
  sync_files
  
  log_info "Restarting services..."
  ssh_cmd "cd $REMOTE_DIR && $COMPOSE_CMD -f docker-compose.yml -f deploy/production/docker-compose.prod.yml up -d"
  
  # Restart EDR API to reload config files (e.g., new collections)
  log_info "Restarting EDR API to reload config..."
  ssh_cmd "cd $REMOTE_DIR && $COMPOSE_CMD -f docker-compose.yml -f deploy/production/docker-compose.prod.yml restart edr-api" || true
  
  # Restart gateway nginx to refresh upstream DNS for restarted services
  log_info "Restarting gateway nginx to refresh upstream DNS..."
  ssh_cmd "docker restart gateway-nginx" || log_warn "Could not restart gateway-nginx (is the gateway running?)"
  
  log_success "Update complete!"
}

do_rebuild() {
  load_config
  generate_secrets
  validate_remote
  build_images
  transfer_images
  sync_files
  
  log_info "Restarting services..."
  ssh_cmd "cd $REMOTE_DIR && $COMPOSE_CMD -f docker-compose.yml -f deploy/production/docker-compose.prod.yml down"
  ssh_cmd "cd $REMOTE_DIR && $COMPOSE_CMD -f docker-compose.yml -f deploy/production/docker-compose.prod.yml up -d"
  
  # Restart gateway nginx to refresh upstream DNS for restarted services
  log_info "Restarting gateway nginx to refresh upstream DNS..."
  ssh_cmd "docker restart gateway-nginx" || log_warn "Could not restart gateway-nginx (is the gateway running?)"
  
  log_success "Rebuild complete!"
}

do_ingest_viirs() {
  load_config
  validate_remote
  
  log_step "Ingesting VIIRS light pollution data..."
  
  # Find VIIRS file in project root
  local viirs_file=$(ls "$PROJECT_ROOT"/VNL*.tif.gz 2>/dev/null | head -1)
  
  if [[ -z "$viirs_file" ]]; then
    log_error "VIIRS data file not found!"
    echo ""
    echo "Please download the VIIRS nighttime lights data:"
    echo "  1. Visit: https://eogdata.mines.edu/nighttime_light/annual/v22/"
    echo "  2. Download the latest VNL_*.tif.gz file"
    echo "  3. Place it in the project root: $PROJECT_ROOT/"
    echo ""
    echo "Example filename: VNL_npp_2024_global_vcmslcfg_v2_c202502261200.average_masked.dat.tif.gz"
    exit 1
  fi
  
  local viirs_filename=$(basename "$viirs_file")
  local viirs_size=$(du -h "$viirs_file" | cut -f1)
  log_info "Found VIIRS file: $viirs_filename ($viirs_size)"
  
  # Create static data directory on remote (mounted into ingester container)
  ssh_cmd "sudo mkdir -p $REMOTE_DIR/data/static/viirs && sudo chown -R \$USER:\$USER $REMOTE_DIR/data"
  
  # Check if file already exists on remote
  if ssh_cmd "test -f $REMOTE_DIR/data/static/viirs/$viirs_filename" 2>/dev/null; then
    log_info "VIIRS file already exists on remote server"
  else
    # Transfer VIIRS file to remote
    log_info "Transferring VIIRS file to remote server (this may take a while)..."
    rsync_cmd --progress "$viirs_file" "$REMOTE_HOST:$REMOTE_DIR/data/static/viirs/"
    log_success "VIIRS file transferred"
  fi
  
  # Restart ingester to pick up the new volume mount (in case it was added)
  log_info "Restarting ingester to ensure volume mount is active..."
  ssh_cmd "cd $REMOTE_DIR && $COMPOSE_CMD -f docker-compose.yml -f deploy/production/docker-compose.prod.yml restart ingester"
  
  # Wait for ingester to be healthy
  log_info "Waiting for ingester to be ready..."
  local retries=30
  while [[ $retries -gt 0 ]]; do
    if ssh_cmd "curl -s http://localhost:8082/health" 2>/dev/null | grep -q '"status":"ok"'; then
      break
    fi
    sleep 2
    retries=$((retries - 1))
  done
  
  if [[ $retries -le 0 ]]; then
    log_error "Ingester did not become healthy"
    exit 1
  fi
  
  # Ingest the file using the HTTP API
  # The file is mounted at /data/static/viirs/ in the container
  log_info "Ingesting VIIRS data into the system (this may take several minutes for a 300MB+ file)..."
  
  local ingest_response
  ingest_response=$(ssh_cmd "curl -s -X POST -H 'Content-Type: application/json' \
    -d '{\"file_path\": \"/data/static/viirs/$viirs_filename\", \"model\": \"viirs\"}' \
    http://localhost:8082/ingest" 2>&1)
  
  # Check if ingestion succeeded
  if echo "$ingest_response" | grep -q '"success":true'; then
    log_success "VIIRS light pollution data ingested successfully!"
    echo ""
    echo "Response: $ingest_response"
    echo ""
    echo "The light pollution / Bortle scale data is now available via:"
    echo "  - EDR API: https://$DOMAIN/edr/collections/viirs"
    echo "  - Light pollution endpoint: https://$DOMAIN/edr/light-pollution?lon=-105&lat=40"
  else
    log_error "VIIRS ingestion failed!"
    echo ""
    echo "Response: $ingest_response"
    echo ""
    echo "Check the ingester logs for more details:"
    echo "  ./scripts/deploy-remote.sh --logs ingester"
    exit 1
  fi
}

# =============================================================================
# MAIN
# =============================================================================

main() {
  echo ""
  echo "╔═══════════════════════════════════════════════════════════════╗"
  echo "║           Weather WMS - Production Deployment                 ║"
  echo "╚═══════════════════════════════════════════════════════════════╝"
  echo ""
  
  case "${1:-}" in
    --help|-h)
      show_help
      ;;
    --status)
      show_status
      ;;
    --logs)
      show_logs "${2:-}"
      ;;
    --ssh)
      do_ssh
      ;;
    --update)
      do_update
      ;;
    --rebuild)
      do_rebuild
      ;;
    --ingest-viirs)
      do_ingest_viirs
      ;;
    *)
      # Full deployment
      load_config
      generate_secrets
      validate_remote
      setup_remote
      build_images
      transfer_images
      sync_files
      setup_tls
      start_services
      verify_security
      print_summary
      ;;
  esac
}

main "$@"
