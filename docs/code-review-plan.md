# Code Review Plan - Weather WMS

## Overview

**Goal:** Systematically review all Rust code in the weather-wms codebase to ensure high quality, identify improvements, and build comprehensive understanding of the system.

**Focus Areas:**
- Code quality & idiomatic Rust patterns
- Documentation completeness
- Performance optimization opportunities
- Error handling consistency
- Security considerations

**Total Scope:** ~101,780 lines of Rust across 11 library crates + 4 services

---

## Architecture Overview

```
+---------------------------------------------------------------------+
|                         Protocol Layers                             |
|  (edr-protocol, wms-protocol)                                       |
+---------------------------------------------------------------------+
                                  |
+---------------------------------------------------------------------+
|                         Service Layer                               |
|  (grid-processor: GridDataService, DatasetQuery)                    |
+---------------------------------------------------------------------+
                                  |
+---------------------------------------------------------------------+
|                        Processing Layer                             |
|  (renderer, projection, ingestion)                                  |
+---------------------------------------------------------------------+
                                  |
+---------------------------------------------------------------------+
|                       Data Format Layer                             |
|  (grib2-parser, netcdf-parser)                                      |
+---------------------------------------------------------------------+
                                  |
+---------------------------------------------------------------------+
|                         Storage Layer                               |
|  (storage: MinIO/S3, PostgreSQL, Redis)                             |
+---------------------------------------------------------------------+
                                  |
+---------------------------------------------------------------------+
|                       Common Foundation                             |
|  (wms-common, test-utils)                                           |
+---------------------------------------------------------------------+
```

---

## Review Methodology

For each module/file:
1. **Read & Understand** - Comprehend the purpose and implementation
2. **Document** - Ensure module-level docs exist and are accurate
3. **Quality Check** - Verify idiomatic patterns, proper error handling
4. **Testing** - Verify that testing for the code exists somewhere? Can we validate the behavior somehow?
5. **Performance** - Identify hot paths, allocation patterns
6. **Security** - Check input validation, SQL safety
7. **Action Items** - Note improvements to make, clearly mark TODOs with context and possible fix actions

---

## Phase 1: Foundation Crates

### 1.1 wms-common (~2,646 lines)
*Shared types used across all services*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~50 | [ ] | |
| `bbox.rs` | ~200 | [ ] | |
| `crs.rs` | ~300 | [ ] | |
| `error.rs` | ~150 | [ ] | |
| `grid.rs` | ~250 | [ ] | |
| `layer.rs` | ~400 | [ ] | |
| `style.rs` | ~500 | [ ] | |
| `tile.rs` | ~400 | [ ] | |
| `time.rs` | ~300 | [ ] | |

**Review Questions:**
- [ ] Are all public types well-documented?
- [ ] Is `Clone` vs `Copy` used appropriately?
- [ ] Are error types comprehensive?

---

### 1.2 projection (~2,425 lines)
*CRS transformations*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~100 | [ ] | |
| `geographic.rs` | ~300 | [ ] | |
| `geostationary.rs` | ~500 | [ ] | |
| `lambert.rs` | ~400 | [ ] | |
| `mercator.rs` | ~350 | [ ] | |
| `polar.rs` | ~400 | [ ] | |
| `transform.rs` | ~250 | [ ] | |

**Review Questions:**
- [ ] Are projection formulas mathematically correct?
- [ ] Is there proper handling of edge cases (poles, antimeridian)?
- [ ] Performance of coordinate transforms in hot paths?

---

### 1.3 test-utils (~1,233 lines)
*Test infrastructure*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~100 | [ ] | |
| `fixtures.rs` | ~300 | [ ] | |
| `generators.rs` | ~500 | [ ] | |
| `paths.rs` | ~200 | [ ] | |

**Review Questions:**
- [ ] Are test utilities comprehensive?
- [ ] Do generators produce realistic test data?

---

## Phase 2: Data Format Crates

