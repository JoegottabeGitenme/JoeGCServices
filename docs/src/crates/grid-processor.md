# grid-processor

The grid-processor crate provides efficient access to chunked gridded weather data using the Zarr V3 format with multi-resolution pyramid support. It handles both reading (for rendering) and writing (during ingestion) of weather data.

## Overview

**Location**: `crates/grid-processor/`  
**Dependencies**: `zarrs`, `zarrs-filesystem`, `object_store`, `ndarray`  
**LOC**: ~4,500

## Key Features

- **Zarr V3 Format**: Industry-standard chunked array format with sharding
- **Multi-Resolution Pyramids**: Pre-computed downsampled levels for fast zoom-out rendering
- **Partial Reads**: Only fetch the chunks needed for a tile request (byte-range requests)
- **LRU Chunk Cache**: In-memory caching of decompressed chunks
- **Configurable Compression**: Blosc with LZ4/Zstd, chunk sizes, etc.
- **Unified Query API**: `DatasetQuery` builder for finding datasets by model/parameter/time/level

## API Levels

The crate provides three levels of abstraction:

### High-Level: `GridDataService`

The recommended interface for most use cases. Combines catalog queries with data access:

```rust
use grid_processor::{GridDataService, DatasetQuery, BoundingBox};

// Create service at application startup
let service = GridDataService::new(catalog, minio_config, 1024); // 1GB cache

// Query for a forecast dataset using the builder pattern
let query = DatasetQuery::forecast("gfs", "TMP")
    .at_level("2 m above ground")
    .at_forecast_hour(6);

// Read a region (for tile rendering)
let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
let region = service.read_region(&query, &bbox, Some((256, 256))).await?;

// Query a single point (for GetFeatureInfo or EDR Position)
let value = service.read_point(&query, -95.0, 35.0).await?;
println!("Temperature: {} {}", value.value.unwrap_or(f32::NAN), value.units);
```

### Mid-Level: `GridProcessorFactory`

For cases where you have a catalog entry and want direct control:

```rust
use grid_processor::{GridProcessorFactory, MinioConfig, ZarrGridProcessor};

// Factory manages shared chunk cache
let factory = GridProcessorFactory::new(MinioConfig::from_env(), 1024);

// Create storage and processor manually
let store = create_minio_storage(factory.minio_config())?;
let processor = ZarrGridProcessor::with_metadata(
    store, 
    &zarr_path,
    grid_metadata,
    factory.chunk_cache(),
    factory.config().clone(),
)?;

let region = processor.read_region(&bbox).await?;
```

### Low-Level: `GridProcessor` Trait

For custom implementations or direct Zarr access:

```rust
use grid_processor::{GridProcessor, ZarrGridProcessor, GridProcessorConfig};

let processor = ZarrGridProcessor::open(store, "/path/to/array.zarr", config)?;
let region = processor.read_region(&bbox).await?;
let point = processor.read_point(-95.0, 35.0).await?;
```

## DatasetQuery Builder

The `DatasetQuery` type provides a fluent API for specifying which dataset to access:

```rust
use grid_processor::{DatasetQuery, TimeSpecification};
use chrono::Utc;

// Forecast model query
let query = DatasetQuery::forecast("gfs", "TMP")
    .at_level("2 m above ground")
    .at_forecast_hour(6)
    .at_run(Utc::now());  // Optional: specific model run

// Observation data query (GOES, MRMS)
let query = DatasetQuery::observation("goes18", "CMI_C13")
    .at_time(Utc::now());

// Get latest available data
let query = DatasetQuery::forecast("hrrr", "REFC")
    .at_level("entire atmosphere")
    .latest();
```

## Architecture

```
WMS Request (tile x, y, z)
     │
     ▼
GridProcessor::read_region(bbox)
     │
     ├─► Select pyramid level based on zoom
     │
     ├─► Calculate needed chunks (O(1) arithmetic)
     │
     ├─► Check ChunkCache for each chunk
     │         │
     │         ├─► Cache hit: return decompressed data
     │         │
     │         └─► Cache miss: fetch via S3 byte-range request
     │                  │
     │                  └─► Decompress (Blosc LZ4)
     │                  │
     │                  └─► Cache result
     │
     └─► Assemble chunks into GridRegion
              │
              ▼
         Return to renderer for resampling/styling
```

