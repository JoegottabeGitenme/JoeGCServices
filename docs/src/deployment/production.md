# Production Deployment

This guide covers deploying Weather WMS to a single production server with nginx reverse proxy, TLS, and optional Cloudflare Tunnel support for networks behind CGNAT (like Starlink).

## Overview

The production deployment uses:

- **Nginx reverse proxy** as the single entry point
- **TLS termination** via Cloudflare Origin Certificates
- **Cloudflare Tunnel** for networks without port forwarding (optional)
- **Basic authentication** on admin endpoints
- **Rate limiting** on public API endpoints
- **Auto-generated secrets** for all services
- **Docker Compose** with production overrides

### Architecture

```
                    Internet
                        │
        ┌───────────────┴───────────────┐
        │                               │
   Port Forward                   Cloudflare Tunnel
   (standard ISP)                 (CGNAT/Starlink)
        │                               │
        └───────────────┬───────────────┘
                        │
                   ┌────▼────┐
                   │  Nginx  │ :80/:443
                   │ (proxy) │
                   └────┬────┘
                        │
        ┌───────────────┼───────────────┐
        │               │               │
   ┌────▼────┐    ┌────▼────┐    ┌────▼────┐
   │ WMS API │    │ EDR API │    │Dashboard│
   │  :8080  │    │  :8083  │    │  :8000  │
   └─────────┘    └─────────┘    └─────────┘
        │               │
   ┌────▼───────────────▼────┐
   │   PostgreSQL │ MinIO    │
   │   Redis      │ Grafana  │
   └─────────────────────────┘
```

### Public vs Admin Endpoints

| Endpoint | Auth Required | Rate Limited | Description |
|----------|---------------|--------------|-------------|
| `/` | No | Yes | Public splash page |
| `/wms` | No | Yes | WMS/WMTS API |
| `/wmts` | No | Yes | WMTS API |
| `/edr` | No | Yes | EDR API |
| `/admin` | Yes | No | Admin dashboard |
| `/grafana/` | Yes | No | Monitoring |
| `/minio/` | Yes | No | Object storage console |
| `/downloader/` | Yes | No | Download status |

## Prerequisites

### Server Requirements

| Resource | Minimum | Recommended |
|----------|---------|-------------|
| CPU | 4 cores | 16+ cores |
| RAM | 8 GB | 32 GB |
| Storage | 100 GB SSD | 1 TB NVMe |
| Network | 100 Mbps | 1 Gbps |
| OS | Debian 11+ / Ubuntu 22.04+ | Debian 12 |

### Required Software on Server

```bash
# Install Docker
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER

# Install Docker Compose plugin
sudo apt-get install docker-compose-plugin

# Verify
docker --version
docker compose version
```

### Local Requirements

- SSH key access to the server
- rsync installed locally
- Domain with Cloudflare DNS (for TLS and optional tunnel)

## Quick Start

```bash
# 1. Copy the example configuration
cp .env.nuc.example .env.nuc

# 2. Edit with your settings (at minimum: REMOTE_HOST, DOMAIN, SSH_KEY_PATH)
nano .env.nuc

# 3. Deploy
./scripts/deploy-remote.sh
```

The script will:
1. Validate configuration
2. Generate secure passwords for any blank fields
3. Build Docker images locally
4. Transfer images to the remote server
5. Sync configuration files
6. Set up TLS certificates
7. Start all services
8. Verify deployment

## Configuration

Configuration is managed via `.env.nuc` (copy from `.env.nuc.example`). The file is organized into sections:

### Section 1: Remote Server Access

```bash
REMOTE_HOST=user@server.local      # SSH target
REMOTE_DIR=/opt/weather-wms        # Deployment directory
SSH_KEY_PATH=~/.ssh/id_server      # SSH private key
```

### Section 2: Domain & TLS

```bash
DOMAIN=example.com                 # Your domain
```

