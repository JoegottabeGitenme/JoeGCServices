# Comprehensive Environment Configuration & Load Test Dashboard Plan

**Date**: November 28, 2024  
**Status**: Planning Complete

---

## Overview

This plan covers four major improvements:

1. **Centralized Environment Configuration** - Single `.env.example` file for all system settings
2. **Feature Flags for Optimizations** - Toggle performance features on/off
3. **Enhanced Data Ingestion** - Control HRRR/GOES ingestion amounts in startup script
4. **Load Test Dashboard** - Web UI for viewing load test results history

---

## Part 1: Centralized Environment Configuration

### 1.1 Create `.env.example` at Repository Root

**File**: `.env.example`

This file will document ALL configurable environment variables across the system, organized by category.

```env
# ============================================================================
# Weather WMS - Environment Configuration
# ============================================================================
# Copy this file to .env and customize values for your deployment.
# These settings are read by docker-compose.yml and the startup scripts.
# ============================================================================

# ----------------------------------------------------------------------------
# DATABASE SETTINGS
# ----------------------------------------------------------------------------
POSTGRES_USER=weatherwms
POSTGRES_PASSWORD=weatherwms
POSTGRES_DB=weatherwms
DATABASE_URL=postgresql://weatherwms:weatherwms@postgres:5432/weatherwms
DATABASE_POOL_SIZE=50                # PostgreSQL connection pool size (default: 50)

# ----------------------------------------------------------------------------
# REDIS CACHE SETTINGS
# ----------------------------------------------------------------------------
REDIS_URL=redis://redis:6379
REDIS_TILE_TTL_SECS=3600             # L2 tile cache TTL in seconds (default: 3600 = 1hr)

# ----------------------------------------------------------------------------
# OBJECT STORAGE (MINIO/S3)
# ----------------------------------------------------------------------------
S3_ENDPOINT=http://minio:9000
S3_BUCKET=weather-data
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_REGION=us-east-1
S3_ALLOW_HTTP=true

# ----------------------------------------------------------------------------
# PERFORMANCE TUNING
# ----------------------------------------------------------------------------
TOKIO_WORKER_THREADS=8               # Async runtime worker threads (default: CPU cores)
RUST_LOG=info                        # Logging level: debug, info, warn, error

# ============================================================================
# PERFORMANCE OPTIMIZATION FEATURE FLAGS
# ============================================================================
# Set to "true" to enable, "false" to disable
# Useful for benchmarking individual optimizations

# --- L1 In-Memory Tile Cache (Phase 7.A) ---
ENABLE_L1_CACHE=true                 # Enable in-memory tile cache (huge performance boost)
TILE_CACHE_SIZE=10000                # Max tiles in L1 cache (~300MB at 30KB/tile)
TILE_CACHE_TTL_SECS=300              # L1 cache entry TTL (default: 5 minutes)

# --- Zarr Chunk Cache ---
ENABLE_CHUNK_CACHE=true              # Enable Zarr chunk caching (reduces storage I/O)
CHUNK_CACHE_SIZE_MB=1024             # Chunk cache size in MB (~1GB)

# --- Tile Prefetching (Phase 7.B) ---
ENABLE_PREFETCH=true                 # Enable predictive tile prefetching
PREFETCH_RINGS=2                     # Rings to prefetch: 1=8 tiles, 2=24 tiles (default: 2)
PREFETCH_MIN_ZOOM=3                  # Minimum zoom level for prefetch
PREFETCH_MAX_ZOOM=12                 # Maximum zoom level for prefetch

# --- Cache Warming (Phase 7.D) ---
ENABLE_CACHE_WARMING=true            # Pre-render tiles at startup
CACHE_WARMING_MAX_ZOOM=4             # Max zoom level to warm (0-4 = 341 tiles)
CACHE_WARMING_HOURS=0                # Forecast hours to warm (comma-separated: 0,3,6)
CACHE_WARMING_LAYERS=gfs_TMP:temperature  # Layers to warm (semicolon-separated layer:style)
CACHE_WARMING_CONCURRENCY=10         # Parallel warming tasks

# ============================================================================
# DATA INGESTION SETTINGS
# ============================================================================
# Controls which data sources are ingested at startup

# --- GFS (Global Forecast System) ---
INGEST_GFS=true                      # Enable GFS ingestion
GFS_FORECAST_HOURS=0,3,6,12,24       # Forecast hours to download (comma-separated)

# --- HRRR (High-Resolution Rapid Refresh) ---
INGEST_HRRR=true                     # Enable HRRR ingestion
HRRR_FORECAST_HOURS=0,1,2,3,6        # Forecast hours to download (comma-separated)
HRRR_MAX_FILES=6                     # Maximum HRRR files to download (limit for demos)

# --- GOES Satellite Imagery ---
INGEST_GOES=true                     # Enable GOES satellite ingestion
GOES_CHANNELS=C02,C13                # Channels to download (C02=Visible, C13=IR)
GOES_MAX_FILES=2                     # Maximum GOES files to download

# --- MRMS Radar Data ---
INGEST_MRMS=true                     # Enable MRMS radar ingestion
MRMS_MAX_FILES=60                    # Maximum MRMS timestamps to keep

# ============================================================================
# MONITORING & OBSERVABILITY
# ============================================================================
GRAFANA_ADMIN_PASSWORD=admin
PROMETHEUS_RETENTION_DAYS=15

# ============================================================================
# LOAD TESTING
# ============================================================================
LOAD_TEST_RESULTS_DIR=./validation/load-test/results
LOAD_TEST_DEFAULT_CONCURRENCY=10
LOAD_TEST_DEFAULT_DURATION=60
```

