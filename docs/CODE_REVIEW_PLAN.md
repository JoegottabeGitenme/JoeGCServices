# Code Review Plan: Weather WMS Backend

This document outlines a systematic approach to reviewing the Rust codebase, progressing from data acquisition through to the WMS API layer.

## Overview

The system follows a data pipeline architecture:

```
┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│  Downloader │───►│  Ingester   │───►│   Storage   │───►│   WMS-API   │
│  (fetch)    │    │  (parse)    │    │  (persist)  │    │  (serve)    │
└─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
                          │                  ▲
                          ▼                  │
                   ┌─────────────┐    ┌─────────────┐
                   │ grib2-parser│    │  renderer   │
                   │ netcdf-parse│    │ projection  │
                   └─────────────┘    └─────────────┘
```

---

## Phase 1: Core Parsing Libraries

### 1.1 `crates/grib2-parser/` (~1,200 lines)

**Purpose**: Parse GRIB2 meteorological data files

**Key Files**:
| File | Lines | Focus Areas |
|------|-------|-------------|
| `src/lib.rs` | 345 | `Grib2Reader`, `Grib2Message`, `unpack_data()` |
| `src/sections/mod.rs` | 679 | Section parsing (0-7), parameter tables |
| `src/unpacking/mod.rs` | 139 | Simple packing algorithm |

**Review Checklist**:
- [ ] **Error handling**: Are all error cases properly caught? Can malformed files crash the parser?
- [ ] **Bounds checking**: Buffer reads use safe indexing?
- [ ] **Parameter tables**: Are NCEP local parameters correctly mapped? Missing mappings?
- [ ] **Memory safety**: `Bytes` slicing within bounds? Raw pointer usage?
- [ ] **Edge cases**: Empty messages, truncated files, invalid section lengths
- [ ] **Compression handling**: PNG/JPEG2000 via `grib` crate - error propagation correct?
- [ ] **Grid dimension extraction**: Correct for all grid templates (only 3.0 fully supported)?
- [ ] **Scanning mode**: `scanning_mode` flags interpreted correctly for data layout?

**Questions to Answer**:
1. What happens when the `grib` crate fails to decode a message?
2. Are there any `unwrap()`/`expect()` calls that could panic on bad input?
3. Is the parameter table comprehensive enough for production use?

---

### 1.2 `crates/netcdf-parser/` (~minimal)

**Purpose**: Parse NetCDF files (GOES satellite data)

**Review Checklist**:
- [ ] Uses external `netcdf` crate - review integration points
- [ ] Error handling for missing variables/dimensions
- [ ] Memory handling for large arrays

---

### 1.3 `crates/projection/` (~2,000 lines)

**Purpose**: Coordinate transformations and projection handling

**Key Files**:
| File | Focus Areas |
|------|-------------|
| `src/lib.rs` | Public API, projection traits |
| `src/geostationary.rs` | GOES satellite projection math |
| `src/lambert.rs` | Lambert Conformal Conic (HRRR) |
| `src/mercator.rs` | Web Mercator (EPSG:3857) |
| `src/lut.rs` | Pre-computed lookup tables |

**Review Checklist**:
- [ ] **Math correctness**: Projection formulas match OGC/PROJ standards?
- [ ] **Edge cases**: Poles, antimeridian, out-of-bounds coordinates
- [ ] **LUT implementation**: Memory safety, bounds checking
- [ ] **Numeric precision**: f32 vs f64 usage, floating point comparisons
- [ ] **Performance**: Hot paths identified and optimized?

---

## Phase 2: Storage Layer

### 2.1 `crates/storage/` (~2,500 lines)

**Key Files**:
| File | Lines | Focus Areas |
|------|-------|-------------|
| `src/catalog.rs` | ~800 | PostgreSQL queries, SQL injection risk |
| `src/object_store.rs` | ~400 | S3/MinIO operations, path handling |
| `src/cache.rs` | ~200 | Redis tile cache |
| `src/grib_cache.rs` | ~250 | LRU cache for raw GRIB bytes |
| `src/grid_cache.rs` | ~300 | LRU cache for parsed grids |
| `src/tile_memory_cache.rs` | ~300 | L1 in-memory tile cache |
| `src/queue.rs` | ~300 | Redis Streams job queue |

**Review Checklist**:
- [ ] **SQL safety**: Parameterized queries throughout? Any string interpolation?
- [ ] **Connection pooling**: Pool size appropriate? Connection leaks?
- [ ] **Cache invalidation**: TTL handling correct? Stale data scenarios?
- [ ] **Concurrent access**: RwLock usage correct? Deadlock potential?
- [ ] **Memory management**: Cache eviction triggers? Memory limits enforced?
- [ ] **Error propagation**: Database/Redis errors handled gracefully?
- [ ] **Path traversal**: Storage paths sanitized? Can users access arbitrary files?

**Questions to Answer**:
1. What happens when Redis is unavailable?
2. Can the LRU caches grow unbounded under load?
3. Are database migrations idempotent?

---

### 2.2 `crates/wms-common/` (~2,000 lines)

