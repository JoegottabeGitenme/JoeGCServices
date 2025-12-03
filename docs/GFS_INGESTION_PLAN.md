# GFS Data Ingestion Plan

## Executive Summary

This document outlines the plan to implement automated ingestion of NOAA GFS (Global Forecast System) weather data into the Weather WMS system. The ingestion service will poll NOAA's AWS Open Data registry, download GRIB2 files, parse weather parameters, store the data in MinIO object storage, and catalog metadata in PostgreSQL for WMS queries.

**Timeline**: 2-3 weeks for full implementation  
**Complexity**: Medium (leverages existing infrastructure)  
**Status**: Planning Phase

---

## 1. Data Source Details

### NOAA AWS Open Data (S3)
- **Bucket**: `noaa-gfs-bdp-pds` (public, no authentication)
- **Region**: `us-east-1`
- **Access**: HTTPS (anonymous)
- **Update Frequency**: 4 times daily (00, 06, 12, 18 UTC)
- **Latency**: Data available 3-4 hours after cycle time
- **Documentation**: https://registry.opendata.aws/noaa-gfs-bdp-pds/

### GFS Model Specifications
- **Resolution**: 0.25° (~25km at equator)
- **Coverage**: Global (0°E to 360°E, 90°S to 90°N)
- **Forecast Hours**: 0-120 hours in 3-hour increments (41 time steps)
- **File Format**: GRIB2 (GRIB Edition 2)
- **File Naming**: `gfs.YYYYMMDD/HH/atmos/gfs.tHHz.pgrb2.0p25.fFFF`
  - Example: `gfs.20251125/06/atmos/gfs.t06z.pgrb2.0p25.f003`

### File Structure
```
noaa-gfs-bdp-pds/
└── gfs.20251125/          # Run date
    └── 06/                # Cycle hour (00, 06, 12, 18)
        └── atmos/         # Atmospheric data
            ├── gfs.t06z.pgrb2.0p25.f000  # Analysis (0-hour forecast)
            ├── gfs.t06z.pgrb2.0p25.f003  # +3 hour forecast
            ├── gfs.t06z.pgrb2.0p25.f006  # +6 hour forecast
            └── ...
            └── gfs.t06z.pgrb2.0p25.f120  # +120 hour forecast
```

### File Sizes
- **Per forecast hour**: ~80-120 MB (compressed GRIB2)
- **Per cycle**: 41 files × 100 MB avg = ~4.1 GB
- **Per day**: 4 cycles × 4.1 GB = ~16.4 GB
- **Storage requirement**: ~115 GB/week (with 7-day retention)

### Initial Parameters to Ingest
Based on `services/ingester/src/config.rs` configuration:

1. **Temperature at 2m** (`TMP:2 m above ground`)
   - Units: Kelvin
   - Use case: Surface weather maps
   
2. **Wind U-component at 10m** (`UGRD:10 m above ground`)
   - Units: m/s
   - Use case: Wind barb visualization (with V-component)

3. **Wind V-component at 10m** (`VGRD:10 m above ground`)
   - Units: m/s
   - Use case: Wind barb visualization (with U-component)

4. **Mean Sea Level Pressure** (`PRMSL:mean sea level`)
   - Units: Pascals
   - Use case: Isobar maps, weather analysis

---

## 2. Existing Infrastructure

### Already Implemented ✅
The following components already exist in the codebase:

#### Configuration (`services/ingester/src/config.rs`)
- `IngesterConfig` with environment variable loading
- `ModelConfig` with GFS default configuration
- `DataSource::NoaaAws` for S3 bucket access
- `ParameterConfig` with GRIB filter criteria
- File pattern generation for GFS files

#### Data Sources (`services/ingester/src/sources.rs`)
- `AwsDataSource` HTTP client for S3 access
- `list_files()` to enumerate S3 objects
- `fetch_file()` to download GRIB2 files
- `file_exists()` to check availability
- `latest_available_cycle()` to determine current run
- `cycles_to_check()` for lookback logic

#### Ingestion Pipeline (`services/ingester/src/ingest.rs`)
- `IngestionPipeline` orchestration
- `run_forever()` continuous polling loop
- `ingest_model()` model-specific ingestion
- `ingest_cycle()` processes a single model run
- `process_file()` downloads and stores files
- Parallel download coordination (semaphore)
- Catalog registration
- Old data cleanup

