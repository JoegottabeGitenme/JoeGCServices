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

// Create time series
let cov = CoverageJson::point_series(
    -97.5, 35.2,
    vec!["2024-12-29T12:00:00Z".to_string(), "2024-12-29T13:00:00Z".to_string()],
    Some(850.0)
);

// Create vertical profile
let cov = CoverageJson::vertical_profile(
    -97.5, 35.2,
    Some("2024-12-29T12:00:00Z".to_string()),
    vec![1000.0, 850.0, 700.0, 500.0]
);
```

### GeoJSON

The crate provides GeoJSON output format support for EDR data queries.

```rust
use edr_protocol::{EdrFeatureCollection, EdrFeature, EdrProperties, ParameterValue};

// Create a GeoJSON feature collection
let mut fc = EdrFeatureCollection::new();

// Add a point feature with data
let props = EdrProperties::new()
    .with_datetime("2024-12-29T12:00:00Z")
    .with_z(850.0)
    .with_parameter("TMP", ParameterValue::new(288.5).with_unit("K"));

let feature = EdrFeature::point(-97.5, 35.2).with_properties(props);
let fc = fc.with_feature(feature);

// Convert from CoverageJSON to GeoJSON
use edr_protocol::CoverageJson;
let covjson = CoverageJson::point(-97.5, 35.2, None, None);
let geojson = EdrFeatureCollection::from(&covjson);
```

### Named Locations

Support for pre-defined named locations (airports, cities, weather stations).

```rust
use edr_protocol::{Location, LocationsConfig, LocationFeatureCollection};

// Create a location
let location = Location::new("KJFK", "JFK Airport", -73.7781, 40.6413)
    .with_description("New York, NY")
    .with_property("type", "airport")
    .with_property("country", "US");

// Access coordinates
let lon = location.lon();  // -73.7781
let lat = location.lat();  // 40.6413

// Load locations from config
let config = LocationsConfig::from_locations(vec![location]);

// Case-insensitive lookup
let loc = config.find("kjfk");  // Returns Some(&Location)
let loc = config.find("KJFK");  // Also works

// Convert to GeoJSON for /locations endpoint
let geojson = LocationFeatureCollection::from_config(&config);
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
let points = PositionQuery::parse_coords_multi("MULTIPOINT((-97.5 35.2),(-98.0 36.0))")?;
```

#### Area Query (Polygon Coordinates)

```rust
use edr_protocol::AreaQuery;

// Parse POLYGON
let polygon = AreaQuery::parse_polygon("POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))")?;

// Parse MULTIPOLYGON
let polygons = AreaQuery::parse_polygon_multi("MULTIPOLYGON(((-98 35,-97 35,-97 36,-98 36,-98 35)))")?;

// Get bounding box and area
let bbox = polygon.bbox();
let area = polygon.area_sq_degrees();
```

#### Radius Query

```rust
use edr_protocol::{RadiusQuery, DistanceUnit};

// Parse within parameter
let radius_km = RadiusQuery::parse_within("50", "km")?;  // Returns 50.0
let radius_m = RadiusQuery::parse_within("50000", "m")?; // Returns 50000.0

// Calculate Haversine distance
let dist = RadiusQuery::haversine_distance(-97.5, 35.2, -98.0, 36.0);
```

#### Trajectory Query (Linestring Coordinates)

```rust
use edr_protocol::TrajectoryQuery;

// Parse LINESTRING
let traj = TrajectoryQuery::parse_coords("LINESTRING(-100 40,-99 40.5,-98 41)")?;

// Parse LINESTRINGZ (with altitude)
let traj = TrajectoryQuery::parse_coords("LINESTRINGZ(-100 40 850,-99 40.5 700,-98 41 500)")?;

// Parse LINESTRINGM (with time as Unix epoch)
let traj = TrajectoryQuery::parse_coords("LINESTRINGM(-100 40 1735574400,-99 40.5 1735578000,-98 41 1735581600)")?;

// Parse LINESTRINGZM (with both)
let traj = TrajectoryQuery::parse_coords("LINESTRINGZM(-100 40 850 1735574400,-99 40.5 700 1735578000)")?;

// Get trajectory metadata
let bbox = traj.bounding_box();
let length = traj.path_length_meters();
let time_range = traj.time_range();
let z_range = traj.z_range();
```

#### Corridor Query

```rust
use edr_protocol::CorridorQuery;

// Corridor uses trajectory coords plus width/height
let coords = CorridorQuery::parse_coords("LINESTRING(-100 40,-99 40.5,-98 41)")?;

// Parse corridor dimensions
let width = CorridorQuery::parse_width("10", "km")?;   // 10 km
let height = CorridorQuery::parse_height("1000", "m")?; // 1000 m

// Get dimensions in meters
let width_m = width.width_meters();
let height_m = height.height_meters();
let half_width = width.half_width_meters();
```

#### Cube Query (Bounding Box)

```rust
use edr_protocol::BboxQuery;

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

// List of times
let dt = DateTimeQuery::parse("2024-12-29T12:00:00Z,2024-12-29T13:00:00Z")?;

// Expand interval against available times
let available = vec!["2024-12-29T12:00:00Z", "2024-12-29T13:00:00Z", "2024-12-29T14:00:00Z"];
let times = dt.expand_against_available_times(&available);
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
├── geojson.rs       # GeoJSON output types
├── locations.rs     # Named location types
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
conformance::LOCATIONS  // Named locations
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
media_types::OPENAPI_JSON   // "application/vnd.oai.openapi+json;version=3.0"
```

## Unit Presets

Common units are provided as presets:

```rust
use edr_protocol::Unit;

Unit::kelvin()           // Temperature in K
Unit::celsius()          // Temperature in °C
Unit::meters_per_second() // Wind speed in m/s
Unit::pascals()          // Pressure in Pa
Unit::hectopascals()     // Pressure in hPa
Unit::percent()          // Percentage
Unit::dbz()              // Reflectivity in dBZ
Unit::joules_per_kg()    // Energy in J/kg
Unit::kg_per_m2()        // Precipitation in kg/m²
Unit::meters()           // Distance/height in m
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

The crate includes comprehensive unit tests (150+ tests):

```bash
cargo test -p edr-protocol
```

## See Also

- [EDR API Service](../services/edr-api.md) - Uses this crate
- [EDR Endpoints](../api-reference/edr.md) - API documentation
- [OGC EDR Spec](https://docs.ogc.org/is/19-086r6/19-086r6.html) - Official specification
