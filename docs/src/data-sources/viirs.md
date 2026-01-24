# VIIRS (Nighttime Lights / Light Pollution)

VIIRS (Visible Infrared Imaging Radiometer Suite) provides annual composites of nighttime lights from the Day/Night Band (DNB) sensor on the Suomi NPP satellite. This data is used for light pollution assessment and astrophotography planning via the Bortle scale.

## Overview

| Property | Value |
|----------|-------|
| **Source** | NOAA Earth Observation Group (EOG) |
| **Satellite** | Suomi NPP (VIIRS DNB) |
| **Coverage** | Global (-180 to 180 lon, -65 to 75 lat) |
| **Resolution** | ~500m (15 arcsec) |
| **Update Frequency** | Annual (static dataset) |
| **Format** | GeoTIFF (gzip compressed) |
| **File Size** | ~300-400 MB compressed |
| **EDR Collection** | `viirs-light-pollution` |

## Key Differences from Other Data Sources

Unlike GFS, HRRR, MRMS, and GOES which are automatically downloaded and ingested on a schedule, VIIRS data is:

1. **Static**: Annual composite updated once per year
2. **Manually ingested**: Requires explicit command to download and ingest
3. **No time dimension**: Always returns the same annual composite
4. **No forecast**: Pure observation data

## Parameter

| Parameter | Description | Units | Valid Range |
|-----------|-------------|-------|-------------|
| `radiance_average` | Average nighttime radiance (background removed) | nW/cm²/sr | 0-300 |

## Bortle Scale

The radiance values map to the Bortle Dark-Sky Scale, which astronomers use to rate the darkness of a location:

| Bortle Class | Description | Radiance (nW/cm²/sr) | Astrophotography Notes |
|--------------|-------------|---------------------|------------------------|
| 1 | Excellent dark site | < 0.25 | Zodiacal light, gegenschein visible. Best Milky Way photography. |
| 2 | Typical dark site | < 0.5 | Milky Way casts shadows. Excellent deep-sky imaging. |
| 3 | Rural sky | < 1.0 | Milky Way still impressive. Good for wide-field astrophotography. |
| 4 | Rural/suburban transition | < 2.0 | Milky Way visible but washed out near horizon. |
| 5 | Suburban sky | < 4.0 | Milky Way weak. Light domes visible in most directions. |
| 6 | Bright suburban | < 8.0 | Milky Way only visible at zenith. Most DSOs invisible. |
| 7 | Suburban/urban transition | < 20.0 | Milky Way invisible. Only bright planets and stars visible. |
| 8 | City sky | < 50.0 | Sky glow makes finding faint stars difficult. |
| 9 | Inner-city sky | >= 50.0 | Only Moon, planets, and brightest stars visible. |

### Astrophotography Recommendations