**Key Files**:
| File | Focus Areas |
|------|-------------|
| `src/error.rs` | Error types, WMS exception codes |
| `src/bbox.rs` | Bounding box parsing and validation |
| `src/crs.rs` | CRS parsing, axis order handling |
| `src/tile.rs` | Tile matrix calculations |
| `src/style.rs` | Style JSON parsing |

**Review Checklist**:
- [ ] **Input validation**: All WMS parameters validated before use?
- [ ] **Bbox parsing**: Handles all edge cases (negative values, swapped corners)?
- [ ] **CRS handling**: WMS 1.3.0 axis order correctly applied?
- [ ] **Error messages**: Informative without leaking internals?

---

## Phase 3: Data Acquisition

### 3.1 `services/downloader/` (~3,000 lines)

**Key Files**:
| File | Lines | Focus Areas |
|------|-------|-------------|
| `src/main.rs` | 208 | CLI, initialization, shutdown |
| `src/scheduler.rs` | 713 | S3 listing, cycle detection |
| `src/download.rs` | 462 | HTTP client, resumable downloads |
| `src/state.rs` | 740 | SQLite persistence |
| `src/config.rs` | 329 | YAML config loading |
| `src/server.rs` | 534 | Status API endpoints |

**Review Checklist**:
- [ ] **Network resilience**: Retry logic correct? Exponential backoff?
- [ ] **Partial downloads**: Resume logic handles all edge cases?
- [ ] **S3 interaction**: Pagination for large listings? Rate limiting?
- [ ] **State persistence**: SQLite transactions? Recovery from crashes?
- [ ] **Resource cleanup**: Partial files cleaned up on failure?
- [ ] **URL construction**: Template injection vulnerabilities?
- [ ] **Shutdown handling**: Graceful shutdown? In-progress downloads?

**Questions to Answer**:
1. What happens if S3 returns 503 (rate limited)?
2. Can two downloader instances run concurrently safely?
3. How are file integrity issues detected (corrupted downloads)?

---

### 3.2 `services/ingester/` (~2,800 lines)

**Key Files**:
| File | Lines | Focus Areas |
|------|-------|-------------|
| `src/main.rs` | 696 | CLI, test file ingestion |
| `src/ingest.rs` | 511 | Ingestion pipeline |
| `src/config.rs` | 355 | Configuration structs |
| `src/config_loader.rs` | 797 | YAML loading, env var expansion |
| `src/sources.rs` | 408 | Data source fetchers |

**Review Checklist**:
- [ ] **GRIB2 parsing integration**: Errors handled? Missing parameters?
- [ ] **Parameter extraction**: All target parameters extracted correctly?
- [ ] **Level handling**: Pressure levels, height above ground, etc.?
- [ ] **Duplicate handling**: Upsert logic correct? Race conditions?
- [ ] **Storage paths**: Consistent naming? Path collisions?
- [ ] **Cleanup logic**: Old data properly expired? Orphan detection?
- [ ] **Memory usage**: Large files streamed or loaded entirely?

**Questions to Answer**:
1. What happens if the same file is ingested twice concurrently?
2. How are parameter name conflicts resolved (same name, different levels)?
3. Is the ingestion atomic (all-or-nothing per file)?

---

## Phase 4: Rendering Pipeline

### 4.1 `crates/renderer/` (~2,800 lines)

**Key Files**:
| File | Lines | Focus Areas |
|------|-------|-------------|
| `src/gradient.rs` | 453 | Grid resampling, color mapping |
| `src/contour.rs` | 890 | Marching squares, isoline rendering |
| `src/barbs.rs` | 594 | Wind barb SVG rendering |
| `src/numbers.rs` | 281 | Numeric label rendering |
| `src/style.rs` | 516 | Style config parsing |
| `src/png.rs` | 78 | PNG encoding |

**Review Checklist**:
- [ ] **Resampling**: Bilinear interpolation correct at edges?
- [ ] **NaN handling**: Missing values handled throughout pipeline?
- [ ] **Color interpolation**: Correct for all color stop configurations?
- [ ] **Contour algorithm**: Edge cases (saddle points, boundaries)?
- [ ] **Wind barb selection**: Speed thresholds correct? Direction math?
- [ ] **Memory allocation**: Large tile sizes handled? Buffer overflows?
- [ ] **PNG encoding**: Correct RGBA format? Alpha handling?

**Questions to Answer**:
1. What happens when style config is missing required fields?
2. Are there numeric overflow risks in coordinate calculations?
3. How are tiles at data boundaries rendered (no data on one side)?

---

## Phase 5: WMS/WMTS API

### 5.1 `services/wms-api/` (~12,000+ lines)

**Key Files**:
| File | Lines | Focus Areas |
|------|-------|-------------|
| `src/main.rs` | 345 | Server startup, background tasks |
| `src/handlers.rs` | 3813 | WMS/WMTS request handlers |
| `src/rendering.rs` | 2000+ | Tile rendering pipeline |
| `src/state.rs` | 477 | AppState, cache configuration |
| `src/admin.rs` | 2100+ | Admin API endpoints |
| `src/metrics.rs` | 1263 | Prometheus metrics |
| `src/warming.rs` | 327 | Cache warming |
| `src/grid_warming.rs` | 427 | Grid pre-caching |
| `src/memory_pressure.rs` | 251 | Memory management |
| `src/cleanup.rs` | 478 | Data retention |