#### Catalog (`crates/storage/src/catalog.rs`)
- PostgreSQL connection pool
- `register_dataset()` for metadata storage
- `find_datasets()` query interface
- `get_latest()` for most recent data
- `find_by_time()` for temporal queries
- `get_available_times()` for WMS dimensions
- Database schema with indexes

#### Object Storage (`crates/storage/src/object_store.rs`)
- MinIO/S3 client wrapper
- `put()` to store files
- `get()` to retrieve files
- `exists()` to check presence
- Path organization

### Missing Components ❌
These need to be implemented:

1. **GRIB2 Parser** (`crates/grib2-parser/`)
   - Currently just placeholder stubs
   - Need to decode GRIB2 message structure
   - Extract parameter values from grid
   - Handle compression (JPEG2000, simple packing)

2. **Parameter Extraction**
   - Filter GRIB2 messages by discipline/parameter/level
   - Extract grid definition section
   - Decode bitmap and data sections
   - Convert to internal grid format

3. **Data Validation**
   - Verify grid dimensions match expected
   - Check for missing values
   - Validate coordinate systems
   - Detect corrupted downloads

4. **Retry Logic**
   - Handle transient S3 errors (429, 503)
   - Retry failed downloads with exponential backoff
   - Mark datasets as "failed" vs "available"

5. **Monitoring & Metrics**
   - Download success/failure rates
   - Parse error tracking
   - Ingestion latency metrics
   - Storage usage monitoring

---

## 3. Implementation Plan

### Phase 1: GRIB2 Parser Core (Week 1)
**Goal**: Decode GRIB2 file structure and extract metadata

#### Tasks:
1. **Section Parsing** (`crates/grib2-parser/src/sections/`)
   - [ ] Section 0 (Indicator): Magic bytes, discipline, message length
   - [ ] Section 1 (Identification): Reference time, model info
   - [ ] Section 3 (Grid Definition): Lat/lon grid, dimensions
   - [ ] Section 4 (Product Definition): Parameter, level, forecast hour
   - [ ] Section 5 (Data Representation): Packing method, bit depth
   - [ ] Section 6 (Bitmap): Missing value indicators
   - [ ] Section 7 (Data): Compressed grid values
   - [ ] Section 8 (End): Terminator ("7777")

2. **GRIB2 Reader** (`crates/grib2-parser/src/lib.rs`)
   - [ ] `Grib2Reader::new(bytes)` constructor
   - [ ] `iter_messages()` iterator over messages in file
   - [ ] `Grib2Message` struct with all sections
   - [ ] Section length/number validation
   - [ ] CRC32 checksum verification (optional)

3. **Template Support** (`crates/grib2-parser/src/templates/`)
   - [ ] Template 0 (regular lat/lon grid)
   - [ ] Template 40 (Gaussian grid) - if needed
   - [ ] Product definition template 0 (analysis/forecast)
   - [ ] Product definition template 8 (time intervals)

4. **Testing**
   - [ ] Unit tests with sample GRIB2 messages
   - [ ] Download real GFS file for integration test
   - [ ] Verify parsing against `wgrib2` reference tool

**Deliverable**: Parse GRIB2 metadata (params, levels, grid) without decoding values

---

### Phase 2: Data Unpacking (Week 2)
**Goal**: Extract and decode grid data values

#### Tasks:
1. **Simple Packing** (`crates/grib2-parser/src/unpacking/`)
   - [ ] Decode binary scaling/decimal scaling
   - [ ] Apply reference value and scale factor
   - [ ] Handle bitmap (masked values)
   - [ ] Output `Vec<Option<f32>>` (None = missing)

2. **JPEG2000 Decompression** (if needed)
   - [ ] Integrate `jpeg2000` or `openjp2` crate
   - [ ] Handle JPEG2000 codestream in Section 7
   - [ ] Fallback to simple packing if unsupported

3. **Grid Processing**
   - [ ] Convert to internal `Grid` type (wms-common)
   - [ ] Store lat/lon coordinate arrays
   - [ ] Validate grid dimensions (1440×721 for 0.25° global)
   - [ ] Handle sub-grids if needed

