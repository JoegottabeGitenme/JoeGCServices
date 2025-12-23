# grid-processor

Grid processing abstraction layer with Zarr V3 support for efficient chunked data access.

## Purpose

This crate provides a **storage-agnostic interface** for accessing gridded weather data. It serves as the data abstraction layer for OGC services (WMS, WMTS, EDR, WCS) and any other consumers that need access to weather grid data.

**Key capabilities:**
- **Partial reads**: Only fetch the chunks needed for a specific geographic region
- **Pyramid support**: Automatically select optimal resolution for output size
- **Chunk caching**: LRU cache for decompressed chunks shared across requests
- **Unified query interface**: Find datasets by model, parameter, time, and level
- **Zarr V3 format**: Industry-standard format with sharding support

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Consumer Services                               │
│                     (WMS, WMTS, EDR, WCS, Custom APIs)                       │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           GridDataService (High-Level)                       │
│  - Unified query interface (DatasetQuery)                                   │
│  - Catalog integration (finds datasets by model/param/time/level)           │
│  - Automatic pyramid level selection                                        │
│  - Model-specific handling (0-360 longitude, Lambert projection, etc.)      │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         GridProcessorFactory (Mid-Level)                     │
│  - Shared chunk cache across all requests                                   │
│  - Shared storage connection (MinIO/S3)                                     │
│  - Configuration management                                                 │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           GridProcessor Trait (Low-Level)                    │
│  - read_region(bbox) → GridRegion                                           │
│  - read_point(lon, lat) → Option<f32>                                       │
│  - metadata() → GridMetadata                                                │
│  - cache_stats() → CacheStats                                               │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              ZarrGridProcessor                               │
│  - Zarr V3 array access                                                     │
│  - Chunk-level byte-range requests                                          │
│  - Automatic chunk caching                                                  │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            Object Storage (MinIO/S3)                         │
│                          Zarr V3 arrays with sharding                        │
└─────────────────────────────────────────────────────────────────────────────┘
```

## API Levels

### High-Level: `GridDataService`

The recommended interface for most use cases. Handles catalog queries, storage access, and model-specific quirks automatically.

```rust
use grid_processor::{GridDataService, DatasetQuery, BoundingBox};

// Create service (typically done once at application startup)
let service = GridDataService::new(catalog, storage_config, cache_size_mb).await?;

// Query for a specific forecast dataset
let query = DatasetQuery::forecast("gfs", "TMP")
    .at_level("2 m above ground")
    .at_forecast_hour(6);

// Read a region (for tile rendering)
let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
let region = service.read_region(&query, &bbox, Some((256, 256))).await?;

println!("Got {}x{} grid values", region.width, region.height);

// Query a point (for GetFeatureInfo or EDR Position)
let value = service.read_point(&query, -95.0, 35.0).await?;
if let Some(temp) = value.value {
    println!("Temperature: {} {}", temp, value.units);
}
```

### Mid-Level: `GridProcessorFactory`

For cases where you have a catalog entry and want direct control over the processor:

```rust
use grid_processor::{GridProcessorFactory, ZarrGridProcessor, BoundingBox};

// Create factory with shared cache
let factory = GridProcessorFactory::new(storage, cache_size_mb);

// Create processor for a specific Zarr array
let processor = factory.create_processor(&zarr_path, &zarr_metadata)?;

// Read data
let region = processor.read_region(&bbox).await?;
```

### Low-Level: `GridProcessor` Trait

For custom implementations or direct Zarr access:

```rust
use grid_processor::{GridProcessor, ZarrGridProcessor, GridProcessorConfig};

// Open Zarr array directly
let processor = ZarrGridProcessor::open(store, "/path/to/array.zarr", config)?;

// Use the trait methods
let metadata = processor.metadata();
let region = processor.read_region(&bbox).await?;
let point = processor.read_point(-95.0, 35.0).await?;
```

## Query Interface

The `DatasetQuery` builder provides a fluent interface for specifying which dataset to access:

```rust
use grid_processor::{DatasetQuery, TimeSpecification};
use chrono::{Utc, DateTime};