TLS uses Cloudflare Origin Certificates. Create one in the Cloudflare dashboard under SSL/TLS > Origin Server, then place the files in `deploy/production/nginx/ssl/`:
- `origin.cert` - Certificate
- `origin.key` - Private key

### Section 3: Security

Passwords are auto-generated if left blank:

```bash
ADMIN_USER=admin                   # Basic auth username
ADMIN_PASSWORD=                    # Auto-generated
POSTGRES_PASSWORD=                 # Auto-generated
REDIS_PASSWORD=                    # Auto-generated
S3_ACCESS_KEY=                     # Auto-generated
S3_SECRET_KEY=                     # Auto-generated
GRAFANA_ADMIN_PASSWORD=            # Auto-generated
```

### Section 4: Rate Limiting

```bash
RATE_LIMIT_PER_MINUTE=10000        # Requests/minute per IP
RATE_LIMIT_BURST=500               # Burst allowance
```

The burst parameter allows short spikes above the rate limit. Increase these if you're seeing 429 errors while using the dashboard.

### Section 5: Performance Tuning

Adjust based on your server resources:

```bash
TOKIO_WORKER_THREADS=16            # Async runtime threads
TILE_CACHE_SIZE_MB=8192            # In-memory tile cache
CHUNK_CACHE_SIZE_MB=8192           # Zarr chunk cache
MEMORY_LIMIT_MB=28000              # Total memory limit
```

### Section 8: Cloudflare Tunnel (for CGNAT)

If your ISP uses CGNAT (like Starlink), you cannot use port forwarding. Instead, use Cloudflare Tunnel:

1. Go to Cloudflare Dashboard > Zero Trust > Networks > Tunnels
2. Create a new tunnel
3. Copy the tunnel token
4. Add a public hostname route: `yourdomain.com` -> `http://nginx:80`

```bash
CLOUDFLARE_TUNNEL_TOKEN=eyJ...     # Tunnel token
```

## Deploy Script Commands

### Full Deployment

```bash
./scripts/deploy-remote.sh
```

Performs a complete deployment: builds images, transfers them, syncs config, starts services.

### Update Configuration

```bash
./scripts/deploy-remote.sh --update
```

Syncs configuration files and restarts services. Use after changing `.env.nuc` or config files. Does not rebuild images.

### Rebuild and Deploy

```bash
./scripts/deploy-remote.sh --rebuild
```

Rebuilds Docker images locally and redeploys. Use after code changes.

### Check Status

```bash
./scripts/deploy-remote.sh --status
```

Shows running containers and their health status.

### View Logs

```bash
./scripts/deploy-remote.sh --logs           # All services
./scripts/deploy-remote.sh --logs wms-api   # Specific service
./scripts/deploy-remote.sh --logs nginx     # Nginx logs
```

### SSH to Server

```bash
./scripts/deploy-remote.sh --ssh
```

Opens an SSH session to the remote server.

## File Structure

```
deploy/production/
├── .env.nuc.example           # Configuration template
├── docker-compose.prod.yml    # Production overrides
└── nginx/
    ├── Dockerfile             # Nginx container
    ├── nginx.conf.template    # Nginx config (templated)
    └── ssl/
        ├── origin.cert        # Cloudflare origin certificate
        └── origin.key         # Certificate private key
```

The production compose file (`docker-compose.prod.yml`) extends the base `docker-compose.yml` with:

- Nginx reverse proxy as the only exposed service
- Cloudflare Tunnel container (cloudflared)
- Resource limits on all services
- Removed development tools (pgAdmin)
- Security hardening

## Security Features

### Basic Authentication

Admin endpoints require HTTP Basic Auth. Credentials are set via `ADMIN_USER` and `ADMIN_PASSWORD` in the configuration.

### Rate Limiting

All public endpoints (`/`, `/wms`, `/wmts`, `/edr`) are rate limited per IP address. The rate limit uses the `CF-Connecting-IP` header when behind Cloudflare to get the real client IP.

