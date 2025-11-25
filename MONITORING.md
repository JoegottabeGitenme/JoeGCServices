# Monitoring & Debugging Guide

Since the Kubernetes dashboard may have network issues in some environments, use kubectl commands for monitoring.

## Quick Status Checks

```bash
# Overall cluster status
kubectl cluster-info

# Node status
kubectl get nodes

# All resources in namespace
kubectl get all -n weather-wms

# Just pods
kubectl get pods -n weather-wms

# Detailed pod info
kubectl get pods -n weather-wms -o wide

# Resource usage (if metrics-server is enabled)
kubectl top nodes
kubectl top pods -n weather-wms
```

## Watching in Real-Time

```bash
# Watch pods as they change
kubectl get pods -n weather-wms -w

# Watch deployments rolling out
kubectl get deployment -n weather-wms -w

# Watch events (useful for troubleshooting)
kubectl get events -n weather-wms -w
```

## Viewing Logs

```bash
# View pod logs
kubectl logs -n weather-wms <pod-name>

# Follow logs (like tail -f)
kubectl logs -n weather-wms <pod-name> -f

# View logs from all pods of a deployment
kubectl logs -n weather-wms -l app=wms-api -f

# View previous logs (from a crashed pod)
kubectl logs -n weather-wms <pod-name> --previous

# View last 50 lines with timestamps
kubectl logs -n weather-wms <pod-name> --tail=50 --timestamps
```

## Debugging Pods

```bash
# Get detailed pod information
kubectl describe pod -n weather-wms <pod-name>

# Execute command in a pod
kubectl exec -it -n weather-wms <pod-name> -- bash

# Copy files from/to pod
kubectl cp -n weather-wms <pod-name>:/path/in/pod ./local/path
kubectl cp ./local/path -n weather-wms <pod-name>:/path/in/pod

# Port-forward to a pod
kubectl port-forward -n weather-wms <pod-name> 8080:8080
```

## Service Access

```bash
# List all services
kubectl get svc -n weather-wms

# Get service details
kubectl describe svc -n weather-wms <service-name>

# Port-forward to a service
kubectl port-forward -n weather-wms svc/<service-name> 8080:8080

# Get service endpoints
kubectl get endpoints -n weather-wms <service-name>
```

## Troubleshooting Common Issues

### Pod Stuck in Pending

```bash
# Check for errors
kubectl describe pod -n weather-wms <pod-name>

# Usually means:
# - Not enough resources (check node capacity)
# - Image pull issues (check Events section)
# - PVC not bound
```

### ImagePullBackOff

```bash
# Check what image is failing
kubectl describe pod -n weather-wms <pod-name> | grep Image

# Manually pull and load image into minikube
docker pull <image-name>
minikube -p weather-wms image load <image-name>
```

### Pod CrashLoopBackOff

```bash
# Check logs
kubectl logs -n weather-wms <pod-name>

# Check previous logs
kubectl logs -n weather-wms <pod-name> --previous

# Get more details
kubectl describe pod -n weather-wms <pod-name>
```

### Network Issues

```bash
# Check DNS from within a pod
kubectl exec -it -n weather-wms <pod-name> -- nslookup kubernetes.default

# Check pod networking
kubectl get networkpolicy -n weather-wms

# View cluster DNS
kubectl get svc -n kube-system | grep dns
```

## Environment Inspection

```bash
# View pod environment variables
kubectl exec -n weather-wms <pod-name> -- env

# View pod limits and requests
kubectl get pod -n weather-wms <pod-name> -o yaml | grep -A 10 "resources:"

# View pod mounts
kubectl get pod -n weather-wms <pod-name> -o yaml | grep -A 10 "volumeMounts:"
```

## Useful One-Liners

```bash
# Get all pod IPs
kubectl get pods -n weather-wms -o wide | awk '{print $6}' | tail -n +2

# Delete all pods (forces restart)
kubectl delete pods --all -n weather-wms

# Restart a deployment
kubectl rollout restart deployment/<deployment-name> -n weather-wms

# Watch deployment rollout
kubectl rollout status deployment/<deployment-name> -n weather-wms -w

# Scale a deployment
kubectl scale deployment/<deployment-name> -n weather-wms --replicas=3

# Get all resources consuming most resources
kubectl top pods -n weather-wms --sort-by=memory
kubectl top pods -n weather-wms --sort-by=cpu
```

## Accessing Services

```bash
# Port forward to PostgreSQL
kubectl port-forward -n weather-wms svc/postgresql 5432:5432
psql -h localhost -U weatherwms -d weatherwms

# Port forward to Redis
kubectl port-forward -n weather-wms svc/redis-master 6379:6379
redis-cli -h localhost

# Port forward to MinIO
kubectl port-forward -n weather-wms svc/minio 9000:9000
# Then use S3 client pointing to http://localhost:9000

# Port forward to WMS API
kubectl port-forward -n weather-wms svc/wms-api 8080:8080
curl http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities
```

## Viewing Configuration

```bash
# View ConfigMaps
kubectl get cm -n weather-wms
kubectl get cm -n weather-wms <name> -o yaml

# View Secrets (masked)
kubectl get secrets -n weather-wms
kubectl get secret -n weather-wms <name> -o yaml

# Decode a secret value
kubectl get secret -n weather-wms <name> -o jsonpath='{.data.password}' | base64 -d
```

## Helm Management

```bash
# List releases
helm list -n weather-wms

# Get release status
helm status wms -n weather-wms

# Get release values
helm get values wms -n weather-wms

# Get full manifest
helm get manifest wms -n weather-wms

# Upgrade release
helm upgrade wms ./deploy/helm/weather-wms -n weather-wms

# Rollback release
helm rollback wms 1 -n weather-wms
```

## Metrics & Performance

```bash
# Get node metrics (requires metrics-server)
kubectl top nodes

# Get pod metrics
kubectl top pods -n weather-wms

# Get pod metrics with timestamps
kubectl top pods -n weather-wms --containers

# Most resource-hungry pods
kubectl top pods -n weather-wms --sort-by=cpu
kubectl top pods -n weather-wms --sort-by=memory
```

## YAML Export & Backup

```bash
# Export current state
kubectl get all -n weather-wms -o yaml > weather-wms-backup.yaml

# Export just deployments
kubectl get deployments -n weather-wms -o yaml > deployments-backup.yaml

# Restore from backup
kubectl apply -f weather-wms-backup.yaml
```