4. **Parameter Filtering**
   - [ ] Match `ParameterConfig.grib_filter` criteria
   - [ ] Filter by discipline code (0 = meteorological)
   - [ ] Filter by parameter category/number
   - [ ] Filter by level type and value
   - [ ] Skip unwanted parameters to save processing

5. **Testing**
   - [ ] Unit tests for unpacking algorithms
   - [ ] Validate against known reference values
   - [ ] Performance benchmarks (target: <1s per parameter)

**Deliverable**: Extract temperature_2m values from real GFS file

---

### Phase 3: Integration & Storage (Week 2-3)
**Goal**: Store parsed data and update catalog

#### Tasks:
1. **Update `extract_parameter()`** (`services/ingester/src/ingest.rs:247`)
   - [ ] Parse GRIB2 with `Grib2Reader::new(data)`
   - [ ] Filter messages matching `param_config.grib_filter`
   - [ ] Extract grid data
   - [ ] Store as NetCDF or custom format (TBD)
   - [ ] Fall back to raw GRIB2 if parsing fails

2. **Storage Strategy Decision**
   - **Option A**: Store raw GRIB2, parse on-demand
     - Pro: Simple, preserves original data
     - Con: Parse overhead on every WMS request
   - **Option B**: Store parsed NetCDF/COG
     - Pro: Fast WMS rendering
     - Con: Storage space increase, conversion overhead
   - **Recommendation**: Start with Option A, migrate to B later

3. **Catalog Enhancement**
   - [ ] Store grid metadata (nx, ny, dx, dy)
   - [ ] Store min/max data values for visualization
   - [ ] Index by parameter + level (improve queries)
   - [ ] Add `parsed_at` timestamp field

4. **Error Handling**
   - [ ] Add retry logic for S3 errors
   - [ ] Mark datasets as "parse_failed" status
   - [ ] Log parse errors with diagnostic info
   - [ ] Alert on consecutive failures

5. **Testing**
   - [ ] End-to-end test: download → parse → store → catalog
   - [ ] Verify PostgreSQL entries correct
   - [ ] Verify MinIO files accessible
   - [ ] Test catalog queries work

**Deliverable**: Ingester successfully processes real GFS data

---

### Phase 4: Production Readiness (Week 3)
**Goal**: Robust, monitored, production-ready ingestion

#### Tasks:
1. **Operational Configuration**
   - [ ] Document environment variables
   - [ ] Add health check endpoint
   - [ ] Configure log levels (JSON structured logs)
   - [ ] Set poll intervals (3600s = 1 hour)
   - [ ] Tune parallel downloads (4-8 concurrent)

2. **Monitoring & Observability**
   - [ ] Add Prometheus metrics:
     - `ingester_downloads_total{status}` (success/failure)
     - `ingester_parse_duration_seconds`
     - `ingester_file_size_bytes`
     - `ingester_last_successful_run`
   - [ ] Add tracing spans for each ingestion stage
   - [ ] Dashboard for ingestion health

3. **Deployment**
   - [ ] Update Helm chart (`deploy/helm/weather-wms/`)
   - [ ] Add resource limits (CPU, memory)
   - [ ] Configure persistent volumes (if caching)
   - [ ] Add liveness/readiness probes
   - [ ] Deploy to dev environment

4. **Data Retention**
   - [ ] Implement `cleanup_old_data()` file deletion
   - [ ] Test retention policy (7 days default)
   - [ ] Document storage requirements

5. **Documentation**
   - [ ] Update DEVELOPMENT.md with ingester setup
   - [ ] Add ingestion monitoring to MONITORING.md
   - [ ] Create INGESTION.md with troubleshooting guide
   - [ ] Document GRIB2 parser usage

**Deliverable**: Production-ready ingester service

---