**Review Checklist**:

#### Request Handling
- [ ] **Parameter parsing**: All WMS params validated? Case sensitivity?
- [ ] **Bbox validation**: Invalid bbox rejected? Axis order applied?
- [ ] **Layer validation**: Non-existent layers return proper error?
- [ ] **Time parsing**: ISO8601 and forecast hour formats both work?
- [ ] **CRS handling**: All supported CRS work correctly?

#### Rendering
- [ ] **Cache key uniqueness**: All parameters included in cache key?
- [ ] **Cache invalidation**: Stale tiles purged when data updates?
- [ ] **Error recovery**: Render failures return proper WMS exceptions?
- [ ] **Timeout handling**: Long-running renders cancelled properly?

#### Caching
- [ ] **L1/L2 consistency**: Both caches updated atomically?
- [ ] **Cache poisoning**: Can invalid tiles get cached?
- [ ] **Memory pressure**: Eviction triggered at correct thresholds?

#### Background Tasks
- [ ] **Shutdown handling**: All tasks terminate gracefully?
- [ ] **Error isolation**: One task failure doesn't crash server?
- [ ] **Resource contention**: Background tasks vs request handling?

#### Security
- [ ] **Input sanitization**: No path traversal in layer names?
- [ ] **Error messages**: No internal details leaked?
- [ ] **Rate limiting**: DDoS protection?
- [ ] **Resource limits**: Max tile dimensions? Request body size?

**Questions to Answer**:
1. What happens if PostgreSQL becomes unavailable mid-request?
2. Can concurrent requests for the same tile cause duplicate rendering?
3. How does the server behave when all caches are exhausted?

---

## Phase 6: Integration & Cross-Cutting Concerns

### 6.1 Error Handling Audit
- [ ] Consistent error types across all crates
- [ ] No panics in production code paths
- [ ] Errors logged with appropriate context
- [ ] WMS exceptions comply with OGC spec

### 6.2 Concurrency Review
- [ ] No data races (run with `--cfg tokio_unstable` and tokio-console)
- [ ] Deadlock-free (consistent lock ordering)
- [ ] Appropriate use of `Arc` vs `Rc`
- [ ] Connection pools properly sized

### 6.3 Performance Review
- [ ] Hot paths identified via profiling
- [ ] Unnecessary allocations eliminated
- [ ] Caches sized appropriately for workload
- [ ] Database queries use appropriate indexes

### 6.4 Configuration Review
- [ ] All env vars documented
- [ ] Sensible defaults for all options
- [ ] Invalid config detected at startup
- [ ] No secrets logged or exposed

---

## Review Priority

| Priority | Component | Risk Level | Complexity |
|----------|-----------|------------|------------|
| 1 | `handlers.rs` (WMS params) | High | High |
| 2 | `catalog.rs` (SQL) | High | Medium |
| 3 | `rendering.rs` (data flow) | Medium | High |
| 4 | `grib2-parser` | Medium | High |
| 5 | `download.rs` (network) | Medium | Medium |
| 6 | `ingest.rs` (pipeline) | Medium | Medium |
| 7 | `renderer/` | Low | High |
| 8 | `projection/` | Low | High |
| 9 | `storage/` (caches) | Low | Medium |

---

## Suggested Review Process

1. **Static Analysis First**
   ```bash
   cargo clippy --all-targets --all-features -- -D warnings
   cargo audit
   cargo deny check
   ```

2. **Unit Test Coverage**
   ```bash
   cargo tarpaulin --out Html
   ```

3. **Manual Code Review** (per phase above)

4. **Integration Testing**
   - Invalid input fuzzing
   - Load testing under memory pressure
   - Failure injection (kill Redis, Postgres, MinIO)

5. **Security Review**
   - Input validation audit
   - SQL injection testing
   - Path traversal testing

---

## Appendix: File Inventory

### Services (Binaries)
| Service | Lines | Purpose |
|---------|-------|---------|
| `services/downloader/` | ~3,000 | Fetch data from NOAA |
| `services/ingester/` | ~2,800 | Parse and store data |
| `services/wms-api/` | ~12,000 | Serve WMS/WMTS requests |
| `services/renderer-worker/` | ~500 | Worker for distributed rendering |

### Libraries (Crates)
| Crate | Lines | Purpose |
|-------|-------|---------|
| `crates/grib2-parser/` | ~1,200 | GRIB2 file parsing |
| `crates/netcdf-parser/` | ~200 | NetCDF file parsing |
| `crates/projection/` | ~2,000 | Coordinate projections |
| `crates/renderer/` | ~2,800 | Tile rendering |
| `crates/storage/` | ~2,500 | Storage abstractions |
| `crates/wms-common/` | ~2,000 | Shared types |
| `crates/wms-protocol/` | ~1,000 | WMS/WMTS protocol types |

**Total Rust LOC**: ~28,000+