### 1.2 Update `docker-compose.yml`

Modify to use `env_file` directive and reference `.env`:

```yaml
services:
  wms-api:
    env_file: .env
    environment:
      # Override/add service-specific vars that need interpolation
      DATABASE_URL: ${DATABASE_URL}
      REDIS_URL: ${REDIS_URL}
      # ... etc
```

### 1.3 Files to Modify

| File | Changes |
|------|---------|
| `.env.example` | **CREATE** - Template with all variables |
| `.gitignore` | Add `.env` (keep secrets out of git) |
| `docker-compose.yml` | Add `env_file: .env`, use variable references |
| `scripts/start.sh` | Load `.env` if exists, set defaults |
| `scripts/ingest_test_data.sh` | Read ingestion settings from env |

---

## Part 2: Feature Flags for Optimizations

### 2.1 Update `services/wms-api/src/state.rs`

Add feature flag parsing and storage:

```rust
pub struct OptimizationConfig {
    // L1 Cache
    pub l1_cache_enabled: bool,
    pub l1_cache_size: usize,
    pub l1_cache_ttl_secs: u64,
    
    // Chunk Cache
    pub chunk_cache_enabled: bool,
    pub chunk_cache_size_mb: usize,
    
    // Prefetch
    pub prefetch_enabled: bool,
    pub prefetch_rings: u32,
    pub prefetch_min_zoom: u32,
    pub prefetch_max_zoom: u32,
    
    // Cache Warming
    pub cache_warming_enabled: bool,
    pub cache_warming_max_zoom: u32,
    pub cache_warming_hours: Vec<u32>,
    pub cache_warming_layers: Vec<(String, String)>,
    pub cache_warming_concurrency: usize,
}

impl OptimizationConfig {
    pub fn from_env() -> Self {
        fn parse_bool(key: &str, default: bool) -> bool {
            env::var(key)
                .map(|v| v.to_lowercase() == "true" || v == "1")
                .unwrap_or(default)
        }
        
        // ... parsing logic
    }
}

pub struct AppState {
    // ... existing fields ...
    pub optimization_config: OptimizationConfig,
}
```

### 2.2 Update `services/wms-api/src/handlers.rs`

Conditionally enable prefetching:

```rust
// Only prefetch if enabled in config
if state.optimization_config.prefetch_enabled 
   && z >= state.optimization_config.prefetch_min_zoom
   && z <= state.optimization_config.prefetch_max_zoom 
{
    spawn_tile_prefetch(...);
}
```

### 2.3 Update L1 Cache Usage

Make L1 cache lookups conditional:

