# Automatic Data Ingestion & Purge System

## Overview

This document outlines the plan for implementing automatic data ingestion via cron-based downloaders and a data retention/purge system to manage storage.

---

## Current State

### Manual Ingestion
Currently, data ingestion is triggered manually via:
- Shell scripts: `scripts/download_gfs.sh`, `scripts/download_hrrr.sh`, etc.
- Direct ingester invocation: `cargo run --package ingester -- --file <path>`

### No Automatic Cleanup
- Data accumulates indefinitely in MinIO
- No mechanism to purge old data
- Retention hours are defined in YAML configs but not enforced

### Model Schedule Information
Each model YAML config already defines scheduling info:

```yaml
# config/models/gfs.yaml
schedule:
  cycles: [0, 6, 12, 18]      # UTC hours
  forecast_hours: 
    start: 0
    end: 384
    step: 3
  poll_interval_secs: 3600    # 1 hour
  delay_hours: 4              # Data available 4 hours after cycle

retention:
  hours: 168                  # 7 days
```

---

## Proposed Architecture

### High-Level Design

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           CRON SCHEDULER                                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │ GFS Job     │  │ HRRR Job    │  │ GOES Job    │  │ MRMS Job    │         │
│  │ */1 * * * * │  │ */1 * * * * │  │ */5 * * * * │  │ */2 * * * * │         │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘         │
│         │                │                │                │                 │
└─────────┼────────────────┼────────────────┼────────────────┼─────────────────┘
          │                │                │                │
          ▼                ▼                ▼                ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          INGESTER SERVICE                                    │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                        Download Manager                               │   │
│  │  • Check S3 for new files                                            │   │
│  │  • Track already-ingested files (dedup)                              │   │
│  │  • Parallel downloads                                                 │   │
│  │  • Retry with backoff                                                │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                    │                                         │
│                                    ▼                                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                        Shredding Pipeline                             │   │
│  │  • Parse GRIB2/NetCDF                                                │   │
│  │  • Extract configured parameters                                      │   │
│  │  • Store to MinIO                                                    │   │
│  │  • Register in Catalog                                               │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
          │                                                      │
          ▼                                                      ▼
┌──────────────────┐                              ┌──────────────────────────┐
│    PostgreSQL    │                              │   PURGE WORKER           │
│                  │                              │                          │
│  - datasets      │                              │  • Runs every hour       │
│  - ingest_log    │                              │  • Checks retention age  │
│  - ingest_state  │                              │  • Deletes old files     │
│                  │                              │  • Updates catalog       │
└──────────────────┘                              └──────────────────────────┘
```

---

## Implementation Plan

### Phase 1: Ingester Daemon Mode

**Goal**: Enable the ingester to run continuously, checking for new data on a schedule.

#### 1.1 Add Daemon Mode Flag

```rust
// services/ingester/src/main.rs

#[derive(Parser)]
struct Cli {
    /// Run once and exit (for cron/batch)
    #[arg(long)]
    once: bool,
    
    /// Run as daemon (continuous polling)
    #[arg(long)]
    daemon: bool,
    
    /// Models to ingest (comma-separated, empty = all enabled)
    #[arg(long)]
    models: Option<String>,
    
    /// Override poll interval (seconds)
    #[arg(long)]
    poll_interval: Option<u64>,
}
```

#### 1.2 Implement Poll Loop

```rust
async fn run_daemon(config: &IngestConfig) -> Result<()> {
    info!("Starting ingester daemon mode");
    
    loop {
        for model_config in &config.models {
            if !model_config.model.enabled {
                continue;
            }
            
            match check_and_ingest_model(model_config).await {
                Ok(count) => {
                    info!(model = %model_config.model.id, files = count, "Ingestion cycle complete");
                }
                Err(e) => {
                    error!(model = %model_config.model.id, error = %e, "Ingestion failed");
                }
            }
        }
        
        // Wait for next poll cycle
        let interval = config.min_poll_interval();
        info!(seconds = interval, "Sleeping until next poll");
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}
```

#### 1.3 Track Ingestion State

Add a table to track what's been ingested:

```sql
CREATE TABLE IF NOT EXISTS ingest_state (
    id SERIAL PRIMARY KEY,
    model VARCHAR(32) NOT NULL,
    source_key VARCHAR(512) NOT NULL UNIQUE,  -- S3 key or URL
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    file_size BIGINT,
    parameters_extracted INT,
    status VARCHAR(16) DEFAULT 'success'
);

CREATE INDEX idx_ingest_state_model ON ingest_state(model);
CREATE INDEX idx_ingest_state_ingested ON ingest_state(ingested_at);
```

```rust
async fn is_already_ingested(catalog: &Catalog, source_key: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM ingest_state WHERE source_key = $1)"
    )
    .bind(source_key)
    .fetch_one(&catalog.pool)
    .await
    .unwrap_or(false)
}

