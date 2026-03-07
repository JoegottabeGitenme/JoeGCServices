# Kubernetes Migration Plan for weather-wms

## Goal

Create an alternative deployment path using Kubernetes (k3s) alongside the existing Docker Compose deployment. Includes a Helm chart, GitHub Actions CI for image builds, and support for horizontal scaling of the WMS and EDR API services.

## Scope

| Attribute | Value |
|---|---|
| Target platform | k3s (single-node) |
| Ingress controller | Traefik (k3s default) |
| Storage provisioner | local-path-provisioner (k3s default) |
| Deployment tooling | Helm 3 chart |
| Container registry | Gitea (self-hosted on cluster), configurable |
| Monitoring | kube-prometheus-stack + loki-stack Helm charts |
| CI/CD | GitHub Actions (build + push), manual deploy (helm upgrade) |
| Coexistence | Docker Compose remains the primary dev/deploy path |

## Architecture

```
                      k3s cluster (single node)

Internet -> Traefik Ingress
              |-> /wms, /wmts, /api  -> wms-api (1-N pods)
              |-> /edr               -> edr-api (1-N pods)
              |-> /                  -> web-dashboard
              |-> /downloader        -> downloader

  data-pipeline pod (singleton):
    container: downloader --POST localhost:8082--> container: ingester
    shared emptyDir: /data/downloads

  postgres  <- StatefulSet (1 replica)
  redis     <- StatefulSet (1 replica)
  minio     <- StatefulSet (1 replica)
```

### Key Design Decisions

1. **Downloader + Ingester co-located** -- Share files via emptyDir, communicate on localhost:8082.
2. **WMS API scalable** -- ENABLE_CLEANUP/ENABLE_SYNC env vars control which replica runs background tasks. Dedicated cleanup pod when scaling.
3. **EDR API fully scalable** -- Stateless reads, no constraints.
4. **No autoheal** -- K8s liveness probes handle restarts natively.
5. **Same Dockerfiles** -- Chart coexists with Docker Compose, no image changes needed.

## Scaling Analysis

| Service | Scalable | Constraints |
|---------|----------|-------------|
| edr-api | Yes | Each replica has independent in-memory cache |
| wms-api | Yes (with caveats) | Cleanup/sync must run on one replica only |
| ingester | Yes (with caveats) | No deduplication; tied to downloader in same pod |
| downloader | No (singleton) | SQLite state DB, assumes single instance |

## Completed (Phases 0-3)

### Phase 0: Code Changes
- [x] ENABLE_CLEANUP/ENABLE_SYNC env vars already existed in WMS API
- [x] Fixed EDR API hardcoded `"config/edr"` path to use CONFIG_DIR env var
- [x] Verified ingester binds 0.0.0.0:8082, INGESTER_URL defaults to localhost:8082
- [x] Verified all config dirs < 1 MiB (total 816K)
- [x] Confirmed data/static is empty, wms-api doesn't read /data/downloads

### Phase 1: Helm Chart Scaffold
- [x] Chart.yaml, values.yaml, values-production.yaml
- [x] _helpers.tpl with image, label, URL helpers
- [x] NOTES.txt with post-install instructions

### Phase 2: Infrastructure Templates
- [x] ConfigMaps for models, layers, edr, styles (populated via .Files.Glob)
- [x] Secret with all credentials (stringData, no base64 encoding needed)
- [x] PostgreSQL StatefulSet + Service (PostGIS 16-3.4)
- [x] Redis StatefulSet + Service (conditional --requirepass)
- [x] MinIO StatefulSet + Service + post-install Job (bucket setup)

### Phase 3: Application Templates
- [x] Data Pipeline Deployment (2 containers, emptyDir shared volume, PVC for SQLite)
- [x] WMS API Deployment + Service + conditional cleanup Deployment
- [x] EDR API Deployment + Service
- [x] Web Dashboard Deployment + Service
- [x] HPA templates for wms-api and edr-api (disabled by default)
- [x] Ingress with path-based routing

## Remaining (Phases 4-7)

### Phase 4: Ingress Refinement
- [ ] Rate limiting middleware for Traefik
- [ ] TLS configuration with cert-manager

### Phase 5: Monitoring
- [ ] values-monitoring.yaml for kube-prometheus-stack
- [ ] values-loki.yaml for loki-stack
- [ ] ServiceMonitor templates in chart
- [ ] Grafana dashboard ConfigMaps with auto-discovery labels

### Phase 6: CI/CD
- [ ] GitHub Actions workflow for building 4 service images
- [ ] Push to Gitea registry with SHA + latest tags
- [ ] Document Gitea registry setup

### Phase 7: Documentation
- [ ] docs/src/deployment/kubernetes.md
- [ ] Update docs/src/SUMMARY.md

## Chart Structure

```
deploy/k8s/weather-wms/
  Chart.yaml
  values.yaml
  values-production.yaml
  .helmignore
  templates/
    _helpers.tpl
    NOTES.txt
    secret.yaml
    ingress.yaml
    configmap-{models,layers,edr,styles}.yaml
    data-pipeline/
      deployment.yaml       # Downloader + Ingester (same pod)
      service-downloader.yaml
      service-ingester.yaml
      pvc-state.yaml
    wms-api/
      deployment.yaml       # Scalable serving replicas
      deployment-cleanup.yaml  # Conditional cleanup pod
      service.yaml
      hpa.yaml
    edr-api/
      deployment.yaml
      service.yaml
      hpa.yaml
    web-dashboard/
      deployment.yaml
      service.yaml
    postgres/
      statefulset.yaml
      service.yaml
    redis/
      statefulset.yaml
      service.yaml
    minio/
      statefulset.yaml
      service.yaml
      job-setup.yaml
```

## Usage

```bash
# Sync config files into chart directory
rsync -a --delete config/ deploy/k8s/weather-wms/config/

# Dev install
helm install weather-wms deploy/k8s/weather-wms -n weather-wms --create-namespace

# Production install
helm install weather-wms deploy/k8s/weather-wms \
  -n weather-wms --create-namespace \
  -f deploy/k8s/weather-wms/values-production.yaml \
  --set secrets.postgres.password=XXX \
  --set secrets.redis.password=XXX \
  --set secrets.s3.accessKey=XXX \
  --set secrets.s3.secretKey=XXX

# Upgrade
helm upgrade weather-wms deploy/k8s/weather-wms -n weather-wms

# Use the deploy script
./scripts/deploy-k8s.sh              # Dev
./scripts/deploy-k8s.sh --production  # Production
./scripts/deploy-k8s.sh --dry-run     # Preview
```