## 4. Workflow Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                     INGESTION PIPELINE                           │
└─────────────────────────────────────────────────────────────────┘

   ┌──────────────┐
   │   Scheduler  │ (Runs every 1 hour)
   └──────┬───────┘
          │
          v
   ┌──────────────────┐
   │ Determine Cycles │ (Check last 24h: 00z, 06z, 12z, 18z)
   └──────┬───────────┘
          │
          v
   ┌──────────────────┐
   │   List Files     │ (S3 ListObjectsV2 API)
   │ noaa-gfs-bdp-pds │ → Get file list for cycle
   └──────┬───────────┘
          │
          v
   ┌──────────────────┐
   │ Check Catalog    │ (Query PostgreSQL)
   └──────┬───────────┘  → Skip already ingested files
          │
          v
   ┌──────────────────┐
   │  Download GRIB2  │ (4 parallel downloads)
   │   via HTTPS      │ → GET https://noaa-gfs-bdp-pds.s3...
   └──────┬───────────┘
          │
          ├─────────────────┬──────────────────┬─────────────────┐
          v                 v                  v                 v
   ┌───────────┐      ┌───────────┐     ┌───────────┐    ┌───────────┐
   │ f000.grib2│      │ f003.grib2│     │ f006.grib2│    │   ...     │
   └─────┬─────┘      └─────┬─────┘     └─────┬─────┘    └─────┬─────┘
         │                  │                  │                │
         └──────────────────┴──────────────────┴────────────────┘
                            │
                            v
                   ┌─────────────────┐
                   │  Parse GRIB2    │
                   │ Extract Params  │ (Temperature, Wind, Pressure)
                   └────────┬────────┘
                            │
              ┌─────────────┼─────────────┐
              v             v             v
      ┌──────────┐   ┌──────────┐  ┌──────────┐
      │ TMP:2m   │   │ UGRD:10m │  │PRMSL:MSL │
      └────┬─────┘   └────┬─────┘  └────┬─────┘
           │              │              │
           └──────────────┴──────────────┘
                          │
                          v
                 ┌─────────────────┐
                 │  Store in MinIO │
                 │   /raw/gfs/...  │
                 └────────┬────────┘
                          │
                          v
              ┌────────────────────────┐
              │ Register in Catalog DB │
              │    (PostgreSQL)        │
              └────────────────────────┘
                          │
                ┌─────────┴──────────┐
                v                    v
         ┌─────────────┐      ┌─────────────┐
         │   datasets  │      │layer_styles │
         │   table     │      │   table     │
         └─────────────┘      └─────────────┘
                │
                v
      ┌──────────────────┐
      │ WMS API Ready to │
      │  Serve Requests  │
      └──────────────────┘
```

---

## 5. Storage Structure

### MinIO Object Paths
```
weather-data/
└── raw/
    └── gfs/
        └── 20251125/           # Reference date
            └── 06/             # Cycle hour
                ├── gfs.t06z.pgrb2.0p25.f000
                ├── gfs.t06z.pgrb2.0p25.f003
                ├── gfs.t06z.pgrb2.0p25.f006
                └── ...
```

### PostgreSQL Catalog Schema
```sql
-- Example entries after ingesting one GFS cycle

-- datasets table
| id (UUID) | model | parameter      | level             | reference_time      | forecast_hour | valid_time          | bbox            | storage_path                     | file_size |
|-----------|-------|----------------|-------------------|---------------------|---------------|---------------------|-----------------|----------------------------------|-----------|
| uuid-001  | gfs   | temperature_2m | 2 m above ground  | 2025-11-25 06:00:00 | 0             | 2025-11-25 06:00:00 | global          | raw/gfs/20251125/06/f000.grib2   | 98234567  |
| uuid-002  | gfs   | temperature_2m | 2 m above ground  | 2025-11-25 06:00:00 | 3             | 2025-11-25 09:00:00 | global          | raw/gfs/20251125/06/f003.grib2   | 98123456  |
| uuid-003  | gfs   | wind_u_10m     | 10 m above ground | 2025-11-25 06:00:00 | 0             | 2025-11-25 06:00:00 | global          | raw/gfs/20251125/06/f000.grib2   | 98234567  |
...
```

Queries used by WMS API:
```sql
-- Get latest temperature data
SELECT * FROM datasets 
WHERE model = 'gfs' AND parameter = 'temperature_2m' 
ORDER BY valid_time DESC LIMIT 1;

-- Get data for specific time
SELECT * FROM datasets 
WHERE model = 'gfs' AND parameter = 'temperature_2m' 
ORDER BY ABS(EXTRACT(EPOCH FROM (valid_time - '2025-11-25 12:00:00'))) ASC 
LIMIT 1;

