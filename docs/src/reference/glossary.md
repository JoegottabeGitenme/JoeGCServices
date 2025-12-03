# Glossary

Technical terms, acronyms, and definitions used in Weather WMS documentation.

## Weather Data Formats

### GRIB2
**GRIB Edition 2** - Binary format for meteorological data standardized by WMO (World Meteorological Organization). Used by GFS, HRRR, and MRMS.

### NetCDF
**Network Common Data Form** - Self-describing binary format for scientific data. NetCDF-4 uses HDF5 as the storage layer. Used by GOES satellites.

### HDF5
**Hierarchical Data Format version 5** - Container format for complex scientific data with built-in compression and chunking.

---

## Weather Models & Data Sources

### GFS
**Global Forecast System** - NOAA's global numerical weather prediction model. 0.25° resolution (~25 km), updated every 6 hours, forecasts out to 16 days.

### HRRR
**High-Resolution Rapid Refresh** - NOAA's high-resolution model for CONUS. 3 km resolution, updated hourly, forecasts out to 48 hours.

### MRMS
**Multi-Radar Multi-Sensor** - Real-time radar mosaic combining data from 146 radar sites across CONUS. 1 km resolution, updated every 2 minutes.

### GOES
**Geostationary Operational Environmental Satellite** - NOAA's weather satellites (GOES-16 East, GOES-18 West). Provides continuous hemisphere imagery in 16 spectral channels.

### NOMADS
**NOAA Operational Model Archive and Distribution System** - NOAA's data distribution service for weather models.

### NCEP
**National Centers for Environmental Prediction** - NOAA center producing weather models and forecasts.

### NOAA
**National Oceanic and Atmospheric Administration** - U.S. scientific agency focused on oceans and atmosphere.

---

## OGC Standards

### OGC
**Open Geospatial Consortium** - International standards organization for geospatial data and services.

### WMS
**Web Map Service** - OGC standard for serving georeferenced map images over HTTP. Weather WMS implements versions 1.1.1 and 1.3.0.

### WMTS
**Web Map Tile Service** - OGC standard for serving pre-rendered map tiles in a grid structure. Weather WMS implements version 1.0.0.

### GetCapabilities
OGC operation that returns XML describing service capabilities, available layers, and supported operations.

### GetMap (WMS)
OGC operation that renders a map image for specified layers, bbox, and dimensions.

### GetTile (WMTS)
OGC operation that returns a pre-rendered tile from a tile matrix.

### GetFeatureInfo
OGC operation that queries data values at a specific pixel location.

---

## Coordinate Systems

### CRS
**Coordinate Reference System** - Defines how coordinates relate to positions on Earth. Examples: EPSG:4326, EPSG:3857.

### EPSG
**European Petroleum Survey Group** (now OGP) - Organization that maintains a registry of CRS codes.

### EPSG:4326
Geographic coordinate system using latitude/longitude on WGS84 ellipsoid. Range: ±180° lon, ±90° lat.

### EPSG:3857
**Web Mercator** - Projected coordinate system used by most web maps (Google, Leaflet, OpenLayers). Range: ±20,037,508 meters.

### WGS84
**World Geodetic System 1984** - Global reference ellipsoid used by GPS and most modern coordinate systems.

### Projection
Mathematical transformation from Earth's curved surface to flat map. Examples: Mercator, Lambert Conformal Conic, Geostationary.

---

## Technical Terms

### Shredding
Splitting large data grids into smaller chunks (~1MB) for efficient partial access and parallel processing.

### Tile
Square map image (typically 256×256 pixels) at a specific zoom level and location. Tiles are assembled to create seamless maps.

### Tile Matrix
Grid of tiles at a specific zoom level. Higher zoom = more tiles = more detail.

### Zoom Level
Scale of the map. Zoom 0 = whole world in 1 tile. Each zoom level doubles resolution: zoom N has 2^(2N) tiles.

### XYZ Tiles
Simplified tile addressing: `z` (zoom), `x` (column), `y` (row). Non-standard but widely used.

### Bounding Box (BBOX)
Geographic rectangle defined by west, south, east, north coordinates.

### Forecast Hour
Hours since model initialization. Forecast hour 0 = analysis (nowcast), hour 24 = 24-hour forecast.

### Valid Time
Actual time that a forecast is valid for. Forecast time + forecast hour = valid time.

---

## Architecture Terms

### L1 Cache
First-level cache stored in process memory (RAM). Fastest access (<1ms) but limited capacity and not shared.

