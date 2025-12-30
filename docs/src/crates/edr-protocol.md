# edr-protocol

Types and utilities for implementing an OGC API - Environmental Data Retrieval server.

## Overview

**Location**: `crates/edr-protocol/`  
**Purpose**: Provides Rust types that serialize/deserialize to OGC EDR-compliant JSON

## Key Types

### Response Types

```rust
use edr_protocol::{LandingPage, ConformanceClasses, Collection, CollectionList};

// Landing page
let landing = LandingPage::new(
    "Weather EDR API",
    "Environmental data retrieval for weather models",
    "http://localhost:8083/edr",
);

// Conformance
let conformance = ConformanceClasses::current();

// Collections
let mut collection = Collection::new("hrrr-isobaric")
    .with_title("HRRR Isobaric")
    .with_description("Upper-air parameters");
collection.build_links("http://localhost:8083/edr");
```

### CoverageJSON

```rust
use edr_protocol::{CoverageJson, CovJsonParameter, Unit};
use edr_protocol::coverage_json::CovJsonParameter;

// Create a point coverage
let cov = CoverageJson::point(-97.5, 35.2, Some("2024-12-29T12:00:00Z".to_string()), Some(850.0));

// Add a parameter with value
let param = CovJsonParameter::new("Temperature")
    .with_unit(Unit::kelvin());

let cov = cov.with_parameter("TMP", param, 288.5);
```

### Query Parsing

```rust
use edr_protocol::{PositionQuery, DateTimeQuery};
use edr_protocol::queries::BboxQuery;

// Parse WKT POINT
let (lon, lat) = PositionQuery::parse_coords("POINT(-97.5 35.2)")?;

// Parse simple coords
let (lon, lat) = PositionQuery::parse_coords("-97.5,35.2")?;

// Parse vertical levels
let levels = PositionQuery::parse_z("850,700,500")?; // [850.0, 700.0, 500.0]

// Parse datetime
let dt = DateTimeQuery::parse("2024-12-29T00:00:00Z/2024-12-29T23:59:59Z")?;
```

### Error Types

```rust
use edr_protocol::{EdrError, ExceptionResponse};

let err = EdrError::CollectionNotFound("invalid-id".to_string());
assert_eq!(err.status_code(), 404);

let response = err.to_exception(); // OGC-compliant error response
```

## Module Structure

```
src/
├── lib.rs           # Re-exports and conformance URIs
├── types.rs         # Link, Extent, CRS types
├── collections.rs   # Collection, Instance types
├── parameters.rs    # Parameter, Unit types
├── coverage_json.rs # CoverageJSON types
├── queries.rs       # Query parameter parsing
├── responses.rs     # Landing page, conformance
└── errors.rs        # Error types
```

## Conformance URIs

```rust
use edr_protocol::conformance;

conformance::CORE       // Core conformance class
conformance::COLLECTIONS
conformance::POSITION
conformance::INSTANCES
conformance::COVJSON
conformance::GEOJSON
```

## Media Types

```rust
use edr_protocol::media_types;

media_types::COVERAGE_JSON  // "application/vnd.cov+json"
media_types::GEO_JSON       // "application/geo+json"
media_types::JSON           // "application/json"
```

## Serialization

All types implement `Serialize` and `Deserialize` from serde, producing OGC-compliant JSON:

```rust
let collection = Collection::new("test")
    .with_title("Test Collection");

let json = serde_json::to_string_pretty(&collection)?;
// Produces valid OGC EDR collection JSON
```

## Testing

The crate includes comprehensive unit tests (80+ tests):

```bash
cargo test -p edr-protocol
```

## See Also

- [EDR API Service](../services/edr-api.md) - Uses this crate
- [EDR Endpoints](../api-reference/edr.md) - API documentation
- [OGC EDR Spec](https://docs.ogc.org/is/19-086r4/19-086r4.html) - Official specification
