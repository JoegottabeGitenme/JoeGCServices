# Weather WMS

**Weather WMS** is a high-performance implementation of OGC Web Map Service (WMS), Web Map Tile Service (WMTS), and Environmental Data Retrieval (EDR) specifications, designed specifically for weather data visualization.

## Key Features

- **OGC Compliant**: Full support for WMS 1.1.1/1.3.0, WMTS 1.0.0, and EDR 1.0 specifications
- **Real-Time Weather Data**: Automatic ingestion from NOAA sources:
  - GFS (Global Forecast System) - Global weather forecasts
  - HRRR (High-Resolution Rapid Refresh) - High-resolution CONUS forecasts
  - MRMS (Multi-Radar Multi-Sensor) - Real-time radar composites
  - GOES-16/18 - Geostationary satellite imagery
- **High Performance**: Written in Rust with two-tier caching (L1 in-memory, L2 Redis)
- **Container-Based**: Docker Compose deployment, horizontal scaling
- **Flexible Visualization**: Multiple rendering styles including gradients, contours, and wind barbs

## Architecture Overview

```
                                    ┌─────────────────────────────────────────────────────────────┐
                                    │                      Weather WMS                            │
                                    │                                                             │
   ┌──────────────┐                 │   ┌─────────────┐         ┌─────────────┐                  │
   │  Map Client  │─── tiles ──────────▶│  WMS/WMTS   │────────▶│    Redis    │                  │
   │  (Leaflet,   │                 │   │    API      │         │ (tile cache)│                  │
   │  OpenLayers) │                 │   │   :8080     │         └─────────────┘                  │
   └──────────────┘                 │   └──────┬──────┘                                          │
                                    │          │                                                  │
   ┌──────────────┐                 │          │         ┌─────────────────────────────────────┐ │
   │  Data Client │─── queries ────────▶┌─────┴─────┐   │                                     │ │
   │  (Python,    │                 │   │  EDR API  │   │         MinIO / S3                  │ │
   │  curl, etc.) │                 │   │   :8083   │──▶│       (Zarr weather data)           │ │
   └──────────────┘                 │   └───────────┘   │                                     │ │
                                    │                    └─────────────────────────────────────┘ │
                                    │                                     ▲                      │
                                    │   ┌─────────────┐       ┌──────────┴──────────┐           │
                                    │   │ PostgreSQL  │       │     Ingester        │           │
                                    │   │  (catalog)  │       │  (GRIB2/NetCDF →    │           │
                                    │   └─────────────┘       │   Zarr pyramids)    │           │
                                    │          ▲              └──────────┬──────────┘           │
                                    │          │              ┌──────────┴──────────┐           │
                                    │          └──────────────│    Downloader       │           │
                                    │                         │  (scheduled NOAA    │           │
                                    │                         │   data fetching)    │           │
                                    │                         └──────────┬──────────┘           │
                                    └─────────────────────────────────────┼───────────────────────┘
                                                                         │
                                                                         ▼
                                                              ┌─────────────────────┐
                                                              │    NOAA Sources     │
                                                              │  (AWS Open Data,    │
                                                              │   NOMADS, etc.)     │
                                                              └─────────────────────┘
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
