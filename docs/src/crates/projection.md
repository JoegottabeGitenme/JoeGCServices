# projection

Coordinate reference system (CRS) transformations and map projections implemented from scratch without external dependencies.

## Overview

**Location**: `crates/projection/`  
**Dependencies**: None (pure math)  
**LOC**: ~1,200

## Supported Projections

| Projection | EPSG Codes | Used By | Complexity |
|------------|------------|---------|------------|
| Geographic (Lat/Lon) | EPSG:4326 | GFS, MRMS | Trivial |
| Web Mercator | EPSG:3857 | Web maps | Simple |
| Lambert Conformal | Various | HRRR | Medium |
| Geostationary | N/A | GOES satellites | Complex |

> **Note**: Polar Stereographic (EPSG:3413, EPSG:3031) is not currently implemented. The `polar.rs` module is a placeholder for future development.

## Usage Example

```rust
use projection::{CRS, transform};

// Web Mercator to Geographic
let (lon, lat) = transform::mercator_to_geographic(
    -13149614.0,  // x (meters)
    4070118.0,    // y (meters)
)?;
println!("Location: {:.4}°N, {:.4}°E", lat, lon);

// Geographic to Web Mercator
let (x, y) = transform::geographic_to_mercator(-118.0, 34.0)?;

// Lambert Conformal (used by HRRR)
let lambert = LambertConformal {
    standard_parallel1: 38.5,
    standard_parallel2: 38.5,
    central_meridian: -97.5,
    latitude_of_origin: 38.5,
};

let (lon, lat) = lambert.xy_to_lonlat(1000000.0, 500000.0)?;
```

## Web Mercator (EPSG:3857)

Most common web map projection:

```rust
pub fn geographic_to_mercator(lon: f64, lat: f64) -> (f64, f64) {
    const R: f64 = 6378137.0;  // Earth radius (meters)
    
    let x = R * lon.to_radians();
    let y = R * (std::f64::consts::PI / 4.0 + lat.to_radians() / 2.0).tan().ln();
    
    (x, y)
}
```

**Limits**: Only valid for ±85.051129° latitude.

## Lambert Conformal Conic

Used by HRRR (3km CONUS forecast):

```rust
pub struct LambertConformal {
    pub standard_parallel1: f64,
    pub standard_parallel2: f64,
    pub central_meridian: f64,
    pub latitude_of_origin: f64,
}

impl LambertConformal {
    pub fn xy_to_lonlat(&self, x: f64, y: f64) -> (f64, f64) {
        // Complex projection math...
    }
}
```

## Geostationary

Used by GOES satellites:

```rust
pub struct Geostationary {
    pub satellite_height: f64,              // 35786023 m for GOES
    pub longitude_of_projection_origin: f64, // -75° for GOES-16, -137° for GOES-18
    pub sweep_angle_axis: char,             // 'x' or 'y'
}
```

## Performance

- Geographic ↔ Mercator: ~10 ns per point
- Lambert Conformal: ~50 ns per point
- Geostationary: ~100 ns per point

Vectorized operations on 1M points: ~50 ms.

## See Also

- [GRIB2 Parser](./grib2-parser.md) - Uses projections for grids
- [Renderer](./renderer.md) - Reprojects data for visualization