## Components

### ZarrWriter

Writes grid data during ingestion:

```rust
use grid_processor::{ZarrWriter, GridProcessorConfig, BoundingBox};

let config = GridProcessorConfig::default();
let writer = ZarrWriter::new(config);

// Write grid to Zarr format
let result = writer.write(
    storage,              // FilesystemStore or S3
    "/",                  // Path within store
    &grid_data,           // f32 values
    width, height,        // Grid dimensions
    &bbox,                // Geographic extent
    "gfs",                // Model
    "TMP",                // Parameter
    "2 m above ground",   // Level
    "K",                  // Units
    reference_time,       // Model run time
    forecast_hour,        // Forecast hour
)?;

println!("Wrote {} bytes", result.bytes_written);
```

### ZarrWriter with Pyramids

Generate multi-resolution pyramids for efficient rendering at all zoom levels:

```rust
use grid_processor::{ZarrWriter, PyramidConfig, DownsampleMethod};

let config = GridProcessorConfig {
    pyramid: Some(PyramidConfig {
        levels: 4,                              // 4 pyramid levels (1x, 2x, 4x, 8x reduction)
        method: DownsampleMethod::Average,      // Averaging for smooth results
        min_dimension: 256,                     // Stop when grid < 256px
    }),
    zarr_chunk_size: 512,                       // 512x512 chunks
    ..Default::default()
};

let writer = ZarrWriter::new(config);

// This writes multiple arrays: /0 (full res), /1 (2x), /2 (4x), /3 (8x)
let result = writer.write_with_pyramids(
    storage, data, width, height, &bbox,
    model, parameter, level, units, reference_time, forecast_hour,
)?;

println!("Wrote {} pyramid levels", result.levels_written.len());
```

### ZarrGridProcessor

Reads grid data for rendering with automatic pyramid level selection:

```rust
use grid_processor::{ZarrGridProcessor, BoundingBox, GridProcessorConfig};

// Open a Zarr grid (auto-detects pyramids)
let processor = ZarrGridProcessor::open(
    storage,
    "grids/gfs/20241217_12z/tmp_2_m_above_ground_f003.zarr",
    config,
).await?;

// Read a region - automatically selects best pyramid level
let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
let region = processor.read_region(&bbox, Some((256, 256))).await?;

// Access the data
println!("Grid shape: {}x{}", region.width, region.height);
println!("Actual bbox: {:?}", region.bounds);
for value in &region.data {
    // Process values...
}
```

### ChunkCache

LRU cache for decompressed chunks:

```rust
use grid_processor::{ChunkCache, ChunkKey};

// Create cache with 1000 chunk capacity
let cache = ChunkCache::new(1000);

// Check cache
let key = ChunkKey {
    path: "grids/gfs/tmp.zarr/0".to_string(),
    chunk_coords: vec![0, 0],
};

if let Some(data) = cache.get(&key).await {
    // Cache hit
} else {
    // Cache miss - fetch and store
    let data = fetch_chunk(...).await?;
    cache.insert(key, data.clone()).await;
}

// Get stats
let stats = cache.stats().await;
println!("Hit rate: {:.1}%", stats.hit_rate() * 100.0);
```

## Storage Format

### Zarr V3 with Sharding

The grid-processor uses Zarr V3 with **sharding** for efficient storage. Sharding combines multiple chunks into a single file, reducing the number of I/O operations:

```
grids/gfs/2024-01-15/00/TMP_f006.zarr/
├── zarr.json                    # Root group metadata
├── 0/                           # Full resolution (level 0)
│   ├── zarr.json               # Array metadata with shard codec
│   └── c/                      
│       └── 0                   # Single sharded file containing all chunks
├── 1/                           # 2x downsampled (level 1)
│   ├── zarr.json
│   └── c/0
└── 2/                           # 4x downsampled (level 2)
    └── ...
```

**Shard configuration**: Each pyramid level is stored as a single sharded file with 512x512 internal chunks. This means:
- Fewer file operations (1 file per level vs hundreds of chunk files)
- Byte-range reads still work for individual chunks within the shard
- Better performance for cloud storage (S3/MinIO)

### Path Formats

**Forecast models** (GFS, HRRR):
```
grids/{model}/{date}/{HH}/{param}_f{fhr:03}.zarr
```
Example: `grids/gfs/2024-01-15/00/TMP_f006.zarr`