### 2.1 grib2-parser (~1,900 lines)
*GRIB2 weather data parsing*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~300 | [ ] | |
| `ndfd.rs` | ~200 | [ ] | |
| `sections/mod.rs` | ~800 | [ ] | |
| `tables.rs` | ~400 | [ ] | |
| `unpacking/mod.rs` | ~200 | [ ] | |

**Review Questions:**
- [ ] Is parsing robust against malformed files?
- [ ] Are all GRIB2 sections handled correctly?
- [ ] Memory efficiency when parsing large files?

---

### 2.2 netcdf-parser (~580 lines)
*GOES satellite data parsing*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~100 | [ ] | |
| `error.rs` | ~50 | [ ] | |
| `native.rs` | ~300 | [ ] | |
| `projection.rs` | ~130 | [ ] | |

**Review Questions:**
- [ ] Is GOES projection handling correct?
- [ ] Are fill values handled properly?

---

## Phase 3: Storage & Processing Crates

### 3.1 storage (~5,057 lines)
*Database and cache abstractions*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~50 | [ ] | |
| `cache.rs` | ~800 | [ ] | |
| `catalog.rs` | ~1,500 | [ ] | |
| `object_store.rs` | ~1,000 | [ ] | |
| `observations.rs` | ~1,000 | [ ] | |
| `stations_bootstrap.rs` | ~300 | [ ] | |
| `tile_memory_cache.rs` | ~400 | [ ] | |

**Review Questions:**
- [ ] SQL injection prevention in catalog.rs?
- [ ] Connection pool management?
- [ ] Cache eviction policies appropriate?
- [ ] Error handling for network failures?

---

### 3.2 ingestion (~4,697 lines)
*GRIB2/NetCDF to Zarr pipeline*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~100 | [ ] | |
| `error.rs` | ~150 | [ ] | |
| `geotiff.rs` | ~500 | [ ] | |
| `grib2.rs` | ~1,200 | [ ] | |
| `ingester.rs` | ~800 | [ ] | |
| `metadata.rs` | ~600 | [ ] | |
| `netcdf.rs` | ~800 | [ ] | |
| `tables.rs` | ~300 | [ ] | |
| `upload.rs` | ~250 | [ ] | |

**Review Questions:**
- [ ] Memory management for large files?
- [ ] Proper cleanup on failure?
- [ ] Zarr format correctness?

---

### 3.3 grid-processor (~6,204 lines)
*Zarr data access layer*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~200 | [ ] | |
| `service.rs` | ~800 | [ ] | |
| `factory.rs` | ~300 | [ ] | |
| `processor/mod.rs` | ~200 | [ ] | |
| `processor/zarr.rs` | ~1,000 | [ ] | |
| `projection/reproject.rs` | ~600 | [ ] | |
| `projection/interpolation.rs` | ~500 | [ ] | |
| `cache/chunk_cache.rs` | ~400 | [ ] | |
| `writer/zarr_writer.rs` | ~500 | [ ] | |
| `config.rs` | ~200 | [ ] | |
| `downsample.rs` | ~400 | [ ] | |
| `error.rs` | ~100 | [ ] | |
| `minio_storage.rs` | ~300 | [ ] | |
| `query.rs` | ~400 | [ ] | |
| `types.rs` | ~300 | [ ] | |

**Review Questions:**
- [ ] Chunk cache memory bounds?
- [ ] Interpolation accuracy?
- [ ] Thread safety?

---

## Phase 4: Protocol Crates

### 4.1 wms-protocol (~997 lines)
*OGC WMS/WMTS implementation*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~100 | [ ] | |
| `exceptions.rs` | ~200 | [ ] | |
| `getfeatureinfo.rs` | ~200 | [ ] | |
| `getmap.rs` | ~300 | [ ] | |
| `wmts.rs` | ~200 | [ ] | |

**Review Questions:**
- [ ] OGC WMS 1.1.1/1.3.0 spec compliance?
- [ ] WMTS 1.0.0 spec compliance?

---

