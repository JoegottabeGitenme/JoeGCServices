# Kubernetes Deployment

Deploy Weather WMS on Kubernetes for production scalability and high availability.

## Prerequisites

- Kubernetes 1.27+
- kubectl configured
- Storage class available (for PVCs)
- Load balancer support (or Ingress controller)

## Quick Deploy

For quick deployment, use the [Helm Chart](./helm.md) instead. This guide covers raw Kubernetes manifests for custom deployments.

## Architecture

```
┌─────────────────────────────────────┐
│      Kubernetes Namespace           │
│                                     │
│  ┌─────────────────────────────┐   │
│  │      WMS API Deployment     │   │
│  │  ┌─────┐ ┌─────┐ ┌─────┐   │   │
│  │  │Pod 1│ │Pod 2│ │Pod 3│   │   │
│  │  └─────┘ └─────┘ └─────┘   │   │
│  │  Horizontal Pod Autoscaler  │   │
│  └─────────────────────────────┘   │
│              ▲                      │
│              │                      │
│     ┌────────┴─────────┐           │
│     │  Service (LB)    │           │
│     │  Port 80/443     │           │
│     └──────────────────┘           │
│                                     │
│  ┌─────────────┐  ┌──────────────┐ │
│  │ PostgreSQL  │  │    MinIO     │ │
│  │StatefulSet  │  │ StatefulSet  │ │
│  │   + PVC     │  │    + PVC     │ │
│  └─────────────┘  └──────────────┘ │
└─────────────────────────────────────┘
```

## Resource Requirements

### Per Service

```yaml
# WMS API (each replica)
resources:
  requests:
    cpu: 1000m
    memory: 2Gi
  limits:
    cpu: 2000m
    memory: 4Gi

# Ingester
resources:
  requests:
    cpu: 2000m
    memory: 4Gi
  limits:
    cpu: 4000m
    memory: 8Gi

```

## Example Deployment

### WMS API Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: wms-api
spec:
  replicas: 3
  selector:
    matchLabels:
      app: wms-api
  template:
    metadata:
      labels:
        app: wms-api
    spec:
      containers:
      - name: wms-api
        image: weather-wms/wms-api:latest
        ports:
        - containerPort: 8080
        env:
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: weather-wms-secrets
              key: database-url
        - name: REDIS_URL
          value: redis://redis:6379
        - name: S3_ENDPOINT
          value: http://minio:9000
        resources:
          requests:
            cpu: 1000m
            memory: 2Gi
          limits:
            cpu: 2000m
            memory: 4Gi
        livenessProbe:
          httpGet:
            path: /health
            port: 8080
          initialDelaySeconds: 30
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /ready
            port: 8080
          initialDelaySeconds: 5
          periodSeconds: 5
```

### Service & Ingress

```yaml
apiVersion: v1
kind: Service
metadata:
  name: wms-api
spec:
  type: LoadBalancer
  ports:
  - port: 80
    targetPort: 8080
  selector:
    app: wms-api
---
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: wms-api
spec:
  tls:
  - hosts:
    - weather-wms.example.com
    secretName: wms-tls
  rules:
  - host: weather-wms.example.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: wms-api
            port:
              number: 80
```

### Horizontal Pod Autoscaler

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: wms-api
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: wms-api
  minReplicas: 3
  maxReplicas: 10
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
  - type: Resource
    resource:
      name: memory
      target:
        type: Utilization
        averageUtilization: 80
```

## Storage

### PostgreSQL StatefulSet

Use a StatefulSet with persistent volume:

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: postgres
spec:
  serviceName: postgres
  replicas: 1
  selector:
    matchLabels:
      app: postgres
  template:
    metadata:
      labels:
        app: postgres
    spec:
      containers:
      - name: postgres
        image: postgres:15
        ports:
        - containerPort: 5432
        env:
        - name: POSTGRES_DB
          value: weatherwms
        - name: POSTGRES_USER
          valueFrom:
            secretKeyRef:
              name: postgres-secret
              key: username
        - name: POSTGRES_PASSWORD
          valueFrom:
            secretKeyRef:
              name: postgres-secret
              key: password
        volumeMounts:
        - name: postgres-data
          mountPath: /var/lib/postgresql/data
  volumeClaimTemplates:
  - metadata:
      name: postgres-data
    spec:
      accessModes: ["ReadWriteOnce"]
      resources:
        requests:
          storage: 100Gi
```

## Secrets Management

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: weather-wms-secrets
type: Opaque
stringData:
  database-url: postgresql://weatherwms:password@postgres:5432/weatherwms
  s3-access-key: minioadmin
  s3-secret-key: minioadmin
```

For production, use:
- [Sealed Secrets](https://github.com/bitnami-labs/sealed-secrets)
- [External Secrets Operator](https://external-secrets.io/)
- Cloud provider secret managers (AWS Secrets Manager, GCP Secret Manager)

## Deployment

```bash
# Create namespace
kubectl create namespace weather-wms

# Apply manifests
kubectl apply -f k8s/ -n weather-wms

# Check status
kubectl get pods -n weather-wms
kubectl get svc -n weather-wms

# View logs
kubectl logs -f deployment/wms-api -n weather-wms

# Scale manually
kubectl scale deployment wms-api --replicas=5 -n weather-wms
```

## Monitoring

Deploy Prometheus and Grafana:

```bash
# Add Helm repos
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
helm repo update

# Install Prometheus
helm install prometheus prometheus-community/kube-prometheus-stack -n monitoring --create-namespace

# Access Grafana
kubectl port-forward -n monitoring svc/prometheus-grafana 3000:80
```

## Next Steps

- [Helm Chart](./helm.md) - Simplified deployment
- [Monitoring](./monitoring.md) - Set up dashboards
- [Configuration](../configuration/README.md) - Customize settings
