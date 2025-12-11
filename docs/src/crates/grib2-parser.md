# grib2-parser

Pure Rust implementation of GRIB2 (GRIB Edition 2) parser for meteorological data. Parses GFS, HRRR, and MRMS weather data files.

## Overview

**Location**: `crates/grib2-parser/`  
**Dependencies**: `bytes`, `thiserror`, `chrono`, `grib` (for PNG/JPEG2000 decompression)  
**LOC**: ~1,200

GRIB2 is the WMO standard format (FM 92) for distributing gridded meteorological data. Each file contains one or more messages, each representing a single weather parameter at a specific time and level.

## GRIB2 File Structure

```
┌─────────────────────────────────────┐
│ Section 0: Indicator (16 bytes)    │  Magic: "GRIB", discipline, edition
├─────────────────────────────────────┤
│ Section 1: Identification (21+ B)  │  Model, reference time, center
├─────────────────────────────────────┤
│ Section 2: Local Use (optional)    │  Implementation-specific
├─────────────────────────────────────┤
│ Section 3: Grid Definition (72+ B) │  Projection, dimensions, resolution
├─────────────────────────────────────┤
│ Section 4: Product Definition (34+)│  Parameter, level, forecast hour
├─────────────────────────────────────┤
│ Section 5: Data Representation (21)│  Packing method, scale factors
├─────────────────────────────────────┤
│ Section 6: Bitmap (optional)       │  Missing value mask
├─────────────────────────────────────┤
│ Section 7: Data (~1-2 MB)          │  Compressed grid values
├─────────────────────────────────────┤
│ Section 8: End (4 bytes)           │  "7777" terminator
└─────────────────────────────────────┘
```

## Usage Example

```rust
use grib2_parser::Grib2Reader;
use bytes::Bytes;

// Read file
let data = std::fs::read("gfs.t00z.pgrb2.0p25.f000")?;
let mut reader = Grib2Reader::new(Bytes::from(data));

// Iterate over messages
while let Some(message) = reader.next_message()? {
    println!("Parameter: {}", message.parameter());
    println!("Level: {}", message.level());
    println!("Reference time: {}", message.identification.reference_time);
    println!("Valid time: {}", message.valid_time());
    
    // Get grid dimensions
    let (nj, ni) = message.grid_dims();
    println!("Grid shape: {}×{}", ni, nj);
    
    // Decode grid data
    let values = message.unpack_data()?;
    println!("Values: {:?}", &values[0..10]);
}
```

## Key Components

### Grib2Reader

Main entry point for parsing:

```rust
pub struct Grib2Reader {
    data: Bytes,
    current_offset: usize,
}

impl Grib2Reader {
    /// Create reader from byte buffer
    pub fn new(data: Bytes) -> Self;
    
    /// Read next message (returns None at end of file)
    pub fn next_message(&mut self) -> Grib2Result<Option<Grib2Message>>;
    
    /// Get total file size
    pub fn size(&self) -> usize;
    
    /// Check if more data available
    pub fn has_more(&self) -> bool;
}
```

### Grib2Message

Represents a single GRIB2 message:

```rust
pub struct Grib2Message {
    pub indicator: Indicator,                   // Section 0
    pub identification: Identification,         // Section 1
    pub grid_definition: GridDefinition,        // Section 3
    pub product_definition: ProductDefinition,  // Section 4
    pub data_representation: DataRepresentation,// Section 5
    pub bitmap: Bitmap,                         // Section 6
    pub data_section: DataSection,              // Section 7
    pub raw_data: Bytes,                        // Raw message bytes
}

impl Grib2Message {
    /// Get parameter short name (e.g., "TMP", "RH", "UGRD")
    pub fn parameter(&self) -> String;
    
    /// Get level description (e.g., "2 m above ground")
    pub fn level(&self) -> String;
    
    /// Get grid dimensions as (nj, ni) = (rows, columns)
    pub fn grid_dims(&self) -> (u32, u32);
    
    /// Get valid time (reference time + forecast hour)
    pub fn valid_time(&self) -> DateTime<Utc>;
    
    /// Decode compressed grid data to f32 values
    pub fn unpack_data(&self) -> Grib2Result<Vec<f32>>;
}
```

### Section Types

#### Section 1: Identification

```rust
pub struct Identification {
    pub center: u16,                 // Originating center (7 = NCEP)
    pub sub_center: u16,
    pub master_table_version: u8,
    pub local_table_version: u8,
    pub reference_time: DateTime<Utc>,  // Model run time
    pub production_status: u8,
    pub data_type: u8,
}
```