### 4.2 edr-protocol (~8,565 lines)
*OGC API - EDR implementation*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~300 | [ ] | |
| `collections.rs` | ~800 | [ ] | |
| `coverage_json.rs` | ~1,500 | [ ] | |
| `crs.rs` | ~400 | [ ] | |
| `errors.rs` | ~200 | [ ] | |
| `geojson.rs` | ~600 | [ ] | |
| `locations.rs` | ~400 | [ ] | |
| `parameters.rs` | ~800 | [ ] | |
| `queries.rs` | ~1,500 | [ ] | |
| `responses.rs` | ~500 | [ ] | |
| `types.rs` | ~1,000 | [ ] | |

**Review Questions:**
- [ ] OGC API - EDR spec compliance?
- [ ] CoverageJSON format correctness?
- [ ] Response size limits?
- [ ] Proper content negotiation?

---

## Phase 5: Renderer Crate

### 5.1 renderer (~5,202 lines)
*Image rendering engine*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `lib.rs` | ~100 | [ ] | |
| `barbs.rs` | ~800 | [ ] | |
| `buffer_pool.rs` | ~300 | [ ] | |
| `contour.rs` | ~1,200 | [ ] | |
| `data_png.rs` | ~500 | [ ] | |
| `gradient.rs` | ~800 | [ ] | |
| `png.rs` | ~500 | [ ] | |
| `style.rs` | ~1,000 | [ ] | |

**Review Questions:**
- [ ] Buffer pooling effectiveness?
- [ ] PNG encoding performance?
- [ ] Marching squares correctness?
- [ ] Memory allocation patterns in hot paths?

---

## Phase 6: Services - Downloader & Ingester

### 6.1 downloader (~8,705 lines)
*Weather data download service*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `main.rs` | ~300 | [ ] | |
| `scheduler.rs` | ~500 | [ ] | |
| `download.rs` | ~800 | [ ] | |
| `model_runner.rs` | ~600 | [ ] | |
| `observation_runner.rs` | ~400 | [ ] | |
| `grib_index.rs` | ~500 | [ ] | |
| `state.rs` | ~400 | [ ] | |
| `server.rs` | ~300 | [ ] | |
| `cleanup.rs` | ~300 | [ ] | |
| `concurrency.rs` | ~300 | [ ] | |
| `config.rs` | ~400 | [ ] | |
| `discovery.rs` | ~500 | [ ] | |
| `goes_runner.rs` | ~600 | [ ] | |
| `metrics.rs` | ~300 | [ ] | |
| `notifications.rs` | ~200 | [ ] | |
| `progress.rs` | ~300 | [ ] | |
| `retry.rs` | ~200 | [ ] | |
| (remaining files) | ~2,000 | [ ] | |

**Review Questions:**
- [ ] Resume logic correctness?
- [ ] Retry/backoff strategy appropriate?
- [ ] Disk space management?
- [ ] Concurrency safety?

---

### 6.2 ingester (~970 lines)
*Data ingestion HTTP service*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `main.rs` | ~175 | [ ] | |
| `server.rs` | ~795 | [ ] | |

**Review Questions:**
- [ ] Request validation?
- [ ] Error response consistency?
- [ ] Memory management during ingestion?

---

## Phase 7: Services - WMS API

### 7.1 wms-api (~22,014 lines)
*OGC WMS/WMTS tile server*

#### Core Files
| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `main.rs` | ~470 | [ ] | |
| `state.rs` | ~260 | [ ] | |
| `layer_config.rs` | ~600 | [ ] | |
| `model_config.rs` | ~300 | [ ] | |
| `warming.rs` | ~500 | [ ] | |
| `chunk_warming.rs` | ~400 | [ ] | |
| `cleanup.rs` | ~300 | [ ] | |
| `memory_pressure.rs` | ~400 | [ ] | |
| `admin.rs` | ~500 | [ ] | |
| `metrics.rs` | ~400 | [ ] | |