**Observation models** (MRMS, GOES):
```
grids/{model}/{date}/{HH}/{param}_{MM}.zarr
```
Example: `grids/mrms/2024-01-15/12/REFL_05.zarr` (12:05 UTC)

The minute component allows minute-level temporal resolution for radar and satellite data.

### Array Metadata (zarr.json)

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
  "fill_value": "NaN",
  "codecs": [
    {
      "name": "bytes",
      "configuration": {
        "endian": "little"
      }
    },
    {
      "name": "blosc",
      "configuration": {
        "cname": "lz4",
        "clevel": 5,
        "shuffle": "shuffle"
      }
    }
  ],
  "attributes": {
    "model": "gfs",
    "parameter": "TMP",
    "level": "2 m above ground",
    "units": "K",
    "bbox": {
      "min_lon": 0.0,
      "min_lat": -90.0,
      "max_lon": 360.0,
      "max_lat": 90.0
    }
  }
}
```

## Pyramid Level Selection

The processor automatically selects the appropriate pyramid level based on the requested output size:

```rust
// For a 256x256 tile request covering a large area:
// - If full grid is 1440x721, and tile covers 1/4 of grid
// - Effective resolution needed: ~360x180
// - Best pyramid level: 2 (4x downsampled = 360x180)

fn select_pyramid_level(
    grid_shape: (usize, usize),
    bbox: &BoundingBox,
    output_size: (usize, usize),
    available_levels: &[PyramidLevel],
) -> usize {
    // Calculate pixels per degree at each level
    // Select level with resolution closest to (but >= ) output needs
}
```

## Downsampling Methods

The downsample method determines how values are aggregated when building lower-resolution pyramid levels. Choose based on the physical meaning of the data:

```rust
pub enum DownsampleMethod {
    /// Simple averaging (good for continuous data like temperature)
    Average,
    
    /// Take maximum value (good for precipitation, reflectivity)
    Max,
    
    /// Take minimum value (good for visibility, CIN)
    Min,
    
    /// Nearest neighbor (preserves exact values, good for categorical data)
    Nearest,
    
    /// Bilinear interpolation (smooth results)
    Bilinear,
}
```

### Recommended Methods by Parameter Type

| Parameter Type | Method | Reason |
|----------------|--------|--------|
| Radar reflectivity | `Max` | Preserve storm intensity at lower zooms |
| Precipitation rate | `Max` | Show peak rainfall intensity |
| Temperature | `Average` | Smooth gradients are physically meaningful |
| Wind U/V components | `Average` | Avoid artificial peaks from averaging |
| Accumulated precip | `Average` | Total amounts matter, not peaks |
| Humidity/cloud cover | `Average` | Percentage fields should average |
| Precipitation type | `Nearest` | Categorical - no interpolation |
| Cloud type | `Nearest` | Discrete values shouldn't blend |

The method is specified per-parameter in model config:
```yaml
parameters:
  - name: REFL
    downsample: max
  - name: TMP
    downsample: mean
```

## NaN Handling

Grid data uses `NaN` (Not a Number) for missing values. This is critical for:

1. **Sentinel value conversion**: Data sources like MRMS use -999 for missing data. During ingestion, values <= -90 are converted to NaN.

2. **Pyramid generation**: NaN values propagate correctly through downsampling - a cell with any NaN input remains NaN (unless all inputs are NaN).

3. **Rendering**: The renderer skips NaN pixels, leaving them transparent.

## Configuration

```rust
pub struct GridProcessorConfig {
    /// Zarr chunk size (default: 512)
    pub zarr_chunk_size: usize,
    
    /// Compression settings
    pub compression: ZarrCompression,
    
    /// Pyramid configuration (None = no pyramids)
    pub pyramid: Option<PyramidConfig>,
    
    /// Chunk cache size in entries (default: 1000)
    pub chunk_cache_size: usize,
}

pub struct PyramidConfig {
    /// Number of pyramid levels to generate
    pub levels: usize,
    
    /// Downsampling method
    pub method: DownsampleMethod,
    
    /// Minimum grid dimension before stopping
    pub min_dimension: usize,
}

pub enum ZarrCompression {
    /// No compression
    None,
    
    /// Blosc with LZ4 (fast, good ratio) - DEFAULT
    BloscLz4 { level: u8 },
    