async fn mark_ingested(catalog: &Catalog, model: &str, source_key: &str, params: i32) {
    sqlx::query(
        "INSERT INTO ingest_state (model, source_key, parameters_extracted) VALUES ($1, $2, $3)
         ON CONFLICT (source_key) DO NOTHING"
    )
    .bind(model)
    .bind(source_key)
    .bind(params)
    .execute(&catalog.pool)
    .await
    .ok();
}
```

---

### Phase 2: Data Discovery

**Goal**: Implement smart discovery of available data for each model.

#### 2.1 GFS Discovery

```rust
async fn discover_gfs_files(config: &ModelConfig) -> Result<Vec<SourceFile>> {
    let client = aws_sdk_s3::Client::new(&aws_config().await);
    let bucket = &config.source.bucket;
    
    // Calculate which cycles to look for
    let now = Utc::now();
    let delay = Duration::hours(config.schedule.delay_hours as i64);
    let available_time = now - delay;
    
    let mut files = Vec::new();
    
    // Check last 24 hours of cycles
    for hours_ago in 0..24 {
        let check_time = available_time - Duration::hours(hours_ago);
        let date = check_time.format("%Y%m%d").to_string();
        
        for cycle in &config.schedule.cycles {
            if check_time.hour() < *cycle as u32 {
                continue; // Cycle hasn't happened yet
            }
            
            let prefix = format!("gfs.{}/{:02}/atmos/", date, cycle);
            
            // List files matching pattern
            let resp = client.list_objects_v2()
                .bucket(bucket)
                .prefix(&prefix)
                .send()
                .await?;
            
            for obj in resp.contents() {
                if let Some(key) = obj.key() {
                    if key.ends_with(".grib2") && key.contains("pgrb2.0p25") {
                        files.push(SourceFile {
                            key: key.to_string(),
                            size: obj.size().unwrap_or(0) as u64,
                            last_modified: obj.last_modified().map(|t| t.to_chrono()),
                        });
                    }
                }
            }
        }
    }
    
    Ok(files)
}
```

#### 2.2 HRRR Discovery

```rust
async fn discover_hrrr_files(config: &ModelConfig) -> Result<Vec<SourceFile>> {
    let client = aws_sdk_s3::Client::new(&aws_config().await);
    let bucket = &config.source.bucket;
    
    let now = Utc::now();
    let delay = Duration::hours(config.schedule.delay_hours as i64);
    let available_time = now - delay;
    
    let mut files = Vec::new();
    
    // HRRR runs hourly - check last 48 hours
    for hours_ago in 0..48 {
        let check_time = available_time - Duration::hours(hours_ago);
        let date = check_time.format("%Y%m%d").to_string();
        let cycle = check_time.hour();
        
        let prefix = format!("hrrr.{}/conus/hrrr.t{:02}z.wrfsfcf", date, cycle);
        
        let resp = client.list_objects_v2()
            .bucket(bucket)
            .prefix(&prefix)
            .send()
            .await?;
        
        for obj in resp.contents() {
            if let Some(key) = obj.key() {
                if key.ends_with(".grib2") {
                    files.push(SourceFile {
                        key: key.to_string(),
                        size: obj.size().unwrap_or(0) as u64,
                        last_modified: obj.last_modified().map(|t| t.to_chrono()),
                    });
                }
            }
        }
    }
    
    Ok(files)
}
```

#### 2.3 GOES Discovery

```rust
async fn discover_goes_files(config: &ModelConfig) -> Result<Vec<SourceFile>> {
    let client = aws_sdk_s3::Client::new(&aws_config().await);
    let bucket = &config.source.bucket;  // noaa-goes16 or noaa-goes18
    
    let now = Utc::now();
    let year = now.year();
    let doy = now.ordinal();
    
    let mut files = Vec::new();
    
    // Check last 2 hours
    for hours_ago in 0..2 {
        let check_time = now - Duration::hours(hours_ago);
        let hour = check_time.hour();
        
        let prefix = format!("ABI-L2-CMIPC/{}/{:03}/{:02}/", year, doy, hour);
        
        let resp = client.list_objects_v2()
            .bucket(bucket)
            .prefix(&prefix)
            .send()
            .await?;
        
        for obj in resp.contents() {
            if let Some(key) = obj.key() {
                // Filter for specific bands we want
                if key.contains("_C02_") || key.contains("_C08_") || key.contains("_C13_") {
                    files.push(SourceFile {
                        key: key.to_string(),
                        size: obj.size().unwrap_or(0) as u64,
                        last_modified: obj.last_modified().map(|t| t.to_chrono()),
                    });
                }
            }
        }
    }
    
    Ok(files)
}
```

#### 2.4 MRMS Discovery

```rust
async fn discover_mrms_files(config: &ModelConfig) -> Result<Vec<SourceFile>> {
    let client = aws_sdk_s3::Client::new(&aws_config().await);
    let bucket = "noaa-mrms-pds";
    
    let products = ["MergedReflectivityQCComposite", "PrecipRate"];
    let mut files = Vec::new();
    
    for product in products {
        let prefix = format!("CONUS/{}/", product);
        
        // MRMS uses a directory per timestamp
        let resp = client.list_objects_v2()
            .bucket(bucket)
            .prefix(&prefix)
            .delimiter("/")
            .send()
            .await?;
        
        // Get the most recent directory
        if let Some(prefixes) = resp.common_prefixes() {
            if let Some(latest) = prefixes.last() {
                let latest_prefix = latest.prefix().unwrap_or("");
                
                let files_resp = client.list_objects_v2()
                    .bucket(bucket)
                    .prefix(latest_prefix)
                    .send()
                    .await?;
                
                for obj in files_resp.contents() {
                    if let Some(key) = obj.key() {
                        if key.ends_with(".grib2") {
                            files.push(SourceFile {
                                key: key.to_string(),
                                size: obj.size().unwrap_or(0) as u64,
                                last_modified: obj.last_modified().map(|t| t.to_chrono()),
                            });
                        }
                    }
                }
            }
        }
    }
    
    Ok(files)
}
```

---

### Phase 3: Data Purge System

**Goal**: Automatically delete data older than the retention period.

#### 3.1 Purge Worker

```rust
// services/ingester/src/purge.rs

