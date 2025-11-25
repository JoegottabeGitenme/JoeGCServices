# Quick Reference Card

## Build & Test (Local Development)

```bash
cargo build              # Build all
cargo test               # Run all tests
cargo test --package wms-common  # Test specific crate
cargo test test_name -- --exact  # Run single test
cargo fmt                # Format code
cargo clippy -- -D warnings       # Lint
```

## Running Services Locally (Fast - No Kubernetes)

```bash
# Terminal 1: Start dependencies
docker-compose up

# Terminal 2: Run API server (automatically loads .env)
cargo run --bin wms-api

# Or with debug logging (overrides .env RUST_LOG):
RUST_LOG=debug cargo run --bin wms-api

# Test it
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities"
```

Note: `.env` file is automatically loaded with database/service credentials

## Kubernetes (Full Setup)

```bash
./scripts/start.sh              # Full setup
./scripts/start.sh --quick      # Skip building images
./scripts/start.sh --status     # Show status
./scripts/start.sh --stop       # Stop cluster
./scripts/start.sh --clean      # Delete & restart
```

## Kubernetes Monitoring (instead of dashboard)

```bash
# Status
kubectl get all -n weather-wms
kubectl get pods -n weather-wms -w   # Watch live

# Logs
kubectl logs -n weather-wms <pod-name> -f

# Details
kubectl describe pod -n weather-wms <pod-name>

# Access services
kubectl port-forward -n weather-wms svc/postgresql 5432:5432
kubectl port-forward -n weather-wms svc/redis-master 6379:6379
kubectl port-forward -n weather-wms svc/wms-api 8080:8080
```

## Service Credentials

```
PostgreSQL:
  User: weatherwms
  Pass: weatherwms
  DB: weatherwms
  Host: localhost:5432

Redis:
  Host: localhost:6379
  No auth

MinIO:
  User: minioadmin
  Pass: minioadmin
  Endpoint: localhost:9000
  Console: localhost:9001
```

## Debugging

```bash
# Check if services are running
docker ps  # local Docker
kubectl get pods -n weather-wms  # Kubernetes

# View logs
cargo test -- --nocapture  # Show test output
kubectl logs -n weather-wms <pod> --previous  # Crashed pod logs
RUST_LOG=debug cargo run --bin wms-api  # Debug logs

# Execute in pod
kubectl exec -it -n weather-wms <pod> -- bash

# Connection test
kubectl exec -it -n weather-wms <pod> -- curl http://localhost:8080
```

## Common Issues

| Issue | Solution |
|-------|----------|
| `ImagePullBackOff` | `docker pull <image> && minikube -p weather-wms image load <image>` |
| `Pending` pods | `kubectl describe pod` to see resource/image issues |
| `CrashLoopBackOff` | `kubectl logs <pod> --previous` to see crash reason |
| Dashboard timeout | Use `kubectl` commands instead (see MONITORING.md) |
| Cargo lock version error | `rustup update` |
| Tests fail locally | `cargo clean && cargo build && cargo test` |

## File Structure

```
crates/              # Shared libraries
  ├── wms-common/   # Core types & errors
  ├── wms-protocol/ # OGC WMS/WMTS spec
  ├── grib2-parser/ # Data format parsing
  ├── renderer/     # Image rendering
  └── storage/      # DB/cache/S3 clients

services/            # Deployable microservices
  ├── wms-api/      # HTTP server
  ├── ingester/     # Data import
  └── renderer-worker/  # Tile rendering

deploy/helm/         # Kubernetes manifests
scripts/             # Automation scripts
```

## Documentation

- **DEVELOPMENT.md** - Full development guide
- **MONITORING.md** - 100+ kubectl commands
- **AGENTS.md** - Code style & build commands
- **QUICKREF.md** - This file!

## Links

- [Kubernetes Docs](https://kubernetes.io/docs/)
- [Minikube Docs](https://minikube.sigs.k8s.io/)
- [kubectl Cheatsheet](https://kubernetes.io/docs/reference/kubectl/cheatsheet/)
- [Rust Book](https://doc.rust-lang.org/book/)
- [OGC WMS Spec](https://www.ogc.org/standards/wms)

## Tips

1. **Fast iteration**: Use `docker-compose up` + `cargo run` instead of full Kubernetes
2. **Debugging**: `kubectl logs -f` is your friend
3. **Testing**: Run `cargo test` before committing
4. **Formatting**: Always run `cargo fmt` before git commit
5. **Port forwarding**: Keep `kubectl port-forward` running in background terminal
