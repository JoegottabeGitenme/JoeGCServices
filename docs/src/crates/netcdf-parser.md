# netcdf-parser

NetCDF-4 parser specialized for GOES satellite data. Reads GOES-R ABI Level 2 Cloud and Moisture Imagery products.

## Overview

**Location**: `crates/netcdf-parser/`  
**Dependencies**: `netcdf`, `hdf5-metno-sys`  
**LOC**: ~400

NetCDF (Network Common Data Form) is a self-describing binary format widely used for scientific data. GOES satellites distribute data in NetCDF-4 format with geostationary projection parameters.

## Module Structure

| Module | Purpose |
|--------|---------|
| `error` | Error types (`NetCdfError`) and result alias |
| `projection` | Geostationary coordinate transformations |
| `native` | High-performance netcdf library parsing |

## Usage Examples

### Loading GOES Data from Bytes

```rust
use netcdf_parser::{load_goes_netcdf_from_bytes, GoesProjection};

// Silence HDF5 error spam (call once at startup)
netcdf_parser::silence_hdf5_errors();

// Load GOES data from bytes (e.g., downloaded from S3)
let bytes = std::fs::read("goes_file.nc")?;
let (data, width, height, projection, x_off, y_off, x_scale, y_scale) =
    load_goes_netcdf_from_bytes(&bytes)?;

println!("Grid size: {}x{}", width, height);
println!("Satellite longitude: {}°", projection.longitude_origin);
```

### Converting Pixel to Lat/Lon

```rust
use netcdf_parser::GoesProjection;

let proj = GoesProjection::goes16();

// Convert pixel coordinates to scan angles (radians)
let pixel_x = 2500;
let pixel_y = 1500;
let x_rad = x_offset as f64 + (pixel_x as f64 * x_scale as f64);
let y_rad = y_offset as f64 + (pixel_y as f64 * y_scale as f64);

// Convert scan angles to geographic coordinates
if let Some((lon, lat)) = proj.to_geographic(x_rad, y_rad) {
    println!("Pixel ({}, {}) is at {:.2}°N, {:.2}°W", pixel_x, pixel_y, lat, -lon);
}
```

### Converting Lat/Lon to Pixel

```rust
use netcdf_parser::GoesProjection;

let proj = GoesProjection::goes16();

// Kansas City
let (lon, lat) = (-94.5786, 39.0997);

if let Some((x_rad, y_rad)) = proj.from_geographic(lon, lat) {
    // Convert scan angles back to pixel coordinates
    let pixel_x = ((x_rad - x_offset as f64) / x_scale as f64) as usize;
    let pixel_y = ((y_rad - y_offset as f64) / y_scale as f64) as usize;
    println!("Kansas City is at pixel ({}, {})", pixel_x, pixel_y);
} else {
    println!("Location not visible from satellite");
}
```

## Key Functions

### `load_goes_netcdf_from_bytes(data: &[u8])`

High-performance native parsing. Returns:
- `data`: Scaled CMI values (reflectance or brightness temperature)
- `width`, `height`: Grid dimensions
- `projection`: GOES projection parameters
- `x_offset`, `y_offset`: Scan angle offsets (radians)
- `x_scale`, `y_scale`: Scan angle scale factors (radians/pixel)

### `silence_hdf5_errors()`

Disables HDF5 C library's verbose stderr output. Call once at program startup.

### `GoesProjection::to_geographic(x_rad, y_rad)`

Convert scan angles (radians) to geographic coordinates (lon/lat degrees).
Returns `None` if the scan angle points to space (off Earth).

### `GoesProjection::from_geographic(lon, lat)`

Convert geographic coordinates to scan angles.
Returns `None` if the point is not visible from the satellite.

## GOES Satellites

| Satellite | Position | Identifier | Coverage |
|-----------|----------|------------|----------|
| GOES-16 | 75.2°W | G16 | US East, Atlantic |
| GOES-18 | 137.2°W | G18 | US West, Pacific |

## GOES Channels

| Channel | Wavelength | Resolution | Type | Common Use |
|---------|------------|------------|------|------------|
| C01 | 0.47 µm (Blue) | 1 km | Reflectance | Aerosols |
| C02 | 0.64 µm (Red) | 0.5 km | Reflectance | Daytime clouds |
| C08 | 6.2 µm (WV) | 2 km | Brightness Temp | Upper-level moisture |
| **C13** | **10.3 µm (IR)** | **2 km** | **Brightness Temp** | **Cloud-top temp** |

C13 (clean longwave IR) is the most commonly used for weather visualization.

## Performance Notes

The native parser writes data to a temp file because the underlying HDF5 library requires file handles. On Linux, `/dev/shm` (memory-backed tmpfs) is used automatically for faster I/O.

Benchmarks:
- Native parsing: ~50-100ms for a CONUS image

## Running Tests

```bash
cd crates/netcdf-parser
./test.sh           # Run all tests
./test.sh --verbose # Verbose output
./test.sh --release # Test in release mode
```

## System Requirements

- `libnetcdf-dev` - NetCDF C library
- `libhdf5-dev` - HDF5 library (NetCDF4 backend)

Install on Ubuntu/Debian:
```bash
apt install libnetcdf-dev libhdf5-dev
```

## See Also

- [Projection Crate](./projection.md) - Full projection library with LUT caching
- [GOES Data Source](../data-sources/goes.md) - GOES ingestion configuration
- [Renderer Crate](./renderer.md) - Uses this crate for GOES tile rendering