### Blocked Endpoints

The `/metrics` endpoint is blocked from external access to prevent information leakage.

### TLS Configuration

- TLS 1.2 and 1.3 only
- Strong cipher suites
- HSTS enabled
- Security headers (X-Content-Type-Options, X-Frame-Options, etc.)

## Web Interface

The deployment includes a public splash page at the root URL (`/`) that provides:

- Links to WMS, WMTS, and EDR capabilities
- Live example map tiles from each data source
- Example EDR queries with sample responses
- API endpoint documentation

The full admin dashboard is available at `/admin` (requires authentication).

## Monitoring

### Grafana

Access Grafana at `/grafana/` (requires authentication). Default dashboards include:

- Service health and uptime
- Request rates and latencies
- Cache hit rates
- Memory and CPU usage

### Prometheus

Metrics are collected by Prometheus and retained based on `PROMETHEUS_RETENTION_DAYS`.

### Logs

View logs via the deploy script:

```bash
./scripts/deploy-remote.sh --logs wms-api
./scripts/deploy-remote.sh --logs edr-api
./scripts/deploy-remote.sh --logs nginx
```

## Troubleshooting

### 502 Bad Gateway

Usually caused by nginx caching stale DNS for backend services after a container restart.

**Fix**: Restart nginx to refresh DNS:
```bash
./scripts/deploy-remote.sh --ssh
docker restart weather-wms-nginx
```

The `--update` command automatically restarts nginx after syncing files.

### 429 Too Many Requests

Rate limiting is blocking requests. This can happen when loading the dashboard, which makes many tile requests.

**Fix**: Increase rate limits in `.env.nuc`:
```bash
RATE_LIMIT_PER_MINUTE=30000
RATE_LIMIT_BURST=1000
```

Then redeploy: `./scripts/deploy-remote.sh --update`

### Cloudflare Tunnel Not Connecting

1. Verify the tunnel token is correct in `.env.nuc`
2. Check tunnel status in Cloudflare Zero Trust dashboard
3. View tunnel logs: `./scripts/deploy-remote.sh --logs cloudflared`

### Services Not Starting

Check container health:
```bash
./scripts/deploy-remote.sh --ssh
docker ps -a
docker logs <container-name>
```

Common issues:
- Database connection failures (check PostgreSQL is healthy first)
- Missing configuration files
- Port conflicts

### TLS Certificate Errors

1. Verify Cloudflare SSL/TLS mode is set to "Full" or "Full (Strict)"
2. Check certificate files exist in `deploy/production/nginx/ssl/`
3. Ensure certificate hasn't expired

## Updating

### Configuration Changes

```bash
# Edit configuration
nano .env.nuc

# Apply changes
./scripts/deploy-remote.sh --update
```

### Code Changes

```bash
# Pull latest code
git pull

# Rebuild and deploy
./scripts/deploy-remote.sh --rebuild
```

### Data Model Changes

For database schema changes, you may need to reset the database:

```bash
./scripts/deploy-remote.sh --ssh
cd /opt/weather-wms
docker compose down -v  # WARNING: Deletes all data
docker compose up -d
```

## Backup and Recovery

### Database Backup

```bash
./scripts/deploy-remote.sh --ssh
docker exec weather-wms-postgres pg_dump -U weatherwms weatherwms > backup.sql
```

### Object Storage

MinIO data is stored in a Docker volume. For backup:

```bash
docker run --rm \
  -v weather-wms_minio_data:/data \
  -v $(pwd):/backup \
  alpine tar czf /backup/minio_backup.tar.gz /data
```

## Next Steps

- [Monitoring Setup](./monitoring.md) - Configure dashboards and alerts
- [Configuration Reference](../configuration/README.md) - Customize data sources and styles
- [API Reference](../api-reference/README.md) - Use the WMS/WMTS/EDR APIs
