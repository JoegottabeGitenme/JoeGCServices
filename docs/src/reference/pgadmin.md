# pgAdmin Database Console

This guide explains how to use pgAdmin to connect to the PostgreSQL database and view data related to the Weather WMS system.

## Accessing pgAdmin

1. **Start the services** (if not already running):
   ```bash
   docker-compose up -d
   ```

2. **Open pgAdmin** in your browser:
   - URL: **http://localhost:5050**
   - Email: `admin@localhost.com`
   - Password: `admin`

## Connecting to PostgreSQL

Once logged into pgAdmin, you need to register the PostgreSQL server:

1. Right-click **Servers** → **Register** → **Server...**
2. Fill in the connection details:

| Field | Value |
|-------|-------|
| **Name** (General tab) | `WeatherWMS` (or any name you prefer) |
| **Host** (Connection tab) | `postgres` |
| **Port** | `5432` |
| **Username** | `weatherwms` |
| **Password** | `weatherwms` |
| **Database** | `weatherwms` |

> **Note**: Use `postgres` as the hostname since both pgAdmin and PostgreSQL containers are on the same Docker network. If connecting from outside Docker (e.g., a local pgAdmin installation), use `localhost` instead.

## Database Schema

The WMS system uses two main tables in the `weatherwms` database:

### `datasets` Table

Stores metadata for all ingested weather data:

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID | Primary key |
| `model` | VARCHAR(50) | Data source (GFS, HRRR, GOES, MRMS) |
| `parameter` | VARCHAR(100) | Weather parameter (temperature, wind, etc.) |
| `level` | VARCHAR(50) | Vertical level (surface, 500mb, etc.) |
| `reference_time` | TIMESTAMPTZ | Model run time |
| `forecast_hour` | INTEGER | Forecast hour offset |
| `valid_time` | TIMESTAMPTZ | Time the data is valid for |
| `bbox_min_x` | DOUBLE | Bounding box minimum X coordinate |
| `bbox_min_y` | DOUBLE | Bounding box minimum Y coordinate |
| `bbox_max_x` | DOUBLE | Bounding box maximum X coordinate |
| `bbox_max_y` | DOUBLE | Bounding box maximum Y coordinate |
| `storage_path` | TEXT | Path to the data file on disk |
| `file_size` | BIGINT | Size of the data file in bytes |
| `ingested_at` | TIMESTAMPTZ | When the data was ingested |
| `status` | VARCHAR(20) | Availability status (`available`, etc.) |

### `layer_styles` Table

Stores custom layer styling configurations:

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID | Primary key |
| `layer_id` | VARCHAR(200) | Layer identifier |
| `style_name` | VARCHAR(100) | Name of the style |
| `style_config` | JSONB | Style configuration as JSON |
| `created_at` | TIMESTAMPTZ | When the style was created |

## Useful Queries

Open the **Query Tool** in pgAdmin (right-click on the database → Query Tool) and try these queries:

### View Datasets by Model and Parameter

```sql
SELECT model, parameter, COUNT(*) 
FROM datasets 
GROUP BY model, parameter 
ORDER BY model, parameter;
```

### View Recent Ingestions

```sql
SELECT model, parameter, level, valid_time, storage_path 
FROM datasets 
ORDER BY ingested_at DESC 
LIMIT 20;
```

### Check Dataset Status Distribution

```sql
SELECT status, COUNT(*) 
FROM datasets 
GROUP BY status;
```

### View Available Time Range per Model

```sql
SELECT 
    model,
    MIN(valid_time) AS earliest,
    MAX(valid_time) AS latest,
    COUNT(*) AS total_datasets
FROM datasets 
WHERE status = 'available'
GROUP BY model;
```

### Find Datasets for a Specific Parameter

```sql
SELECT model, level, valid_time, storage_path
FROM datasets
WHERE parameter = 'TMP'
ORDER BY valid_time DESC
LIMIT 50;
```

### Check Storage Usage by Model

```sql
SELECT 
    model,
    COUNT(*) AS dataset_count,
    pg_size_pretty(SUM(file_size)) AS total_size
FROM datasets
GROUP BY model
ORDER BY SUM(file_size) DESC;
```

### View Custom Layer Styles

```sql
SELECT layer_id, style_name, created_at
FROM layer_styles
ORDER BY created_at DESC;
```

## Alternative: Command Line Access

If you prefer the command line, you can connect directly via `psql`:

```bash
# Interactive psql session
docker-compose exec postgres psql -U weatherwms -d weatherwms

# Run a single query
docker-compose exec postgres psql -U weatherwms -d weatherwms \
  -c "SELECT model, parameter, COUNT(*) FROM datasets GROUP BY model, parameter;"

# List all tables
docker-compose exec postgres psql -U weatherwms -c "\dt"
```

## Connection Details Summary

| Service | URL/Host | Credentials |
|---------|----------|-------------|
| PostgreSQL | `localhost:5432` (external) / `postgres:5432` (docker) | `weatherwms` / `weatherwms` |
| pgAdmin | http://localhost:5050 | `admin@localhost.com` / `admin` |

## Troubleshooting

### Cannot connect to PostgreSQL from pgAdmin

1. Ensure both containers are running:
   ```bash
   docker-compose ps
   ```

2. Verify PostgreSQL is healthy:
   ```bash
   docker-compose exec postgres pg_isready -U weatherwms
   ```

3. Check that you're using `postgres` (not `localhost`) as the hostname when connecting from within Docker.

### pgAdmin is slow or unresponsive

The pgAdmin container may take a moment to initialize on first startup. Wait 30-60 seconds after starting the services before accessing the web interface.

### Tables don't exist

The schema is created automatically when the WMS API or Ingester service starts. Run:
```bash
docker-compose up -d wms-api
```

Then refresh the database in pgAdmin.