```rust
// Check L1 cache only if enabled
if state.optimization_config.l1_cache_enabled {
    if let Some(data) = state.tile_memory_cache.get(&cache_key).await {
        return l1_hit_response(data);
    }
}
```

### 2.4 Add API Endpoint for Config Inspection

New endpoint: `GET /api/config`

```rust
pub async fn config_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    Json(json!({
        "optimizations": {
            "l1_cache": {
                "enabled": state.optimization_config.l1_cache_enabled,
                "size": state.optimization_config.l1_cache_size,
                "ttl_secs": state.optimization_config.l1_cache_ttl_secs,
            },
            "chunk_cache": {
                "enabled": state.optimization_config.chunk_cache_enabled,
                "size_mb": state.optimization_config.chunk_cache_size_mb,
            },
            "prefetch": {
                "enabled": state.optimization_config.prefetch_enabled,
                "rings": state.optimization_config.prefetch_rings,
            },
            "cache_warming": {
                "enabled": state.optimization_config.cache_warming_enabled,
                "max_zoom": state.optimization_config.cache_warming_max_zoom,
            }
        }
    }))
}
```

### 2.5 Files to Modify

| File | Changes |
|------|---------|
| `services/wms-api/src/state.rs` | Add `OptimizationConfig` struct |
| `services/wms-api/src/handlers.rs` | Add config conditionals, `/api/config` endpoint |
| `services/wms-api/src/main.rs` | Wire up config endpoint |
| `services/wms-api/src/warming.rs` | Use config from state |

---

## Part 3: Enhanced Data Ingestion

### 3.1 Update `scripts/ingest_test_data.sh`

Add environment variable controls for each data source:

```bash
# Load environment settings
if [ -f "$PROJECT_ROOT/.env" ]; then
    source "$PROJECT_ROOT/.env"
fi

# Set defaults from env or use fallbacks
INGEST_GFS="${INGEST_GFS:-true}"
INGEST_HRRR="${INGEST_HRRR:-true}"
INGEST_GOES="${INGEST_GOES:-true}"
INGEST_MRMS="${INGEST_MRMS:-true}"

HRRR_MAX_FILES="${HRRR_MAX_FILES:-6}"
GOES_MAX_FILES="${GOES_MAX_FILES:-2}"

# Conditionally ingest each data source
if [ "$INGEST_GFS" = "true" ]; then
    log_info "=== Ingesting GFS data ==="
    # ... existing GFS ingestion
fi

if [ "$INGEST_HRRR" = "true" ]; then
    log_info "=== Downloading and Ingesting HRRR data ==="
    # Download up to HRRR_MAX_FILES
    bash "$SCRIPT_DIR/download_hrrr.sh" "$DATE" "00" "${HRRR_FORECAST_HOURS:-0 1 2 3 6}" | head -$HRRR_MAX_FILES
    # ... ingest
fi

if [ "$INGEST_GOES" = "true" ]; then
    log_info "=== Downloading and Ingesting GOES data ==="
    # Download up to GOES_MAX_FILES
    bash "$SCRIPT_DIR/download_goes.sh" "data/goes" | head -$GOES_MAX_FILES
    # ... ingest
fi
```

### 3.2 Modify Download Scripts

Update `download_hrrr.sh`:
- Accept `MAX_FILES` environment variable
- Exit after downloading N files

Update `download_goes.sh`:
- Accept `CHANNELS` and `MAX_FILES` environment variables
- Exit after downloading N files

### 3.3 Update `scripts/start.sh`

Add `.env` loading at the beginning:

```bash
# Load environment configuration if exists
if [ -f "$PROJECT_ROOT/.env" ]; then
    log_info "Loading configuration from .env"
    set -a  # Export all variables
    source "$PROJECT_ROOT/.env"
    set +a
fi
```

### 3.4 Files to Modify

| File | Changes |
|------|---------|
| `scripts/start.sh` | Load `.env` file |
| `scripts/ingest_test_data.sh` | Add conditional ingestion, env var controls |
| `scripts/download_hrrr.sh` | Add `MAX_FILES` limit |
| `scripts/download_goes.sh` | Add `MAX_FILES`, `CHANNELS` controls |

---

