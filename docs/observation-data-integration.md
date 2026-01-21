# EDR Observation Data Integration Plan

**METARs, TAFs, and MADIS Surface Observations**  
**Created:** January 2026

---

## Overview

This document outlines the plan for integrating point observation data (METARs, TAFs, MADIS) into the OGC EDR server. The design extends the existing gridded data infrastructure while maintaining clean separation between gridded (MinIO-backed) and point observation (PostgreSQL-only) data paths.

---

## Data Sources

### METARs (Phase 1)

| Attribute | Value |
|-----------|-------|
| **Source** | FAA Aviation Weather Center API |
| **Format** | JSON |
| **Endpoint** | `https://aviationweather.gov/api/data/metar?bbox=-125,24,-66,50&format=json` |
| **Poll Interval** | 5 minutes |
| **Dedup Key** | `icaoId` + `obsTime` |

### TAFs (Future)

| Attribute | Value |
|-----------|-------|
| **Source** | FAA Aviation Weather Center API |
| **Format** | JSON |
| **Endpoint** | `https://aviationweather.gov/api/data/taf?bbox=-125,24,-66,50&format=json` |
| **Poll Interval** | 15 minutes |
| **Dedup Key** | `icaoId` + `issueTime` |

### MADIS (Future - Requires Registration)

| Attribute | Value |
|-----------|-------|
| **Source** | NOAA MADIS FTP |
| **Format** | netCDF (gzipped) |
| **Server** | `madis-data.ncep.noaa.gov` |
| **Poll Interval** | 60 minutes (at :30 past hour) |

---

## Architecture

### Data Flow

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                          Data Ingestion Paths                                    │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                  │
│  GRIDDED DATA (existing)                                                        │
│  ───────────────────────                                                        │
│  Downloader (ModelRunner)                                                       │
│       │                                                                          │
│       ▼                                                                          │
│  Download GRIB2/NetCDF to /data/downloads/                                      │
│       │                                                                          │
│       ▼                                                                          │
│  POST /ingest { file_path }                                                     │
│       │                                                                          │
│       ▼                                                                          │
│  Ingester: Parse → Zarr → MinIO + PostgreSQL (datasets table)                   │
│                                                                                  │
│  ─────────────────────────────────────────────────────────────────────────────  │
│                                                                                  │
│  POINT OBSERVATIONS (new)                                                       │
│  ────────────────────────                                                       │
│  Downloader (ObservationRunner) ◄── NEW, separate thread                        │
│       │                                                                          │
│       ▼                                                                          │
│  Fetch JSON from Aviation Weather API (in memory)                               │
│       │                                                                          │
│       ▼                                                                          │
│  POST /ingest/observations { source: "metar", data: [...] }                     │
│       │                                                                          │
│       ▼                                                                          │
│  Ingester: Parse JSON → PostgreSQL only (locations + observations tables)       │
│            NO MinIO, NO file writes                                             │
│                                                                                  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

### Database Schema

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                             PostgreSQL Schema                                    │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                  │
│  EXISTING TABLES                       NEW TABLES (PostGIS)                     │
│  ───────────────                       ────────────────────                     │
│  datasets                              locations                                │
│    - model                               - id (KJFK, NYC, etc.)                 │
│    - parameter                           - name                                 │
│    - level                               - location (GEOGRAPHY)                 │
│    - reference_time                      - elevation_m                          │
│    - forecast_hour                       - location_type                        │
│    - storage_path ──────► MinIO          - properties (JSONB)                   │
│    - zarr_metadata                                                              │
│                                        observations                             │
│  layer_styles                            - location_id ──────► locations        │
│    - ...                                 - source (metar, madis_mesonet)        │
│                                          - obs_time                             │
│                                          - temperature_k, dewpoint_k, ...       │
│                                          - raw_text                             │
│                                          - NO storage_path (data inline)        │
│                                                                                  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

---

## EDR API Design

### OGC EDR Compliant Endpoints (Per-Collection)

```
# Observation Collections
GET /edr/collections/metar                          # Collection metadata
GET /edr/collections/metar/locations                # List all stations with data
GET /edr/collections/metar/locations/{station_id}   # Query data at station
GET /edr/collections/metar/radius?coords=...&within=50&within-units=km
GET /edr/collections/metar/area?coords=POLYGON(...)

# Unified observations collection (all sources)
GET /edr/collections/observations/locations
GET /edr/collections/observations/locations/{id}?source=metar,madis
```

### Extension Endpoints (Cross-Collection)

```
# List all known locations
GET /edr/locations
GET /edr/locations?bbox=-110,35,-100,42
GET /edr/locations?q=Denver

# Quick availability check (headers only)
HEAD /edr/locations/{id}

# Detailed availability metadata
GET /edr/locations/{id}?f=metadata

# Aggregated data across all collections
GET /edr/locations/{id}
GET /edr/locations/{id}?collections=metar,hrrr-surface,gfs-surface
GET /edr/locations/{id}?datetime=...&parameter-name=temperature
```

### Response Formats