// Forecast model query (GFS, HRRR)
let query = DatasetQuery::forecast("gfs", "TMP")
    .at_level("2 m above ground")
    .at_forecast_hour(6)
    .at_run(reference_time);  // Optional: specific model run

// Observation query (GOES, MRMS)
let query = DatasetQuery::observation("goes18", "CMI_C13")
    .at_time(observation_time);

// Latest available data
let query = DatasetQuery::forecast("gfs", "TMP")
    .at_level("2 m above ground")
    .latest();  // Gets most recent run, earliest forecast hour
```

## Use Cases by Service

| Service | Primary Methods | Example |
|---------|-----------------|---------|
| **WMS/WMTS** | `read_region()` | Tile rendering with bbox |
| **GetFeatureInfo** | `read_point()` | Point query for popup info |
| **EDR Position** | `read_point()` | Single coordinate query |
| **EDR Area** | `read_region()` | Bbox query, convert to CoverageJSON |
| **EDR Trajectory** | Multiple `read_point()` | Iterate over path coordinates |
| **WCS GetCoverage** | `read_region()` | Raw grid data export |

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CHUNK_CACHE_SIZE_MB` | `1024` | Memory budget for decompressed chunks |
| `ZARR_CHUNK_SIZE` | `512` | Default chunk dimension for writes |
| `ZARR_COMPRESSION` | `blosc_zstd` | Compression codec |
| `GRID_INTERPOLATION` | `bilinear` | Point query interpolation |

### Programmatic Configuration

```rust
use grid_processor::{GridProcessorConfig, ZarrCompression, PyramidConfig, DownsampleMethod};

let config = GridProcessorConfig {
    zarr_chunk_size: 512,
    compression: ZarrCompression::BloscZstd { level: 3 },
    pyramid: Some(PyramidConfig {
        levels: 4,
        method: DownsampleMethod::Mean,
        min_dimension: 256,
    }),
    ..Default::default()
};
```

## Model-Specific Behavior

Some models require special handling due to their coordinate systems:

| Model | Projection | Special Handling |
|-------|------------|------------------|
| GFS | Geographic (0-360°) | Longitude normalization from -180/180 requests |
| HRRR | Lambert Conformal | Requires full grid read, projection in renderer |
| MRMS | Geographic | Standard partial reads |
| GOES | Geostationary → Geographic | Pre-reprojected during ingestion |

This behavior is configured per-model in the model YAML configuration:

```yaml
# config/models/hrrr.yaml
grid:
  projection: lambert_conformal
  requires_full_grid: true  # Cannot do partial bbox reads
```

## Writing Data

For ingestion pipelines, use `ZarrWriter`:

```rust
use grid_processor::{ZarrWriter, GridProcessorConfig, BoundingBox};

let writer = ZarrWriter::new(config);

// Write with automatic pyramid generation
let result = writer.write_multiscale(
    store,
    "/grids/gfs/2024-12-22/00/TMP_f006.zarr",
    &grid_data,
    width, height,
    &bbox,
    "gfs", "TMP", "2 m above ground", "K",
    reference_time, forecast_hour,
)?;

println!("Wrote {} pyramid levels", result.levels.len());
```

## Performance

| Operation | Typical Latency | Notes |
|-----------|-----------------|-------|
| `read_region()` (cache hit) | 1-5 ms | All chunks cached |
| `read_region()` (cache miss) | 30-100 ms | Fetches 1-4 chunks from S3 |
| `read_point()` | 1-10 ms | Single chunk access |
| Pyramid level selection | <1 ms | O(1) calculation |
| Cache hit rate (warm) | >90% | After initial tile requests |

## Testing

Run tests with the included test data:

```bash
cargo test --package grid-processor

# Run benchmarks
cargo bench --package grid-processor
```

## See Also

- [API Reference](https://docs.rs/grid-processor)
- [Architecture Guide](../../docs/src/crates/grid-processor.md)
- [Data Flow](../../docs/src/architecture/data-flow.md)