#### Handlers
| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `handlers/mod.rs` | ~100 | [ ] | |
| `handlers/wms.rs` | ~1,500 | [ ] | |
| `handlers/wmts.rs` | ~1,800 | [ ] | |
| `handlers/api.rs` | ~800 | [ ] | |
| `handlers/metrics.rs` | ~300 | [ ] | |
| `handlers/validation.rs` | ~400 | [ ] | |
| `handlers/cache.rs` | ~300 | [ ] | |
| `handlers/benchmarks.rs` | ~200 | [ ] | |
| `handlers/docs.rs` | ~200 | [ ] | |

#### Rendering
| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `rendering/mod.rs` | ~100 | [ ] | |
| `rendering/colorscales.rs` | ~600 | [ ] | |
| `rendering/isolines.rs` | ~800 | [ ] | |
| `rendering/wind.rs` | ~600 | [ ] | |
| `rendering/sampling.rs` | ~500 | [ ] | |
| `rendering/resampling.rs` | ~600 | [ ] | |
| (remaining files) | ~10,000 | [ ] | |

**Review Questions:**
- [ ] Request validation completeness?
- [ ] Memory pressure handling effective?
- [ ] Cache invalidation correctness?
- [ ] Tile rendering performance?
- [ ] Thread pool sizing?

---

## Phase 8: Services - EDR API

### 8.1 edr-api (~18,158 lines)
*OGC API - EDR data query service*

#### Core Files
| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `main.rs` | ~290 | [ ] | |
| `state.rs` | ~145 | [ ] | |
| `config.rs` | ~820 | [ ] | |
| `validation.rs` | ~135 | [ ] | |
| `resampling.rs` | ~685 | [ ] | |
| `temporal_interpolation.rs` | ~330 | [ ] | |
| `content_negotiation.rs` | ~600 | [ ] | |
| `availability.rs` | ~290 | [ ] | |
| `metrics.rs` | ~830 | [ ] | |
| `astro.rs` | ~427 | [ ] | |

#### Handlers
| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `handlers/mod.rs` | ~100 | [ ] | |
| `handlers/landing.rs` | ~200 | [ ] | |
| `handlers/conformance.rs` | ~150 | [ ] | |
| `handlers/collections.rs` | ~800 | [ ] | |
| `handlers/instances.rs` | ~600 | [ ] | |
| `handlers/position.rs` | ~1,000 | [ ] | |
| `handlers/area.rs` | ~1,200 | [ ] | |
| `handlers/radius.rs` | ~800 | [ ] | |
| `handlers/trajectory.rs` | ~600 | [ ] | |
| `handlers/corridor.rs` | ~500 | [ ] | |
| `handlers/cube.rs` | ~400 | [ ] | |
| `handlers/locations.rs` | ~800 | [ ] | |
| `handlers/observations.rs` | ~1,000 | [ ] | |
| `handlers/health.rs` | ~200 | [ ] | |
| `handlers/api.rs` | ~300 | [ ] | |
| `handlers/catalog_check.rs` | ~200 | [ ] | |
| `handlers/forecast_params.rs` | ~300 | [ ] | |
| `handlers/light_pollution.rs` | ~400 | [ ] | |
| (remaining files) | ~6,000 | [ ] | |

**Review Questions:**
- [ ] Query parameter validation?
- [ ] Response size limits enforced?
- [ ] SQL injection prevention?
- [ ] CoverageJSON spec compliance?
- [ ] Temporal interpolation accuracy?

---

## Phase 9: Load Testing

### 9.1 load-test (~1,755 lines)
*Performance validation tools*

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `main.rs` | ~330 | [ ] | |
| `lib.rs` | ~20 | [ ] | |
| `config.rs` | ~130 | [ ] | |
| `generator.rs` | ~380 | [ ] | |
| `metrics.rs` | ~275 | [ ] | |
| `report.rs` | ~90 | [ ] | |
| `runner.rs` | ~370 | [ ] | |
| `wms_client.rs` | ~130 | [ ] | |

**Review Questions:**
- [ ] Test scenarios realistic?
- [ ] Metrics collection accurate?
- [ ] Report generation useful?

---

## Review Checklist Template

For each file, check:

