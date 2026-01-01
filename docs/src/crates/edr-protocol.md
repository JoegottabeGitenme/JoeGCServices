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

The crate provides parsers for all EDR query parameters.

#### Position Query (Point Coordinates)

```rust
use edr_protocol::PositionQuery;

// Parse WKT POINT
let (lon, lat) = PositionQuery::parse_coords("POINT(-97.5 35.2)")?;

// Parse simple coords
let (lon, lat) = PositionQuery::parse_coords("-97.5,35.2")?;

// Parse MULTIPOINT
let points = PositionQuery::parse_multipoint("MULTIPOINT((-97.5 35.2),(-98.0 36.0))")?;
```

#### Area Query (Polygon Coordinates)

```rust
use edr_protocol::AreaQuery;

// Parse POLYGON
let polygon = AreaQuery::parse_coords("POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))")?;

// Parse MULTIPOLYGON
let polygons = AreaQuery::parse_coords("MULTIPOLYGON(((-98 35,-97 35,-97 36,-98 36,-98 35)))")?;
```

#### Radius Query

```rust
use edr_protocol::RadiusQuery;

// Parse within parameter
let radius_km = RadiusQuery::parse_within("50", "km")?;  // Returns 50.0
let radius_m = RadiusQuery::parse_within("50000", "m")?; // Returns 50000.0
```

#### Trajectory Query (Linestring Coordinates)

```rust
use edr_protocol::TrajectoryQuery;

// Parse LINESTRING
let coords = TrajectoryQuery::parse_coords("LINESTRING(-100 40,-99 40.5,-98 41)")?;

// Parse LINESTRINGZ (with altitude)
let coords = TrajectoryQuery::parse_coords("LINESTRINGZ(-100 40 850,-99 40.5 700,-98 41 500)")?;

// Parse LINESTRINGM (with time as Unix epoch)
let coords = TrajectoryQuery::parse_coords("LINESTRINGM(-100 40 1735574400,-99 40.5 1735578000,-98 41 1735581600)")?;

// Parse LINESTRINGZM (with both)
let coords = TrajectoryQuery::parse_coords("LINESTRINGZM(-100 40 850 1735574400,-99 40.5 700 1735578000)")?;
```

#### Corridor Query

```rust
use edr_protocol::CorridorQuery;

// Corridor uses trajectory coords plus width/height
let coords = CorridorQuery::parse_coords("LINESTRING(-100 40,-99 40.5,-98 41)")?;

// Parse corridor dimensions
let width = CorridorQuery::parse_width("10", "km")?;   // 10 km
let height = CorridorQuery::parse_height("1000", "m")?; // 1000 m
```

#### Cube Query (Bounding Box)

```rust
use edr_protocol::queries::BboxQuery;

// Parse bbox parameter
let bbox = BboxQuery::parse("-98,35,-97,36")?;
// bbox.west = -98, bbox.south = 35, bbox.east = -97, bbox.north = 36
```

#### Vertical Level (z) Parameter

```rust
use edr_protocol::PositionQuery;

// Single level
let levels = PositionQuery::parse_z("850")?;           // [850.0]

// Multiple levels
let levels = PositionQuery::parse_z("850,700,500")?;   // [850.0, 700.0, 500.0]

// Range (min/max)
let levels = PositionQuery::parse_z("1000/500")?;      // [1000.0, 500.0]

// Recurring (R{count}/{start}/{decrement})
let levels = PositionQuery::parse_z("R5/1000/100")?;   // [1000.0, 900.0, 800.0, 700.0, 600.0]
```

#### Datetime Parameter

```rust
use edr_protocol::DateTimeQuery;

// Single instant
let dt = DateTimeQuery::parse("2024-12-29T12:00:00Z")?;

// Interval
let dt = DateTimeQuery::parse("2024-12-29T00:00:00Z/2024-12-29T23:59:59Z")?;

// Open-ended interval
let dt = DateTimeQuery::parse("2024-12-29T00:00:00Z/..")?;  // From time to now
let dt = DateTimeQuery::parse("../2024-12-29T23:59:59Z")?;  // Up to time
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
conformance::AREA
conformance::RADIUS
conformance::TRAJECTORY
conformance::CORRIDOR
conformance::CUBE
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

The crate includes comprehensive unit tests (120+ tests):

```bash
cargo test -p edr-protocol
```

## See Also

- [EDR API Service](../services/edr-api.md) - Uses this crate
- [EDR Endpoints](../api-reference/edr.md) - API documentation
- [OGC EDR Spec](https://docs.ogc.org/is/19-086r6/19-086r6.html) - Official specification