## Part 4: Load Test Dashboard

### 4.1 Enhanced Results Storage

Modify load test tool to output structured JSON with more metadata:

**File**: `validation/load-test/src/report.rs`

```rust
#[derive(Serialize)]
pub struct LoadTestRun {
    pub id: String,                    // UUID for this run
    pub timestamp: DateTime<Utc>,
    pub scenario_name: String,
    pub scenario_file: String,
    pub duration_secs: f64,
    pub concurrency: u32,
    pub warmup_secs: u64,
    
    // Results
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub requests_per_second: f64,
    
    // Latency percentiles (ms)
    pub latency_p50: f64,
    pub latency_p90: f64,
    pub latency_p95: f64,
    pub latency_p99: f64,
    pub latency_min: f64,
    pub latency_max: f64,
    pub latency_avg: f64,
    
    // Cache stats
    pub cache_hit_rate: f64,
    pub l1_hit_rate: Option<f64>,
    pub l2_hit_rate: Option<f64>,
    
    // Throughput
    pub bytes_per_second: f64,
    
    // System info at test time
    pub system_config: SystemConfig,
    
    // Layers tested
    pub layers: Vec<String>,
}

#[derive(Serialize)]
pub struct SystemConfig {
    pub l1_cache_enabled: bool,
    pub l1_cache_size: usize,
    pub chunk_cache_enabled: bool,
    pub prefetch_enabled: bool,
    pub prefetch_rings: u32,
    pub cache_warming_enabled: bool,
}
```

### 4.2 Update `scripts/run_load_test.sh`

Add automatic JSON result saving:

```bash
# Always save results to JSON (in addition to displayed format)
RESULTS_FILE="$RESULTS_DIR/runs.jsonl"  # JSON Lines format for easy appending

# After running test, append to JSONL file
"$LOAD_TEST_BIN" run --scenario "$SCENARIO_FILE" --output json >> "$RESULTS_FILE"
```

### 4.3 Create Load Test API Endpoints

**File**: `services/wms-api/src/handlers.rs`

New endpoints:

```rust
/// GET /api/loadtest/results - List all load test runs
pub async fn loadtest_results_handler() -> impl IntoResponse {
    let results_file = std::env::var("LOAD_TEST_RESULTS_DIR")
        .unwrap_or_else(|_| "./validation/load-test/results".to_string());
    let runs_file = format!("{}/runs.jsonl", results_file);
    
    // Read JSONL file and parse into array
    let runs: Vec<LoadTestRun> = if let Ok(content) = std::fs::read_to_string(&runs_file) {
        content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    } else {
        Vec::new()
    };
    
    Json(json!({
        "count": runs.len(),
        "runs": runs,
    }))
}

/// GET /api/loadtest/results/:id - Get specific run details
pub async fn loadtest_result_handler(
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Find run by ID
}

/// GET /api/loadtest/compare?ids=a,b,c - Compare multiple runs
pub async fn loadtest_compare_handler(
    Query(params): Query<CompareParams>,
) -> impl IntoResponse {
    // Return comparison data
}
```

### 4.4 Create Load Test Dashboard Page

**File**: `services/wms-api/src/handlers.rs`

Add HTML endpoint: `GET /loadtest`

```rust
pub async fn loadtest_dashboard_handler() -> impl IntoResponse {
    let html = include_str!("../../../web/loadtest.html");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        html,
    )
}
```

**File**: `web/loadtest.html`