### Code Quality
- [ ] Follows Rust naming conventions
- [ ] Uses appropriate visibility (pub/pub(crate)/private)
- [ ] No unnecessary `clone()` or allocations
- [ ] Proper use of lifetimes where beneficial
- [ ] Consistent error handling pattern
- [ ] No `unwrap()` in production code paths

### Documentation
- [ ] Module-level documentation exists
- [ ] Public items have doc comments
- [ ] Complex algorithms explained
- [ ] Examples provided where helpful
- [ ] Edge cases documented

### Performance
- [ ] No allocations in hot paths (or justified)
- [ ] Appropriate use of iterators vs loops
- [ ] Parallel processing where beneficial
- [ ] Caching used appropriately
- [ ] No unnecessary copies

### Error Handling
- [ ] Custom error types where appropriate
- [ ] Errors provide context (using `anyhow` or `thiserror`)
- [ ] No `unwrap()` or `expect()` in production paths
- [ ] Errors are actionable for users
- [ ] Proper error propagation with `?`

### Security
- [ ] Input validation on all external data
- [ ] SQL queries use parameterization
- [ ] File paths are sanitized
- [ ] No sensitive data in logs
- [ ] Bounds checking on array access

---

## Progress Tracking

| Phase | Component | Lines | Status | Reviewer | Date Started | Date Completed |
|-------|-----------|-------|--------|----------|--------------|----------------|
| 1 | wms-common | 2,646 | Not Started | | | |
| 1 | projection | 2,425 | Not Started | | | |
| 1 | test-utils | 1,233 | Not Started | | | |
| 2 | grib2-parser | 1,900 | Not Started | | | |
| 2 | netcdf-parser | 580 | Not Started | | | |
| 3 | storage | 5,057 | Not Started | | | |
| 3 | ingestion | 4,697 | Not Started | | | |
| 3 | grid-processor | 6,204 | Not Started | | | |
| 4 | wms-protocol | 997 | Not Started | | | |
| 4 | edr-protocol | 8,565 | Not Started | | | |
| 5 | renderer | 5,202 | Not Started | | | |
| 6 | downloader | 8,705 | Not Started | | | |
| 6 | ingester | 970 | Not Started | | | |
| 7 | wms-api | 22,014 | Not Started | | | |
| 8 | edr-api | 18,158 | Not Started | | | |
| 9 | load-test | 1,755 | Not Started | | | |
| | **TOTAL** | **101,108** | | | | |

---

## Known Issues to Address

From the codebase scan, 22 TODO/FIXME comments exist that should be reviewed:

| Location | Issue |
|----------|-------|
| `crates/renderer/src/lib.rs` | "TODO: Implement rendering algorithms" |
| `crates/storage/src/catalog.rs` | Dynamic query building |
| `crates/wms-common/src/crs.rs` | Projection parameters for transformation |
| `crates/grid-processor/src/downsample.rs` | Configurable downsample factor |
| `crates/edr-protocol/src/queries.rs` | Distance to line segments check |
| `services/ingester/src/server.rs` | Metrics integration |
| `services/wms-api/src/admin.rs` | Uptime tracking |
| `services/edr-api/src/handlers/health.rs` | MinIO health check |
| `services/edr-api/src/handlers/area.rs` | Bilinear interpolation option |
| `services/edr-api/src/handlers/area.rs` | Multi-z support |

*(Additional TODOs will be cataloged during review)*

---

## Findings Log

### Critical Issues
*(Document any critical issues found during review)*

| Date | Component | File | Issue | Resolution |
|------|-----------|------|-------|------------|
| | | | | |

### Improvements Made
*(Track improvements made during review)*

| Date | Component | File | Change | PR/Commit |
|------|-----------|------|--------|-----------|
| | | | | |

### Technical Debt
*(Document technical debt identified)*

| Component | Issue | Priority | Effort |
|-----------|-------|----------|--------|
| | | | |

---

## Notes

- Line counts are approximate and will be refined during review
- Review order follows dependency graph (foundations first)
- Each phase can be done in parallel by different reviewers
- Update this document as review progresses
