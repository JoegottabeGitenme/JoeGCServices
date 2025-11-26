# Startup Workflow - Automatic Data Ingestion

## Overview

The Weather WMS system now automatically ingests test data on every startup. This ensures:
- ✅ Complete pipeline validation
- ✅ Fresh data with each restart
- ✅ Reproducible testing environment
- ✅ Automatic verification of all components

## Startup Sequence

When you run `bash scripts/start.sh`, the system performs:

### Phase 1: Infrastructure Setup (30-60 seconds)
```
1. Start Docker Compose stack
   - PostgreSQL (database)
   - Redis (job queue)
   - MinIO (object storage)
   - WMS API service
   - Web Dashboard
   
2. Wait for all services to be healthy
   - PostgreSQL: ready for connections
   - Redis: responding to ping
   - API: listening on port 8080
   - Dashboard: listening on port 8000
```

### Phase 2: Data Ingestion (15-30 seconds)
```
1. Call ingest_test_data.sh script
   ↓
2. Wait for PostgreSQL (verify connectivity)
   ↓
3. Clear previous data (fresh start)
   DELETE FROM datasets WHERE status = 'available'
   ↓
4. Run ingester with test file
   cargo run --package ingester -- --test-file testdata/gfs_sample.grib2
   ↓
5. Parser reads 372 GRIB2 messages
   ↓
6. Extract 4 temperature datasets
   ↓
7. Register in PostgreSQL catalog
   ↓
8. Verify ingestion (database check)
   SELECT COUNT(*) FROM datasets WHERE status = 'available'
   Returns: 4 datasets registered
   ↓
9. Verify storage in MinIO
   Check if test file exists in weather-data bucket
```

### Phase 3: Validation (30-60 seconds)
```
1. Call test_rendering.sh script
   ↓
2. Wait for API to be ready
   ↓
3. Generate 5 test requests at different resolutions
   - Global (512x256)
   - North America (512x384)
   - Europe (512x384)
   - Tropical (512x256)
   - High-res (1024x1024)
   ↓
4. Verify PNG files are valid
   Each image: 2.6KB - 28KB
   ↓
5. Confirm color gradients present
```

## Workflow Diagram

```
┌─────────────────────────────────────────────────────────┐
│                 bash scripts/start.sh                    │
└────────────────────────┬────────────────────────────────┘
                         │
                         ▼
        ┌────────────────────────────────┐
        │ 1. Start Docker Compose Stack   │
        │    (PostgreSQL, Redis, MinIO)   │
        └────────┬───────────────────────┘
                 │ (30-60s)
                 ▼
        ┌────────────────────────────────┐
        │ 2. Wait for Services Healthy    │
        │    Check PostgreSQL/Redis ready │
        └────────┬───────────────────────┘
                 │ (30s timeout)
                 ▼
        ┌────────────────────────────────┐
        │ 3. Run Data Ingestion           │
        │    ingest_test_data.sh          │
        └────────┬───────────────────────┘
                 │ (15-30s)
                 │
        ┌────────┴───────────────────────┐
        │                                 │
        ▼                                 ▼
    Clear Old Data              Ingest New Data
    (DELETE datasets)           (cargo ingester)
                                ↓
                            Parse GRIB2
                            Extract Parameters
                            Register in DB
                            Store in MinIO
        │                                 │
        └────────┬───────────────────────┘
                 │
                 ▼
        ┌────────────────────────────────┐
        │ 4. Run Test Rendering           │
        │    test_rendering.sh            │
        └────────┬───────────────────────┘
                 │ (30-60s)
                 │
                 ▼
        ┌────────────────────────────────┐
        │ 5. System Ready                 │
        │    Dashboard: localhost:8000    │
        │    WMS API: localhost:8080      │
        └────────────────────────────────┘
```

## Key Features

### Automatic Data Management
- **Clear Previous Data**: Each startup clears old datasets for fresh state
- **Re-ingest Test Data**: 254MB GFS sample always available
- **Verify Storage**: Confirms files in MinIO
- **Validate Catalog**: Checks PostgreSQL registrations

### Reproducible Testing
- **Same Starting Point**: Every startup starts clean
- **Complete Pipeline**: Full ingestion to rendering validated
- **Automated Checks**: Database and storage verification built-in
- **No Manual Steps**: Single command to full system

### Monitoring
The ingestion script provides:
- ✅ Progress messages at each step
- ✅ Success/error indicators
- ✅ Data counts and timestamps
- ✅ Storage verification results

## Data Flow

```
testdata/gfs_sample.grib2 (254 MB, persisted locally)
        │
        ├─→ Ingester reads file
        │
        ├─→ Parser extracts messages
        │
        ├─→ Temperature data extracted (4 datasets)
        │
        ├─→ PostgreSQL: Register metadata
        │   • model: gfs
        │   • parameter: TMP
        │   • level: surface
        │   • reference_time: 2025-11-25 00:00:00
        │   • forecast_hour: 0
        │   • file_size: 266072064 bytes
        │
        └─→ MinIO: Store GRIB2 file
            • Bucket: weather-data
            • Path: test/gfs_sample.grib2
            • Size: 254 MB
```

## Example Output

```
[INFO] Ingesting test weather data...
[INFO] Starting test data ingestion...
[INFO] Waiting for PostgreSQL to be ready...
[SUCCESS] PostgreSQL is ready
[INFO] Clearing previous ingestion data...
[SUCCESS] Cleared previous data
[INFO] Running ingester with test data: testdata/gfs_sample.grib2
{"timestamp":"...", "level":"INFO", "message":"Test file ingestion completed", "messages":372, "parameters":4}
[INFO] Verifying ingestion...
[SUCCESS] Ingestion verified: 4 datasets registered
[INFO] Ingested datasets:
 model | parameter | level   | count
-------+-----------+---------+-------
 gfs   | TMP       | surface |     4
[INFO] Verifying storage in MinIO...
[SUCCESS] Test data confirmed in MinIO
[SUCCESS] ============================================
[SUCCESS] Data ingestion completed successfully!
```

## Troubleshooting

### Ingestion Fails
```bash
# Check logs
docker-compose logs ingester

# Verify test file exists
ls -lh testdata/gfs_sample.grib2

# Check database connectivity
docker-compose exec postgres psql -U weatherwms -d weatherwms
```

### No Datasets After Startup
```bash
# Query catalog
docker-compose exec postgres psql -U weatherwms -d weatherwms \
  -c "SELECT COUNT(*) FROM datasets WHERE status = 'available';"

# Check MinIO
docker-compose logs minio | grep gfs_sample
```

### Services Not Starting
```bash
# Check health
docker-compose ps

# View service logs
docker-compose logs wms-api
docker-compose logs postgres
```

## Timing

Typical full startup:
- Docker startup: 15-30 seconds
- Service health checks: 10-20 seconds
- Data ingestion: 15-30 seconds
- Test rendering: 30-60 seconds
- **Total: 70-140 seconds (typical ~2 minutes)**

## Next Run

To restart the system:
```bash
bash scripts/start.sh --stop
bash scripts/start.sh
```

This automatically:
1. Clears all old data
2. Starts fresh services
3. Re-ingests test data
4. Validates everything
5. Shows you the dashboard

No manual data management needed!