### L2 Cache
Second-level cache stored in Redis. Shared across instances, larger capacity, slightly slower (2-5ms).

### LRU
**Least Recently Used** - Cache eviction policy that removes least-recently-accessed items when full.

### TTL
**Time To Live** - Duration before cached entry expires and is removed.

### Cache Hit
Request served from cache without rendering. Fast response.

### Cache Miss
Cache doesn't have requested tile, requires rendering from source data. Slower response.

### Cache Warming
Pre-rendering tiles before they're requested to improve performance.

### Prefetching
Predictively fetching surrounding tiles when one tile is requested.

---

## Storage Terms

### Object Storage
Storage system treating data as discrete objects (blobs) with metadata. Examples: MinIO, AWS S3, Google Cloud Storage.

### MinIO
Open-source, S3-compatible object storage server. Default storage backend for Weather WMS.

### S3
**Simple Storage Service** - AWS object storage. MinIO provides S3-compatible API.

### Bucket
Top-level container in object storage, analogous to a filesystem drive or root directory.

### Catalog
PostgreSQL database storing metadata about available weather data (grids, times, parameters, locations).

---

## Performance Terms

### Throughput
Number of requests processed per unit time (e.g., requests/second).

### Latency
Time from request start to response completion. Usually measured at percentiles (p50, p95, p99).

### p50, p95, p99
**Percentiles** - p99 = 99% of requests faster than this. p50 = median.

### Horizontal Scaling
Adding more service instances to increase capacity.

### Vertical Scaling
Adding more resources (CPU, RAM) to existing instances.

### Load Balancer
Distributes requests across multiple service instances.

---

## Development Terms

### Cargo
Rust's package manager and build tool.

### Crate
Rust's term for a library or package.

### Workspace
Cargo project containing multiple related crates.

### Tokio
Async runtime for Rust, provides non-blocking I/O and task scheduling.

### Axum
Web framework for Rust built on Tokio and Tower.

### SQLx
Async SQL toolkit for Rust with compile-time query verification.

---

## Meteorological Terms

### dBZ
**Decibels relative to Z** - Radar reflectivity scale. Higher values = stronger returns = heavier precipitation.

### CAPE
**Convective Available Potential Energy** - Measure of atmospheric instability. Higher values = greater storm potential.

### REFL
**Reflectivity** - Radar return intensity indicating precipitation or cloud particles.

### Temperature (TMP)
Usually in Kelvin (K). 0°C = 273.15 K, 0°F = 255.37 K.

### Wind Components (UGRD, VGRD)
**U**: East-west wind (positive = eastward)  
**V**: North-south wind (positive = northward)

### Relative Humidity (RH)
Percentage of moisture in air relative to saturation. 0-100%.

### Pressure (PRMSL)
Atmospheric pressure reduced to mean sea level. Standard: 1013.25 mb (101325 Pa).

---

## File Formats

### PNG
**Portable Network Graphics** - Lossless image format supporting transparency. Default format for tiles.

### JPEG
**Joint Photographic Experts Group** - Lossy image format. Smaller than PNG but no transparency.

### YAML
**YAML Ain't Markup Language** - Human-readable data serialization format. Used for configuration files.

### JSON
**JavaScript Object Notation** - Text format for structured data. Used for API responses and some config.

---

## Acronyms

- **API**: Application Programming Interface
- **ASCII**: American Standard Code for Information Interchange
- **CI/CD**: Continuous Integration / Continuous Deployment
- **CLI**: Command Line Interface
- **CONUS**: Continental United States
- **CPU**: Central Processing Unit
- **DNS**: Domain Name System
- **GB**: Gigabyte (1024 MB)
- **HTTP**: Hypertext Transfer Protocol
- **HTTPS**: HTTP Secure
- **IR**: Infrared
- **ISO**: International Organization for Standardization
- **K8s**: Kubernetes (8 letters between K and s)
- **LRU**: Least Recently Used
- **MB**: Megabyte (1024 KB)
- **MSL**: Mean Sea Level
- **REST**: Representational State Transfer
- **SQL**: Structured Query Language
- **TLS**: Transport Layer Security
- **TTL**: Time To Live
- **UI**: User Interface
- **URL**: Uniform Resource Locator
- **UTC**: Coordinated Universal Time
- **UUID**: Universally Unique Identifier
- **WMO**: World Meteorological Organization
- **XML**: Extensible Markup Language

---

## See Also

- [Data Sources](../data-sources/README.md) - Detailed source documentation
- [API Reference](../api-reference/README.md) - API terminology
- [Architecture](../architecture/README.md) - System components