pub async fn run_purge_cycle(catalog: &Catalog, storage: &ObjectStorage) -> Result<PurgeStats> {
    info!("Starting purge cycle");
    
    let mut stats = PurgeStats::default();
    
    // Load all model configs to get retention periods
    let model_configs = load_all_model_configs()?;
    
    for config in model_configs {
        let retention_hours = config.retention.hours;
        let cutoff = Utc::now() - Duration::hours(retention_hours as i64);
        
        info!(
            model = %config.model.id, 
            retention_hours = retention_hours,
            cutoff = %cutoff,
            "Purging old data"
        );
        
        // Find datasets older than retention period
        let old_datasets = catalog.find_datasets_before(&config.model.id, cutoff).await?;
        
        for dataset in old_datasets {
            // Delete from storage
            if let Err(e) = storage.delete(&dataset.storage_path).await {
                warn!(path = %dataset.storage_path, error = %e, "Failed to delete file");
                stats.failed += 1;
                continue;
            }
            
            // Delete from catalog
            if let Err(e) = catalog.delete_dataset(&dataset.model, &dataset.parameter, 
                                                    &dataset.level, dataset.reference_time,
                                                    dataset.forecast_hour).await {
                warn!(error = %e, "Failed to delete catalog entry");
                stats.failed += 1;
                continue;
            }
            
            stats.deleted += 1;
            stats.bytes_freed += dataset.file_size;
        }
        
        // Also clean up ingest_state entries
        let deleted_state = catalog.cleanup_ingest_state(&config.model.id, cutoff).await?;
        stats.state_entries_cleaned += deleted_state;
    }
    
    info!(
        deleted = stats.deleted,
        bytes_freed = stats.bytes_freed,
        failed = stats.failed,
        "Purge cycle complete"
    );
    
    Ok(stats)
}

#[derive(Default)]
pub struct PurgeStats {
    pub deleted: u64,
    pub bytes_freed: u64,
    pub failed: u64,
    pub state_entries_cleaned: u64,
}
```

#### 3.2 Catalog Extensions

```rust
// crates/storage/src/catalog.rs

impl Catalog {
    /// Find datasets older than a cutoff time
    pub async fn find_datasets_before(
        &self, 
        model: &str, 
        cutoff: DateTime<Utc>
    ) -> WmsResult<Vec<CatalogEntry>> {
        let rows = sqlx::query_as::<_, DatasetRow>(
            r#"
            SELECT model, parameter, level, reference_time, forecast_hour,
                   bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y,
                   storage_path, file_size
            FROM datasets
            WHERE model = $1 AND reference_time < $2
            ORDER BY reference_time ASC
            LIMIT 10000
            "#
        )
        .bind(model)
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;
        
        Ok(rows.into_iter().map(|r| r.into()).collect())
    }
    
