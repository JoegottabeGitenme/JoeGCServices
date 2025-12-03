# netcdf-parser

NetCDF-4 parser specialized for GOES satellite data. Reads GOES-R ABI Level 2 Cloud and Moisture Imagery products.

## Overview

**Location**: `crates/netcdf-parser/`  
**Dependencies**: `netcdf`, `ndarray`  
**LOC**: ~800

NetCDF (Network Common Data Form) is a self-describing binary format widely used for scientific data. GOES satellites distribute data in NetCDF-4 format with geostationary projection parameters.

## Usage Example

```rust
use netcdf_parser::GoesFile;

// Open GOES file
let goes = GoesFile::open("OR_ABI-L2-CMIPF-M6C13_G18_...nc")?;

println!("Satellite: GOES-{}", goes.satellite_id());
println!("Channel: {}", goes.channel());

// Read brightness temperature data
let cmi_data = goes.read_cmi()?;  // 5424×5424 array
let (ny, nx) = cmi_data.dim();

// Convert pixel to lat/lon
let (lat, lon) = goes.pixel_to_latlon(2712, 2712)?;
println!("Center: {:.2}°N, {:.2}°W", lat, -lon);
```

## GOES Channels

| Channel | Wavelength | Resolution | Common Use |
|---------|------------|------------|------------|
| C02 | 0.64 µm (Red) | 0.5 km | Daytime clouds |
| **C13** | **10.3 µm (IR)** | **2 km** | **Cloud-top temp** |

C13 (clean longwave IR) is the most commonly used for weather visualization.

## See Also

- [Projection Crate](./projection.md) - Geostationary projection
- [GOES Data](../data-sources/goes.md)