- **Milky Way Core**: Bortle 4 or darker (radiance < 2.0)
- **Deep Sky Objects**: Bortle 5 or darker (radiance < 4.0)
- **Star trails**: Bortle 6 or darker (radiance < 8.0)
- **Planetary imaging**: Any Bortle class (light pollution doesn't significantly affect planetary imaging)

## Data Download

VIIRS data must be manually downloaded from the Earth Observation Group:

### Download Steps

1. Visit the EOG data portal: [https://eogdata.mines.edu/nighttime_light/annual/v22/](https://eogdata.mines.edu/nighttime_light/annual/v22/)

2. Navigate to the latest year (e.g., `2024/`)

3. Download the "average_masked" or "median_masked" GeoTIFF:
   - **average_masked**: Mean radiance with background removed (recommended)
   - **median_masked**: Median radiance, more robust to outliers

4. Example filename:
   ```
   VNL_npp_2024_global_vcmslcfg_v2_c202502261200.average_masked.dat.tif.gz
   ```

5. Place the downloaded file in the project root directory

### File Naming

The filename contains important metadata:
```
VNL_npp_2024_global_vcmslcfg_v2_c202502261200.average_masked.dat.tif.gz
│   │   │    │      │        │  │              │
│   │   │    │      │        │  │              └─ Processing type
│   │   │    │      │        │  └─ Creation timestamp
│   │   │    │      │        └─ Version
│   │   │    │      └─ Product type (VCMSLCFG = stray-light corrected)
│   │   │    └─ Coverage (global)
│   │   └─ Year
│   └─ Satellite (npp = Suomi NPP)
└─ Product (VNL = VIIRS Nighttime Lights)
```

## Ingestion

### Production Deployment

Use the deploy script to ingest VIIRS data on a remote server:

```bash
# 1. Download the VNL file and place in project root
# 2. Run ingestion
./scripts/deploy-remote.sh --ingest-viirs
```

This will:
1. Transfer the file to the remote server
2. Restart the ingester with the volume mount
3. Trigger ingestion via the HTTP API
4. Register the dataset in the catalog

### Local Development

For local development, use the ingester's test-file mode:

```bash
# Start services
./scripts/start.sh

# Ingest the VIIRS file
docker exec weather-wms-ingester-1 /app/ingester \
  --test-file /path/to/VNL_*.tif.gz \
  --test-model viirs
```

Or via the HTTP API:

```bash
curl -X POST -H "Content-Type: application/json" \
  -d '{"file_path": "/data/viirs/VNL_*.tif.gz", "model": "viirs"}' \
  http://localhost:8082/ingest
```

## EDR API Access

### Standard Collection Queries

Query the `viirs-light-pollution` collection like any other EDR collection:

```bash
# Position query - get radiance at a point
curl "https://example.com/edr/collections/viirs-light-pollution/position?\
coords=POINT(-105 40)&parameter-name=radiance_average"

# Area query - get radiance for a region
curl "https://example.com/edr/collections/viirs-light-pollution/area?\
coords=POLYGON((-106 39,-104 39,-104 41,-106 41,-106 39))&\
parameter-name=radiance_average"
```

### Dedicated Light Pollution Endpoint

A convenience endpoint provides Bortle scale interpretation:

```bash
# Get light pollution with Bortle class
curl "https://example.com/edr/light-pollution?lon=-105&lat=40"
```

**Response**:
```json
{
  "type": "Feature",
  "geometry": {
    "type": "Point",
    "coordinates": [-105.0, 40.0]
  },
  "properties": {
    "radiance_nw_cm2_sr": 18.09,
    "bortle_class": 7,
    "bortle_description": "Suburban/urban transition",
    "sky_quality": "Poor",
    "milky_way_visible": false,
    "recommended_for": ["Planetary imaging", "Moon photography"]
  }
}
```

### Area Query with PNG Output

For mapping applications, request PNG output:

```bash
curl "https://example.com/edr/collections/viirs-light-pollution/area?\
coords=POLYGON((-120 30,-100 30,-100 50,-120 50,-120 30))&\
parameter-name=radiance_average&f=image/png" \
  --output light_pollution.png
```

## Configuration

### EDR Collection Config

Located at `config/edr/viirs.yaml`:

```yaml
model: viirs
data_type: observation

collections:
  - id: viirs-light-pollution
    title: "VIIRS Nighttime Lights"
    description: "Annual nighttime radiance composite for light pollution assessment"
    parameters:
      - name: radiance_average
        levels: [surface]
        valid_range: { min: 0, max: 300 }
    run_mode: latest

limits:
  max_area_sq_degrees: 100      # ~300km x 300km
  max_area_sq_degrees_png: 2500 # Full CONUS for PNG
```

### Model Config

Located at `config/models/viirs.yaml`:

```yaml
model:
  id: viirs
  name: "VIIRS Nighttime Lights"
  enabled: false  # Manual ingestion only

grid:
  projection: geographic
  resolution: "15arcsec"
  bbox:
    min_lon: -180.0
    min_lat: -65.0
    max_lon: 180.0
    max_lat: 75.0
```

## Use Cases

### Dark Sky Site Planning

Find locations with low light pollution for astronomy:

```bash
# Check multiple potential sites
for site in "POINT(-111.5 36.1)" "POINT(-116.9 36.5)" "POINT(-118.3 37.2)"; do
  echo "Checking $site..."
  curl -s "https://example.com/edr/collections/viirs-light-pollution/position?\
coords=$site&parameter-name=radiance_average" | jq '.ranges.radiance_average.values[0]'
done
```

### Light Pollution Mapping

Generate a light pollution map for a region:

```bash
# Get PNG for western US
curl "https://example.com/edr/collections/viirs-light-pollution/area?\
coords=POLYGON((-125 32,-102 32,-102 49,-125 49,-125 32))&\
parameter-name=radiance_average&f=image/png" \
  --output western_us_light_pollution.png
```

### Integration with Weather Data

Combine with cloud cover forecasts for optimal observation planning:

```bash
# Get light pollution
LP=$(curl -s "https://example.com/edr/collections/viirs-light-pollution/position?\
coords=POINT(-105 40)&parameter-name=radiance_average" | jq -r '.ranges.radiance_average.values[0]')

# Get cloud cover forecast
CLOUDS=$(curl -s "https://example.com/edr/collections/gfs-surface/position?\
coords=POINT(-105 40)&parameter-name=TCDC" | jq -r '.ranges.TCDC.values[0]')

echo "Light pollution: $LP nW/cm²/sr"
echo "Cloud cover: $CLOUDS%"
```

## Troubleshooting

### Collection Not Appearing

If the `viirs-light-pollution` collection doesn't appear after ingestion:

1. **Restart EDR API** to reload config:
   ```bash
   ./scripts/deploy-remote.sh --ssh
   docker restart weather-wms-edr-api-1
   ```

2. **Check ingestion logs**:
   ```bash
   ./scripts/deploy-remote.sh --logs ingester
   ```

3. **Verify data in database**:
   ```bash
   docker exec weather-wms-postgres-1 psql -U weatherwms -d weatherwms \
     -c "SELECT model, parameter, level FROM datasets WHERE model = 'viirs';"
   ```

### High Radiance Values

Values > 300 nW/cm²/sr may indicate:
- Fires or gas flares (transient sources)
- Industrial facilities
- Sensor artifacts

The `average_masked` product filters most of these, but some may remain.

## Data Citation

When using VIIRS nighttime lights data, please cite:

> Elvidge, C.D., Zhizhin, M., Ghosh, T., Hsu, F-C., Taneja, J. (2021). Annual Time Series of Global VIIRS Nighttime Lights Derived from Monthly Averages: 2012 to 2019. Remote Sensing, 13(5), 922.

## Next Steps

- [EDR API Reference](../api-reference/edr.md) - Full EDR endpoint documentation
- [Production Deployment](../deployment/production.md) - Deploy with VIIRS support
- [GFS](./gfs.md) - Cloud cover data for observation planning