```html
<!DOCTYPE html>
<html lang="en">
<head>
    <title>Load Test Dashboard - Weather WMS</title>
    <style>
        /* Modern dashboard styling */
    </style>
</head>
<body>
    <header>
        <h1>Load Test Dashboard</h1>
        <a href="/" class="back-link">← Back to Map</a>
    </header>
    
    <main>
        <!-- Summary Stats -->
        <section class="summary-cards">
            <div class="card">
                <h3>Total Runs</h3>
                <span class="stat" id="total-runs">--</span>
            </div>
            <div class="card">
                <h3>Best RPS</h3>
                <span class="stat" id="best-rps">--</span>
            </div>
            <div class="card">
                <h3>Best p99 Latency</h3>
                <span class="stat" id="best-p99">--</span>
            </div>
        </section>
        
        <!-- Run History Table -->
        <section class="runs-table">
            <h2>Test Run History</h2>
            <table id="runs-table">
                <thead>
                    <tr>
                        <th>Timestamp</th>
                        <th>Scenario</th>
                        <th>Duration</th>
                        <th>Concurrency</th>
                        <th>Requests/sec</th>
                        <th>p50 (ms)</th>
                        <th>p99 (ms)</th>
                        <th>Cache Hit %</th>
                        <th>Config</th>
                        <th>Actions</th>
                    </tr>
                </thead>
                <tbody></tbody>
            </table>
        </section>
        
        <!-- Comparison Chart -->
        <section class="comparison-chart">
            <h2>Performance Comparison</h2>
            <canvas id="comparison-chart"></canvas>
        </section>
        
        <!-- Run New Test Form -->
        <section class="new-test">
            <h2>Run New Test</h2>
            <form id="new-test-form">
                <select id="scenario-select">
                    <option value="quick">Quick Smoke Test</option>
                    <option value="cold_cache">Cold Cache</option>
                    <option value="warm_cache">Warm Cache</option>
                    <option value="stress">Stress Test</option>
                </select>
                <button type="submit">Start Test</button>
            </form>
            <div id="test-progress" style="display: none;">
                <div class="progress-bar"></div>
                <span class="progress-text">Running test...</span>
            </div>
        </section>
    </main>
    
    <script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
    <script>
        // Dashboard JavaScript
        async function loadResults() {
            const response = await fetch('/api/loadtest/results');
            const data = await response.json();
            updateDashboard(data);
        }
        
        function updateDashboard(data) {
            // Update summary cards
            document.getElementById('total-runs').textContent = data.count;
            
            if (data.runs.length > 0) {
                const bestRps = Math.max(...data.runs.map(r => r.requests_per_second));
                const bestP99 = Math.min(...data.runs.map(r => r.latency_p99));
                document.getElementById('best-rps').textContent = bestRps.toFixed(1);
                document.getElementById('best-p99').textContent = bestP99.toFixed(2) + 'ms';
            }
            
            // Populate table
            const tbody = document.querySelector('#runs-table tbody');
            tbody.innerHTML = data.runs.map(run => `
                <tr>
                    <td>${new Date(run.timestamp).toLocaleString()}</td>
                    <td>${run.scenario_name}</td>
                    <td>${run.duration_secs}s</td>
                    <td>${run.concurrency}</td>
                    <td class="${getRpsClass(run.requests_per_second)}">${run.requests_per_second.toFixed(1)}</td>
                    <td>${run.latency_p50.toFixed(2)}</td>
                    <td class="${getLatencyClass(run.latency_p99)}">${run.latency_p99.toFixed(2)}</td>
                    <td>${run.cache_hit_rate.toFixed(1)}%</td>
                    <td>
                        <span class="badge ${run.system_config.l1_cache_enabled ? 'enabled' : 'disabled'}">L1</span>
                        <span class="badge ${run.system_config.prefetch_enabled ? 'enabled' : 'disabled'}">PF</span>
                    </td>
                    <td>
                        <button onclick="showDetails('${run.id}')">Details</button>
                        <input type="checkbox" class="compare-check" data-id="${run.id}">
                    </td>
                </tr>
            `).join('');
            
            // Update chart
            updateChart(data.runs.slice(-10));  // Last 10 runs
        }
        
        function getRpsClass(rps) {
            if (rps >= 10000) return 'excellent';
            if (rps >= 5000) return 'good';
            if (rps >= 1000) return 'ok';
            return 'slow';
        }
        
        function getLatencyClass(ms) {
            if (ms <= 1) return 'excellent';
            if (ms <= 10) return 'good';
            if (ms <= 100) return 'ok';
            return 'slow';
        }
        
        // Initialize
        loadResults();
        setInterval(loadResults, 30000);  // Refresh every 30s
    </script>
</body>
</html>
```

### 4.5 Add Link to Dashboard from Main UI

Update `web/index.html`:

