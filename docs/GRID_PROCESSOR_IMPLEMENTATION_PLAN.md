# Comprehensive Implementation Plan: Grid Processing Abstraction Layer with Zarr

*A cup-of-tea read for transforming Weather WMS data storage and access*

**Author:** Claude (AI Assistant)  
**Date:** December 12, 2024  
**Status:** Proposed

---

## Executive Summary

This plan introduces a **Grid Processing Abstraction Layer** between the WMS-API and MinIO storage, using the **Zarr V3 format with sharding** to enable efficient byte-range access to weather grid data. The goal is to dramatically reduce rendering latency by fetching only the chunks needed for a specific tile request, rather than loading entire grid files.

**Key Benefits:**
- **10-100x reduction** in data transferred from MinIO per request
- **Standardized format** with industry support (NASA, NOAA, Pangeo)
- **Future-proof architecture** that cleanly separates data access from rendering
- **Support for additional OGC services** (GetFeatureInfo, future WCS, etc.)

**Timeline Estimate:** 3-4 weeks for GFS implementation, additional time for other models

---

## Table of Contents

1. [Current State Analysis](#1-current-state-analysis)
2. [Target Architecture](#2-target-architecture)
3. [Zarr Format Specification](#3-zarr-format-specification)
4. [New Crate: `grid-processor`](#4-new-crate-grid-processor)
5. [Ingestion Pipeline Changes](#5-ingestion-pipeline-changes)
6. [WMS-API Integration](#6-wms-api-integration)
7. [Configuration & Environment Variables](#7-configuration--environment-variables)
8. [Migration Strategy](#8-migration-strategy)
9. [Testing Plan](#9-testing-plan)
10. [Future Considerations](#10-future-considerations)
11. [Risk Assessment](#11-risk-assessment)
12. [Implementation Phases](#12-implementation-phases)

---

## 1. Current State Analysis

### 1.1 Current Data Flow

```
NOAA S3 → Downloader → Ingester → MinIO (GRIB2/NetCDF) → WMS-API → Client
                                         ↓
                              Load ENTIRE file (~4MB)
                                         ↓
                              Parse GRIB2/NetCDF
                                         ↓
                              Extract region for tile
                                         ↓
                              Render 256×256 PNG
```

### 1.2 Current Performance Bottlenecks

| Model | File Size | Grid Points | Cache Miss Latency | Bottleneck |
|-------|-----------|-------------|-------------------|------------|
| GFS | ~4 MB | 1,037,440 | ~400-600 ms | Full file load |
| HRRR | ~2-5 MB | ~1.9M | ~400-600 ms | Full file load + projection |
| GOES | ~2.8 MB | 3.75M | ~600-800 ms | Temp file I/O + projection |
| MRMS | ~400 KB | 24.5M | ~600 ms | GRIB2 decompression |

### 1.3 The Core Problem

A WMS tile request at zoom level 8 needs approximately **0.01-1%** of the grid data, yet the system loads **100%** of the file. For GFS:

- Tile covers: ~1° × 1° area
- Grid covers: 360° × 180° area
- Data actually needed: ~16 grid points (4×4)
- Data loaded: 1,037,440 grid points

**This is a 65,000x over-fetch.**

---

## 2. Target Architecture

### 2.1 New Data Flow

```
NOAA S3 → Downloader → Ingester → [NEW] Convert to Zarr → MinIO (Zarr)
                                                              ↓
                                                    [NEW] GridProcessor
                                                              ↓
                                                    Calculate needed chunks
                                                              ↓
                                                    Byte-range request (1-4 chunks)
                                                              ↓
                                                    ChunkCache (decompressed)
                                                              ↓
                                                    WMS-API renders tile
```

### 2.2 Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              INGESTION PIPELINE                              │
│                                                                              │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐    ┌───────────┐ │
│  │  Downloader  │───▶│   Ingester   │───▶│ ZarrWriter   │───▶│   MinIO   │ │
│  │  (existing)  │    │  (existing)  │    │    (new)     │    │  (Zarr)   │ │
│  └──────────────┘    └──────────────┘    └──────────────┘    └───────────┘ │
│                              │                                       │       │
│                              ▼                                       │       │
│                    Parse GRIB2/NetCDF                                │       │
│                    Re-project to Geographic                          │       │
│                    Chunk + Compress                                  │       │
│                    Write Zarr format                                 │       │
└──────────────────────────────────────────────────────────────────────┼───────┘
                                                                       │
┌──────────────────────────────────────────────────────────────────────┼───────┐
│                              WMS-API SERVICE                         │       │
│                                                                      ▼       │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                    GridProcessor (new crate)                         │   │
│  │  ┌────────────────────────────────────────────────────────────────┐  │   │
│  │  │                     ZarrGridProcessor                          │  │   │
│  │  │  • Opens Zarr array from MinIO                                 │  │   │
│  │  │  • Calculates chunk indices for bbox                           │  │   │
│  │  │  • Issues byte-range requests via object_store                 │  │   │
│  │  │  • Decompresses and assembles grid region                      │  │   │
│  │  └────────────────────────────────────────────────────────────────┘  │   │
│  │                              │                                        │   │
│  │                              ▼                                        │   │
│  │  ┌────────────────────────────────────────────────────────────────┐  │   │
│  │  │                      ChunkCache                                │  │   │
│  │  │  • LRU cache of decompressed chunks                           │  │   │
│  │  │  • Keyed by (zarr_path, chunk_x, chunk_y)                     │  │   │
│  │  │  • Memory-bounded via CHUNK_CACHE_SIZE_MB                     │  │   │
│  │  │  • Integrates with memory pressure management                 │  │   │
│  │  └────────────────────────────────────────────────────────────────┘  │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                              │                                               │
│                              ▼                                               │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                    Rendering Pipeline (existing)                     │   │
│  │  • Receives GridRegion instead of full grid                         │   │
│  │  • Resample to tile resolution                                       │   │
│  │  • Apply color scale                                                 │   │
│  │  • Encode PNG                                                        │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                              │                                               │
│                              ▼                                               │
│                    Tile Caches (existing L1/L2)                             │
│                              │                                               │
│                              ▼                                               │
│                         HTTP Response                                        │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 2.3 Key Design Principles

1. **Separation of Concerns**: `GridProcessor` handles all data access; rendering only deals with `GridRegion` objects
2. **Format Agnostic Interface**: The `GridProcessor` trait allows future format support without changing consumers
3. **Efficient Caching**: Chunk-level caching maximizes reuse across adjacent tile requests
4. **Standard Format**: Zarr V3 with sharding is an industry standard, not a custom invention

---

## 3. Zarr Format Specification

### 3.1 Why Zarr V3 with Sharding?

Zarr V3 introduced **sharding** (ZEP-2), which solves the "millions of small files" problem by storing multiple chunks in a single file with an index for byte-range access.

**Without sharding:**
```
array/
├── zarr.json
├── c/0/0    # Chunk file
├── c/0/1    # Chunk file
├── c/1/0    # Chunk file
└── c/1/1    # Chunk file
```

**With sharding:**
```
array/
├── zarr.json       # Metadata (~500 bytes)
└── c/0/0           # Single shard containing all chunks (~1.5 MB)
```

### 3.2 File Structure for Weather Data

**Storage path pattern:**
```
grids/{model}/{run_date}/{parameter}_{level}_f{forecast:03}.zarr/
├── zarr.json       # Array metadata
└── c/0/0           # Sharded data (all chunks in one file)
```

**Example for GFS temperature:**
```
grids/gfs/20241212_00z/TMP_2m_f006.zarr/
├── zarr.json
└── c/0/0
```

### 3.3 Zarr Metadata (`zarr.json`)

```json
{
  "zarr_format": 3,
  "node_type": "array",
  "shape": [721, 1440],
  "data_type": "float32",
  "chunk_grid": {
    "name": "regular",
    "configuration": {
      "chunk_shape": [512, 512]
    }
  },
  "chunk_key_encoding": {
    "name": "default",
    "configuration": {
      "separator": "/"
    }
  },
  "codecs": [
    {
      "name": "transpose",
      "configuration": {"order": [1, 0]}
    },
    {
      "name": "bytes",
      "configuration": {"endian": "little"}
    },
    {
      "name": "blosc",
      "configuration": {
        "cname": "zstd",
        "clevel": 1,
        "shuffle": "shuffle",
        "typesize": 4,
        "blocksize": 0
      }
    }
  ],
  "fill_value": "NaN",
  "attributes": {
    "model": "gfs",
    "parameter": "TMP",
    "level": "2 m above ground",
    "units": "K",
    "reference_time": "2024-12-12T00:00:00Z",
    "forecast_hour": 6,
    "bbox": [0.0, -90.0, 360.0, 90.0],
    "projection": "geographic",
    "source_format": "GRIB2"
  }
}
```

### 3.4 Sharding Configuration

For GFS (1440×721 grid with 512×512 chunks = 6 chunks total):

```json
{
  "codecs": [
    {
      "name": "sharding_indexed",
      "configuration": {
        "chunk_shape": [512, 512],
        "codecs": [
          {"name": "bytes"},
          {"name": "blosc", "configuration": {"cname": "zstd", "clevel": 1, "shuffle": "shuffle"}}
        ],
        "index_codecs": [
          {"name": "bytes", "configuration": {"endian": "little"}},
          {"name": "crc32c"}
        ],
        "index_location": "end"
      }
    }
  ]
}
```

### 3.5 Chunk Layout

For GFS 1440×721 grid with 512×512 chunks:

```
┌─────────────────────────────────────────────────────────────────┐
│                     GFS Grid (1440 × 721)                       │
├───────────────┬───────────────┬───────────────┬─────────────────┤
│  Chunk (0,0)  │  Chunk (1,0)  │  Chunk (2,0)  │  (partial)      │
│  512 × 512    │  512 × 512    │  416 × 512    │                 │
├───────────────┼───────────────┼───────────────┤   Row 0         │
│  Chunk (0,1)  │  Chunk (1,1)  │  Chunk (2,1)  │                 │
│  512 × 209    │  512 × 209    │  416 × 209    │   Row 1         │
└───────────────┴───────────────┴───────────────┴─────────────────┘
        Col 0         Col 1          Col 2

Total: 6 chunks (3 columns × 2 rows)
```

### 3.6 Byte-Range Access Pattern

**Metadata access (zero MinIO requests):**
- `zarr.json` metadata (~500 bytes) → Stored in PostgreSQL catalog at ingestion time
- Shard index (~100 bytes) → Stored in PostgreSQL catalog at ingestion time
- **No MinIO requests needed for metadata!** It comes with the catalog query.

**Per-tile request:**
1. Query catalog → Returns dataset entry WITH `zarr_metadata` and `shard_index`
2. Calculate which chunks intersect the requested bbox (pure arithmetic, ~1μs)
3. Look up byte offsets in shard index (from catalog, already in memory)
4. Issue byte-range request(s) to MinIO for needed chunks (typically 1-4)
5. Decompress and assemble

**Example request flow:**
```
Tile Request: z=8, x=45, y=102

Step 1: Catalog query (already needed to find storage_path)
  → Returns: storage_path, zarr_metadata, shard_index
  → MinIO requests: 0

Step 2: Calculate chunks needed
  → BBox: [-98.5°, 35.2°, -97.1°, 36.6°]
  → Needed chunks: [(1, 0)]  # Just one chunk!
  → Time: ~1 microsecond (pure math)

Step 3: Fetch chunk data from MinIO
  → Byte range: 524288-786432
  → Transfer: ~260 KB (compressed)
  → MinIO requests: 1

Step 4: Decompress and extract
  → Decompress: ~1 MB
  → Extract: 16×16 grid points for tile

Total MinIO requests: 1 (just the chunk data)
```

#### Chunk-to-Tile Mapping: Computed, Not Stored

**The mapping from tile (z/x/y) to chunk indices is deterministic and computed on-the-fly** - no pre-computation or database lookup required. The calculation is pure arithmetic (~1 microsecond) and only needs the tile coordinates plus grid metadata (already cached).

```rust
/// Calculate which chunks are needed for a tile request.
/// This is O(1) - just arithmetic, no iteration or lookup.
fn chunks_for_tile(
    tile: &TileCoord,
    grid_bbox: &BoundingBox,      // e.g., [0, -90, 360, 90] for GFS
    grid_shape: (usize, usize),   // e.g., (1440, 721) for GFS
    chunk_shape: (usize, usize),  // e.g., (512, 512)
) -> Vec<(usize, usize)> {
    // 1. Calculate tile bbox (standard Web Mercator formula) - O(1)
    let tile_bbox = tile_to_bbox(tile);
    
    // 2. Calculate grid resolution
    let lon_per_cell = (grid_bbox.max_lon - grid_bbox.min_lon) / grid_shape.0 as f64;
    let lat_per_cell = (grid_bbox.max_lat - grid_bbox.min_lat) / grid_shape.1 as f64;
    
    // 3. Convert tile bbox to grid cell indices
    let min_col = ((tile_bbox.min_lon - grid_bbox.min_lon) / lon_per_cell).floor() as usize;
    let max_col = ((tile_bbox.max_lon - grid_bbox.min_lon) / lon_per_cell).ceil() as usize;
    let min_row = ((grid_bbox.max_lat - tile_bbox.max_lat) / lat_per_cell).floor() as usize;
    let max_row = ((grid_bbox.max_lat - tile_bbox.min_lat) / lat_per_cell).ceil() as usize;
    
    // 4. Convert grid indices to chunk indices
    let min_chunk_x = min_col / chunk_shape.0;
    let max_chunk_x = (max_col + chunk_shape.0 - 1) / chunk_shape.0;
    let min_chunk_y = min_row / chunk_shape.1;
    let max_chunk_y = (max_row + chunk_shape.1 - 1) / chunk_shape.1;
    
    // 5. Return chunk coordinates (typically 1-4 chunks)
    (min_chunk_y..=max_chunk_y)
        .flat_map(|cy| (min_chunk_x..=max_chunk_x).map(move |cx| (cx, cy)))
        .collect()
}
```

**Why this works without pre-computation:**

| Property | Explanation |
|----------|-------------|
| **Deterministic** | Same tile always maps to same chunks - pure math |
| **Fast** | ~1μs computation, negligible vs network I/O (milliseconds) |
| **Stateless** | Only needs tile coords + grid metadata (cached in processor) |
| **No storage needed** | No database tables or lookup files required |

**Example for GFS:**
```
GFS Grid: 1440×721, bbox [0°, -90°, 360°, 90°]
Chunk size: 512×512
Number of chunks: 3×2 = 6 total

Tile request: z=6, x=15, y=25
  → Tile bbox: [-101.25°, 31.95°, -95.625°, 36.60°]
  → Grid cells: cols 282-298, rows 190-205
  → Chunks needed: (0, 0) only!

Most tiles at typical zoom levels (z=4-10) only need 1 chunk.
At very low zoom (z=0-3), might need 2-4 chunks.
```

The `ZarrGridProcessor` caches the grid metadata (bbox, shape, chunk_shape) when opened, so `chunks_for_tile()` requires no I/O - it's essentially free.

### 3.7 Compression Analysis

**File size comparison for GFS (1440×721 grid = 4.15 MB uncompressed):**

| Compression | Ratio | Chunk Size (512×512) | Full Grid Size | Decompress Speed |
|-------------|-------|---------------------|----------------|------------------|
| None | 1.0x | 1,048 KB | 4,150 KB | N/A |
| LZ4 | 2.0-2.5x | 420-525 KB | 1,660-2,075 KB | 4,000 MB/s |
| Zstd L1 | 2.8-3.0x | 350-375 KB | 1,380-1,480 KB | 1,550 MB/s |
| Blosc+Zstd+Shuffle | 4-5x | 210-260 KB | 830-1,040 KB | 1,400 MB/s |

**Recommendation:** Blosc with Zstd and shuffle filter provides the best balance of compression ratio and decompression speed for f32 weather data.

---

## 4. New Crate: `grid-processor`

### 4.1 Crate Structure

```
crates/grid-processor/
├── Cargo.toml
├── src/
│   ├── lib.rs                 # Public API exports
│   ├── config.rs              # Configuration from environment
│   ├── error.rs               # Error types
│   ├── types.rs               # Core types (GridRegion, BoundingBox, etc.)
│   │
│   ├── processor/
│   │   ├── mod.rs             # GridProcessor trait
│   │   └── zarr.rs            # ZarrGridProcessor implementation
│   │
│   ├── cache/
│   │   ├── mod.rs             # Cache module
│   │   └── chunk_cache.rs     # LRU chunk cache
│   │
│   ├── projection/
│   │   ├── mod.rs             # Projection utilities
│   │   ├── geographic.rs      # Geographic (lat/lon) handling
│   │   └── interpolation.rs   # Bilinear, nearest, cubic
│   │
│   └── writer/
│       ├── mod.rs             # Writer module
│       └── zarr_writer.rs     # Write Zarr from grid data
```

### 4.2 Cargo.toml Dependencies

```toml
[package]
name = "grid-processor"
version = "0.1.0"
edition = "2021"

[dependencies]
# Zarr support
zarrs = "0.18"                    # Zarr V3 with sharding support
zarrs_storage = "0.3"             # Storage backends

# Async runtime
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"

# Storage
object_store = { version = "0.11", features = ["aws"] }
bytes = "1"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Error handling
thiserror = "1"
anyhow = "1"

# Caching
lru = "0.12"

# Logging
tracing = "0.1"

# Utilities
bytemuck = { version = "1", features = ["derive"] }

# Internal dependencies
wms-common = { path = "../wms-common" }
storage = { path = "../storage" }
```

### 4.3 Core Types (`types.rs`)

```rust
use serde::{Deserialize, Serialize};

/// A geographic bounding box in WGS84 coordinates
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl BoundingBox {
    pub fn new(min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Self {
        Self { min_lon, min_lat, max_lon, max_lat }
    }

    pub fn intersects(&self, other: &BoundingBox) -> bool {
        !(self.max_lon < other.min_lon
            || self.min_lon > other.max_lon
            || self.max_lat < other.min_lat
            || self.min_lat > other.max_lat)
    }

    pub fn width(&self) -> f64 {
        self.max_lon - self.min_lon
    }

    pub fn height(&self) -> f64 {
        self.max_lat - self.min_lat
    }
}

/// Grid data for a specific region
#[derive(Debug, Clone)]
pub struct GridRegion {
    /// The grid values (row-major order)
    pub data: Vec<f32>,
    /// Width of the region in grid points
    pub width: usize,
    /// Height of the region in grid points
    pub height: usize,
    /// Geographic bounds of this region
    pub bbox: BoundingBox,
    /// Resolution in degrees per grid point (lon, lat)
    pub resolution: (f64, f64),
}

/// Metadata about a grid dataset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridMetadata {
    /// Model identifier (e.g., "gfs", "hrrr")
    pub model: String,
    /// Parameter name (e.g., "TMP")
    pub parameter: String,
    /// Level description (e.g., "2 m above ground")
    pub level: String,
    /// Physical units (e.g., "K")
    pub units: String,
    /// Reference time (model run time)
    pub reference_time: chrono::DateTime<chrono::Utc>,
    /// Forecast hour
    pub forecast_hour: u32,
    /// Full grid bounding box
    pub bbox: BoundingBox,
    /// Grid dimensions (width, height)
    pub shape: (usize, usize),
    /// Chunk dimensions
    pub chunk_shape: (usize, usize),
    /// Number of chunks (x, y)
    pub num_chunks: (usize, usize),
    /// Fill/missing value
    pub fill_value: f32,
}

/// Interpolation method for grid resampling
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpolationMethod {
    /// Nearest neighbor (preserves exact values)
    Nearest,
    /// Bilinear interpolation (smooth, slight value changes)
    #[default]
    Bilinear,
    /// Bicubic interpolation (smoothest, more compute)
    Cubic,
}

impl InterpolationMethod {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "nearest" => Self::Nearest,
            "cubic" => Self::Cubic,
            _ => Self::Bilinear,
        }
    }
}
```

### 4.4 GridProcessor Trait (`processor/mod.rs`)

```rust
use async_trait::async_trait;
use crate::{BoundingBox, GridMetadata, GridRegion, GridProcessorError};

/// Trait for accessing grid data with efficient partial reads
#[async_trait]
pub trait GridProcessor: Send + Sync {
    /// Read grid data for a geographic region.
    ///
    /// Returns only the grid points that fall within the bounding box,
    /// efficiently fetching only the chunks needed.
    ///
    /// # Arguments
    /// * `bbox` - Geographic bounding box to read
    ///
    /// # Returns
    /// * `GridRegion` containing the data and metadata for the region
    async fn read_region(&self, bbox: &BoundingBox) -> Result<GridRegion, GridProcessorError>;

    /// Read a single point value (for GetFeatureInfo).
    ///
    /// # Arguments
    /// * `lon` - Longitude in degrees
    /// * `lat` - Latitude in degrees
    ///
    /// # Returns
    /// * `Some(value)` if the point is within the grid and has data
    /// * `None` if the point is outside the grid or is a fill value
    async fn read_point(&self, lon: f64, lat: f64) -> Result<Option<f32>, GridProcessorError>;

    /// Get metadata about the grid.
    fn metadata(&self) -> &GridMetadata;

    /// Prefetch chunks for anticipated requests.
    ///
    /// This is a hint to the processor that these regions will likely
    /// be requested soon. The implementation may choose to fetch
    /// and cache the relevant chunks proactively.
    async fn prefetch(&self, bboxes: &[BoundingBox]);

    /// Get cache statistics for monitoring.
    fn cache_stats(&self) -> CacheStats;
}

/// Statistics about the chunk cache
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub memory_bytes: u64,
    pub evictions: u64,
}
```

### 4.5 ZarrGridProcessor (`processor/zarr.rs`)

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use zarrs::array::Array;
use zarrs::array_subset::ArraySubset;
use zarrs_storage::AsyncReadableStorage;

use crate::{
    BoundingBox, CacheStats, ChunkCache, GridMetadata, GridProcessorError,
    GridRegion, GridProcessor, GridProcessorConfig,
};

/// Grid processor implementation using Zarr V3 with sharding
pub struct ZarrGridProcessor<S: AsyncReadableStorage> {
    /// The Zarr array
    array: Array<S>,
    /// Grid metadata extracted from Zarr attributes
    metadata: GridMetadata,
    /// Chunk cache for decompressed data
    chunk_cache: Arc<RwLock<ChunkCache>>,
    /// Configuration
    config: GridProcessorConfig,
}

impl<S: AsyncReadableStorage + Send + Sync + 'static> ZarrGridProcessor<S> {
    /// Open a Zarr array from storage
    pub async fn open(
        storage: S,
        path: &str,
        config: GridProcessorConfig,
    ) -> Result<Self, GridProcessorError> {
        // Open the Zarr array
        let array = Array::async_open(storage, path)
            .await
            .map_err(|e| GridProcessorError::OpenFailed(e.to_string()))?;

        // Extract metadata from Zarr attributes
        let metadata = Self::extract_metadata(&array)?;

        // Create chunk cache
        let chunk_cache = Arc::new(RwLock::new(
            ChunkCache::new(config.chunk_cache_size_mb * 1024 * 1024)
        ));

        Ok(Self {
            array,
            metadata,
            chunk_cache,
            config,
        })
    }

    /// Calculate which chunks intersect a bounding box
    fn chunks_for_bbox(&self, bbox: &BoundingBox) -> Vec<(usize, usize)> {
        let grid_bbox = &self.metadata.bbox;
        let (grid_width, grid_height) = self.metadata.shape;
        let (chunk_w, chunk_h) = self.metadata.chunk_shape;

        // Calculate grid resolution
        let lon_per_cell = (grid_bbox.max_lon - grid_bbox.min_lon) / grid_width as f64;
        let lat_per_cell = (grid_bbox.max_lat - grid_bbox.min_lat) / grid_height as f64;

        // Convert bbox to grid indices
        let min_col = ((bbox.min_lon - grid_bbox.min_lon) / lon_per_cell).floor().max(0.0) as usize;
        let max_col = ((bbox.max_lon - grid_bbox.min_lon) / lon_per_cell).ceil().min(grid_width as f64) as usize;
        let min_row = ((grid_bbox.max_lat - bbox.max_lat) / lat_per_cell).floor().max(0.0) as usize;
        let max_row = ((grid_bbox.max_lat - bbox.min_lat) / lat_per_cell).ceil().min(grid_height as f64) as usize;

        // Convert grid indices to chunk indices
        let min_chunk_x = min_col / chunk_w;
        let max_chunk_x = (max_col + chunk_w - 1) / chunk_w;
        let min_chunk_y = min_row / chunk_h;
        let max_chunk_y = (max_row + chunk_h - 1) / chunk_h;

        // Generate list of chunk coordinates
        (min_chunk_y..max_chunk_y)
            .flat_map(|cy| (min_chunk_x..max_chunk_x).map(move |cx| (cx, cy)))
            .collect()
    }

    /// Read and decompress a single chunk
    async fn read_chunk(&self, chunk_x: usize, chunk_y: usize) -> Result<Vec<f32>, GridProcessorError> {
        let cache_key = (chunk_x, chunk_y);

        // Check cache first
        {
            let cache = self.chunk_cache.read().await;
            if let Some(data) = cache.get(&cache_key) {
                return Ok(data.clone());
            }
        }

        // Cache miss - read from Zarr
        let (chunk_w, chunk_h) = self.metadata.chunk_shape;
        let start = [chunk_y * chunk_h, chunk_x * chunk_w];
        let shape = [chunk_h, chunk_w];

        let subset = ArraySubset::new_with_start_shape(
            start.map(|x| x as u64).to_vec(),
            shape.map(|x| x as u64).to_vec(),
        )?;

        let data: Vec<f32> = self.array
            .async_retrieve_array_subset_elements(&subset)
            .await
            .map_err(|e| GridProcessorError::ReadFailed(e.to_string()))?;

        // Cache the result
        {
            let mut cache = self.chunk_cache.write().await;
            cache.insert(cache_key, data.clone());
        }

        Ok(data)
    }
}

#[async_trait]
impl<S: AsyncReadableStorage + Send + Sync + 'static> GridProcessor for ZarrGridProcessor<S> {
    async fn read_region(&self, bbox: &BoundingBox) -> Result<GridRegion, GridProcessorError> {
        // 1. Calculate needed chunks
        let chunks = self.chunks_for_bbox(bbox);
        
        tracing::debug!(
            bbox = ?bbox,
            chunks = ?chunks,
            "Reading region from {} chunks",
            chunks.len()
        );

        // 2. Read all needed chunks (potentially in parallel)
        let chunk_data: Vec<_> = futures::future::try_join_all(
            chunks.iter().map(|(cx, cy)| self.read_chunk(*cx, *cy))
        ).await?;

        // 3. Assemble chunks into contiguous region
        let region = self.assemble_region(bbox, &chunks, &chunk_data)?;

        Ok(region)
    }

    async fn read_point(&self, lon: f64, lat: f64) -> Result<Option<f32>, GridProcessorError> {
        // Calculate grid indices
        let grid_bbox = &self.metadata.bbox;
        let (grid_width, grid_height) = self.metadata.shape;

        let lon_per_cell = (grid_bbox.max_lon - grid_bbox.min_lon) / grid_width as f64;
        let lat_per_cell = (grid_bbox.max_lat - grid_bbox.min_lat) / grid_height as f64;

        let col = ((lon - grid_bbox.min_lon) / lon_per_cell).floor() as usize;
        let row = ((grid_bbox.max_lat - lat) / lat_per_cell).floor() as usize;

        if col >= grid_width || row >= grid_height {
            return Ok(None);
        }

        // Calculate chunk
        let (chunk_w, chunk_h) = self.metadata.chunk_shape;
        let chunk_x = col / chunk_w;
        let chunk_y = row / chunk_h;

        // Read chunk
        let chunk_data = self.read_chunk(chunk_x, chunk_y).await?;

        // Extract value
        let local_col = col % chunk_w;
        let local_row = row % chunk_h;
        let idx = local_row * chunk_w + local_col;

        let value = chunk_data.get(idx).copied().unwrap_or(f32::NAN);

        if value.is_nan() || value == self.metadata.fill_value {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    fn metadata(&self) -> &GridMetadata {
        &self.metadata
    }

    async fn prefetch(&self, bboxes: &[BoundingBox]) {
        for bbox in bboxes {
            let chunks = self.chunks_for_bbox(bbox);
            for (cx, cy) in chunks {
                // Fire and forget - errors are logged but not propagated
                let _ = self.read_chunk(cx, cy).await;
            }
        }
    }

    fn cache_stats(&self) -> CacheStats {
        CacheStats::default()
    }
}
```

### 4.6 ChunkCache (`cache/chunk_cache.rs`)

```rust
use lru::LruCache;
use std::num::NonZeroUsize;

/// Cache key for chunks
pub type ChunkKey = (usize, usize);  // (chunk_x, chunk_y)

/// LRU cache for decompressed chunks
pub struct ChunkCache {
    cache: LruCache<ChunkKey, Vec<f32>>,
    memory_limit: usize,
    current_memory: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl ChunkCache {
    pub fn new(memory_limit: usize) -> Self {
        // Estimate max entries (assuming ~1MB per chunk)
        let max_entries = (memory_limit / (512 * 512 * 4)).max(16);
        
        Self {
            cache: LruCache::new(NonZeroUsize::new(max_entries).unwrap()),
            memory_limit,
            current_memory: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    pub fn get(&mut self, key: &ChunkKey) -> Option<&Vec<f32>> {
        if self.cache.get(key).is_some() {
            self.hits += 1;
            self.cache.get(key)
        } else {
            self.misses += 1;
            None
        }
    }

    pub fn insert(&mut self, key: ChunkKey, data: Vec<f32>) {
        let data_size = data.len() * 4;

        // Evict if necessary to make room
        while self.current_memory + data_size > self.memory_limit && !self.cache.is_empty() {
            if let Some((_, evicted)) = self.cache.pop_lru() {
                self.current_memory -= evicted.len() * 4;
                self.evictions += 1;
            }
        }

        self.cache.put(key, data);
        self.current_memory += data_size;
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.cache.len(),
            memory_bytes: self.current_memory as u64,
            evictions: self.evictions,
        }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.current_memory = 0;
    }

    /// Evict entries to reach target memory usage
    pub fn evict_to_target(&mut self, target_bytes: usize) -> usize {
        let mut evicted = 0;
        while self.current_memory > target_bytes && !self.cache.is_empty() {
            if let Some((_, data)) = self.cache.pop_lru() {
                self.current_memory -= data.len() * 4;
                self.evictions += 1;
                evicted += 1;
            }
        }
        evicted
    }
}
```

---

## 5. Ingestion Pipeline Changes

### 5.1 Overview

The ingestion pipeline will be modified to:
1. Parse GRIB2/NetCDF as today (no changes to parsers)
2. **NEW**: Re-project non-geographic grids to geographic coordinates
3. **NEW**: Write data in Zarr format instead of storing raw GRIB2
4. Update catalog with new storage path

### 5.2 Modified Ingestion Flow

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        MODIFIED INGESTER                                 │
│                                                                          │
│  1. Download GRIB2/NetCDF from NOAA (unchanged)                         │
│                     │                                                    │
│                     ▼                                                    │
│  2. Parse with existing parsers (unchanged)                             │
│     - grib2-parser for GRIB2                                            │
│     - netcdf-parser for NetCDF                                          │
│                     │                                                    │
│                     ▼                                                    │
│  3. [NEW] Check projection                                              │
│     - If geographic (GFS, MRMS): pass through                           │
│     - If Lambert (HRRR): re-project to geographic                       │
│     - If Geostationary (GOES): re-project to geographic                 │
│                     │                                                    │
│                     ▼                                                    │
│  4. [NEW] Write Zarr format                                             │
│     - Chunk into 512×512 blocks                                         │
│     - Compress with Blosc (zstd + shuffle)                              │
│     - Write sharded Zarr to MinIO                                       │
│                     │                                                    │
│                     ▼                                                    │
│  5. Register in catalog (path change only)                              │
│     - Old: shredded/gfs/.../TMP_2m/f006.grib2                           │
│     - New: grids/gfs/.../TMP_2m_f006.zarr/                              │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 5.3 Re-projection Strategy

For HRRR (Lambert Conformal) and GOES (Geostationary):

```rust
/// Re-project a grid to geographic coordinates
pub fn reproject_to_geographic(
    source_data: &[f32],
    source_width: usize,
    source_height: usize,
    source_projection: &Projection,
    interpolation: InterpolationMethod,
) -> Result<(Vec<f32>, usize, usize, BoundingBox), ProjectionError> {
    // 1. Calculate output grid dimensions to match source resolution
    let source_resolution = source_projection.native_resolution();
    
    // 2. Create output grid covering the same area
    let output_bbox = source_projection.geographic_bounds();
    let output_width = ((output_bbox.width()) / source_resolution.0).ceil() as usize;
    let output_height = ((output_bbox.height()) / source_resolution.1).ceil() as usize;
    
    // 3. For each output cell, find corresponding source cell
    let mut output = vec![f32::NAN; output_width * output_height];
    
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            let lon = output_bbox.min_lon + (out_x as f64 + 0.5) * source_resolution.0;
            let lat = output_bbox.max_lat - (out_y as f64 + 0.5) * source_resolution.1;
            
            // Project to source grid coordinates
            if let Some((src_x, src_y)) = source_projection.geographic_to_grid(lon, lat) {
                let value = match interpolation {
                    InterpolationMethod::Nearest => {
                        sample_nearest(source_data, source_width, source_height, src_x, src_y)
                    }
                    InterpolationMethod::Bilinear => {
                        sample_bilinear(source_data, source_width, source_height, src_x, src_y)
                    }
                    InterpolationMethod::Cubic => {
                        sample_cubic(source_data, source_width, source_height, src_x, src_y)
                    }
                };
                output[out_y * output_width + out_x] = value;
            }
        }
    }
    
    Ok((output, output_width, output_height, output_bbox))
}
```

### 5.4 Storage Path Changes

**Before (current):**
```
shredded/gfs/20241212_00z/TMP_2m/f006.grib2
shredded/hrrr/20241212_00z/TMP_2m/f006.grib2
raw/goes16/20241212_00z/CMI_C13_...nc
```

**After (new):**
```
grids/gfs/20241212_00z/TMP_2m_f006.zarr/zarr.json
grids/gfs/20241212_00z/TMP_2m_f006.zarr/c/0/0
grids/hrrr/20241212_00z/TMP_2m_f006.zarr/zarr.json
grids/hrrr/20241212_00z/TMP_2m_f006.zarr/c/0/0
grids/goes16/20241212_00z/CMI_C13_...zarr/zarr.json
grids/goes16/20241212_00z/CMI_C13_...zarr/c/0/0
```

### 5.5 Catalog Schema Changes

The existing `datasets` table is extended to store Zarr metadata at ingestion time. This eliminates metadata fetches from MinIO entirely - the WMS-API gets everything it needs from the catalog query it already performs.

```sql
-- Existing columns (unchanged)
model, parameter, level, reference_time, forecast_hour, file_size, 
storage_path, bbox, grid_shape, created_at

-- NEW: Zarr metadata cached at ingestion time
ALTER TABLE datasets ADD COLUMN zarr_metadata JSONB;
ALTER TABLE datasets ADD COLUMN shard_index BYTEA;

-- The zarr_metadata column contains the parsed zarr.json (~500 bytes):
-- {
--   "shape": [721, 1440],
--   "chunk_shape": [512, 512],
--   "dtype": "float32",
--   "fill_value": "NaN",
--   "bbox": [0, -90, 360, 90],
--   "compression": "blosc_zstd",
--   ...
-- }

-- The shard_index column contains the binary chunk index (~100 bytes):
-- This is the index from the end of the shard file that maps
-- chunk coordinates to byte offsets.

-- Storage path now points to Zarr:
-- OLD: shredded/gfs/20241212_00z/TMP_2m/f006.grib2
-- NEW: grids/gfs/20241212_00z/TMP_2m_f006.zarr
```

**Why store metadata in PostgreSQL?**

| Approach | First Request Latency | Implementation |
|----------|----------------------|----------------|
| Fetch from MinIO on demand | +2 HTTP requests | Simple but slower |
| Cache in Redis | +1 Redis query | Extra infrastructure |
| **Store in catalog (recommended)** | +0 requests | Already querying catalog! |

Since every tile request already queries the catalog to find the `storage_path`, we can piggyback the metadata on that same query at zero additional cost. The ingester writes the metadata when it creates the catalog entry.

**Ingester writes metadata at ingestion time:**
```rust
// In ingester, after writing Zarr to MinIO:
let zarr_metadata = serde_json::json!({
    "shape": [grid_height, grid_width],
    "chunk_shape": [chunk_size, chunk_size],
    "dtype": "float32",
    "fill_value": f32::NAN,
    "bbox": [bbox.min_lon, bbox.min_lat, bbox.max_lon, bbox.max_lat],
    "compression": "blosc_zstd",
    "num_chunks": [chunks_y, chunks_x],
});

// Read the shard index from the file we just wrote
let shard_index = read_shard_index_from_file(&zarr_path)?;

// Insert into catalog with metadata
sqlx::query!(
    r#"
    INSERT INTO datasets (model, parameter, level, ..., zarr_metadata, shard_index)
    VALUES ($1, $2, $3, ..., $n, $m)
    "#,
    model, parameter, level, ..., zarr_metadata, shard_index
).execute(&pool).await?;
```

**WMS-API gets metadata from catalog query:**
```rust
// In WMS-API, the existing catalog query now returns metadata too:
let entry = catalog.find_dataset(model, parameter, forecast_hour).await?;

// entry.zarr_metadata and entry.shard_index are already populated!
// No additional MinIO requests needed.
```

---

## 6. WMS-API Integration

### 6.1 Changes Overview

The WMS-API changes are primarily in how grid data is loaded. The rendering pipeline remains largely unchanged.

### 6.2 State Changes (`services/wms-api/src/state.rs`)

```rust
// Add to AppState
pub struct AppState {
    // ... existing fields ...
    
    /// Grid processor factory for Zarr-based data access
    pub grid_processor_factory: Arc<GridProcessorFactory>,
}

/// Factory for creating grid processors.
/// 
/// METADATA STRATEGY:
/// - Metadata and shard indices come from the PostgreSQL catalog
///   (written at ingestion time) - NO MinIO fetches needed!
/// - Only chunk data is fetched from MinIO via byte-range requests.
/// - Chunks are LRU-cached with configurable memory limits.
/// 
/// Request flow:
/// 1. Catalog query returns dataset entry WITH zarr_metadata and shard_index
/// 2. Create processor using metadata from catalog (no MinIO request)
/// 3. Processor fetches only the chunks needed via byte-range requests
pub struct GridProcessorFactory {
    storage: Arc<ObjectStorage>,
    config: GridProcessorConfig,
    /// LRU cache for decompressed chunk data (large, memory-bounded)
    chunk_cache: Arc<RwLock<ChunkCache>>,
}

impl GridProcessorFactory {
    pub fn new(storage: Arc<ObjectStorage>, config: GridProcessorConfig) -> Self {
        let chunk_cache = Arc::new(RwLock::new(
            ChunkCache::new(config.chunk_cache_size_mb * 1024 * 1024)
        ));
        
        Self {
            storage,
            config,
            chunk_cache,
        }
    }

    /// Create a grid processor using metadata from the catalog entry.
    /// 
    /// The CatalogEntry already contains zarr_metadata and shard_index
    /// (written at ingestion time), so NO MinIO requests are needed
    /// to create the processor.
    pub fn create_processor(&self, entry: &CatalogEntry) -> Result<Arc<dyn GridProcessor>> {
        // Parse metadata from catalog (already in memory from catalog query)
        let metadata = GridMetadata::from_json(&entry.zarr_metadata)?;
        let shard_index = ShardIndex::from_bytes(&entry.shard_index)?;
        
        // Create processor with metadata from catalog and shared chunk cache
        Ok(Arc::new(ZarrGridProcessor::new(
            self.storage.clone(),
            entry.storage_path.clone(),
            Arc::new(metadata),
            Arc::new(shard_index),
            self.chunk_cache.clone(),
        )))
    }
    
    /// Get chunk cache statistics for monitoring
    pub async fn cache_stats(&self) -> ChunkCacheStats {
        self.chunk_cache.read().await.stats()
    }
}
```

**Why metadata comes from the catalog, not MinIO:**

| Approach | Requests per Tile | Latency |
|----------|-------------------|---------|
| Fetch metadata from MinIO | 2-3 HTTP requests | +10-50ms |
| Cache metadata in memory | 1st request slower | +0-50ms |
| **Metadata in catalog (recommended)** | 0 extra requests | +0ms |

Since we already query the catalog to find the dataset, including `zarr_metadata` and `shard_index` in that query is essentially free. The ingester writes this data when creating the catalog entry, so the WMS-API never needs to fetch it from MinIO.

**CatalogEntry with embedded metadata:**
```rust
pub struct CatalogEntry {
    // Existing fields
    pub id: i64,
    pub model: String,
    pub parameter: String,
    pub level: String,
    pub reference_time: DateTime<Utc>,
    pub forecast_hour: u32,
    pub storage_path: String,
    pub bbox: BoundingBox,
    pub grid_shape: (usize, usize),
    pub file_size: u64,
    
    // NEW: Zarr metadata embedded in catalog
    pub zarr_metadata: serde_json::Value,  // ~500 bytes
    pub shard_index: Vec<u8>,              // ~100 bytes
}
```

**Request flow (zero metadata overhead):**
```
1. WMS Request arrives
2. Query catalog: SELECT * FROM datasets WHERE model=$1 AND parameter=$2 ...
   → Returns CatalogEntry with zarr_metadata and shard_index included
3. Create processor: factory.create_processor(&entry)
   → Uses metadata from entry, no MinIO request
4. Fetch chunks: processor.read_region(&bbox)
   → Only now do we hit MinIO, with byte-range requests for specific chunks
```

### 6.3 Rendering Changes (`services/wms-api/src/rendering.rs`)

Replace `load_grid_data()` with new implementation:

```rust
/// Load grid data for a specific region using the GridProcessor
async fn load_grid_region(
    state: &AppState,
    entry: &CatalogEntry,
    bbox: &BoundingBox,
) -> Result<GridRegion, RenderError> {
    // Get or create processor for this dataset
    let processor = state.grid_processor_factory
        .get_processor(&entry.storage_path)
        .await
        .map_err(|e| RenderError::DataLoadFailed(e.to_string()))?;
    
    // Read just the region we need
    let region = processor.read_region(bbox)
        .await
        .map_err(|e| RenderError::DataLoadFailed(e.to_string()))?;
    
    Ok(region)
}

/// Updated tile rendering function
async fn render_tile_with_zarr(
    state: &AppState,
    entry: &CatalogEntry,
    tile_bbox: &BoundingBox,
    style: &Style,
    output_size: (usize, usize),
) -> Result<Vec<u8>, RenderError> {
    // 1. Load only the grid region we need
    let region = load_grid_region(state, entry, tile_bbox).await?;
    
    // 2. Resample to output size (existing logic)
    let resampled = resample_grid(
        &region.data,
        region.width,
        region.height,
        output_size.0,
        output_size.1,
        state.config.interpolation,
    );
    
    // 3. Apply color scale (existing logic)
    let rgba = apply_style(&resampled, style);
    
    // 4. Encode PNG (existing logic)
    let png = encode_png(&rgba, output_size.0, output_size.1)?;
    
    Ok(png)
}
```

### 6.4 GetFeatureInfo Changes

```rust
/// Handle GetFeatureInfo requests
async fn get_feature_info(
    state: &AppState,
    entry: &CatalogEntry,
    lon: f64,
    lat: f64,
) -> Result<Option<FeatureInfo>, RenderError> {
    let processor = state.grid_processor_factory
        .get_processor(&entry.storage_path)
        .await?;
    
    // Read single point - efficient with chunk cache
    let value = processor.read_point(lon, lat).await?;
    
    Ok(value.map(|v| FeatureInfo {
        value: v,
        units: processor.metadata().units.clone(),
        parameter: processor.metadata().parameter.clone(),
        // ... other fields
    }))
}
```

---

## 7. Configuration & Environment Variables

### 7.1 New Environment Variables

```bash
# ============================================================================
# GRID PROCESSOR CONFIGURATION (NEW)
# ============================================================================

# --- Chunk Cache ---
CHUNK_CACHE_SIZE_MB=1024           # Memory budget for decompressed chunks (default: 1024)
                                   # Recommendation: 25-50% of available RAM after other caches

# --- Zarr Format Options ---
ZARR_CHUNK_SIZE=512                # Chunk dimension in grid points (default: 512)
                                   # Smaller = more granular access, more overhead
                                   # Larger = less overhead, coarser access
                                   # NOTE: Does NOT need to be a power of 2 (see below)

ZARR_COMPRESSION=blosc_zstd        # Compression codec (default: blosc_zstd)
                                   # Options: "blosc_zstd", "blosc_lz4", "zstd", "lz4", "none"

ZARR_COMPRESSION_LEVEL=1           # Compression level 1-9 (default: 1)
                                   # Higher = better compression, slower

ZARR_SHUFFLE=true                  # Enable byte shuffle filter (default: true)
                                   # Highly recommended for f32 data

# --- Projection / Interpolation ---
GRID_INTERPOLATION=bilinear        # Interpolation method (default: bilinear)
                                   # Options: "nearest", "bilinear", "cubic"
                                   # "nearest" preserves exact values
                                   # "bilinear" is smooth and accurate within ±0.1° tolerance

# --- Metadata Cache ---
# NOTE: Zarr metadata (zarr.json + shard indices) is cached PERMANENTLY
# in memory since it's tiny (~600 bytes per grid). This eliminates
# metadata fetches after the first request for each grid.
# No configuration needed - all metadata is always cached.
```

#### Chunk Size Flexibility

**Zarr V3 does NOT require chunk sizes to be powers of 2.** Any positive integer is valid. The `zarrs` crate follows the Zarr specification without additional constraints.

| Chunk Size | Uncompressed (f32) | Valid? | Notes |
|------------|-------------------|--------|-------|
| `512×512` | 1 MB | ✓ | Default, good balance |
| `500×500` | ~1 MB | ✓ | Works fine |
| `721×1440` | ~4 MB | ✓ | Entire GFS grid as one chunk |
| `1000×1000` | ~4 MB | ✓ | Larger chunks, fewer fetches |
| `256×256` | 256 KB | ✓ | Finer granularity |
| `100×100` | 40 KB | ✓ | Valid but small (more I/O overhead) |

**Chunk size selection guidelines:**
- **Target ~1MB+ uncompressed** - Amortizes I/O overhead
- **Match access patterns** - Chunk along dimensions you'll slice
- **No power-of-2 requirement** - Choose what makes sense for your data
- **Compression efficiency** - Larger chunks generally compress better

### 7.2 Updated `.env.example`

```bash
# ============================================================================
# Weather WMS - Environment Configuration
# ============================================================================

# ... existing sections ...

# ============================================================================
# GRID PROCESSOR (Zarr-based data access)
# ============================================================================
# Configuration for the chunked grid data access layer

# Chunk cache holds decompressed grid chunks in memory
# This is SEPARATE from tile caches - it caches raw grid data
CHUNK_CACHE_SIZE_MB=1024

# Zarr format settings (used during ingestion)
ZARR_CHUNK_SIZE=512                # Grid points per chunk dimension
ZARR_COMPRESSION=blosc_zstd        # blosc_zstd, blosc_lz4, zstd, lz4, none
ZARR_COMPRESSION_LEVEL=1           # 1-9 (1=fastest, 9=smallest)
ZARR_SHUFFLE=true                  # Byte shuffle for better compression

# Interpolation for projection conversion and GetFeatureInfo
GRID_INTERPOLATION=bilinear        # nearest, bilinear, cubic

# NOTE: Zarr metadata (zarr.json + shard indices) is stored in the PostgreSQL
# catalog at ingestion time. No separate metadata cache configuration needed -
# it comes "for free" with the catalog query.
```

### 7.3 Memory Budget Guidelines

```
Total RAM: 16 GB
├── OS + Base processes: ~2 GB
├── PostgreSQL: ~1 GB (includes zarr_metadata, ~600 bytes/grid)
├── Redis: ~1 GB
├── WMS-API process:
│   ├── L1 Tile Cache (TILE_CACHE_SIZE=10000): ~300 MB
│   ├── Chunk Cache (CHUNK_CACHE_SIZE_MB=4096): ~4 GB
│   └── Working memory: ~1 GB
├── Ingester process: ~2 GB
└── Buffer/headroom: ~4 GB

Recommendation for 16GB system:
  CHUNK_CACHE_SIZE_MB=4096

Note: Metadata caching is automatic via PostgreSQL catalog.
      No separate configuration needed.
```

---

## 8. Migration Strategy

### 8.1 Overview

This is a **hard cutover** migration. The system will be shut down, data re-ingested in the new format, and brought back up.

### 8.2 Pre-Migration Checklist

- [ ] All code changes merged and tested
- [ ] New environment variables documented
- [ ] Backup current MinIO data (optional, for rollback)
- [ ] Notify users of planned downtime
- [ ] Prepare monitoring dashboards for new metrics

### 8.3 Migration Steps

```
Day 0: Preparation
├── 1. Tag current release as "pre-zarr-migration"
├── 2. Document current MinIO storage usage
└── 3. Test migration process in staging environment

Day 1: Migration
├── 1. Stop all services
│      docker-compose down
│
├── 2. Clear MinIO grid data (keep configs)
│      mc rm --recursive --force minio/weather-data/shredded/
│      mc rm --recursive --force minio/weather-data/raw/
│
├── 3. Clear PostgreSQL catalog
│      psql -c "TRUNCATE TABLE datasets CASCADE;"
│
├── 4. Clear Redis cache
│      redis-cli FLUSHALL
│
├── 5. Deploy new code
│      git pull
│      docker-compose build
│
├── 6. Update .env with new variables
│      CHUNK_CACHE_SIZE_MB=1024
│      ZARR_CHUNK_SIZE=512
│      ZARR_COMPRESSION=blosc_zstd
│      GRID_INTERPOLATION=bilinear
│
├── 7. Start infrastructure services
│      docker-compose up -d postgres redis minio
│
├── 8. Run ingestion for GFS (initial model)
│      docker-compose run ingester --model gfs --cycles 0,6,12,18 --hours 0-15
│
├── 9. Verify data in MinIO
│      mc ls minio/weather-data/grids/gfs/
│
├── 10. Start WMS-API and verify
│       docker-compose up -d wms-api
│       curl http://localhost:8080/wms?service=WMS&request=GetCapabilities
│
├── 11. Run validation tests
│       cargo run --package wms-validation
│
└── 12. Start remaining services
        docker-compose up -d
```

### 8.4 Rollback Plan

If critical issues are discovered:

1. Stop services: `docker-compose down`
2. Checkout pre-migration tag: `git checkout pre-zarr-migration`
3. Restore `.env` to previous version
4. Rebuild: `docker-compose build`
5. Restore MinIO data from backup (if available)
6. Or re-ingest in old format
7. Start services: `docker-compose up -d`

### 8.5 Post-Migration Validation

```bash
# 1. Check capabilities
curl "http://localhost:8080/wms?service=WMS&request=GetCapabilities" | grep gfs

# 2. Request a tile
curl -o test.png "http://localhost:8080/wmts/rest/gfs_TMP/temperature/default/WebMercatorQuad/4/3/5.png"

# 3. Check GetFeatureInfo
curl "http://localhost:8080/wms?service=WMS&request=GetFeatureInfo&layers=gfs_TMP&query_layers=gfs_TMP&x=128&y=128&width=256&height=256&bbox=-100,30,-90,40&info_format=application/json"

# 4. Check metrics
curl http://localhost:8080/api/metrics | jq '.chunk_cache'

# 5. Run load test
cargo run --package load-test -- --scenario gfs_temporal --duration 60
```

---

## 9. Testing Plan

### 9.1 Unit Tests

**`crates/grid-processor/src/`:**

```rust
#[cfg(test)]
mod tests {
    // Test BoundingBox operations
    #[test]
    fn test_bbox_intersects() { ... }
    
    // Test chunk calculation
    #[test]
    fn test_chunks_for_bbox_single_chunk() { ... }
    
    #[test]
    fn test_chunks_for_bbox_multiple_chunks() { ... }
    
    #[test]
    fn test_chunks_for_bbox_edge_cases() { ... }
    
    // Test interpolation
    #[test]
    fn test_bilinear_interpolation() { ... }
    
    #[test]
    fn test_nearest_interpolation_preserves_values() { ... }
    
    // Test chunk cache
    #[test]
    fn test_chunk_cache_lru_eviction() { ... }
    
    #[test]
    fn test_chunk_cache_memory_limit() { ... }
}
```

### 9.2 Integration Tests

**`crates/grid-processor/tests/`:**

```rust
#[tokio::test]
async fn test_zarr_roundtrip() {
    // 1. Create test grid data
    let data = generate_test_grid(1440, 721);
    
    // 2. Write to Zarr
    let temp_dir = tempdir()?;
    ZarrWriter::write(&temp_dir, "test.zarr", &data, 1440, 721, &config).await?;
    
    // 3. Read back via GridProcessor
    let processor = ZarrGridProcessor::open(&temp_dir, "test.zarr", config).await?;
    
    // 4. Verify full grid read
    let region = processor.read_region(&full_bbox).await?;
    assert_eq!(region.data.len(), data.len());
    
    // 5. Verify partial read
    let small_bbox = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
    let partial = processor.read_region(&small_bbox).await?;
    assert!(partial.data.len() < data.len());
}

#[tokio::test]
async fn test_read_point_accuracy() {
    // Verify GetFeatureInfo accuracy within tolerance
    let processor = create_test_processor().await;
    
    for (lon, lat, expected) in test_points {
        let value = processor.read_point(lon, lat).await?.unwrap();
        assert!((value - expected).abs() < 0.1, 
            "Value at ({}, {}) was {}, expected {} (±0.1)",
            lon, lat, value, expected);
    }
}
```

### 9.3 WMS Validation Tests

**`validation/wms-validation/`:**

```yaml
# Test tile rendering with Zarr backend
tests:
  - name: gfs_temperature_tile
    request:
      type: wmts
      layer: gfs_TMP
      style: temperature
      z: 4
      x: 3
      y: 5
    expect:
      status: 200
      content_type: image/png
      dimensions: [256, 256]
      
  - name: gfs_getfeatureinfo
    request:
      type: wms_getfeatureinfo
      layers: gfs_TMP
      lon: -95.0
      lat: 35.0
    expect:
      status: 200
      value_range: [250, 320]  # Valid temperature range in K
```

### 9.4 Performance Tests

**New load test scenarios:**

```yaml
# validation/load-test/scenarios/zarr_chunk_efficiency.yaml
name: Zarr Chunk Efficiency Test
description: Verify that chunk caching improves performance for adjacent tiles

phases:
  - name: cold_cache
    duration: 30s
    requests:
      - type: wmts_tile
        layer: gfs_TMP
        z: 6
        pattern: random
    metrics:
      - chunk_cache_hits
      - chunk_cache_misses
      - request_latency_p99
      
  - name: warm_cache
    duration: 60s
    requests:
      - type: wmts_tile
        layer: gfs_TMP
        z: 6
        pattern: clustered  # Adjacent tiles
    expect:
      chunk_cache_hit_rate: ">80%"
      request_latency_p99: "<50ms"
```

### 9.5 Accuracy Validation

```rust
/// Validate that Zarr data matches original GRIB2 within tolerance
#[test]
fn test_data_accuracy_vs_original() {
    // 1. Load original GRIB2
    let grib_data = load_grib2("testdata/gfs.grib2");
    
    // 2. Convert to Zarr
    let zarr_data = convert_to_zarr(&grib_data);
    
    // 3. Compare all values
    for (i, (orig, zarr)) in grib_data.iter().zip(zarr_data.iter()).enumerate() {
        if orig.is_nan() && zarr.is_nan() {
            continue;
        }
        let diff = (orig - zarr).abs();
        assert!(diff < 0.001, 
            "Value mismatch at index {}: original={}, zarr={}, diff={}",
            i, orig, zarr, diff);
    }
}

/// Validate GetFeatureInfo against known reference values
#[test]
fn test_getfeatureinfo_accuracy() {
    // Test points with known values from authoritative source
    let test_cases = vec![
        // (lon, lat, expected_value, tolerance)
        (-95.0, 35.0, 288.5, 0.1),  // Temperature in K
        (-100.0, 40.0, 285.2, 0.1),
    ];
    
    for (lon, lat, expected, tolerance) in test_cases {
        let value = processor.read_point(lon, lat).await?.unwrap();
        assert!((value - expected).abs() <= tolerance,
            "GetFeatureInfo at ({}, {}) returned {}, expected {} ±{}",
            lon, lat, value, expected, tolerance);
    }
}
```

---

## 10. Future Considerations

### 10.1 Additional Models

After GFS is working, extend to other models:

| Model | Projection | Notes |
|-------|------------|-------|
| **HRRR** | Lambert Conformal | Re-project to geographic, ~3km resolution |
| **GOES** | Geostationary | Re-project to geographic, preserve resolution |
| **MRMS** | Geographic | Direct chunking, very large grid (7000×3500) |

### 10.2 Time Dimension

Zarr naturally supports N-dimensional arrays. Future enhancement:

```
Current: grids/gfs/20241212_00z/TMP_2m_f000.zarr
         grids/gfs/20241212_00z/TMP_2m_f003.zarr
         grids/gfs/20241212_00z/TMP_2m_f006.zarr

Future:  grids/gfs/20241212_00z/TMP_2m.zarr  (with time dimension)
         shape: [41, 721, 1440]  # 41 forecast hours × lat × lon
```

Benefits:
- Single file per parameter per model run
- Efficient time-series queries
- Better for animations

### 10.3 Additional OGC Services

The `GridProcessor` abstraction enables:

- **EDR Environmental Data Retrieval - raw data returned for various geometries returned in non-image formats

### 10.4 Monitoring Enhancements

New metrics to track:

```rust
// Chunk cache metrics
chunk_cache_hits_total
chunk_cache_misses_total
chunk_cache_memory_bytes
chunk_cache_evictions_total

// Zarr access metrics
zarr_bytes_read_total
zarr_chunks_read_total
zarr_read_latency_seconds

// Per-model metrics
gfs_chunk_cache_hit_rate
hrrr_chunk_cache_hit_rate
```

---

## 11. Risk Assessment

### 11.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| `zarrs` crate bugs | Low | High | Pin version, test thoroughly, have fallback plan |
| Re-projection accuracy | Medium | Medium | Validate against reference data, document tolerance |
| Memory pressure from chunk cache | Medium | Medium | Implement eviction, integrate with existing memory management |
| MinIO byte-range performance | Low | Medium | Test with production-like data volumes |
| Migration takes longer than expected | Medium | Low | Plan for extended downtime window |

### 11.2 Mitigation Strategies

**For `zarrs` crate issues:**
- Pin to specific version in Cargo.toml
- Write comprehensive tests against edge cases
- Keep original GRIB2 parsing code (but don't use in production)

**For accuracy concerns:**
- Implement validation suite comparing Zarr output to original
- Document interpolation method and expected tolerance
- Provide "nearest neighbor" option for exact value preservation

**For memory issues:**
- Chunk cache has hard memory limit
- Integrate with existing memory pressure management
- Add metrics for early warning

---

## 12. Implementation Phases

### Phase 1: Core Grid Processor (Week 1)

**Goal:** Create the `grid-processor` crate with Zarr reading capability.

**Tasks:**
1. Create crate structure and Cargo.toml
2. Implement core types (BoundingBox, GridRegion, GridMetadata)
3. Implement GridProcessor trait
4. Implement ZarrGridProcessor with `zarrs` crate
5. Implement ChunkCache
6. Unit tests for all components

**Deliverable:** Working `grid-processor` crate that can read Zarr files.

### Phase 2: Zarr Writer & Ingestion (Week 2)

**Goal:** Enable ingestion pipeline to output Zarr format.

**Tasks:**
1. Implement ZarrWriter
2. Add re-projection utilities (for future HRRR/GOES)
3. Modify ingester to call ZarrWriter after parsing
4. Update storage paths in catalog registration
5. Integration tests for ingestion → Zarr → read cycle

**Deliverable:** Ingester that produces Zarr files for GFS.

### Phase 3: WMS-API Integration (Week 3)

**Goal:** Connect WMS-API to use GridProcessor for data access.

**Tasks:**
1. Add GridProcessorFactory to AppState
2. Replace `load_grid_data()` with GridProcessor calls
3. Update GetFeatureInfo handler
4. Add new environment variables
5. Add chunk cache metrics
6. End-to-end tests

**Deliverable:** WMS-API serving tiles from Zarr data.

### Phase 4: Migration & Validation (Week 4)

**Goal:** Migrate production system to new format.

**Tasks:**
1. Update documentation
2. Prepare migration scripts
3. Execute migration on staging
4. Run full validation suite
5. Execute production migration
6. Monitor and tune

**Deliverable:** Production system running on Zarr format.

---

## Summary

This plan transforms Weather WMS from loading entire grid files to efficiently fetching only the chunks needed for each request. Key points:

1. **Zarr V3 with sharding** provides industry-standard, single-file chunked storage
2. **GridProcessor abstraction** cleanly separates data access from rendering
3. **ChunkCache** enables efficient reuse across adjacent tile requests
4. **Pre-projection to geographic** simplifies rendering at the cost of ingestion time
5. **Hard cutover migration** ensures clean transition without legacy format support

Expected outcomes:
- **10-100x reduction** in data transferred per request
- **Faster cold-cache response times** (fetch 1-4 chunks instead of entire file)
- **Better cache efficiency** (chunks shared across tiles)
- **Future-proof architecture** for additional OGC services

---

## Appendix A: Key Decisions Summary

| Decision | Choice | Rationale |
|----------|--------|-----------|
| File format | Zarr V3 with sharding | Industry standard, excellent Rust support, single-file storage |
| Chunk size | 512×512 | Balance between granularity and overhead, tunable later |
| Compression | Blosc + Zstd + Shuffle | Best compression ratio for f32 data with fast decompression |
| Projection strategy | Pre-project to geographic | Simpler WMS rendering, acceptable accuracy tradeoff |
| Interpolation | Bilinear (configurable) | Smooth results within ±0.1° tolerance |
| Chunk metadata | In-file (cached by processor) | Self-contained files, no database schema changes |
| Migration | Hard cutover | Clean transition, no legacy format complexity |

---

## Appendix B: Related Documentation

- `docs/MRMS_RENDERING_PERFORMANCE_ANALYSIS.md` - Current MRMS bottleneck analysis
- `docs/GOES_RENDERING_PERFORMANCE_ANALYSIS.md` - Current GOES bottleneck analysis
- `docs/INGESTION.md` - Current ingestion pipeline documentation
- `docs/src/architecture/README.md` - System architecture overview

---

*End of Plan*