-- Get available time steps (for WMS TIME dimension)
SELECT DISTINCT valid_time FROM datasets 
WHERE model = 'gfs' AND parameter = 'temperature_2m' 
ORDER BY valid_time DESC;
```

---

## 6. Testing Strategy

### Unit Tests
- GRIB2 section parsing (each section type)
- Data unpacking algorithms (simple packing, JPEG2000)
- Parameter filtering logic
- Grid coordinate calculations

### Integration Tests
1. **Download Real File**
   ```rust
   #[tokio::test]
   async fn test_download_gfs_file() {
       let fetcher = AwsDataSource::new(...);
       let files = fetcher.list_files("20251125", 6).await.unwrap();
       let data = fetcher.fetch_file(&files[0]).await.unwrap();
       assert!(data.len() > 1_000_000); // At least 1 MB
   }
   ```

2. **Parse Real GRIB2**
   ```rust
   #[test]
   fn test_parse_gfs_grib2() {
       let data = std::fs::read("testdata/gfs.t00z.pgrb2.0p25.f000").unwrap();
       let reader = Grib2Reader::new(data.into());
       let messages: Vec<_> = reader.iter_messages().collect();
       assert!(messages.len() > 100); // GFS has 300+ messages
   }
   ```

3. **End-to-End Ingestion**
   ```rust
   #[tokio::test]
   async fn test_ingest_gfs_cycle() {
       let config = IngesterConfig::from_env().unwrap();
       let pipeline = IngestionPipeline::new(&config).await.unwrap();
       
       // Ingest one forecast hour
       pipeline.ingest_model("gfs").await.unwrap();
       
       // Verify catalog entry
       let entry = pipeline.catalog.get_latest("gfs", "temperature_2m").await.unwrap();
       assert!(entry.is_some());
   }
   ```

### Validation Tests
- Compare parsed values against `wgrib2 -d 1 -text output.txt`
- Verify grid dimensions (1440×721 for 0.25°)
- Check coordinate accuracy (lat/lon within 0.01°)
- Validate min/max ranges (temp in Kelvin, pressure in Pa)

### Performance Tests
- Parse time per GRIB2 file (target: <5s)
- Download throughput (target: >10 MB/s)
- Catalog insertion rate (target: >100/s)
- Storage I/O bandwidth

---

## 7. Dependencies & Libraries

### Rust Crates to Add

#### GRIB2 Parsing
- **Option A**: Implement from scratch (recommended for control)
  - `bytes` - Already in workspace
  - `nom` - Parser combinator for binary formats
  
- **Option B**: Use existing crate
  - `grib` (v0.8) - Pure Rust, incomplete
  - Issue: Limited template support, unmaintained

#### JPEG2000 Decompression (if needed)
- `openjp2-sys` - Bindings to OpenJPEG C library
- `jpeg2000` - Pure Rust (limited support)
- Alternative: Shell out to `wgrib2` for complex packing

#### HTTP Client
- `reqwest` - Already in workspace ✅
- `bytes` - Already in workspace ✅

#### Async Runtime
- `tokio` - Already in workspace ✅
- `futures` - Already in workspace ✅

### External Tools (for validation)
- **wgrib2**: NOAA's reference GRIB2 tool
  ```bash
  # Ubuntu/Debian
  apt-get install wgrib2
  
  # Verify parsing
  wgrib2 gfs.t00z.pgrb2.0p25.f000 | grep "TMP:2 m above ground"
  wgrib2 -d 1 -text output.txt gfs.t00z.pgrb2.0p25.f000
  ```

---

## 8. Risks & Mitigations

### Technical Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| GRIB2 parsing complexity | High | Medium | Start with simple packing only; fallback to wgrib2 for complex files |
| JPEG2000 decoding issues | Medium | Medium | Use well-tested C library (OpenJPEG) with FFI |
| Large file downloads timeout | Medium | Low | Increase HTTP timeout to 10 minutes; add retry logic |
| S3 rate limiting | Medium | Low | Add exponential backoff; space requests by 1s |
| Disk space exhaustion | High | Medium | Implement retention cleanup; monitor storage metrics |
| Parse errors on corrupt files | Low | Medium | Add CRC validation; mark failed files; alert on errors |

### Operational Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| NOAA data delays | Medium | Medium | Poll last 24h of cycles; don't rely on latest only |
| Service downtime during data gap | Medium | Low | WMS API serves last available data; show warning |
| High bandwidth costs | Low | Low | Use NOAA Open Data (free); monitor egress |
| Database growth | Medium | Medium | Implement retention policy; partition by time |

---

## 9. Success Criteria

### Phase 1 Complete
- [ ] Parse GRIB2 Section 0-8 from real GFS file
- [ ] Extract parameter metadata (name, level, units)
- [ ] Extract grid definition (lat/lon bounds, dimensions)
- [ ] Unit tests pass with 90%+ coverage

### Phase 2 Complete
- [ ] Unpack simple packed data
- [ ] Extract temperature_2m values
- [ ] Values match wgrib2 output (within 0.01%)
- [ ] Performance: <3s to parse one parameter

### Phase 3 Complete
- [ ] Ingester downloads real GFS cycle
- [ ] Files stored in MinIO at correct paths
- [ ] PostgreSQL catalog entries created
- [ ] WMS API can query and retrieve data

### Phase 4 Complete
- [ ] Ingester runs continuously without crashes
- [ ] Prometheus metrics exported
- [ ] Kubernetes deployment stable
- [ ] Documentation complete

### Production Ready
- [ ] 4 cycles per day ingested successfully
- [ ] <5% parse failure rate
- [ ] <30 minute lag behind NOAA publication
- [ ] Zero data loss over 7-day retention window
- [ ] Monitoring dashboard operational

---

## 10. Timeline & Milestones

```
Week 1
├─ Day 1-2: GRIB2 Section parsing (0-8)
├─ Day 3-4: Message iteration, template support
└─ Day 5:   Unit tests, validate with wgrib2