**GeoJSON** (default for spatial queries):
```json
{
  "type": "FeatureCollection",
  "features": [{
    "type": "Feature",
    "id": "http://localhost:8083/edr/collections/metar/locations/KJFK",
    "geometry": { "type": "Point", "coordinates": [-73.7781, 40.6413] },
    "properties": {
      "station_id": "KJFK",
      "name": "John F. Kennedy International Airport",
      "datetime": "2026-01-19T14:51:00Z",
      "temperature": 271.15,
      "dewpoint": 265.15,
      "wind_speed": 7.7,
      "wind_direction": 310
    }
  }]
}
```

**CoverageJSON** (for time series):
```json
{
  "type": "Coverage",
  "domain": {
    "type": "Domain",
    "domainType": "PointSeries",
    "axes": {
      "x": { "values": [-73.7781] },
      "y": { "values": [40.6413] },
      "t": { "values": ["2026-01-19T12:00:00Z", "2026-01-19T13:00:00Z"] }
    }
  },
  "parameters": {
    "temperature": {
      "type": "Parameter",
      "unit": { "symbol": "K" }
    }
  },
  "ranges": {
    "temperature": {
      "type": "NdArray",
      "values": [270.15, 271.15]
    }
  }
}
```

---

## Unit Handling

### Storage

All values stored in SI units:
- Temperature: Kelvin (K)
- Wind speed: meters/second (m/s)
- Pressure: Pascals (Pa)
- Visibility: meters (m)
- Precipitation: millimeters (mm)

### API Output

Configurable via `?units=` parameter:
- `SI` (default): Kelvin, m/s, Pa
- `US`: Fahrenheit, knots, inHg
- `metric`: Celsius, m/s, hPa

---

## Implementation Phases

### Phase 1: Database Foundation
- [ ] Add PostGIS extension to PostgreSQL
- [ ] Create `locations` table with spatial index
- [ ] Create `observations` table
- [ ] Extend `Catalog` with observation methods
- [ ] Bootstrap FAA stations database (~5000 airports)

### Phase 2: Ingestion Pipeline
- [ ] Create `ObservationRunner` in downloader (separate thread)
- [ ] Implement Aviation Weather API client
- [ ] Create `config/models/metar.yaml`
- [ ] Add `POST /ingest/observations` endpoint to ingester
- [ ] Implement METAR JSON parser with SI unit conversion

### Phase 3: EDR Observation Collections
- [ ] Add `ObservationCollectionConfig` type
- [ ] Create `config/edr/metar.yaml` and `config/edr/observations.yaml`
- [ ] Implement `/collections/metar/locations` endpoints
- [ ] Implement `/collections/metar/radius` (PostGIS ST_DWithin)
- [ ] Implement `/collections/metar/area` (PostGIS ST_Within)

### Phase 4: Cross-Collection Locations
- [ ] Implement `GET /edr/locations` (list all)
- [ ] Implement `GET /edr/locations/{id}` (aggregated data)
- [ ] Implement `HEAD /edr/locations/{id}` (availability headers)
- [ ] Implement `GET /edr/locations/{id}?f=metadata`

### Phase 5: Polish
- [ ] Add unit conversion parameter
- [ ] Implement observation retention/cleanup
- [ ] Unit tests

---

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Storage backend | PostgreSQL only (no MinIO) | Point data is small, relational queries are efficient |
| Spatial queries | PostGIS | Native spatial indexes, ST_DWithin/ST_Within |
| Catalog extension | Extend existing `Catalog` | Reuses connection pool, single migration path |
| Downloader design | New `ObservationRunner` | Separate thread, won't block gridded downloads |
| Ingester endpoint | `POST /ingest/observations` | Receives JSON directly, no file I/O |
| EDR collections | Both unified + specific | `observations` (all) + `metar` (specific) |
| Stations bootstrap | Load FAA airports on startup | ~5000 entries, negligible performance impact |
| Unit storage | SI only | Consistent internal representation |

---

## Configuration Examples

### Downloader: `config/models/metar.yaml`

```yaml
model:
  id: metar
  name: "METAR Surface Observations"
  enabled: true

source:
  type: point_observation
  api: aviation_weather
  base_url: "https://aviationweather.gov/api/data/metar"
  query:
    bbox: "-125,24,-66,50"
    format: json
    hours: 1

schedule:
  type: observation
  poll_interval_secs: 300

retention:
  hours: 168
```

### EDR: `config/edr/metar.yaml`

```yaml
collection:
  id: metar
  title: "METAR Surface Observations"
  description: "Hourly aviation weather reports from airports"
  data_type: point_observation
  source: metar

parameters:
  - name: temperature
    units: K
    db_column: temperature_k
  - name: dewpoint
    units: K
    db_column: dewpoint_k
  - name: wind_speed
    units: m/s
    db_column: wind_speed_ms
  - name: wind_direction
    units: degree
    db_column: wind_direction_deg
  - name: visibility
    units: m
    db_column: visibility_m
  - name: altimeter
    units: Pa
    db_column: altimeter_pa

data_queries:
  - locations
  - radius
  - area

settings:
  output_formats:
    - application/geo+json
    - application/vnd.cov+json
```

---

## Future Work

- **TAF Support**: Forecast periods require different data model (JSONB array)
- **MADIS Integration**: Requires FTP registration, netCDF parsing
- **Upper-air Observations**: MADIS radiosonde data (vertical profiles)
- **International Stations**: Expand beyond CONUS bbox
- **Real-time WebSocket**: Push new observations to connected clients