    /// Blosc with Zstd (slower, better ratio)
    BloscZstd { level: u8 },
}
```

## Buffer Expansion for Tile Edge Interpolation

When reading partial regions for tile rendering, the processor adds a buffer around the requested bbox to ensure smooth bilinear interpolation at tile edges:

```rust
// In read_region():
let buffer_cells = 2.0;  // 2 grid cells buffer

// Expand bbox by buffer
let buffered_bbox = BoundingBox::new(
    (bbox.min_lon - res_x * buffer_cells).max(grid_bbox.min_lon),
    (bbox.min_lat - res_y * buffer_cells).max(grid_bbox.min_lat),
    (bbox.max_lon + res_x * buffer_cells).min(grid_bbox.max_lon),
    (bbox.max_lat + res_y * buffer_cells).min(grid_bbox.max_lat),
);
```

This prevents visible discontinuities at tile boundaries when bilinear interpolation needs neighboring values.

## Performance

| Operation | Time | Notes |
|-----------|------|-------|
| Open Zarr array | ~5ms | Read metadata |
| Read single chunk | ~10-20ms | S3 byte-range + decompress |
| Read tile region (cached) | ~2ms | From chunk cache |
| Read tile region (uncached) | ~30-50ms | 1-4 chunks typical |
| Write GFS grid + pyramids | ~500ms | 1440x721 grid, 4 levels |
| Chunk cache hit rate | >90% | After warm-up |

## Error Handling

```rust
pub enum GridProcessorError {
    /// Storage I/O error
    StorageError(String),
    
    /// Invalid Zarr metadata
    InvalidMetadata(String),
    
    /// Requested region out of bounds
    OutOfBounds { requested: BoundingBox, grid: BoundingBox },
    
    /// Chunk not found
    ChunkNotFound { coords: Vec<u64> },
    
    /// Decompression failed
    DecompressionError(String),
    
    /// Configuration error
    ConfigError(String),
}
```

## Integration with Ingestion

The ingestion pipeline uses ZarrWriter to store grid data:

```rust
// In services/wms-api/src/admin.rs ingest_handler()

// 1. Parse GRIB2 and extract grid data
let grid_data = message.unpack_data()?;
let width = message.grid_definition.num_points_longitude;
let height = message.grid_definition.num_points_latitude;

// 2. Create storage path
let zarr_path = format!(
    "grids/{}/{}/{}_{}_f{:03}.zarr",
    model, run_date, param.to_lowercase(), level_sanitized, forecast_hour
);

// 3. Write to Zarr with pyramids
let writer = ZarrWriter::new(GridProcessorConfig {
    pyramid: Some(PyramidConfig {
        levels: 4,
        method: DownsampleMethod::Average,
        min_dimension: 256,
    }),
    ..Default::default()
});

let result = writer.write_with_pyramids(
    filesystem_store,
    &grid_data, width, height, &bbox,
    model, parameter, level, units,
    reference_time, forecast_hour,
)?;

// 4. Register in catalog
catalog.insert(CatalogEntry {
    storage_path: zarr_path,
    zarr_metadata: result.metadata.to_json(),
    ...
}).await?;
```

## Integration with Other OGC Services

### EDR (Environmental Data Retrieval)

The grid-processor provides the data access layer for EDR implementations. EDR is responsible for:
- Query parsing (position, area, trajectory, corridor)
- Output format conversion (CoverageJSON, GeoJSON)
- Temporal aggregation across multiple datasets

Grid-processor is responsible for:
- Finding the right dataset given query parameters
- Efficiently fetching grid data for a region or point
- Caching chunks to optimize repeated queries

#### EDR Query Type Mapping

| EDR Query Type | grid-processor Method | Post-Processing |
|----------------|----------------------|-----------------|
| Position | `read_point(lon, lat)` | Convert to CoverageJSON point |
| Area | `read_region(bbox)` | Convert GridRegion to CoverageJSON |
| Cube | Multiple `read_region()` calls | Aggregate across time/levels |
| Trajectory | Multiple `read_point()` calls | Iterate over path coordinates |
| Corridor | `read_region()` + buffer | Sample along path with width |

#### Example: EDR Position Query

```rust
use grid_processor::{GridDataService, DatasetQuery, BoundingBox};