Week 2
├─ Day 6-7:  Data unpacking (simple packing)
├─ Day 8-9:  Parameter filtering, grid extraction
└─ Day 10:   Integration tests with real GFS file

Week 3
├─ Day 11-12: Update ingester extract_parameter()
├─ Day 13:    Catalog storage, error handling
├─ Day 14:    End-to-end testing
└─ Day 15:    Production deployment, monitoring

Week 4+ (Future)
└─ Rendering implementation (separate task)
```

---

## 11. Future Enhancements

### Phase 5: Advanced Features (Post-MVP)
- [ ] JPEG2000 unpacking support
- [ ] Additional parameters (humidity, precipitation, cloud cover)
- [ ] HRRR model ingestion (high-resolution US)
- [ ] NAM model ingestion (North America)
- [ ] Vertical level support (500mb, 850mb heights)
- [ ] Ensemble model support (GEFS)

### Optimization Opportunities
- [ ] Incremental downloads (byte-range requests for specific parameters)
- [ ] Pre-rendered tile cache (COG/Zarr format)
- [ ] Parallel parsing with Rayon
- [ ] Compression for stored grids (Zarr, HDF5)
- [ ] Database partitioning by time
- [ ] CDN for tile distribution

### Monitoring Enhancements
- [ ] Alerting on ingestion failures (PagerDuty, Slack)
- [ ] Data quality metrics (min/max bounds checking)
- [ ] Historical trends dashboard (Grafana)
- [ ] Cost tracking (bandwidth, storage)

---

## 12. Next Steps

### Immediate Actions (This Week)
1. **Create GRIB2 parser skeleton**
   ```bash
   cd crates/grib2-parser
   cargo add nom bytes
   # Implement Section 0 parsing in src/sections/indicator.rs
   ```

2. **Download sample GFS file for testing**
   ```bash
   mkdir -p testdata
   curl -o testdata/gfs.t00z.pgrb2.0p25.f000 \
     https://noaa-gfs-bdp-pds.s3.amazonaws.com/gfs.20251125/00/atmos/gfs.t00z.pgrb2.0p25.f000
   ```

3. **Install wgrib2 for validation**
   ```bash
   # Use for comparing parsed output
   wgrib2 testdata/gfs.t00z.pgrb2.0p25.f000 | head -20
   ```

4. **Create GitHub issues for tracking**
   - Issue #1: Implement GRIB2 Section parsing
   - Issue #2: Implement data unpacking
   - Issue #3: Integrate parser into ingester
   - Issue #4: Production deployment

### Questions to Resolve
- [ ] Storage format: Raw GRIB2 vs. parsed NetCDF/COG?
- [ ] JPEG2000: Implement or shell out to wgrib2?
- [ ] Deployment: Single ingester instance or distributed?
- [ ] Monitoring: Self-hosted Prometheus or cloud service?

---

## 13. References

### Documentation
- [WMO GRIB2 Standard (FM 92)](https://www.wmo.int/pages/prog/www/WMOCodes/Guides/GRIB/GRIB2_062006.pdf)
- [NOAA GFS Documentation](https://www.emc.ncep.noaa.gov/emc/pages/numerical_forecast_systems/gfs.php)
- [AWS Open Data Registry](https://registry.opendata.aws/noaa-gfs-bdp-pds/)
- [wgrib2 User Guide](https://www.cpc.ncep.noaa.gov/products/wesley/wgrib2/)

### Code Examples
- [NCEP GRIB2 Tables](https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/)
- [Python pygrib library](https://github.com/jswhit/pygrib) (reference implementation)
- [Rust grib crate](https://github.com/noritada/grib-rs) (incomplete but useful)

### Related Work in Codebase
- `services/ingester/src/config.rs` - GFS configuration
- `services/ingester/src/ingest.rs` - Pipeline scaffolding
- `crates/storage/src/catalog.rs` - Database schema
- `crates/wms-common/src/grid.rs` - Grid data structures

---

## Appendix A: Example GRIB2 Message Structure

```
┌─────────────────────────────────────┐
│ Section 0: Indicator (16 bytes)     │  "GRIB" magic, discipline, length
├─────────────────────────────────────┤
│ Section 1: Identification (21 bytes)│  Center, subcenter, ref time
├─────────────────────────────────────┤
│ Section 2: Local Use (optional)     │  Custom data (rarely used)
├─────────────────────────────────────┤
│ Section 3: Grid Definition (72 bytes│  Lat/lon grid: nx=1440, ny=721
│            for template 0)           │  dx=0.25°, dy=0.25°, bounds
├─────────────────────────────────────┤
│ Section 4: Product Definition        │  Parameter: TMP (temperature)
│            (34 bytes for template 0) │  Level: 103 (2m above ground)
│                                      │  Forecast hour: 3
├─────────────────────────────────────┤
│ Section 5: Data Representation       │  Packing: Simple (template 0)
│            (21 bytes for template 0) │  Reference value, scale factor
│                                      │  Bit width: 12 bits
├─────────────────────────────────────┤
│ Section 6: Bitmap (optional)         │  0xFF = value present
│                                      │  0x00 = missing value
├─────────────────────────────────────┤
│ Section 7: Data (~1.2 MB)            │  Binary packed grid values
│         1440 × 721 = 1,037,440 points│  Compressed to ~1-2 MB
├─────────────────────────────────────┤
│ Section 8: End (4 bytes)             │  "7777" terminator
└─────────────────────────────────────┘

One GFS file contains 300+ such messages (different parameters/levels)
```

---

## Appendix B: Sample Environment Configuration

```bash
# .env for ingester service

# Database
DATABASE_URL=postgresql://postgres:postgres@postgres:5432/weatherwms

# Redis
REDIS_URL=redis://redis:6379

# Object Storage
S3_ENDPOINT=http://minio:9000
S3_BUCKET=weather-data
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_REGION=us-east-1
S3_ALLOW_HTTP=true

# Ingester Configuration
POLL_INTERVAL_SECS=3600           # Poll every 1 hour
PARALLEL_DOWNLOADS=4              # Download 4 files concurrently
RETENTION_HOURS=168               # Keep 7 days (168 hours)
LOG_LEVEL=info                    # trace, debug, info, warn, error

# Model-specific (optional, overrides defaults in config.rs)
GFS_ENABLED=true
GFS_CYCLES=0,6,12,18              # UTC hours
GFS_FORECAST_HOURS=0-120          # Range with step
GFS_FORECAST_STEP=3               # 3-hour increments
```

---

**Document Version**: 1.0  
**Last Updated**: 2025-11-25  
**Author**: OpenCode Assistant  
**Status**: Planning Phase
