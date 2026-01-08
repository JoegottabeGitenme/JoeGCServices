# Deployment Overview

Weather WMS can be deployed in multiple ways depending on your needs: local development, staging, or production.

## Deployment Options

| Method | Best For | Complexity | Scalability |
|--------|----------|------------|-------------|
| [Docker Compose](./docker-compose.md) | Development, testing | Low | Single node |
| [Production (Single Server)](./production.md) | Small-medium production | Low | Single node |

## Quick Comparison

### Docker Compose
**Pros**:
- Simple setup (one command)
- Good for development
- Easy debugging

**Cons**:
- Single-node only
- Manual scaling
- No built-in HA

**Use when**: Local development, testing, small deployments

---

### Production (Single Server)
**Pros**:
- Production-ready with TLS, rate limiting, auth
- Nginx reverse proxy included
- Cloudflare Tunnel support (for CGNAT/Starlink)
- Auto-generated secure passwords
- One-command deployment and updates

**Cons**:
- Single-node only
- No horizontal scaling
- Requires Cloudflare for TLS

**Use when**: Small-medium production deployments, home servers, single-node setups

## Architecture

### Docker Compose
```
┌─────────────────────────────────┐
│       Single Docker Host        │
│                                 │
│  ┌────────┐  ┌──────────────┐  │
│  │WMS API │  │  PostgreSQL  │  │
│  │ (x1)   │  │   (single)   │  │
│  └────────┘  └──────────────┘  │
│                                 │
│  ┌────────┐  ┌──────────────┐  │
│  │Ingester│  │    MinIO     │  │
│  │ (x1)   │  │   (single)   │  │
│  └────────┘  └──────────────┘  │
└─────────────────────────────────┘
```

## Resource Requirements

### Minimum (Development)
- **CPU**: 4 cores
- **RAM**: 8 GB
- **Storage**: 50 GB
- **Network**: 10 Mbps

### Recommended (Production)
- **CPU**: 16+ cores
- **RAM**: 64 GB
- **Storage**: 1 TB SSD
- **Network**: 1 Gbps

## Deployment Checklist

### Pre-Deployment

- [ ] Choose deployment method
- [ ] Provision infrastructure (servers)
- [ ] Configure networking (ports, firewall)
- [ ] Set up DNS (if needed)
- [ ] Prepare TLS certificates (for HTTPS)
- [ ] Review security settings

### Database Setup

- [ ] PostgreSQL 15+ available
- [ ] Database created
- [ ] User credentials configured
- [ ] Connection tested

### Storage Setup

- [ ] MinIO or S3-compatible storage available
- [ ] Bucket created
- [ ] Access keys configured
- [ ] Connection tested

### Cache Setup

- [ ] Redis 7+ available
- [ ] Memory allocated (4+ GB recommended)
- [ ] Connection tested

### Post-Deployment

- [ ] Verify all services running
- [ ] Test health endpoints
- [ ] Download sample data
- [ ] Test WMS GetMap request
- [ ] Configure monitoring
- [ ] Set up backups
- [ ] Document access URLs

## Monitoring

All deployment methods support monitoring via:

- **Prometheus**: Metrics collection
- **Grafana**: Visualization dashboards
- **Logs**: Structured JSON logging

See [Monitoring](./monitoring.md) for details.

## Next Steps

Choose your deployment method:

- [Docker Compose](./docker-compose.md) - Local development
- [Production (Single Server)](./production.md) - Single-server production with TLS
- [Monitoring](./monitoring.md) - Set up observability
