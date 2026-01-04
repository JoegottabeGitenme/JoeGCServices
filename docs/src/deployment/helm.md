# Helm Chart Deployment

Deploy Weather WMS to Kubernetes with a single command using Helm.

## Prerequisites

- Kubernetes 1.27+
- Helm 3.12+
- kubectl configured
- Storage class available

## Local Development with Minikube

For local development and testing, use the `start.sh` script which automates the entire Kubernetes setup:

```bash
# Start full Kubernetes stack with minikube
./scripts/start.sh --k8s

# Restart port-forwards for existing cluster
./scripts/start.sh --forward

# Stop port-forwards (cluster keeps running)  
./scripts/start.sh --stop-k8s

# Delete cluster and start fresh
./scripts/start.sh --clean
```

The script automatically:
- Creates a minikube cluster (`weather-wms` profile)
- Builds and loads Docker images into minikube
- Deploys PostgreSQL, Redis, MinIO (standalone)
- Deploys Prometheus and Grafana for monitoring
- Installs the Weather WMS Helm chart
- Sets up port-forwards to access services locally

**Services available after deployment:**

| Service | URL | Credentials |
|---------|-----|-------------|
| Web Dashboard | http://localhost:8000 | - |
| WMS API | http://localhost:8080 | - |
| Grafana | http://localhost:3000 | admin/admin |
| Prometheus | http://localhost:9090 | - |
| MinIO Console | http://localhost:9001 | minioadmin/minioadmin |

## Quick Deploy

```bash
# Add Helm repository (when published)
helm repo add weather-wms https://charts.weather-wms.io
helm repo update

# Install
helm install my-weather-wms weather-wms/weather-wms \
  --namespace weather-wms \
  --create-namespace

# Or install from local chart
helm install my-weather-wms ./deploy/helm/weather-wms \
  --namespace weather-wms \
  --create-namespace
```

## Configuration

### values.yaml

```yaml
# Number of WMS API replicas
replicaCount: 3

image:
  repository: weather-wms/wms-api
  tag: latest
  pullPolicy: IfNotPresent

resources:
  limits:
    cpu: 2000m
    memory: 4Gi
  requests:
    cpu: 1000m
    memory: 2Gi

autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 10
  targetCPUUtilizationPercentage: 70
  targetMemoryUtilizationPercentage: 80

postgresql:
  enabled: true
  auth:
    username: weatherwms
    password: changeme
    database: weatherwms
  primary:
    persistence:
      size: 100Gi

redis:
  enabled: true
  master:
    persistence:
      size: 10Gi

minio:
  enabled: true
  persistence:
    size: 1Ti
  resources:
    requests:
      memory: 4Gi

ingress:
  enabled: true
  className: nginx
  hosts:
    - host: weather-wms.example.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: weather-wms-tls
      hosts:
        - weather-wms.example.com
```

## Custom Values

```bash
# Create custom values
cat > my-values.yaml <<EOF
replicaCount: 5

postgresql:
  auth:
    password: super-secret-password

ingress:
  hosts:
    - host: wms.mydomain.com
      paths:
        - path: /
          pathType: Prefix
EOF

# Install with custom values
helm install my-weather-wms weather-wms/weather-wms \
  -f my-values.yaml \
  --namespace weather-wms \
  --create-namespace
```

## Common Operations

### Upgrade

```bash
# Upgrade to new version
helm upgrade my-weather-wms weather-wms/weather-wms \
  --namespace weather-wms \
  -f my-values.yaml
```

### Rollback

```bash
# List revisions
helm history my-weather-wms -n weather-wms

# Rollback to previous revision
helm rollback my-weather-wms -n weather-wms
```

### Uninstall

```bash
# Uninstall release
helm uninstall my-weather-wms -n weather-wms

# Delete namespace
kubectl delete namespace weather-wms
```

## External Databases

For production, use managed databases:

```yaml
# values.yaml

# Disable built-in PostgreSQL
postgresql:
  enabled: false

# Use external database
externalDatabase:
  host: my-postgres.amazonaws.com
  port: 5432
  database: weatherwms
  username: weatherwms
  existingSecret: postgres-credentials  # K8s secret with password

# Disable built-in Redis
redis:
  enabled: false

externalRedis:
  host: my-redis.memorystore.googleapis.com
  port: 6379
```

## Production Configuration

```yaml
# values.yaml for production

replicaCount: 5

resources:
  limits:
    cpu: 4000m
    memory: 8Gi
  requests:
    cpu: 2000m
    memory: 4Gi

autoscaling:
  enabled: true
  minReplicas: 5
  maxReplicas: 20

# Use external managed services
postgresql:
  enabled: false
  
redis:
  enabled: false

# Use S3 instead of MinIO
minio:
  enabled: false

externalStorage:
  endpoint: https://s3.amazonaws.com
  bucket: weather-data-prod
  region: us-east-1
  existingSecret: s3-credentials

ingress:
  enabled: true
  className: nginx
  annotations:
    nginx.ingress.kubernetes.io/rate-limit: "100"
    nginx.ingress.kubernetes.io/ssl-redirect: "true"
  hosts:
    - host: wms.production.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: wms-tls-prod
      hosts:
        - wms.production.com

monitoring:
  enabled: true
  serviceMonitor:
    enabled: true
```

## Monitoring Integration

The Helm chart includes Prometheus ServiceMonitor:

```yaml
# values.yaml
monitoring:
  enabled: true
  serviceMonitor:
    enabled: true
    interval: 30s
    labels:
      release: prometheus
```

## Troubleshooting

```bash
# Check all resources
helm status my-weather-wms -n weather-wms

# View rendered templates
helm template my-weather-wms weather-wms/weather-wms \
  -f my-values.yaml > rendered.yaml

# Debug installation
helm install my-weather-wms weather-wms/weather-wms \
  --namespace weather-wms \
  --create-namespace \
  --debug \
  --dry-run
```

## Next Steps

- [Kubernetes](./kubernetes.md) - Raw manifest reference
- [Monitoring](./monitoring.md) - Set up dashboards
- [Configuration](../configuration/README.md) - Customize settings