#### Section 3: Grid Definition

```rust
pub struct GridDefinition {
    pub grid_shape: u8,
    pub num_points_longitude: u32,   // Ni (columns)
    pub num_points_latitude: u32,    // Nj (rows)
    pub first_latitude_millidegrees: i32,
    pub first_longitude_millidegrees: i32,
    pub last_latitude_millidegrees: i32,
    pub last_longitude_millidegrees: i32,
    pub latitude_increment_millidegrees: u32,
    pub longitude_increment_millidegrees: u32,
    pub scanning_mode: u8,
}
```

#### Section 4: Product Definition

```rust
pub struct ProductDefinition {
    pub parameter_category: u8,      // 0=temp, 1=moisture, 2=momentum
    pub parameter_number: u8,        // Within category
    pub generating_process: u8,
    pub forecast_hour: u32,          // Hours since reference time
    pub level_type: u8,              // 1=surface, 103=height above ground
    pub level_value: u32,            // e.g., 2 for 2m
}
```

#### Section 5: Data Representation

```rust
pub struct DataRepresentation {
    pub num_data_points: u32,
    pub packing_method: u8,          // 0=simple, 40=JPEG2000, 41=PNG
    pub reference_value: f32,        // Minimum value (R)
    pub binary_scale_factor: i16,    // E: multiply by 2^E
    pub decimal_scale_factor: i16,   // D: multiply by 10^-D
    pub bits_per_value: u8,
}
```

## Supported Compressions

| Template | Name | Used By | Implementation |
|----------|------|---------|----------------|
| 0 | Simple packing | All models | Native `unpack_simple()` |
| 40 | JPEG2000 | GFS, HRRR | Via `grib` crate |
| 41 | PNG | GFS, HRRR | Via `grib` crate |

### Simple Packing (Template 0)

The unpacking formula is: `value = (R + packed_value × 2^E) × 10^-D`

```rust
pub fn unpack_simple(
    packed_data: &[u8],
    num_points: u32,
    bits_per_value: u8,
    reference_value: f32,      // R
    binary_scale_factor: i16,  // E
    decimal_scale_factor: i16, // D
    bitmap: Option<&[u8]>,
) -> Result<Vec<Option<f32>>, Grib2Error>;
```

### PNG/JPEG2000 (Templates 40, 41)

For PNG and JPEG2000 compressed data, the parser delegates to the external `grib` crate which handles the decompression.

## Parameter Lookup

GRIB2 uses discipline/category/number triplets to identify parameters. Common mappings:

| Discipline | Category | Number | Name | Short Name |
|------------|----------|--------|------|------------|
| 0 | 0 | 0 | Temperature | TMP |
| 0 | 1 | 1 | Relative Humidity | RH |
| 0 | 2 | 2 | U-component wind | UGRD |
| 0 | 2 | 3 | V-component wind | VGRD |
| 0 | 3 | 0 | Pressure | PRES |
| 0 | 3 | 5 | Geopotential Height | HGT |
| 0 | 6 | 1 | Total Cloud Cover | TCDC |
| 0 | 7 | 6 | CAPE | CAPE |

## Error Handling

```rust
pub enum Grib2Error {
    InvalidFormat(String),
    ParseError { offset: usize, reason: String },
    InvalidSection { section: u8, reason: String },
    UnpackingError(String),
}

// Usage
match reader.next_message() {
    Ok(Some(msg)) => { /* process message */ },
    Ok(None) => { /* end of file */ },
    Err(Grib2Error::InvalidFormat(e)) => eprintln!("Bad file: {}", e),
    Err(e) => eprintln!("Error: {}", e),
}
```

## Testing

```bash
# Run all tests (uses synthetic test data)
cargo test -p grib2-parser

# Generate synthetic test data
cargo test -p grib2-parser --test testdata_generator generate_test_files -- --ignored

# Run validation tests with real GFS data
cargo test -p grib2-parser --test validate_grib2 -- --ignored --nocapture
```

## References

- [WMO GRIB2 Specification](https://www.wmo.int/pages/prog/www/WMOCodes/Guides/GRIB/GRIB2_062006.pdf)
- [NCEP GRIB2 Tables](https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/)
- [GRIB2 Template Catalog](https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/grib2_table4-0.shtml)

## See Also

- [Ingester Service](../services/ingester.md) - Uses this crate
- [Projection Crate](./projection.md) - Grid coordinate transforms