```html
<a href="/loadtest" target="_blank" class="external-link" title="Load Test Dashboard">
    <span class="link-icon">⚡</span>
    <span class="link-text">Load Tests</span>
</a>
```

### 4.6 Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `web/loadtest.html` | **CREATE** | Load test dashboard HTML |
| `validation/load-test/src/report.rs` | **MODIFY** | Enhanced JSON output with metadata |
| `validation/load-test/src/main.rs` | **MODIFY** | Fetch system config before test |
| `services/wms-api/src/handlers.rs` | **MODIFY** | Add load test API endpoints |
| `services/wms-api/src/main.rs` | **MODIFY** | Wire up new routes |
| `scripts/run_load_test.sh` | **MODIFY** | Auto-save JSON results |
| `web/index.html` | **MODIFY** | Add link to load test dashboard |

---

## Implementation Order

### Phase 1: Environment Configuration (Est: 2-3 hours)
1. Create `.env.example`
2. Update `.gitignore`
3. Update `docker-compose.yml` to use env_file
4. Update `scripts/start.sh` to load `.env`
5. Test docker-compose with new config

### Phase 2: Feature Flags (Est: 3-4 hours)
1. Create `OptimizationConfig` struct in `state.rs`
2. Update `AppState::new()` to parse config
3. Add conditionals in `handlers.rs` for prefetch
4. Add conditionals in cache lookups
5. Create `/api/config` endpoint
6. Test toggling features on/off

### Phase 3: Data Ingestion Controls (Est: 2 hours)
1. Update `scripts/ingest_test_data.sh` with env controls
2. Modify download scripts to support limits
3. Test with different ingestion settings
4. Update documentation

### Phase 4: Load Test Dashboard (Est: 4-5 hours)
1. Update `report.rs` with enhanced JSON output
2. Create JSONL results storage
3. Add API endpoints for results
4. Create `loadtest.html` dashboard
5. Add routes to main.rs
6. Update run_load_test.sh
7. Add link from main dashboard
8. Test end-to-end

**Total Estimated Time**: 11-14 hours

---

## Testing Strategy

### Environment Configuration Tests
```bash
# Test with defaults (no .env)
rm .env && ./scripts/start.sh

# Test with custom config
cp .env.example .env
echo "ENABLE_L1_CACHE=false" >> .env
./scripts/start.sh --rebuild
```

### Feature Flag Tests
```bash
# Test with L1 cache disabled
ENABLE_L1_CACHE=false docker-compose up -d wms-api
./scripts/run_load_test.sh warm_cache --save

# Compare with L1 cache enabled
ENABLE_L1_CACHE=true docker-compose up -d wms-api
./scripts/run_load_test.sh warm_cache --save
```

### Data Ingestion Tests
```bash
# Test minimal ingestion
INGEST_HRRR=false INGEST_GOES=false ./scripts/start.sh

# Test limited HRRR files
HRRR_MAX_FILES=3 ./scripts/start.sh
```

### Load Test Dashboard Tests
```bash
# Run several tests
./scripts/run_load_test.sh quick --save
./scripts/run_load_test.sh warm_cache --save
./scripts/run_load_test.sh cold_cache --save

# Open dashboard
open http://localhost:8080/loadtest
```

---

## Success Criteria

1. **Environment Config**: All services start correctly from `.env` file
2. **Feature Flags**: Each optimization can be toggled independently
3. **Data Ingestion**: HRRR and GOES data can be controlled via env vars
4. **Load Test Dashboard**: 
   - Shows history of all test runs
   - Displays key metrics (RPS, latency percentiles, cache rates)
   - Shows which optimizations were enabled for each run
   - Allows comparison between runs

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Breaking existing deployments | Keep all defaults identical to current behavior |
| Complex env parsing | Use single config struct with clear defaults |
| Large JSONL files over time | Add optional cleanup/rotation in script |
| Dashboard performance | Limit to last 100 runs in UI, paginate if needed |

---

## Future Enhancements

1. **Config Hot-Reload**: Allow changing optimization flags without restart
2. **A/B Testing**: Auto-run comparison tests with different configs
3. **Alerting**: Notify when performance degrades below threshold
4. **Export**: Download results as CSV for external analysis