    /// Delete a specific dataset entry
    pub async fn delete_dataset(
        &self,
        model: &str,
        parameter: &str,
        level: &str,
        reference_time: DateTime<Utc>,
        forecast_hour: u32,
    ) -> WmsResult<()> {
        sqlx::query(
            r#"
            DELETE FROM datasets 
            WHERE model = $1 AND parameter = $2 AND level = $3 
              AND reference_time = $4 AND forecast_hour = $5
            "#
        )
        .bind(model)
        .bind(parameter)
        .bind(level)
        .bind(reference_time)
        .bind(forecast_hour as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Delete failed: {}", e)))?;
        
        Ok(())
    }
    
    /// Clean up old ingest state entries
    pub async fn cleanup_ingest_state(&self, model: &str, cutoff: DateTime<Utc>) -> WmsResult<u64> {
        let result = sqlx::query(
            "DELETE FROM ingest_state WHERE model = $1 AND ingested_at < $2"
        )
        .bind(model)
        .bind(cutoff)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Cleanup failed: {}", e)))?;
        
        Ok(result.rows_affected())
    }
}
```

#### 3.3 Purge Scheduling

The purge can run either:
1. As part of the ingester daemon (run after each ingestion cycle)
2. As a separate cron job
3. Triggered manually via admin API

```rust
// In daemon mode
async fn run_daemon(config: &IngestConfig) -> Result<()> {
    let mut last_purge = Instant::now();
    let purge_interval = Duration::from_secs(config.ingestion.cleanup_interval_hours * 3600);
    
    loop {
        // ... ingestion logic ...
        
        // Run purge if interval elapsed
        if config.ingestion.cleanup_enabled && last_purge.elapsed() > purge_interval {
            match run_purge_cycle(&catalog, &storage).await {
                Ok(stats) => {
                    info!(deleted = stats.deleted, bytes = stats.bytes_freed, "Purge complete");
                }
                Err(e) => {
                    error!(error = %e, "Purge failed");
                }
            }
            last_purge = Instant::now();
        }
        
        tokio::time::sleep(poll_interval).await;
    }
}
```

---

### Phase 4: Kubernetes Deployment Options

#### Option A: Single Ingester Deployment (Daemon Mode)

```yaml
# deploy/helm/weather-wms/templates/ingester-deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "weather-wms.fullname" . }}-ingester
spec:
  replicas: 1
  template:
    spec:
      containers:
        - name: ingester
          image: "{{ .Values.ingester.image.repository }}:{{ .Values.ingester.image.tag }}"
          args: ["--daemon"]
          env:
            - name: LOG_LEVEL
              value: "info"
            - name: ENABLED_MODELS
              value: "gfs,hrrr,goes16,mrms"
          resources:
            requests:
              memory: "512Mi"
              cpu: "500m"
            limits:
              memory: "2Gi"
              cpu: "2000m"
```

#### Option B: CronJob per Model

```yaml
# GFS CronJob (runs every hour)
apiVersion: batch/v1
kind: CronJob
metadata:
  name: ingester-gfs
spec:
  schedule: "0 * * * *"  # Every hour at :00
  concurrencyPolicy: Forbid
  jobTemplate:
    spec:
      template:
        spec:
          containers:
            - name: ingester
              image: "weather-wms-ingester:latest"
              args: ["--once", "--models", "gfs"]
          restartPolicy: OnFailure

---
# HRRR CronJob (runs every hour)
apiVersion: batch/v1
kind: CronJob
metadata:
  name: ingester-hrrr
spec:
  schedule: "15 * * * *"  # Every hour at :15
  concurrencyPolicy: Forbid
  jobTemplate:
    spec:
      template:
        spec:
          containers:
            - name: ingester
              image: "weather-wms-ingester:latest"
              args: ["--once", "--models", "hrrr"]
          restartPolicy: OnFailure

---
# GOES CronJob (runs every 5 minutes)
apiVersion: batch/v1
kind: CronJob
metadata:
  name: ingester-goes
spec:
  schedule: "*/5 * * * *"
  concurrencyPolicy: Forbid
  jobTemplate:
    spec:
      template:
        spec:
          containers:
            - name: ingester
              image: "weather-wms-ingester:latest"
              args: ["--once", "--models", "goes16,goes18"]
          restartPolicy: OnFailure

---
# MRMS CronJob (runs every 2 minutes)
apiVersion: batch/v1
kind: CronJob
metadata:
  name: ingester-mrms
spec:
  schedule: "*/2 * * * *"
  concurrencyPolicy: Forbid
  jobTemplate:
    spec:
      template:
        spec:
          containers:
            - name: ingester
              image: "weather-wms-ingester:latest"
              args: ["--once", "--models", "mrms"]
          restartPolicy: OnFailure

---
# Purge CronJob (runs every 6 hours)
apiVersion: batch/v1
kind: CronJob
metadata:
  name: ingester-purge
spec:
  schedule: "0 */6 * * *"
  concurrencyPolicy: Forbid
  jobTemplate:
    spec:
      template:
        spec:
          containers:
            - name: ingester
              image: "weather-wms-ingester:latest"
              args: ["--purge-only"]
          restartPolicy: OnFailure
```

---

## Configuration Reference

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `ENABLED_MODELS` | (all) | Comma-separated list of models to ingest |
| `POLL_INTERVAL` | (from YAML) | Override poll interval in seconds |
| `CLEANUP_ENABLED` | true | Enable automatic purge |
| `CLEANUP_INTERVAL_HOURS` | 6 | Hours between purge runs |
| `PARALLEL_DOWNLOADS` | 4 | Max concurrent downloads |
| `MAX_RETRIES` | 3 | Retry count for failed downloads |

### Model-Specific Settings

From `config/models/*.yaml`:

| Model | Poll Interval | Delay | Retention |
|-------|--------------|-------|-----------|
| GFS | 1 hour | 4 hours | 7 days |
| HRRR | 1 hour | 2 hours | 3 days |
| GOES-16/18 | 5 minutes | 15 minutes | 1 day |
| MRMS | 2 minutes | 2 minutes | 1 day |

---

## Monitoring & Observability

### Prometheus Metrics

```rust
// New metrics for ingestion
lazy_static! {
    static ref INGESTION_RUNS: IntCounterVec = register_int_counter_vec!(
        "ingestion_runs_total",
        "Total ingestion runs by model and status",
        &["model", "status"]
    ).unwrap();
    
    static ref FILES_INGESTED: IntCounterVec = register_int_counter_vec!(
        "files_ingested_total",
        "Total files ingested by model",
        &["model"]
    ).unwrap();
    
    static ref INGESTION_DURATION: HistogramVec = register_histogram_vec!(
        "ingestion_duration_seconds",
        "Ingestion cycle duration",
        &["model"]
    ).unwrap();
    
    static ref PURGE_FILES_DELETED: IntCounter = register_int_counter!(
        "purge_files_deleted_total",
        "Total files deleted by purge"
    ).unwrap();
    
    static ref PURGE_BYTES_FREED: IntCounter = register_int_counter!(
        "purge_bytes_freed_total",
        "Total bytes freed by purge"
    ).unwrap();
}
```

### Admin Dashboard Additions

Add to the admin dashboard:
- Ingestion job status per model
- Last successful ingestion time
- Next scheduled poll time
- Purge statistics (files deleted, bytes freed)
- Ingestion error log

---

## Implementation Priority

| Phase | Task | Priority | Effort |
|-------|------|----------|--------|
| 1.1 | Add daemon mode flag | High | Low |
| 1.2 | Implement poll loop | High | Medium |
| 1.3 | Track ingestion state | High | Medium |
| 2.1 | GFS discovery | High | Medium |
| 2.2 | HRRR discovery | High | Low |
| 2.3 | GOES discovery | Medium | Medium |
| 2.4 | MRMS discovery | Medium | Medium |
| 3.1 | Purge worker | High | Medium |
| 3.2 | Catalog extensions | High | Low |
| 4 | K8s deployment | Medium | Low |

---

## Testing Plan

1. **Unit Tests**
   - Test file discovery logic
   - Test retention calculation
   - Test deduplication

2. **Integration Tests**
   - Test against real S3 buckets (read-only)
   - Test catalog state tracking
   - Test purge with mock data

3. **End-to-End Tests**
   - Run daemon for 24 hours
   - Verify all models get fresh data
   - Verify purge removes old data
   - Monitor memory/CPU usage

---

## Rollout Plan

1. **Week 1**: Implement daemon mode + state tracking
2. **Week 2**: Implement discovery for GFS and HRRR
3. **Week 3**: Implement discovery for GOES and MRMS
4. **Week 4**: Implement purge system
5. **Week 5**: Deploy to staging, monitor for 1 week
6. **Week 6**: Deploy to production
