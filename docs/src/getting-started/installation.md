# Installation

This guide walks you through installing Weather WMS using Docker Compose, the recommended method for local development and testing.

## Quick Installation

For experienced users, here's the TL;DR:

```bash
git clone https://github.com/JoegottabeGitenme/JoeGCServices.git
cd JoeGCServices
cp .env.example .env
./scripts/start.sh
```

Then visit http://localhost:8000 to access the web dashboard.

## Detailed Installation Steps

### Step 1: Clone the Repository

```bash
git clone https://github.com/JoegottabeGitenme/JoeGCServices.git
cd JoeGCServices
```

This downloads the complete Weather WMS codebase including:
- Service source code
- Configuration files
- Scripts for data download and management
- Docker Compose definitions

### Step 2: Configure Environment Variables

Copy the example environment file:

```bash
cp .env.example .env
```

For local development, the default values work out of the box. For production or custom deployments, edit `.env` to customize:

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgresql://weatherwms:weatherwms@postgres:5432/weatherwms` | PostgreSQL connection string |
| `REDIS_URL` | `redis://redis:6379` | Redis connection string |
| `S3_ENDPOINT` | `http://minio:9000` | MinIO/S3 endpoint |
| `S3_BUCKET` | `weather-data` | Object storage bucket name |
| `TILE_CACHE_SIZE` | `10000` | L1 in-memory cache size (entries) |
| `REDIS_TILE_TTL_SECS` | `3600` | L2 Redis cache TTL (seconds) |

See [Configuration](./configuration.md) for the full list of environment variables.

### Step 3: Start Services

Use the provided startup script:

```bash
./scripts/start.sh
```

This script:
1. Builds Docker images for all services
2. Starts the complete stack (8 services):
   - PostgreSQL (database)
   - Redis (cache)
   - MinIO (object storage)
   - WMS API (web service)
   - Ingester (data processing)
   - Downloader (data fetching)
   - Web Dashboard (admin UI)
3. Initializes the database schema
4. Creates the MinIO bucket

**First-time startup takes 5-10 minutes** to build all Docker images. Subsequent starts are much faster.

### Step 4: Verify Installation

Check that all services are running:

```bash
docker-compose ps
```

You should see 8 services in the "Up" state:

```
NAME                    STATUS              PORTS
weather-wms-postgres    Up 30 seconds       5432/tcp
weather-wms-redis       Up 30 seconds       6379/tcp
weather-wms-minio       Up 30 seconds       9000-9001/tcp
weather-wms-wms-api     Up 15 seconds       0.0.0.0:8080->8080/tcp
weather-wms-ingester    Up 15 seconds       
weather-wms-downloader  Up 15 seconds       0.0.0.0:8081->8081/tcp
weather-wms-renderer    Up 15 seconds       
weather-wms-web         Up 15 seconds       0.0.0.0:8000->8000/tcp
```

Test the WMS API health endpoint:

```bash
curl http://localhost:8080/health
```

Expected response:
```json
{"status":"ok"}
```

### Step 5: Access the Web Dashboard

Open your browser and navigate to:

```
http://localhost:8000
```

The dashboard provides:
- Interactive map with weather layers
- Download status and controls
- System metrics and logs
- Admin functions (ingestion triggers, cache management)

### Step 6: Download Sample Data (Optional)

To see weather data immediately, download sample GOES satellite imagery:

```bash
./scripts/download_goes.sh
```

This downloads recent GOES-18 satellite data (~500 MB) and automatically triggers ingestion.

Wait 2-3 minutes for ingestion to complete, then refresh the web dashboard to see satellite layers.

## Verify Installation

Test WMS GetCapabilities to see available layers:

```bash
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0"
```

This returns an XML document listing all available layers and styles.

Request a sample tile (after downloading data):

```bash
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=goes18_CMI_C13&STYLES=default&CRS=EPSG:3857&BBOX=-13149614,3503549,-10018754,6634109&WIDTH=256&HEIGHT=256&FORMAT=image/png" -o tile.png
```

This saves a 256x256 PNG tile to `tile.png`.

## Monitoring

Access monitoring dashboards:

- **Grafana**: http://localhost:3001 (admin/admin)
- **Prometheus**: http://localhost:9090
- **MinIO Console**: http://localhost:9001 (minioadmin/minioadmin)

## Troubleshooting

### Services Won't Start

Check Docker logs:
```bash
docker-compose logs <service-name>
```

Common issues:
- **Port already in use**: Change ports in `docker-compose.yml`
- **Out of memory**: Increase Docker memory limit
- **Disk space**: Ensure at least 20 GB free

### No Data Showing

1. Verify data was downloaded:
   ```bash
   docker-compose logs downloader
   ```

2. Check ingestion status:
   ```bash
   docker-compose logs ingester
   ```

3. Query the catalog:
   ```bash
   docker-compose exec postgres psql -U weatherwms -d weatherwms -c "SELECT model, parameter, COUNT(*) FROM grid_catalog GROUP BY model, parameter;"
   ```

See [Troubleshooting](../reference/troubleshooting.md) for more solutions.

## Stopping Services

Stop all services:

```bash
docker-compose down
```

Stop and remove volumes (deletes all data):

```bash
docker-compose down -v
```

## Next Steps

- [Quick Start Guide](./quickstart.md) - Detailed walkthrough with examples
- [Configuration](./configuration.md) - Customize your deployment
- [Data Sources](../data-sources/README.md) - Learn about available weather data
