# Weather WMS

**Weather WMS** is a high-performance, Kubernetes-native implementation of OGC Web Map Service (WMS) and Web Map Tile Service (WMTS) specifications, designed specifically for weather data visualization.

## Key Features

- **OGC Compliant**: Full support for WMS 1.1.1/1.3.0 and WMTS 1.0.0 specifications
- **Real-Time Weather Data**: Automatic ingestion from NOAA sources:
  - GFS (Global Forecast System) - Global weather forecasts
  - HRRR (High-Resolution Rapid Refresh) - High-resolution CONUS forecasts
  - MRMS (Multi-Radar Multi-Sensor) - Real-time radar composites
  - GOES-16/18 - Geostationary satellite imagery
- **High Performance**: Written in Rust with two-tier caching (L1 in-memory, L2 Redis)
- **Cloud Native**: Kubernetes deployment with Helm charts, horizontal scaling
- **Flexible Visualization**: Multiple rendering styles including gradients, contours, and wind barbs

## Architecture Overview

```
                            ┌─────────────────────────────────────────┐
                            │           Kubernetes Cluster            │
                            │                                         │
  ┌──────────┐              │  ┌───────────┐      ┌──────────────┐   │
  │  Map     │─────────────────│  WMS API  │─────▶│ Redis Cache  │   │
  │  Client  │              │  └─────┬─────┘      └──────────────┘   │
  └──────────┘              │        │                               │
                            │        ▼                               │
                            │  ┌───────────┐      ┌──────────────┐   │
                            │  │  MinIO    │◀─────│   Ingester   │   │
  ┌──────────┐              │  │ (Storage) │      └──────┬───────┘   │
  │   NOAA   │◀─────────────│  └───────────┘             │           │
  │  Sources │              │        ▲            ┌──────┴───────┐   │
  └──────────┘              │        │            │  Downloader  │   │
                            │  ┌───────────┐      └──────────────┘   │
                            │  │ PostgreSQL│                         │
                            │  │ (Catalog) │                         │
                            │  └───────────┘                         │
                            └─────────────────────────────────────────┘
```

## Quick Links

- [Getting Started](./getting-started/README.md) - Install and run in minutes
- [API Reference](./api-reference/README.md) - WMS/WMTS endpoint documentation
- [Configuration](./configuration/README.md) - Customize layers and styles
- [Deployment](./deployment/README.md) - Production deployment guides

## Supported Clients

Weather WMS works with any OGC-compliant mapping client:

- [Leaflet](https://leafletjs.com/) with WMS/WMTS plugins
- [OpenLayers](https://openlayers.org/)
- [MapLibre GL JS](https://maplibre.org/)
- [QGIS](https://qgis.org/)
- Any GIS software supporting WMS/WMTS

## License

This project is open source. See the repository for license details.