// EDR handler receives: GET /collections/gfs/position?coords=POINT(-95 35)&parameter-name=TMP
async fn handle_edr_position(
    service: &GridDataService,
    lon: f64,
    lat: f64,
    parameter: &str,
) -> Result<CoverageJSON, Error> {
    // Build query from EDR request parameters
    let query = DatasetQuery::forecast("gfs", parameter)
        .at_level("2 m above ground")
        .latest();
    
    // Use grid-processor for data access
    let point_value = service.read_point(&query, lon, lat).await?;
    
    // EDR layer converts to CoverageJSON (not grid-processor's job)
    let coverage = CoverageJSON::from_point(
        lon, lat,
        point_value.value,
        &point_value.units,
        &point_value.reference_time,
    );
    
    Ok(coverage)
}
```

#### Example: EDR Area Query

```rust
// EDR handler: GET /collections/gfs/area?bbox=-100,30,-90,40&parameter-name=TMP
async fn handle_edr_area(
    service: &GridDataService,
    bbox: BoundingBox,
    parameter: &str,
) -> Result<CoverageJSON, Error> {
    let query = DatasetQuery::forecast("gfs", parameter)
        .at_level("2 m above ground")
        .latest();
    
    // Get grid data - no output_size since we want native resolution
    let region = service.read_region(&query, &bbox, None).await?;
    
    // Convert to CoverageJSON grid coverage
    let coverage = CoverageJSON::from_grid(
        &region.data,
        region.width,
        region.height,
        &region.bbox,
        &region.units,
    );
    
    Ok(coverage)
}
```

### WCS (Web Coverage Service)

For WCS GetCoverage requests returning raw grid data:

```rust
// WCS handler: GetCoverage request
async fn handle_wcs_getcoverage(
    service: &GridDataService,
    coverage_id: &str,  // e.g., "gfs_TMP_2m"
    bbox: BoundingBox,
    format: OutputFormat,
) -> Result<Response, Error> {
    let (model, parameter, level) = parse_coverage_id(coverage_id)?;
    
    let query = DatasetQuery::forecast(model, parameter)
        .at_level(level)
        .latest();
    
    let region = service.read_region(&query, &bbox, None).await?;
    
    // Encode to requested format (GeoTIFF, NetCDF, etc.)
    match format {
        OutputFormat::GeoTiff => encode_geotiff(&region),
        OutputFormat::NetCdf => encode_netcdf(&region),
        OutputFormat::Zarr => encode_zarr(&region),
    }
}
```

## Model-Specific Configuration

Some models require special handling due to their coordinate systems. The grid-processor
automatically infers settings from the `projection` field in model YAML files:

```yaml
# config/models/hrrr.yaml
model:
  id: hrrr
  name: "HRRR - High-Resolution Rapid Refresh"

grid:
  projection: lambert_conformal  # Auto-sets requires_full_grid: true
  
  # Lambert Conformal projection parameters
  projection_params:
    lat_1: 38.5
    lat_2: 38.5
    lat_0: 38.5
    lon_0: -97.5
```

### Projection-Based Inference

The `requires_full_grid` setting is automatically inferred from the projection type:

| Projection | `requires_full_grid` | Reason |
|------------|---------------------|--------|
| `lambert_conformal` | `true` | Non-linear mapping between grid and geographic coords |
| `geostationary` | `true` | Satellite viewing geometry |
| `mercator` | `true` | Non-linear latitude scaling |
| `geographic` / `lat_lon` | `false` | Linear mapping allows partial reads |

You can override the inferred value explicitly if needed:

```yaml
grid:
  projection: geographic
  requires_full_grid: true  # Force full grid reads even for geographic
```

### Configuration Fields

| Config Field | Description | Default |
|--------------|-------------|---------|
| `projection` | Coordinate system type | Required |
| `requires_full_grid` | Force full grid reads | Auto-inferred from projection |
| `uses_360_longitude` | Longitude range 0-360 vs -180/180 | Auto-detected from bbox |

## See Also

- [Ingester Service](../services/ingester.md) - Uses ZarrWriter for storage
- [Rendering Pipeline](../architecture/rendering-pipeline.md) - Uses ZarrGridProcessor for reads
- [Data Flow](../architecture/data-flow.md) - End-to-end data pipeline
- [Storage Crate](./storage.md) - MinIO/S3 and catalog operations
