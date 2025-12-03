# grib2-parser

Pure Rust implementation of GRIB2 (GRIB Edition 2) parser for meteorological data. Parses GFS, HRRR, and MRMS weather data files without external dependencies.

## Overview

**Location**: `crates/grib2-parser/`  
**Dependencies**: `bytes`, `thiserror`  
**LOC**: ~2,500

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
let reader = Grib2Reader::new(Bytes::from(data));

// Iterate over messages
for message in reader.iter_messages() {
    println!("Parameter: {}", message.product_definition.parameter);
    println!("Level: {} {}", message.product_definition.level_value, 
             message.product_definition.level_type);
    println!("Forecast hour: {}", message.product_definition.forecast_hour);
    
    // Decode grid data
    let grid_data = message.decode_data()?;
    println!("Grid shape: {}×{}", grid_data.nx, grid_data.ny);
    println!("Values: {:?}", &grid_data.values[0..10]);
}
```

## Key Components

### Grib2Reader

Main entry point for parsing:

```rust
pub struct Grib2Reader {
    data: Bytes,
    messages: Vec<MessageIndex>,
}

impl Grib2Reader {
    /// Create reader from byte buffer
    pub fn new(data: Bytes) -> Grib2Result<Self>;
    
    /// Iterate over all messages
    pub fn iter_messages(&self) -> impl Iterator<Item = Grib2Message>;
    
    /// Get message by index
    pub fn message(&self, index: usize) -> Grib2Result<Grib2Message>;
}
```

### Grib2Message

Represents a single GRIB2 message:

```rust
pub struct Grib2Message {
    pub indicator: Indicator,                // Section 0
    pub identification: Identification,      // Section 1
    pub grid_definition: GridDefinition,     // Section 3
    pub product_definition: ProductDefinition, // Section 4
    pub data_representation: DataRepresentation, // Section 5
    pub bitmap: Option<Bitmap>,              // Section 6
    pub data: DataSection,                   // Section 7
}

impl Grib2Message {
    /// Decode compressed grid data
    pub fn decode_data(&self) -> Grib2Result<DecodedGrid>;
}
```

### Section Types

#### Section 1: Identification

```rust
pub struct Identification {
    pub center_id: u16,              // Originating center (7 = NCEP)
    pub subcenter_id: u16,
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
    pub source: u8,
    pub num_points: u32,             // Total grid points
    pub grid_template_number: u16,   // 0=lat/lon, 30=Lambert, etc.
    pub grid_template: GridTemplate, // Projection-specific params
}

pub enum GridTemplate {
    LatLon(LatLonGrid),
    LambertConformal(LambertGrid),
    PolarStereographic(PolarGrid),
    Mercator(MercatorGrid),
}
```

#### Section 4: Product Definition

```rust
pub struct ProductDefinition {
    pub template_number: u16,
    pub parameter_category: u8,      // 0=temp, 1=moisture, 2=momentum
    pub parameter_number: u8,        // Within category
    pub generating_process: u8,
    pub forecast_hour: u32,          // Hours since reference time
    pub level_type: u8,              // 1=surface, 103=height above ground
    pub level_value: f64,            // e.g., 2.0 for 2m
}
```

#### Section 5: Data Representation

```rust
pub struct DataRepresentation {
    pub template_number: u16,        // 0=simple, 40=JPEG2000, 41=PNG
    pub reference_value: f32,        // Minimum value
    pub binary_scale_factor: i16,
    pub decimal_scale_factor: i16,
    pub num_bits: u8,                // Bits per value
}
```

## Supported Compressions

| Template | Name | Used By | Complexity |
|----------|------|---------|------------|
| 0 | Simple packing | All models | Low |
| 2 | Complex packing | Rare | Medium |
| 3 | Complex + spatial differencing | Rare | High |
| 40 | JPEG2000 | GFS, HRRR | High |
| 41 | PNG | Some MRMS | Medium |

### Simple Packing (Template 0)

```rust
pub fn unpack_simple(
    data: &[u8],
    num_values: usize,
    num_bits: u8,
    reference_value: f32,
    binary_scale: i16,
    decimal_scale: i16,
) -> Vec<f32> {
    let binary_scale_factor = 2f32.powi(binary_scale as i32);
    let decimal_scale_factor = 10f32.powi(-decimal_scale as i32);
    
    let mut values = Vec::with_capacity(num_values);
    let mut bit_offset = 0;
    
    for _ in 0..num_values {
        let int_value = read_bits(data, bit_offset, num_bits);
        let value = (reference_value + int_value as f32 * binary_scale_factor) 
                    * decimal_scale_factor;
        values.push(value);
        bit_offset += num_bits as usize;
    }
    
    values
}
```

## Performance

| Operation | GFS File | HRRR File |
|-----------|----------|-----------|
| Parse structure | 50 ms | 80 ms |
| Decode all messages | 2s (129 msgs) | 5s (49 msgs) |
| Single message decode | 15 ms | 100 ms |
| Memory usage | ~200 MB | ~400 MB |

**Comparison**: 50-100x faster than spawning `wgrib2` subprocess.

## Parameter Lookup

GRIB2 uses discipline/category/number triplets to identify parameters:

```rust
pub fn parameter_short_name(
    discipline: u8,
    category: u8,
    number: u8,
) -> Option<&'static str> {
    match (discipline, category, number) {
        (0, 0, 0) => Some("TMP"),      // Temperature
        (0, 0, 2) => Some("POT"),      // Potential temperature
        (0, 1, 1) => Some("RH"),       // Relative humidity
        (0, 2, 2) => Some("UGRD"),     // U-component wind
        (0, 2, 3) => Some("VGRD"),     // V-component wind
        (0, 3, 0) => Some("PRES"),     // Pressure
        (0, 3, 1) => Some("PRMSL"),    // Pressure reduced to MSL
        _ => None,
    }
}
```

## Error Handling

```rust
pub enum Grib2Error {
    InvalidFormat(String),
    UnexpectedEnd,
    InvalidSection { section: u8, reason: String },
    UnsupportedTemplate { template_number: u16, reason: String },
    UnpackingError(String),
}

// Usage
match reader.message(0) {
    Ok(msg) => { /* process */ },
    Err(Grib2Error::InvalidFormat(e)) => eprintln!("Bad file: {}", e),
    Err(Grib2Error::UnsupportedTemplate { template_number, .. }) => {
        eprintln!("Unsupported compression: {}", template_number);
    },
    Err(e) => eprintln!("Other error: {}", e),
}
```

## Testing

```bash
# Unit tests
cargo test -p grib2-parser

# With test data
cargo test -p grib2-parser -- --test-threads=1

# Benchmarks
cargo bench -p grib2-parser
```

## References

- [WMO GRIB2 Specification](https://www.wmo.int/pages/prog/www/WMOCodes/Guides/GRIB/GRIB2_062006.pdf)
- [NCEP GRIB2 Tables](https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/)
- [GRIB2 Template Catalog](https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/grib2_table4-0.shtml)

## See Also

- [Ingester Service](../services/ingester.md) - Uses this crate
- [Projection Crate](./projection.md) - Grid coordinate transforms
